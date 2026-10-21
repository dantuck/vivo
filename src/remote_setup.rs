//! Guided remote setup and testing, shared by `vivo init`, `vivo remote`,
//! and the TUI so they all behave the same way.

use std::collections::HashMap;
use std::process::Command;

use inquire::validator::Validation;
use inquire::InquireError;

use crate::backup_config::{decrypt_sops_file, parse_secrets};
use crate::config::Secrets;
use crate::doctor::{self, CheckStatus};
use crate::remote::RustfsBackend;

const RUSTFS: &str = "RustFS / S3-compatible, synced with mc, aws or rclone  (rustfs:)";
const S3: &str = "S3-compatible, synced with restic copy  (s3:)";
const B2: &str = "Backblaze B2  (b2:)";

pub struct NewRemote {
    pub url: String,
    pub credentials: String,
}

pub(crate) fn repo_needs_init(output: &str) -> bool {
    output.contains("Is there a repository")
}

/// Checks a remote URL has a supported prefix and parses.
pub fn validate_url(url: &str) -> Result<(), String> {
    crate::remote::from_url(url).map(|_| ())
}

/// Maps a cancelled prompt (Esc / Ctrl-C) to `None` and any other prompt error to `Err`.
fn cancelled<T>(r: Result<T, InquireError>) -> Result<Option<T>, String> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(InquireError::OperationCanceled | InquireError::OperationInterrupted) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// A yes/no prompt that answers "no" when not interactive.
fn confirm(message: &str, interactive: bool) -> bool {
    interactive
        && inquire::Confirm::new(message)
            .with_default(true)
            .prompt()
            .unwrap_or(false)
}

/// Decrypts and parses the secrets file.
pub fn load_secrets(secrets_path: &str) -> Result<Secrets, String> {
    parse_secrets(&decrypt_sops_file(secrets_path)?)
}

/// Walks the user through picking a backend, entering a validated URL, and
/// choosing or creating a credentials profile. Returns `None` if cancelled.
pub fn prompt_new_remote(secrets_path: &str) -> Result<Option<NewRemote>, String> {
    let Some(kind) = cancelled(inquire::Select::new("Remote type:", vec![RUSTFS, S3, B2]).prompt())?
    else {
        return Ok(None);
    };
    let (prefix, help) = match kind {
        RUSTFS => (
            "rustfs:https://",
            "rustfs:https://host/bucket[/path] — the bucket is created on first sync",
        ),
        S3 => (
            "s3:https://",
            "s3:https://host/bucket — the restic repo must be initialized first",
        ),
        _ => ("b2:", "b2:bucket:path"),
    };

    let Some(url) = cancelled(
        inquire::Text::new("Remote URL:")
            .with_initial_value(prefix)
            .with_help_message(help)
            .with_validator(|input: &str| {
                Ok(match validate_url(input.trim()) {
                    Ok(()) => Validation::Valid,
                    Err(e) => Validation::Invalid(e.into()),
                })
            })
            .prompt(),
    )?
    else {
        return Ok(None);
    };
    let url = url.trim().to_string();

    let Some(credentials) = crate::tui::credentials::select_or_create_profile(&url, secrets_path)?
    else {
        return Ok(None);
    };
    Ok(Some(NewRemote { url, credentials }))
}

/// Like [`test_remote_with_secrets`], decrypting the secrets file first.
/// The `password-profile` of `task`'s backup, if it has one (and the config
/// can be read). Remote tests need it: a task with its own repository password
/// must be tested, and any new remote initialized, with that password.
pub fn password_profile_of(config_path: &str, task: &str) -> Option<String> {
    let config = crate::BackupConfig::from_file(config_path).ok()?;
    config.tasks.iter().find(|t| t.name == task)?.password_profile().map(str::to_string)
}

pub fn test_remote(
    url: &str,
    profile: &str,
    secrets_path: &str,
    password_profile: Option<&str>,
    interactive: bool,
) -> Result<String, String> {
    validate_url(url)?;
    let secrets = load_secrets(secrets_path)?.for_profile(password_profile)?;
    test_remote_with_secrets(url, profile, &secrets, interactive)
}

/// Tests a remote with its credentials profile. With `interactive`, offers to
/// fix what it can: create a missing RustFS bucket, or `restic init` a missing
/// S3 repository. Prints details as it goes; the returned string is a one-line
/// summary, and `Err` means the remote is not usable yet.
pub fn test_remote_with_secrets(
    url: &str,
    profile: &str,
    secrets: &Secrets,
    interactive: bool,
) -> Result<String, String> {
    validate_url(url)?;
    let creds = secrets.credentials.get(profile).ok_or_else(|| {
        format!(
            "credentials profile '{profile}' not found in secrets — \
             add it with `vivo secrets import-s3 --profile {profile}` or `vivo secrets edit`"
        )
    })?;

    if url.starts_with("rustfs:") {
        test_rustfs(url, creds, interactive)
    } else if url.starts_with("s3:") {
        test_s3(url, creds, &secrets.restic_password, interactive)
    } else {
        let r = doctor::check_remote_connectivity(
            url,
            profile,
            &secrets.credentials,
            &secrets.restic_password,
        );
        match r.status {
            CheckStatus::Ok => Ok(format!("{url} is reachable.")),
            _ => Err(r.detail.unwrap_or_else(|| format!("{url} is not reachable"))),
        }
    }
}

fn test_rustfs(url: &str, creds: &HashMap<String, String>, interactive: bool) -> Result<String, String> {
    let backend = RustfsBackend::from_url(url)?;
    if backend.bucket_reachable(creds)? {
        return Ok(format!("Bucket '{}' is reachable at {}.", backend.bucket, backend.endpoint));
    }

    // A failed listing is most often a bucket that doesn't exist yet (vivo
    // creates it on first sync), but can also be bad keys or an unreachable
    // host. Creating the bucket tells them apart: it surfaces the tool's error.
    if !confirm(
        &format!(
            "Could not list bucket '{}' at {}. Try to create it?",
            backend.bucket, backend.endpoint
        ),
        interactive,
    ) {
        return Err(format!(
            "could not list bucket '{}' at {} — it may not exist yet (created on first sync), \
             or the credentials/endpoint are wrong",
            backend.bucket, backend.endpoint
        ));
    }

    backend.ensure_bucket(creds)?;
    Ok(format!("Bucket '{}' is ready at {}.", backend.bucket, backend.endpoint))
}

fn test_s3(
    url: &str,
    creds: &HashMap<String, String>,
    restic_password: &str,
    interactive: bool,
) -> Result<String, String> {
    let restic = |args: &[&str]| {
        Command::new("restic")
            .arg("-r")
            .arg(url)
            .args(args)
            .envs(creds)
            .env("RESTIC_PASSWORD", restic_password)
            .output()
            .map_err(|e| format!("could not run restic: {e}"))
    };

    let out = restic(&["snapshots", "--no-lock", "--no-cache"])?;
    if out.status.success() {
        return Ok(format!("Repository at {url} is reachable."));
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !repo_needs_init(&combined) {
        return Err(format!("restic could not open {url}:\n{}", combined.trim()));
    }
    if !confirm(&format!("No repository found at '{url}'. Initialize it now?"), interactive) {
        return Err(format!(
            "no restic repository at {url} — initialize it with: restic init -r {url}"
        ));
    }

    let out = restic(&["init"])?;
    if out.status.success() {
        Ok(format!("Repository initialized at {url}."))
    } else {
        Err(format!("restic init failed:\n{}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Runs the connection test after a remote was saved, printing the outcome.
/// A failure is only a warning: the remote stays in the config.
pub fn test_after_save(url: &str, profile: &str, secrets_path: &str, password_profile: Option<&str>) {
    println!("\nTesting remote...");
    match test_remote(url, profile, secrets_path, password_profile, true) {
        Ok(msg) => println!("{msg}"),
        Err(e) => eprintln!(
            "warning: {e}\nThe remote was saved. Re-test any time with `vivo remote test`."
        ),
    }
}

/// Guided "add a remote": picks the task, prompts for the remote and its
/// credentials, writes the config, then tests the remote. A failed test is
/// reported but does not undo the add. Returns `Ok(None)` if cancelled.
pub fn guided_add(
    config_path: &str,
    secrets_path: &str,
    task: Option<&str>,
) -> Result<Option<String>, String> {
    let kdl = std::fs::read_to_string(config_path)
        .map_err(|e| format!("could not read config '{config_path}': {e}"))?;

    let task = match task {
        Some(t) => t.to_string(),
        None => {
            let mut names = crate::config_editor::backup_task_names(&kdl)?;
            match names.len() {
                0 => return Err("no task with a backup block — add one with `vivo task add`".into()),
                1 => names.remove(0),
                _ => match cancelled(inquire::Select::new("Add remote to task:", names).prompt())? {
                    Some(t) => t,
                    None => return Ok(None),
                },
            }
        }
    };

    let Some(remote) = prompt_new_remote(secrets_path)? else {
        return Ok(None);
    };

    let new_kdl = crate::config_editor::add_remote(
        &kdl,
        &task,
        crate::config_editor::RemoteSpec {
            url: remote.url.clone(),
            credentials: remote.credentials.clone(),
            mc_max_workers: None,
            mc_limit_upload: None,
        },
    )?;
    std::fs::write(config_path, new_kdl).map_err(|e| e.to_string())?;
    println!("Added remote '{}' to task '{task}'.", remote.url);

    test_after_save(&remote.url, &remote.credentials, secrets_path, password_profile_of(config_path, &task).as_deref());
    Ok(Some(remote.url))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_needs_init_detects_missing_repo() {
        let output = "Fatal: unable to open config file: Stat: The specified key does not exist.\nIs there a repository at the following location?\ns3:https://example.com/bucket\n";
        assert!(repo_needs_init(output));
    }

    #[test]
    fn repo_needs_init_false_for_other_errors() {
        assert!(!repo_needs_init("Fatal: unable to open config file: Forbidden\n"));
    }

    #[test]
    fn repo_needs_init_false_for_success() {
        assert!(!repo_needs_init("snapshot abc123 ...\n"));
    }

    #[test]
    fn test_remote_rejects_bad_url_before_touching_secrets() {
        let err = test_remote("rustfs:https://host", "p", "/nonexistent/secrets.yaml", None, false)
            .unwrap_err();
        assert!(err.contains("missing bucket"), "{err}");
    }
}
