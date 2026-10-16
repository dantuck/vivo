use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;

use regex::Regex;
use std::env;

use kdl::KdlNode;

use crate::backup_config::kdl_decode::{named, ConfigError, Source};
use crate::config::VivoConfig;
use crate::step::Step;
use crate::ui;

#[derive(Debug)]
pub struct Remote {
    pub url: String,
    pub credentials: String,
    pub mc_max_workers: Option<u32>,
    pub mc_limit_upload: Option<String>,
    /// Keep an off-site recovery copy of the config and secrets next to this
    /// remote's repository (`recovery false` opts out).
    pub recovery: bool,
}

impl Remote {
    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(children, &["credentials", "mc-max-workers", "mc-limit-upload", "recovery"])?;
        Ok(Remote {
            url: src.argument(node)?,
            credentials: src.string_value(src.required(node, children, "credentials")?)?,
            mc_max_workers: src
                .optional(children, "mc-max-workers")?
                .map(|n| src.int_value(n))
                .transpose()?,
            mc_limit_upload: src
                .optional(children, "mc-limit-upload")?
                .map(|n| src.string_value(n))
                .transpose()?,
            recovery: src
                .optional(children, "recovery")?
                .map(|n| src.bool_value(n))
                .transpose()?
                .unwrap_or(true),
        })
    }
}

#[derive(Debug)]
pub struct Backup {
    repo: String,
    exclude_file: Option<String>,
    directory: Option<String>,
    files_from: Option<String>,
    remotes: Vec<Remote>,
    retention: BackupRetention,
    dry_run: Option<bool>,
}

impl Backup {
    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(
            children,
            &["repo", "exclude-file", "directory", "files-from", "remote", "retention", "dry-run"],
        )?;
        let string = |name: &str| -> Result<Option<String>, ConfigError> {
            src.optional(children, name)?.map(|n| src.string_value(n)).transpose()
        };
        Ok(Backup {
            repo: src.string_value(src.required(node, children, "repo")?)?,
            exclude_file: string("exclude-file")?,
            directory: string("directory")?,
            files_from: string("files-from")?,
            remotes: named(children, "remote")
                .map(|n| Remote::decode(src, n))
                .collect::<Result<_, _>>()?,
            retention: src
                .optional(children, "retention")?
                .map(|n| BackupRetention::decode(src, n))
                .transpose()?
                .unwrap_or_default(),
            dry_run: src
                .optional(children, "dry-run")?
                .map(|n| src.bool_value(n))
                .transpose()?,
        })
    }
}

/// How many snapshots `restic forget` keeps per period.
#[derive(Debug)]
struct BackupRetention {
    daily: i16,
    weekly: i16,
    monthly: i16,
    yearly: i16,
}

impl Default for BackupRetention {
    fn default() -> Self {
        BackupRetention { daily: 7, weekly: 5, monthly: 12, yearly: 2 }
    }
}

impl BackupRetention {
    fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(children, &["daily", "weekly", "monthly", "yearly"])?;
        let defaults = BackupRetention::default();
        let count = |name: &str, default: i16| -> Result<i16, ConfigError> {
            Ok(src.optional(children, name)?.map(|n| src.int_value(n)).transpose()?.unwrap_or(default))
        };
        Ok(BackupRetention {
            daily: count("daily", defaults.daily)?,
            weekly: count("weekly", defaults.weekly)?,
            monthly: count("monthly", defaults.monthly)?,
            yearly: count("yearly", defaults.yearly)?,
        })
    }
}

static ENV_VAR_RE: OnceLock<Regex> = OnceLock::new();

// restic/std::process never run through a shell, so a literal leading `~` is
// never expanded on its own; without this it resolves relative to cwd instead
// of $HOME.
pub fn expand_env_vars(path: &str) -> String {
    let re = ENV_VAR_RE.get_or_init(|| Regex::new(r"\$([A-Z_][A-Z0-9_]*)").unwrap());
    let expanded = re.replace_all(path, |caps: &regex::Captures| {
        env::var(&caps[1]).unwrap_or_else(|_| caps[0].to_string())
    });

    match expanded.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            match env::var("HOME") {
                Ok(home) => format!("{home}{rest}"),
                Err(_) => expanded.to_string(),
            }
        }
        _ => expanded.to_string(),
    }
}

/// Creates the directory `path` (after env-var expansion), ignoring failures.
pub fn ensure_path(path: &str) {
    let expanded = expand_env_vars(path);
    if !expanded.is_empty() {
        let _ = fs::create_dir_all(&expanded);
    }
}

/// restic exits 3 when it saved a snapshot but could not read some source
/// files (permission-protected folders, files changing mid-backup). The
/// snapshot is valid, so that is a warning, not a failed backup.
const RESTIC_PARTIAL_BACKUP: i32 = 3;

fn run_status(command_name: &str, args: &[String]) -> Result<ExitStatus, String> {
    Command::new(command_name)
        .args(args)
        .status()
        .map_err(|e| format!("failed to run {command_name}: {e}"))
}

fn execute_command(command_name: &str, args: Vec<String>) -> Result<(), String> {
    let status = run_status(command_name, &args)?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{command_name} failed with status {status}"))
    }
}

/// Syncs one remote. A missing B2 credentials profile triggers authorization;
/// any other missing profile is an error. New B2 credentials go into `creds_cache`.
fn sync_remote(
    remote: &Remote,
    local_repo: &str,
    dry_run: bool,
    creds_cache: &mut HashMap<String, HashMap<String, String>>,
) -> Result<(), String> {
    if !creds_cache.contains_key(&remote.credentials) {
        if !remote.url.starts_with("b2:") {
            return Err(format!(
                "credentials profile '{}' not found in secrets — skipping remote {}",
                remote.credentials, remote.url
            ));
        }
        eprintln!("\n[!] B2 credentials not found — starting authorization...\n");
        let new_creds = crate::import_b2_credentials(&crate::config::secrets_path_from())
            .map_err(|e| format!("re-authorization failed: {e} — skipping remote {}", remote.url))?;
        creds_cache.insert(remote.credentials.clone(), new_creds);
    }

    let backend = crate::remote::from_remote(remote)?;
    backend.check_installed()?;

    ui::info(&format!("Syncing to {}", remote.url));
    backend
        .sync(local_repo, dry_run, &creds_cache[&remote.credentials])
        .map_err(|e| format!("sync to {} failed: {e}", remote.url))
}

/// How syncing one remote went; `error` is `None` on success.
#[derive(Debug, Clone)]
pub struct SyncResult {
    pub url: String,
    pub error: Option<String>,
    /// What happened to the remote's recovery copy; `None` if the sync failed
    /// and so it wasn't attempted.
    pub recovery: Option<crate::recovery::Outcome>,
}

/// What a [`Backup::run`] actually did.
#[derive(Debug, Default)]
pub struct RunOutcome {
    pub dry_run: bool,
    pub syncs: Vec<SyncResult>,
}

impl RunOutcome {
    pub fn failed_syncs(&self) -> usize {
        self.syncs.iter().filter(|s| s.error.is_some()).count()
    }
}

impl Backup {
    pub(crate) fn remotes(&self) -> &[Remote] {
        &self.remotes
    }

    /// The task's own `dry-run true` override.
    pub fn dry_run(&self) -> bool {
        self.dry_run.unwrap_or(false)
    }

    pub fn repo(&self) -> &str {
        &self.repo
    }

    pub fn directory(&self) -> Option<&str> {
        self.directory.as_deref()
    }

    pub fn exclude_file(&self) -> Option<&str> {
        self.exclude_file.as_deref()
    }

    pub fn files_from(&self) -> Option<&str> {
        self.files_from.as_deref()
    }

    fn backup(&self, dry_run: bool) -> Result<(), String> {
        let mut args = vec![
            "backup".to_string(),
            "-r".to_string(),
            expand_env_vars(&self.repo),
        ];

        if let Some(files_from) = &self.files_from {
            args.extend(["--files-from".to_string(), expand_env_vars(files_from)]);
        } else if let Some(directory) = &self.directory {
            args.push(expand_env_vars(directory));
        }

        if let Some(exclude_file) = &self.exclude_file {
            args.extend(["--exclude-file".to_string(), expand_env_vars(exclude_file)]);
        }

        if dry_run {
            args.push("--dry-run".to_string());
        }

        let status = run_status("restic", &args)?;
        match status.code() {
            Some(0) => Ok(()),
            Some(RESTIC_PARTIAL_BACKUP) => {
                eprintln!(
                    "warning: some files could not be read and were skipped (restic exit code 3); \
                     the snapshot was still saved, so continuing. On macOS, grant your terminal \
                     (or the scheduler) Full Disk Access to include protected folders."
                );
                Ok(())
            }
            _ => Err(format!("restic failed with status {status}")),
        }
    }

    fn check(&self) -> Result<(), String> {
        execute_command(
            "restic",
            vec!["check".to_string(), "-r".to_string(), expand_env_vars(&self.repo)],
        )
    }

    fn forget(&self, dry_run: bool) -> Result<(), String> {
        let r = &self.retention;
        let mut args: Vec<String> = ["forget", "-r", &expand_env_vars(&self.repo)]
            .into_iter()
            .map(String::from)
            .collect();
        for (flag, keep) in [
            ("--keep-daily", r.daily),
            ("--keep-weekly", r.weekly),
            ("--keep-monthly", r.monthly),
            ("--keep-yearly", r.yearly),
        ] {
            args.extend([flag.to_string(), keep.to_string()]);
        }
        args.push("--prune".to_string());

        if dry_run {
            args.push("--dry-run".to_string());
        }

        execute_command("restic", args)
    }

    /// Syncs every remote, continuing past failures. Unless this is a dry run,
    /// each successful sync is followed by bringing the remote's recovery copy
    /// up to date (see [`crate::recovery`]); a problem there is reported but
    /// never fails the sync.
    fn sync_remotes(
        &self,
        task_name: &str,
        config: &VivoConfig,
        dry_run: bool,
        credentials: &HashMap<String, HashMap<String, String>>,
    ) -> Vec<SyncResult> {
        use crate::recovery::Outcome;

        let local_repo = expand_env_vars(&self.repo);
        let mut creds_cache = credentials.clone();
        let state = crate::state::State::load();
        let mut source = crate::recovery::Source::new(&config.get_config_path(), &config.get_secrets_path());

        self.remotes
            .iter()
            .map(|remote| {
                let error = sync_remote(remote, &local_repo, dry_run, &mut creds_cache).err();
                if let Some(e) = &error {
                    eprintln!("error: {e}");
                }
                if error.is_some() || dry_run {
                    return SyncResult { url: remote.url.clone(), error, recovery: None };
                }

                let previous = state
                    .tasks
                    .get(task_name)
                    .and_then(|t| t.remotes.get(&remote.url))
                    .and_then(|r| r.recovery.as_ref()?.hash());
                let outcome =
                    crate::recovery::sync_to_remote(remote, &creds_cache[&remote.credentials], &mut source, previous);
                match &outcome {
                    Outcome::Uploaded(_) => ui::info(&format!("Updated the recovery copy next to {}", remote.url)),
                    Outcome::Skipped(why) => eprintln!("warning: no recovery copy for {}: {why}", remote.url),
                    Outcome::Failed(why) => {
                        eprintln!("warning: could not update the recovery copy for {}: {why}", remote.url)
                    }
                    Outcome::Unchanged | Outcome::Disabled => {}
                }
                SyncResult { url: remote.url.clone(), error: None, recovery: Some(outcome) }
            })
            .collect()
    }

    pub(crate) fn is_local_repo(repo: &str) -> bool {
        !repo.contains(':')
            || repo.starts_with('/')
            || repo.starts_with('.')
            || repo.starts_with('~')
    }

    /// Path of the local repository if it doesn't exist yet.
    fn missing_local_repo(&self) -> Option<String> {
        let repo = expand_env_vars(&self.repo);
        (Self::is_local_repo(&repo) && !Path::new(&repo).join("config").exists()).then_some(repo)
    }

    fn ensure_local_repo_init(&self) -> Result<(), String> {
        if let Some(repo) = self.missing_local_repo() {
            fs::create_dir_all(&repo)
                .map_err(|e| format!("could not create repo directory '{repo}': {e}"))?;
            execute_command("restic", vec!["init".to_string(), "-r".to_string(), repo])?;
        }
        Ok(())
    }

    /// Runs the backup phases. `Ok` carries what happened (including any
    /// remotes that failed to sync); `Err` means a phase before syncing failed.
    pub fn run(
        &self,
        config: &VivoConfig,
        task_name: &str,
        credentials: &HashMap<String, HashMap<String, String>>,
    ) -> Result<RunOutcome, String> {
        let dry_run = config.dry_run || self.dry_run();
        let mut outcome = RunOutcome { dry_run, ..RunOutcome::default() };

        if config.start_step <= Step::Backup {
            if dry_run {
                // A dry run must not create anything, and every later phase needs a repo.
                if let Some(repo) = self.missing_local_repo() {
                    println!("[dry-run] restic repository {repo} does not exist yet.");
                    println!("[dry-run] a real run would initialize it, then back up {}.",
                        self.directory.as_deref().unwrap_or("the configured files"));
                    return Ok(outcome);
                }
            } else {
                self.ensure_local_repo_init()?;
            }
            self.backup(dry_run)?;
        }
        if config.start_step <= Step::Check {
            self.check()?;
        }
        if config.start_step <= Step::Forget {
            self.forget(dry_run)?;
        }
        if config.start_step <= Step::Sync {
            outcome.syncs = self.sync_remotes(task_name, config, dry_run, credentials);
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::expand_env_vars;
    use crate::backup_config::BackupConfig;

    /// Runs `Backup::backup` against a fake `restic` that exits with `code`.
    #[cfg(unix)]
    fn backup_with_fake_restic(code: i32) -> Result<(), String> {
        use std::os::unix::fs::PermissionsExt;

        let _path_guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("restic");
        std::fs::write(&fake, format!("#!/bin/sh\nexit {code}\n")).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let config = BackupConfig::parse(
            "test.kdl",
            "default-task \"t\"\ntasks { task \"t\" { backup { repo \"/tmp/r\"\ndirectory \"/tmp/d\" } } }",
        )
        .unwrap();
        let backup = config.tasks[0].backup().unwrap();

        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{original}", dir.path().display()));
        let result = backup.backup(false);
        std::env::set_var("PATH", original);
        result
    }

    #[cfg(unix)]
    #[test]
    fn backup_treats_restic_exit_3_as_warning() {
        assert!(backup_with_fake_restic(3).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn backup_still_fails_on_other_restic_errors() {
        assert!(backup_with_fake_restic(1).is_err());
        assert!(backup_with_fake_restic(0).is_ok());
    }

    #[test]
    fn expand_env_vars_expands_leading_tilde() {
        let home = std::env::var("HOME").unwrap();
        assert_eq!(expand_env_vars("~/.restic/repo/sync"), format!("{home}/.restic/repo/sync"));
        assert_eq!(expand_env_vars("~"), home);
    }

    #[test]
    fn expand_env_vars_leaves_non_home_tilde_alone() {
        assert_eq!(expand_env_vars("~backup/.restic"), "~backup/.restic");
    }

    #[test]
    fn remote_parses_mc_max_workers() {
        let src = r#"
default-task "backup"
tasks {
    task "t" {
        backup {
            repo "/tmp/repo"
            remote "rustfs:http://nas/b" {
                credentials "aws"
                mc-max-workers 2
            }
        }
    }
}
"#;
        let cfg = BackupConfig::parse("test", src).unwrap();
        let remote = &cfg.tasks[0].backup().unwrap().remotes[0];
        assert_eq!(remote.mc_max_workers, Some(2));
        assert_eq!(remote.mc_limit_upload, None);
    }

    #[test]
    fn remote_parses_mc_limit_upload() {
        let src = r#"
default-task "backup"
tasks {
    task "t" {
        backup {
            repo "/tmp/repo"
            remote "rustfs:http://nas/b" {
                credentials "aws"
                mc-limit-upload "5MiB"
            }
        }
    }
}
"#;
        let cfg = BackupConfig::parse("test", src).unwrap();
        let remote = &cfg.tasks[0].backup().unwrap().remotes[0];
        assert_eq!(remote.mc_max_workers, None);
        assert_eq!(remote.mc_limit_upload.as_deref(), Some("5MiB"));
    }

    #[test]
    fn remote_parses_both_mc_options() {
        let src = r#"
default-task "backup"
tasks {
    task "t" {
        backup {
            repo "/tmp/repo"
            remote "rustfs:http://nas/b" {
                credentials "aws"
                mc-max-workers 1
                mc-limit-upload "10MiB"
            }
        }
    }
}
"#;
        let cfg = BackupConfig::parse("test", src).unwrap();
        let remote = &cfg.tasks[0].backup().unwrap().remotes[0];
        assert_eq!(remote.mc_max_workers, Some(1));
        assert_eq!(remote.mc_limit_upload.as_deref(), Some("10MiB"));
    }
}
