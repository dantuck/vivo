use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::{Command, ExitStatus};
use std::sync::OnceLock;
use std::time::Instant;

use regex::Regex;
use std::env;

use kdl::KdlNode;

use crate::backup_config::kdl_decode::{named, ConfigError, Source};
use crate::backup_config::steps::Steps;
use crate::config::{Secrets, VivoConfig};
use crate::step::Step;
use crate::ui;

#[derive(Debug)]
pub struct Remote {
    pub url: String,
    pub credentials: String,
    pub mc_max_workers: Option<u32>,
    pub mc_limit_upload: Option<String>,
    /// Upload speed cap, as written (`20MiB`); for `s3:` and `rustfs:` remotes.
    pub limit_upload: Option<String>,
    /// Keep an off-site recovery copy of the config and secrets next to this
    /// remote's repository (`recovery false` opts out).
    pub recovery: bool,
}

impl Remote {
    /// The upload cap in KiB/s, if set (already validated when decoded).
    pub fn limit_upload_kib(&self) -> Option<u64> {
        self.limit_upload.as_deref().and_then(|t| crate::remote::parse_rate_kib(t).ok())
    }

    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(children, &["credentials", "mc-max-workers", "mc-limit-upload", "limit-upload", "recovery"])?;
        let url = src.argument(node)?;
        let limit_upload = match src.optional(children, "limit-upload")? {
            None => None,
            Some(n) => {
                let text = src.string_value(n)?;
                if url.starts_with("b2:") {
                    return Err(src.error(n, "b2 remotes cannot limit upload speed: the b2 CLI has no bandwidth option"));
                }
                crate::remote::parse_rate_kib(&text).map_err(|e| src.error(n, e))?;
                if src.optional(children, "mc-limit-upload")?.is_some() {
                    return Err(src.error(n, "use either 'limit-upload' or 'mc-limit-upload', not both"));
                }
                Some(text)
            }
        };
        Ok(Remote {
            url,
            limit_upload,
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
    /// Extra paths to back up, alongside `directory` (`include` nodes).
    include: Vec<String>,
    /// Patterns passed to `restic backup --exclude` (`exclude` nodes).
    exclude: Vec<String>,
    remotes: Vec<Remote>,
    retention: BackupRetention,
    steps: Steps,
    /// Key into the secrets' `passwords`; `None` uses the global restic password.
    password_profile: Option<String>,
    dry_run: Option<bool>,
}

impl Backup {
    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(
            children,
            &[
                "repo",
                "exclude-file",
                "directory",
                "files-from",
                "include",
                "exclude",
                "remote",
                "retention",
                "steps",
                "password-profile",
                "dry-run",
            ],
        )?;
        let string = |name: &str| -> Result<Option<String>, ConfigError> {
            src.optional(children, name)?.map(|n| src.string_value(n)).transpose()
        };
        Ok(Backup {
            repo: src.string_value(src.required(node, children, "repo")?)?,
            exclude_file: string("exclude-file")?,
            directory: string("directory")?,
            files_from: string("files-from")?,
            include: named(children, "include")
                .map(|n| src.leaf_argument(n))
                .collect::<Result<_, _>>()?,
            exclude: named(children, "exclude")
                .map(|n| src.leaf_argument(n))
                .collect::<Result<_, _>>()?,
            remotes: named(children, "remote")
                .map(|n| Remote::decode(src, n))
                .collect::<Result<_, _>>()?,
            retention: src
                .optional(children, "retention")?
                .map(|n| BackupRetention::decode(src, n))
                .transpose()?
                .unwrap_or_default(),
            steps: src
                .optional(children, "steps")?
                .map(|n| Steps::decode(src, n))
                .transpose()?
                .unwrap_or_default(),
            password_profile: string("password-profile")?,
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

/// Removes the sync deadline from this thread when dropped.
struct ClearDeadline;

impl Drop for ClearDeadline {
    fn drop(&mut self) {
        crate::remote::set_deadline(None);
    }
}

/// Sets `RESTIC_PASSWORD` for one backup and restores the previous value when
/// dropped, so a task's own password doesn't leak into later tasks, hooks or
/// commands.
struct PasswordEnv(Option<std::ffi::OsString>);

impl PasswordEnv {
    fn set(password: &str) -> Self {
        let previous = env::var_os("RESTIC_PASSWORD");
        env::set_var("RESTIC_PASSWORD", password);
        PasswordEnv(previous)
    }
}

impl Drop for PasswordEnv {
    fn drop(&mut self) {
        match self.0.take() {
            Some(v) => env::set_var("RESTIC_PASSWORD", v),
            None => env::remove_var("RESTIC_PASSWORD"),
        }
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
    /// The sync was cut short because its window closed. It is resumable, so it
    /// is reported in `error` but does not fail the run.
    pub interrupted: bool,
    /// What happened to the remote's recovery copy; `None` if the sync failed
    /// and so it wasn't attempted.
    pub recovery: Option<crate::recovery::Outcome>,
}

/// What a [`Backup::run`] actually did.
#[derive(Debug, Default)]
pub struct RunOutcome {
    pub dry_run: bool,
    pub syncs: Vec<SyncResult>,
    /// Steps that ran and succeeded, for `steps` cadence bookkeeping.
    pub steps_done: Vec<Step>,
}

impl RunOutcome {
    /// Syncs that failed outright; a sync paused by its window is not one.
    pub fn failed_syncs(&self) -> usize {
        self.syncs.iter().filter(|s| s.error.is_some() && !s.interrupted).count()
    }

    pub fn interrupted_syncs(&self) -> usize {
        self.syncs.iter().filter(|s| s.interrupted).count()
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

    pub fn password_profile(&self) -> Option<&str> {
        self.password_profile.as_deref()
    }

    pub fn include(&self) -> &[String] {
        &self.include
    }

    pub fn exclude(&self) -> &[String] {
        &self.exclude
    }

    /// Whether anything is configured to be backed up.
    fn has_sources(&self) -> bool {
        self.directory.is_some() || self.files_from.is_some() || !self.include.is_empty()
    }

    fn backup_args(&self, dry_run: bool) -> Vec<String> {
        let mut args = vec![
            "backup".to_string(),
            "-r".to_string(),
            expand_env_vars(&self.repo),
        ];

        if let Some(files_from) = &self.files_from {
            args.extend(["--files-from".to_string(), expand_env_vars(files_from)]);
        }
        if let Some(directory) = &self.directory {
            args.push(expand_env_vars(directory));
        }
        args.extend(self.include.iter().map(|p| expand_env_vars(p)));

        if let Some(exclude_file) = &self.exclude_file {
            args.extend(["--exclude-file".to_string(), expand_env_vars(exclude_file)]);
        }
        for pattern in &self.exclude {
            args.extend(["--exclude".to_string(), expand_env_vars(pattern)]);
        }

        if dry_run {
            args.push("--dry-run".to_string());
        }
        args
    }

    fn backup(&self, dry_run: bool) -> Result<(), String> {
        if !self.has_sources() {
            return Err("nothing to back up: set `directory`, `files-from` or `include`".to_string());
        }
        let args = self.backup_args(dry_run);

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

    fn check_args(&self) -> Vec<String> {
        let mut args = vec!["check".to_string(), "-r".to_string(), expand_env_vars(&self.repo)];
        if let Some(subset) = self.steps.policy(&Step::Check).and_then(|p| p.subset()) {
            args.push(format!("--read-data-subset={subset}"));
        }
        args
    }

    fn check(&self) -> Result<(), String> {
        execute_command("restic", self.check_args())
    }

    pub fn steps(&self) -> &Steps {
        &self.steps
    }

    /// When a sync starting now must stop: `Some(None)` if it may run as long
    /// as it needs, `Some(Some(t))` if its window closes at `t`, and `None` if
    /// it is outside its window and should wait for the next run. `--force`
    /// ignores the window.
    fn sync_deadline(&self, config: &VivoConfig) -> Option<Option<Instant>> {
        let window = self.steps.policy(&Step::Sync).and_then(|p| p.window()).filter(|_| !config.force);
        let Some(window) = window else { return Some(None) };
        match window.remaining(chrono::Local::now().time()) {
            Some(left) => {
                ui::info(&format!("Syncing inside the {window} window ({} left)", crate::state::span(left.as_secs() as i64)));
                Some(Some(Instant::now() + left))
            }
            None => {
                ui::info(&format!("Skipping sync (outside its {window} window); it runs at the next opportunity"));
                None
            }
        }
    }

    /// Whether `step` should run now: always with `--force`, otherwise per its
    /// `steps` cadence. A task never synced to one of its remotes (a new remote)
    /// syncs immediately.
    fn step_due(&self, step: &Step, task_name: &str, config: &VivoConfig, state: &crate::state::State) -> bool {
        let Some(policy) = self.steps.policy(step) else { return true };
        if config.force || policy.every_secs().is_none() {
            return true;
        }
        let recorded = state.tasks.get(task_name);
        if *step == Step::Sync
            && self.remotes.iter().any(|r| !recorded.is_some_and(|t| t.remotes.contains_key(&r.url)))
        {
            return true;
        }
        let last = recorded.and_then(|t| t.steps.get(step.as_str())).copied();
        let due = policy.is_due(last, crate::state::now());
        if !due {
            ui::info(&format!(
                "Skipping {} (runs {}; last ran {})",
                step.as_str(),
                policy.every_text().unwrap_or_default(),
                last.map_or("never".to_string(), |t| crate::state::ago(crate::state::now() - t)),
            ));
        }
        due
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
        deadline: Option<Instant>,
    ) -> Vec<SyncResult> {
        use crate::recovery::Outcome;

        // Transfers stop at the deadline; clear it again whatever happens.
        crate::remote::set_deadline(deadline);
        let _clear = ClearDeadline;

        let local_repo = expand_env_vars(&self.repo);
        let mut creds_cache = credentials.clone();
        let state = crate::state::State::load();
        let mut source = crate::recovery::Source::new(&config.get_config_path(), &config.get_secrets_path());

        self.remotes
            .iter()
            .map(|remote| {
                let closed = deadline.is_some_and(|d| Instant::now() >= d);
                let error = if closed {
                    Some(format!("{}; {} was not synced and will be next time", crate::remote::WINDOW_CLOSED, remote.url))
                } else {
                    sync_remote(remote, &local_repo, dry_run, &mut creds_cache).err()
                };
                let interrupted = error.as_deref().is_some_and(|e| e.contains(crate::remote::WINDOW_CLOSED));
                if let (Some(e), false) = (&error, interrupted) {
                    eprintln!("error: {e}");
                }
                if error.is_some() || dry_run {
                    return SyncResult { url: remote.url.clone(), error, interrupted, recovery: None };
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
                SyncResult { url: remote.url.clone(), error: None, interrupted: false, recovery: Some(outcome) }
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
        secrets: &Secrets,
    ) -> Result<RunOutcome, String> {
        let password = secrets
            .password_for(self.password_profile())
            .ok_or_else(|| crate::config::missing_profile(self.password_profile().unwrap_or_default()))?;
        // restic and the sync backends read the password from the environment.
        let _password = PasswordEnv::set(password);
        let credentials = &secrets.credentials;
        let dry_run = config.dry_run || self.dry_run();
        let mut outcome = RunOutcome { dry_run, ..RunOutcome::default() };

        if config.start_step <= Step::Backup {
            if dry_run {
                // A dry run must not create anything, and every later phase needs a repo.
                if let Some(repo) = self.missing_local_repo() {
                    println!("[dry-run] restic repository {repo} does not exist yet.");
                    let sources = match (&self.directory, self.include.as_slice()) {
                        (Some(d), []) => d.clone(),
                        (None, [only]) => only.clone(),
                        _ => "the configured paths".to_string(),
                    };
                    println!("[dry-run] a real run would initialize it, then back up {sources}.");
                    return Ok(outcome);
                }
            } else {
                self.ensure_local_repo_init()?;
            }
            self.backup(dry_run)?;
        }
        let state = crate::state::State::load();
        if config.start_step <= Step::Check && self.step_due(&Step::Check, task_name, config, &state) {
            self.check()?;
            outcome.steps_done.push(Step::Check);
        }
        if config.start_step <= Step::Forget && self.step_due(&Step::Forget, task_name, config, &state) {
            self.forget(dry_run)?;
            outcome.steps_done.push(Step::Forget);
        }
        if config.start_step <= Step::Sync && self.step_due(&Step::Sync, task_name, config, &state) {
            if let Some(deadline) = self.sync_deadline(config) {
                outcome.syncs = self.sync_remotes(task_name, config, dry_run, credentials, deadline);
                if outcome.failed_syncs() == 0 && outcome.interrupted_syncs() == 0 {
                    outcome.steps_done.push(Step::Sync);
                }
            }
        }
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::{expand_env_vars, RunOutcome};
    use crate::step::Step;
    use std::collections::HashMap;
    use std::time::Instant;
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

    fn config_with(body: &str) -> BackupConfig {
        let src = format!(
            "default-task \"t\"\ntasks {{ task \"t\" {{ backup {{ repo \"/tmp/r\"\n{body} }} }} }}"
        );
        BackupConfig::parse("test", &src).unwrap()
    }

    #[test]
    fn backup_args_combine_directory_include_and_exclude() {
        let cfg = config_with(
            "directory \"/a\"\ninclude \"/b\"\ninclude \"/c d\"\nexclude \"**/*.tmp\"\nexclude \"**/Thumbnails\"",
        );
        let b = cfg.tasks[0].backup().unwrap();
        assert_eq!(
            b.backup_args(false),
            ["backup", "-r", "/tmp/r", "/a", "/b", "/c d", "--exclude", "**/*.tmp", "--exclude", "**/Thumbnails"]
        );
        assert_eq!(b.include(), ["/b", "/c d"]);
        assert_eq!(b.exclude().len(), 2);
    }

    #[test]
    fn backup_args_include_only_with_dry_run() {
        let cfg = config_with("include \"/x\"");
        let b = cfg.tasks[0].backup().unwrap();
        assert_eq!(b.backup_args(true), ["backup", "-r", "/tmp/r", "/x", "--dry-run"]);
    }

    #[test]
    fn backup_args_keep_files_from_next_to_include() {
        let cfg = config_with("files-from \"/list\"\ninclude \"/x\"");
        let b = cfg.tasks[0].backup().unwrap();
        assert_eq!(b.backup_args(false), ["backup", "-r", "/tmp/r", "--files-from", "/list", "/x"]);
    }

    #[test]
    fn backup_without_any_source_is_an_error() {
        let cfg = config_with("exclude \"*.tmp\"");
        let b = cfg.tasks[0].backup().unwrap();
        assert!(b.backup(false).unwrap_err().contains("nothing to back up"));
    }

    #[test]
    fn include_rejects_a_child_block() {
        let src = "default-task \"t\"\ntasks { task \"t\" { backup { repo \"/r\"\ninclude \"/x\" { } } } }";
        assert!(BackupConfig::parse("test", src).is_err());
    }

    const STEPS: &str = "steps {\ncheck { every \"weekly\"; subset \"5%\" }\nforget { every \"3d\" }\nsync { every \"daily\" }\n}";

    #[test]
    fn steps_default_to_every_run_with_plain_check() {
        let cfg = config_with("directory \"/d\"");
        let b = cfg.tasks[0].backup().unwrap();
        assert_eq!(b.check_args(), ["check", "-r", "/tmp/r"]);
        assert!(b.steps().policy(&Step::Sync).unwrap().every_secs().is_none());
    }

    #[test]
    fn steps_parse_and_subset_reaches_restic_check() {
        let cfg = config_with(&format!("directory \"/d\"\n{STEPS}"));
        let b = cfg.tasks[0].backup().unwrap();
        assert_eq!(b.check_args(), ["check", "-r", "/tmp/r", "--read-data-subset=5%"]);
        assert_eq!(b.steps().policy(&Step::Forget).unwrap().every_secs(), Some(3 * 86400));
        assert!(b.steps().policy(&Step::Backup).is_none());
    }

    #[test]
    fn steps_reject_bad_values() {
        for body in [
            "steps { backup { every \"daily\" } }",
            "steps { check { every \"fortnightly\" } }",
            "steps { check { subset \"150%\" } }",
            "steps { sync { subset \"5%\" } }",
            "steps { forget { every \"daily\"\nevery \"weekly\" } }",
        ] {
            let src = format!("default-task \"t\"\ntasks {{ task \"t\" {{ backup {{ repo \"/r\"\n{body} }} }} }}");
            assert!(BackupConfig::parse("test", &src).is_err(), "{body}");
        }
    }

    fn vivo_config(force: bool) -> crate::config::VivoConfig {
        crate::config::VivoConfig { task_name: None, config_file: None, dry_run: false, start_step: Step::Backup, force }
    }

    fn state_with(task: &str, steps: &[(&str, i64)], remotes: &[&str]) -> crate::state::State {
        let t = crate::state::TaskState {
            steps: steps.iter().map(|(k, v)| (k.to_string(), *v)).collect(),
            remotes: remotes.iter().map(|u| (u.to_string(), Default::default())).collect(),
            ..Default::default()
        };
        let mut s = crate::state::State::default();
        s.tasks.insert(task.to_string(), t);
        s
    }

    #[test]
    fn step_due_follows_cadence_state_and_force() {
        let cfg = config_with(&format!("directory \"/d\"\nremote \"b2:x:y\" {{ credentials \"b2\" }}\n{STEPS}"));
        let b = cfg.tasks[0].backup().unwrap();
        let now = crate::state::now();
        let hours_ago = |h: i64| now - h * 3600;

        // never run: due
        let empty = crate::state::State::default();
        assert!(b.step_due(&Step::Check, "t", &vivo_config(false), &empty));

        // check ran 2 days ago, weekly: skipped; sync ran 25h ago, daily: due
        let s = state_with("t", &[("check", hours_ago(48)), ("sync", hours_ago(25))], &["b2:x:y"]);
        assert!(!b.step_due(&Step::Check, "t", &vivo_config(false), &s));
        assert!(b.step_due(&Step::Sync, "t", &vivo_config(false), &s));

        // --force overrides the cadence
        assert!(b.step_due(&Step::Check, "t", &vivo_config(true), &s));

        // a remote never synced is due even if sync ran recently
        let s = state_with("t", &[("sync", hours_ago(1))], &[]);
        assert!(b.step_due(&Step::Sync, "t", &vivo_config(false), &s));
    }

    #[test]
    fn password_env_is_restored_after_a_backup() {
        let _guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original = std::env::var_os("RESTIC_PASSWORD");

        std::env::set_var("RESTIC_PASSWORD", "global");
        {
            let _p = super::PasswordEnv::set("photos-secret");
            assert_eq!(std::env::var("RESTIC_PASSWORD").unwrap(), "photos-secret");
        }
        assert_eq!(std::env::var("RESTIC_PASSWORD").unwrap(), "global");

        std::env::remove_var("RESTIC_PASSWORD");
        {
            let _p = super::PasswordEnv::set("photos-secret");
        }
        assert!(std::env::var_os("RESTIC_PASSWORD").is_none());

        match original {
            Some(v) => std::env::set_var("RESTIC_PASSWORD", v),
            None => std::env::remove_var("RESTIC_PASSWORD"),
        }
    }

    /// Runs a whole backup against a fake `restic` that appends the password it
    /// sees to a file, and returns what it saw per call.
    #[cfg(unix)]
    fn passwords_seen_by_restic(profile_line: &str, secrets: &str) -> (Result<(), String>, Vec<String>) {
        use std::os::unix::fs::PermissionsExt;

        let _path_guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("seen");
        let fake = dir.path().join("restic");
        // PATH is process-wide, so parallel tests may reach this fake: it only
        // answers for its own repository and fails like a missing repo otherwise.
        let repo = dir.path().join("repo");
        let script = format!(
            "#!/bin/sh\ncase \"$*\" in\n*'{repo}'*) echo \"$RESTIC_PASSWORD\" >> '{log}'; exit 0;;\nesac\nexit 1\n",
            repo = repo.display(),
            log = log.display()
        );
        std::fs::write(&fake, script).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();

        let src = format!(
            "default-task \"t\"\ntasks {{ task \"t\" {{ backup {{ repo \"{}\"\ndirectory \"/d\"\n{profile_line} }} }} }}",
            dir.path().join("repo").display()
        );
        let cfg = BackupConfig::parse("t", &src).unwrap();
        let secrets = crate::backup_config::parse_secrets(secrets).unwrap();

        let original_path = std::env::var("PATH").unwrap_or_default();
        let original_pw = std::env::var_os("RESTIC_PASSWORD");
        std::env::set_var("PATH", format!("{}:{original_path}", dir.path().display()));
        std::env::set_var("RESTIC_PASSWORD", "ambient");
        let config = vivo_config(true);
        let result = cfg.tasks[0].backup().unwrap().run(&config, "t", &secrets).map(|_| ());
        let after = std::env::var("RESTIC_PASSWORD").unwrap();
        std::env::set_var("PATH", original_path);
        match original_pw {
            Some(v) => std::env::set_var("RESTIC_PASSWORD", v),
            None => std::env::remove_var("RESTIC_PASSWORD"),
        }
        assert_eq!(after, "ambient", "the task's password must not outlive the run");

        let seen = std::fs::read_to_string(&log).unwrap_or_default().lines().map(str::to_string).collect();
        (result, seen)
    }

    #[cfg(unix)]
    #[test]
    fn backup_runs_restic_with_the_profile_password() {
        let (result, seen) = passwords_seen_by_restic(
            "password-profile \"photos\"",
            "restic_password: global\npasswords:\n  photos: pw-photos\n",
        );
        assert!(result.is_ok(), "{result:?}");
        // init, backup, check, forget
        assert_eq!(seen, ["pw-photos"; 4]);
    }

    #[cfg(unix)]
    #[test]
    fn backup_without_a_profile_uses_the_global_password() {
        let (result, seen) = passwords_seen_by_restic("", "restic_password: global\npasswords:\n  photos: pw-photos\n");
        assert!(result.is_ok(), "{result:?}");
        assert_eq!(seen, ["global"; 4]);
    }

    #[cfg(unix)]
    #[test]
    fn backup_with_an_unknown_profile_never_calls_restic() {
        let (result, seen) = passwords_seen_by_restic("password-profile \"nope\"", "restic_password: global\n");
        assert!(result.unwrap_err().contains("password profile 'nope'"));
        assert!(seen.is_empty());
    }

    #[test]
    fn sync_window_is_decoded_for_sync_only() {
        let cfg = config_with("directory \"/d\"\nsteps { sync { every \"daily\"\nwindow \"01:00-06:00\" } }");
        let w = cfg.tasks[0].backup().unwrap().steps().policy(&Step::Sync).unwrap().window().unwrap();
        assert_eq!(w.to_string(), "01:00-06:00");

        for body in [
            "steps { check { window \"01:00-06:00\" } }",
            "steps { forget { window \"01:00-06:00\" } }",
            "steps { sync { window \"tonight\" } }",
            "steps { sync { window \"03:00-03:00\" } }",
            "steps { sync { window \"01:00-02:00\"\nwindow \"03:00-04:00\" } }",
        ] {
            let src = format!("default-task \"t\"\ntasks {{ task \"t\" {{ backup {{ repo \"/r\"\n{body} }} }} }}");
            assert!(BackupConfig::parse("test", &src).is_err(), "{body}");
        }
    }

    /// `HH:MM-HH:MM` from `from_now` hours before now to `to_now` hours after it.
    fn window_around_now(before_hours: i64, after_hours: i64) -> String {
        use chrono::Timelike;
        let hour = i64::from(chrono::Local::now().hour());
        let wrap = |h: i64| h.rem_euclid(24);
        format!("{:02}:00-{:02}:00", wrap(hour + before_hours), wrap(hour + after_hours))
    }

    #[test]
    fn sync_only_starts_inside_its_window_unless_forced() {
        let inside = config_with(&format!("directory \"/d\"\nsteps {{ sync {{ window \"{}\" }} }}", window_around_now(-1, 2)));
        let b = inside.tasks[0].backup().unwrap();
        let deadline = b.sync_deadline(&vivo_config(false)).expect("inside the window").expect("a deadline");
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(left > std::time::Duration::from_secs(3600) && left <= std::time::Duration::from_secs(2 * 3600), "{left:?}");

        let outside = config_with(&format!("directory \"/d\"\nsteps {{ sync {{ window \"{}\" }} }}", window_around_now(2, 4)));
        let b = outside.tasks[0].backup().unwrap();
        assert!(b.sync_deadline(&vivo_config(false)).is_none(), "outside the window it must wait");
        assert_eq!(b.sync_deadline(&vivo_config(true)), Some(None), "--force ignores the window");

        let none = config_with("directory \"/d\"");
        assert_eq!(none.tasks[0].backup().unwrap().sync_deadline(&vivo_config(false)), Some(None));
    }

    #[test]
    fn a_closed_window_syncs_nothing_and_is_not_a_failure() {
        let cfg = config_with("directory \"/d\"\nremote \"b2:x:y\" { credentials \"b2\" }\nremote \"rustfs:http://h/b\" { credentials \"r\" }");
        let b = cfg.tasks[0].backup().unwrap();
        let past = Instant::now() - std::time::Duration::from_secs(1);
        let results = b.sync_remotes("t", &vivo_config(false), false, &HashMap::new(), Some(past));
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.interrupted && r.recovery.is_none()));
        assert!(results[0].error.as_deref().unwrap().contains(crate::remote::WINDOW_CLOSED));

        let outcome = RunOutcome { syncs: results, ..RunOutcome::default() };
        assert_eq!(outcome.failed_syncs(), 0, "a paused sync must not fail the run");
        assert_eq!(outcome.interrupted_syncs(), 2);
    }

    #[test]
    fn remote_limit_upload_is_validated_per_backend() {
        let remote = |url: &str, body: &str| {
            let src = format!(
                "default-task \"t\"\ntasks {{ task \"t\" {{ backup {{ repo \"/r\"\nremote \"{url}\" {{ credentials \"c\"\n{body} }} }} }} }}"
            );
            BackupConfig::parse("t", &src)
        };

        let cfg = remote("rustfs:http://h/b", "limit-upload \"20MiB\"").unwrap();
        let r = &cfg.tasks[0].remotes()[0];
        assert_eq!(r.limit_upload.as_deref(), Some("20MiB"));
        assert_eq!(r.limit_upload_kib(), Some(20480));
        assert_eq!(remote("s3:http://h/b", "limit-upload \"500k\"").unwrap().tasks[0].remotes()[0].limit_upload_kib(), Some(500));
        assert_eq!(remote("s3:http://h/b", "").unwrap().tasks[0].remotes()[0].limit_upload_kib(), None);

        let b2 = remote("b2:bucket:path", "limit-upload \"5M\"").unwrap_err().to_string();
        assert!(b2.contains("b2 remotes cannot limit upload speed"), "{b2}");
        assert!(remote("s3:http://h/b", "limit-upload \"fast\"").is_err());
        assert!(remote("rustfs:http://h/b", "limit-upload \"5M\"\nmc-limit-upload \"5MiB\"").is_err());
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
