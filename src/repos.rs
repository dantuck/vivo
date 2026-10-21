//! Reading from restic repositories: finds the local and remote repository of
//! each task, builds the environment restic needs for them, and lists snapshots.
//! Shared by `vivo snapshots`, `vivo restore` and `vivo status`.

use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};

use serde::Deserialize;

use crate::backup_config::backup::Backup;
use crate::backup_config::task::Task;
use crate::backup_config::{decrypt_sops_file, is_placeholder_password, parse_secrets, BackupConfig};
use crate::config::Secrets;
use crate::expand_env_vars;

/// One restic repository vivo maintains for a task.
#[derive(Debug, Clone)]
pub struct RepoTarget {
    pub task: String,
    /// The task's own repository, as opposed to a mirror on a remote.
    pub is_local: bool,
    /// The repository URL as it appears in the config (`rustfs:`/`b2:` kept).
    pub config_url: String,
    /// The URL restic understands (`rustfs:` becomes `s3:`, `$HOME` expanded).
    pub restic_url: String,
    /// Credentials-profile variables, plus the password, ready for restic's environment.
    pub env: HashMap<String, String>,
}

impl RepoTarget {
    pub fn label(&self) -> String {
        if self.is_local {
            format!("{} (local)", self.task)
        } else {
            format!("{} \u{2192} {}", self.task, self.config_url)
        }
    }

    /// A `restic` command for this repository with credentials in its environment.
    pub fn restic(&self) -> Command {
        let mut cmd = Command::new("restic");
        cmd.envs(&self.env).arg("--repo").arg(&self.restic_url);
        cmd
    }

    /// False for a local repository directory that has never been initialized.
    pub fn exists(&self) -> bool {
        !self.is_local
            || !Backup::is_local_repo(&self.restic_url)
            || std::path::Path::new(&self.restic_url).join("config").exists()
    }
}

/// Loads config and decrypts secrets without any of `load_config`'s chatter.
/// Errors are phrased for the user.
pub fn load(config_path: &str, secrets_path: &str) -> Result<(BackupConfig, Secrets), String> {
    let config = BackupConfig::from_file(config_path)?;
    let yaml = decrypt_sops_file(secrets_path)
        .map_err(|e| format!("could not decrypt secrets ({secrets_path}): {}", e.trim()))?;
    let secrets = parse_secrets(&yaml)?;
    if is_placeholder_password(&secrets.restic_password) {
        return Err("restic_password is not set — run `vivo secrets edit`".to_string());
    }
    config.check_password_profiles(&secrets)?;
    Ok((config, secrets))
}

/// Tasks that own a backup: `task` itself, or — for a calls-only task — the
/// tasks it calls, recursively. With no `task`, every task that owns a backup.
pub fn backup_tasks<'a>(config: &'a BackupConfig, task: Option<&str>) -> Result<Vec<&'a Task>, String> {
    fn collect<'a>(tasks: &'a [Task], name: &str, seen: &mut HashSet<String>, out: &mut Vec<&'a Task>) {
        if !seen.insert(name.to_string()) {
            return;
        }
        let Some(t) = tasks.iter().find(|t| t.name == name) else { return };
        if t.backup().is_some() {
            out.push(t);
        }
        for call in t.call_names() {
            collect(tasks, call, seen, out);
        }
    }

    match task {
        None => Ok(config.tasks.iter().filter(|t| t.backup().is_some()).collect()),
        Some(name) => {
            if !config.tasks.iter().any(|t| t.name == name) {
                let known: Vec<&str> = config.tasks.iter().map(|t| t.name.as_str()).collect();
                return Err(format!("no task named '{name}' (tasks: {})", known.join(", ")));
            }
            let mut out = Vec::new();
            collect(&config.tasks, name, &mut HashSet::new(), &mut out);
            if out.is_empty() {
                return Err(format!("task '{name}' has no backup to read from"));
            }
            Ok(out)
        }
    }
}

/// The local repository and every remote of `task`, in config order. A remote
/// whose URL vivo doesn't understand is reported and left out.
pub fn targets_for(task: &Task, secrets: &Secrets) -> Vec<RepoTarget> {
    let Some(backup) = task.backup() else { return Vec::new() };
    let Some(password) = secrets.password_for(backup.password_profile()) else {
        eprintln!("warning: skipping task '{}': {}", task.name, crate::config::missing_profile(backup.password_profile().unwrap_or_default()));
        return Vec::new();
    };
    let with_password = |mut env: HashMap<String, String>| {
        env.insert("RESTIC_PASSWORD".to_string(), password.to_string());
        env
    };

    let mut out = vec![RepoTarget {
        task: task.name.clone(),
        is_local: true,
        config_url: backup.repo().to_string(),
        restic_url: expand_env_vars(backup.repo()),
        env: with_password(HashMap::new()),
    }];
    for (url, profile) in task.backup_remotes() {
        let backend = match crate::remote::from_url(&expand_env_vars(url)) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("warning: skipping remote {url}: {e}");
                continue;
            }
        };
        let creds = secrets.credentials.get(profile).cloned().unwrap_or_default();
        out.push(RepoTarget {
            task: task.name.clone(),
            is_local: false,
            config_url: url.to_string(),
            restic_url: backend.restic_url(),
            env: with_password(backend.restic_env(&creds)),
        });
    }
    out
}

/// Picks one of a task's repositories without prompting. `from` is `local`
/// (the task's own repository, whether or not it exists yet) or a configured
/// remote URL. With no `from`, the local repository if it exists, else an error
/// that lists the remotes to choose from.
pub fn find_source(targets: &[RepoTarget], from: Option<&str>) -> Result<RepoTarget, String> {
    let remotes = || targets.iter().filter(|t| !t.is_local).map(|t| t.config_url.as_str()).collect::<Vec<_>>();
    match from {
        Some("local") => targets.iter().find(|t| t.is_local).cloned().ok_or_else(|| "this task has no local repository".to_string()),
        Some(url) => targets
            .iter()
            .find(|t| !t.is_local && (t.config_url == url || t.restic_url == url))
            .cloned()
            .ok_or_else(|| format!("'{url}' is not one of this task's repositories (use 'local' or a configured remote URL)")),
        None => targets.iter().find(|t| t.is_local).filter(|t| t.exists()).cloned().ok_or_else(|| {
            let remotes = remotes();
            if remotes.is_empty() {
                "there is no local repository yet and no remote is configured".to_string()
            } else {
                format!("there is no local repository here; pass --from <remote URL> ({})", remotes.join(", "))
            }
        }),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    pub short_id: String,
    /// RFC 3339.
    pub time: String,
    #[serde(default)]
    pub paths: Vec<String>,
}

impl Snapshot {
    pub fn unix_time(&self) -> Option<i64> {
        chrono::DateTime::parse_from_rfc3339(&self.time).ok().map(|t| t.timestamp())
    }

    /// `2026-10-16 02:00` in the local time zone.
    pub fn local_time(&self) -> String {
        chrono::DateTime::parse_from_rfc3339(&self.time)
            .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|_| self.time.clone())
    }
}

/// Snapshots in `target`, oldest first. `latest` limits it to the newest one.
pub fn list_snapshots(target: &RepoTarget, latest: bool) -> Result<Vec<Snapshot>, String> {
    let mut cmd = target.restic();
    cmd.args(["snapshots", "--json", "--no-lock"]);
    if latest {
        cmd.args(["--latest", "1"]);
    }
    let output = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run restic: {e}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    parse_snapshots(&output.stdout)
}

/// Lists the snapshots of every target at once (each `restic snapshots` pays
/// for key derivation, and for remotes a network round trip). Results are in
/// the order of `targets`; a repository that doesn't exist yet gives `None`.
pub fn list_all(targets: &[RepoTarget], latest: bool) -> Vec<Option<Result<Vec<Snapshot>, String>>> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = targets
            .iter()
            .map(|t| scope.spawn(move || t.exists().then(|| list_snapshots(t, latest))))
            .collect();
        handles.into_iter().map(|h| h.join().expect("snapshot listing panicked")).collect()
    })
}

pub fn parse_snapshots(json: &[u8]) -> Result<Vec<Snapshot>, String> {
    // restic prints `null` rather than `[]` for an empty repository.
    let parsed: Option<Vec<Snapshot>> =
        serde_json::from_slice(json).map_err(|e| format!("unexpected restic output: {e}"))?;
    let mut snapshots = parsed.unwrap_or_default();
    snapshots.sort_by_cached_key(|s| s.unix_time().unwrap_or(0));
    Ok(snapshots)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KDL: &str = r#"default-task "all"
tasks {
    task "docs" { backup { repo "/r/docs"
directory "/d"
remote "rustfs:https://h/bucket" { credentials "rustfs" } } }
    task "all" { calls "docs" }
}"#;

    fn secrets() -> Secrets {
        parse_secrets(
            "restic_password: pw\ncredentials:\n  rustfs:\n    AWS_ACCESS_KEY_ID: k\n  b2:\n    B2_APPLICATION_KEY_ID: id\n    B2_APPLICATION_KEY: key\n",
        )
        .unwrap()
    }

    #[test]
    fn calls_only_task_expands_to_called_backups() {
        let config = BackupConfig::parse("t.kdl", KDL).unwrap();
        let tasks = backup_tasks(&config, Some("all")).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].name, "docs");
        assert!(backup_tasks(&config, Some("nope")).is_err());
    }

    #[test]
    fn targets_translate_rustfs_and_carry_credentials() {
        let config = BackupConfig::parse("t.kdl", KDL).unwrap();
        let targets = targets_for(&config.tasks[0], &secrets());
        assert_eq!(targets.len(), 2);
        assert!(targets[0].is_local);
        assert_eq!(targets[0].restic_url, "/r/docs");
        assert_eq!(targets[1].restic_url, "s3:https://h/bucket");
        assert_eq!(targets[1].env["AWS_ACCESS_KEY_ID"], "k");
        assert_eq!(targets[1].env["RESTIC_PASSWORD"], "pw");
    }

    #[test]
    fn parses_snapshots_sorted_and_tolerates_null() {
        let json = br#"[{"short_id":"b","time":"2026-10-02T02:00:00+00:00","paths":["/d"]},
                        {"short_id":"a","time":"2026-10-01T02:00:00Z","paths":["/d"]}]"#;
        let s = parse_snapshots(json).unwrap();
        assert_eq!(s[0].short_id, "a");
        assert_eq!(s[1].short_id, "b");
        assert!(parse_snapshots(b"null").unwrap().is_empty());
    }

    #[test]
    fn find_source_picks_local_or_a_named_remote() {
        let config = BackupConfig::parse("t.kdl", KDL).unwrap();
        let targets = targets_for(&config.tasks[0], &secrets());

        // an explicit `local` is returned even before the repository exists
        assert!(find_source(&targets, Some("local")).unwrap().is_local);
        // remotes match by config URL or by the URL restic uses
        assert_eq!(find_source(&targets, Some("rustfs:https://h/bucket")).unwrap().restic_url, "s3:https://h/bucket");
        assert!(find_source(&targets, Some("s3:https://h/bucket")).is_ok());
        assert!(find_source(&targets, Some("b2:nope")).unwrap_err().contains("not one of this task's repositories"));

        // with no `from`, a local repository that doesn't exist is not silently picked
        let err = find_source(&targets, None).unwrap_err();
        assert!(err.contains("no local repository") && err.contains("rustfs:https://h/bucket"), "{err}");
    }

    #[test]
    fn find_source_defaults_to_an_existing_local_repository() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config"), "").unwrap();
        let kdl = format!(
            "default-task \"d\"\ntasks {{ task \"d\" {{ backup {{ repo \"{}\"\ndirectory \"/d\" }} }} }}",
            dir.path().display()
        );
        let config = BackupConfig::parse("t.kdl", &kdl).unwrap();
        let targets = targets_for(&config.tasks[0], &secrets());
        assert!(find_source(&targets, None).unwrap().is_local);
    }

    #[test]
    fn targets_use_the_tasks_password_profile() {
        let kdl = "default-task \"p\"\ntasks {\n task \"p\" { backup { repo \"/r/p\"\ndirectory \"/d\"\npassword-profile \"photos\"\nremote \"rustfs:https://h/p\" { credentials \"rustfs\" } } }\n task \"d\" { backup { repo \"/r/d\"\ndirectory \"/d\" } }\n}";
        let config = BackupConfig::parse("t.kdl", kdl).unwrap();
        let s = parse_secrets("restic_password: global\npasswords:\n  photos: pw-photos\ncredentials:\n  rustfs:\n    AWS_ACCESS_KEY_ID: k\n").unwrap();

        // the local repo and its remote copy both use the profile's password
        let photos = targets_for(&config.tasks[0], &s);
        assert_eq!(photos.len(), 2);
        assert!(photos.iter().all(|t| t.env["RESTIC_PASSWORD"] == "pw-photos"));
        // a task without a profile keeps the global one
        assert_eq!(targets_for(&config.tasks[1], &s)[0].env["RESTIC_PASSWORD"], "global");

        // a profile that is not in the secrets yields no targets rather than the wrong password
        let no_profile = parse_secrets("restic_password: global\n").unwrap();
        assert!(targets_for(&config.tasks[0], &no_profile).is_empty());
    }
}
