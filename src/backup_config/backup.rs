use std::collections::HashMap;
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

fn expand_env_vars(path: &str) -> String {
    let re = ENV_VAR_RE.get_or_init(|| Regex::new(r"\$([A-Z_][A-Z0-9_]*)").unwrap());
    re.replace_all(path, |caps: &regex::Captures| {
        env::var(&caps[1]).unwrap_or_else(|_| caps[0].to_string())
    })
    .to_string()
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

            let backend = match crate::remote::from_url(&remote.url) {
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

    pub fn run(
        &self,
        config: &VivoConfig,
        credentials: &HashMap<String, HashMap<String, String>>,
    ) -> Result<(), String> {
        let dry_run = config.dry_run || self.dry_run.unwrap_or(false);

        if config.start_step <= Step::Backup {
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
