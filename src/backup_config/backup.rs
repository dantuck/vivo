use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use regex::Regex;
use std::env;

use crate::config::VivoConfig;
use crate::step::Step;
use crate::ui;

#[derive(knuffel::Decode, Debug)]
pub struct Remote {
    #[knuffel(argument)]
    pub url: String,
    #[knuffel(child, unwrap(argument))]
    pub credentials: String,
    #[knuffel(child, unwrap(argument))]
    pub mc_max_workers: Option<u32>,
    #[knuffel(child, unwrap(argument))]
    pub mc_limit_upload: Option<String>,
}

#[derive(knuffel::Decode, Debug)]
pub struct Backup {
    #[knuffel(child, unwrap(argument))]
    repo: String,
    #[knuffel(child, unwrap(argument))]
    exclude_file: Option<String>,
    #[knuffel(child, unwrap(argument))]
    directory: Option<String>,
    #[knuffel(child, unwrap(argument))]
    files_from: Option<String>,
    #[knuffel(children(name = "remote"))]
    remotes: Vec<Remote>,
    #[knuffel(child)]
    retention: Option<BackupRetention>,
    #[knuffel(child, unwrap(argument))]
    dry_run: Option<bool>,
}

#[derive(knuffel::Decode, Debug)]
struct BackupRetention {
    #[knuffel(child, unwrap(argument))]
    daily: Option<i16>,
    #[knuffel(child, unwrap(argument))]
    weekly: Option<i16>,
    #[knuffel(child, unwrap(argument))]
    monthly: Option<i16>,
    #[knuffel(child, unwrap(argument))]
    yearly: Option<i16>,
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

fn execute_command(command_name: &str, args: Vec<String>) -> Result<(), String> {
    Command::new(command_name)
        .args(&args)
        .status()
        .map_err(|e| format!("failed to run {command_name}: {e}"))
        .and_then(|s| {
            if s.success() {
                Ok(())
            } else {
                Err(format!("{command_name} failed with status {s}"))
            }
        })
}

impl Backup {
    pub(crate) fn remotes(&self) -> &[Remote] {
        &self.remotes
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

        execute_command("restic", args)
    }

    fn check(&self) -> Result<(), String> {
        execute_command(
            "restic",
            vec!["check".to_string(), "-r".to_string(), expand_env_vars(&self.repo)],
        )
    }

    fn forget(&self, dry_run: bool) -> Result<(), String> {
        let r = &self.retention;
        let daily = r.as_ref().and_then(|r| r.daily).unwrap_or(7);
        let weekly = r.as_ref().and_then(|r| r.weekly).unwrap_or(5);
        let monthly = r.as_ref().and_then(|r| r.monthly).unwrap_or(12);
        let yearly = r.as_ref().and_then(|r| r.yearly).unwrap_or(2);

        let mut args = vec![
            "forget".to_string(),
            "-r".to_string(),
            expand_env_vars(&self.repo),
            "--keep-daily".to_string(),
            daily.to_string(),
            "--keep-weekly".to_string(),
            weekly.to_string(),
            "--keep-monthly".to_string(),
            monthly.to_string(),
            "--keep-yearly".to_string(),
            yearly.to_string(),
            "--prune".to_string(),
        ];

        if dry_run {
            args.push("--dry-run".to_string());
        }

        execute_command("restic", args)
    }

    fn sync_remotes(&self, dry_run: bool, credentials: &HashMap<String, HashMap<String, String>>) {
        let local_repo = expand_env_vars(&self.repo);
        let mut creds_cache: HashMap<String, HashMap<String, String>> = credentials.clone();

        for remote in &self.remotes {
            if !creds_cache.contains_key(&remote.credentials) {
                if remote.url.starts_with("b2:") {
                    eprintln!("\n[!] B2 credentials not found — starting authorization...\n");
                    let secrets_path = crate::config::secrets_path_from();
                    match crate::import_b2_credentials(&secrets_path) {
                        Ok(new_creds) => {
                            creds_cache.insert(remote.credentials.clone(), new_creds);
                        }
                        Err(e) => {
                            eprintln!("error: re-authorization failed: {e} — skipping remote {}", remote.url);
                            continue;
                        }
                    }
                } else {
                    eprintln!(
                        "error: credentials profile '{}' not found in secrets — skipping remote {}",
                        remote.credentials, remote.url
                    );
                    continue;
                }
            }
            let creds = creds_cache.get(&remote.credentials).unwrap();

            let backend = match crate::remote::from_remote(remote) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("error: {e}");
                    continue;
                }
            };

            if let Err(e) = backend.check_installed() {
                eprintln!("error: {e}");
                continue;
            }

            ui::info(&format!("Syncing to {}", remote.url));
            if let Err(e) = backend.sync(&local_repo, dry_run, creds) {
                eprintln!("error: sync to {} failed: {e}", remote.url);
            }
        }
    }

    fn is_local_repo(repo: &str) -> bool {
        !repo.contains(':')
            || repo.starts_with('/')
            || repo.starts_with('.')
            || repo.starts_with('~')
    }

    fn ensure_local_repo_init(&self) -> Result<(), String> {
        let repo = expand_env_vars(&self.repo);
        if !Self::is_local_repo(&repo) {
            return Ok(());
        }
        let config_file = Path::new(&repo).join("config");
        if !config_file.exists() {
            fs::create_dir_all(&repo)
                .map_err(|e| format!("could not create repo directory '{repo}': {e}"))?;
            execute_command("restic", vec!["init".to_string(), "-r".to_string(), repo])?;
        }
        Ok(())
    }

    pub fn run(
        &self,
        config: &VivoConfig,
        credentials: &HashMap<String, HashMap<String, String>>,
    ) -> Result<(), String> {
        let dry_run = config.dry_run || self.dry_run.unwrap_or(false);

        if config.start_step <= Step::Backup {
            self.ensure_local_repo_init()?;
            self.backup(dry_run)?;
        }
        if config.start_step <= Step::Check {
            self.check()?;
        }
        if config.start_step <= Step::Forget {
            self.forget(dry_run)?;
        }
        if config.start_step <= Step::Sync {
            self.sync_remotes(dry_run, credentials);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::expand_env_vars;
    use crate::backup_config::BackupConfig;

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
        let cfg = knuffel::parse::<BackupConfig>("test", src).unwrap();
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
        let cfg = knuffel::parse::<BackupConfig>("test", src).unwrap();
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
        let cfg = knuffel::parse::<BackupConfig>("test", src).unwrap();
        let remote = &cfg.tasks[0].backup().unwrap().remotes[0];
        assert_eq!(remote.mc_max_workers, Some(1));
        assert_eq!(remote.mc_limit_upload.as_deref(), Some("10MiB"));
    }
}
