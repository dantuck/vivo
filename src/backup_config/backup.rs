use regex::Regex;
use std::env;
use std::process::Command;

use crate::VivoConfig;

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
    #[knuffel(child, unwrap(argument))]
    b2_repo: Option<String>,
    #[knuffel(child)]
    retention: Option<BackupRetention>,
    #[knuffel(child, unwrap(argument))]
    threads: Option<i16>,
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

struct BackupBuilder {
    backup_args: Vec<String>,
}

fn expand_env_vars(path: &str) -> String {
    // Regular expression to match environment variables like $HOME, $XDG_CONFIG_HOME
    let re = Regex::new(r"\$([A-Z_][A-Z0-9_]*)").unwrap();

    // Replace all occurrences of $VAR with their corresponding values from the environment
    let expanded_str = re.replace_all(path, |caps: &regex::Captures| {
        env::var(&caps[1]).unwrap_or_else(|_| caps[0].to_string())
    });

    expanded_str.to_string()
}

fn execute_command(command_name: &str, args: Vec<String>) {
    let mut command = Command::new(command_name);
    command.args(args);

    let output = command.output().expect("Failed to execute command");
    if output.status.success() {
        println!("{}", String::from_utf8_lossy(&output.stdout));
    } else {
        eprintln!("stderr: {}", String::from_utf8_lossy(&output.stderr));
    }
}

impl BackupBuilder {
    fn new(repo: &str, command: &str) -> Self {
        let backup_args = vec![command.into(), "-r".into(), expand_env_vars(repo)];
        BackupBuilder { backup_args }
    }

    fn files_from(mut self, files_from: Option<&str>, directory: Option<&str>) -> Self {
        if let Some(files_from) = &files_from {
            self.backup_args
                .extend(["--files-from".into(), expand_env_vars(files_from)]);
        } else if let Some(directory) = &directory {
            self.backup_args.push(expand_env_vars(directory));
        }
        self
    }

    fn exclude_file(mut self, exclude_file: Option<&str>) -> Self {
        if let Some(exclude_file) = &exclude_file {
            self.backup_args
                .extend(["--exclude-file".into(), expand_env_vars(exclude_file)]);
        }
        self
    }

    fn additional_args(mut self, args: Vec<String>) -> Self {
        self.backup_args.extend(args);
        self
    }

    fn exec(self, command_name: &str) {
        execute_command(command_name, self.backup_args);
    }
}

impl Backup {
    fn backup(&self) -> &Self {
        let mut additional_args: Vec<String> = vec![];
        if self.dry_run.unwrap_or(false) {
            additional_args.push("--dry-run".into());
        }

        BackupBuilder::new(&self.repo, "backup")
            .files_from(self.files_from.as_deref(), self.directory.as_deref())
            .exclude_file(self.exclude_file.as_deref())
            .additional_args(additional_args)
            .exec("restic");

        self
    }

    fn check(&self) -> &Self {
        BackupBuilder::new(&self.repo, "check").exec("restic");

        self
    }

    fn forget(&self) -> &Self {
        let mut additional_args: Vec<String> = vec![];
        if let Some(retention) = &self.retention {
            if let Some(daily) = retention.daily {
                additional_args.extend(["--keep-daily".into(), daily.to_string()])
            }
            if let Some(weekly) = retention.weekly {
                additional_args.extend(["--keep-weekly".into(), weekly.to_string()]);
            }
            if let Some(monthly) = retention.monthly {
                additional_args.extend(["--keep-monthly".into(), monthly.to_string()]);
            }
            if let Some(yearly) = retention.yearly {
                additional_args.extend(["--keep-yearly".into(), yearly.to_string()]);
            }
        }

        if self.dry_run.unwrap_or(false) {
            additional_args.push("--dry-run".into());
        }

        BackupBuilder::new(&self.repo, "forget")
            .additional_args(additional_args)
            .exec("restic");

        self
    }

    fn b2_sync(&self) -> &Self {
        if !self.dry_run.unwrap_or(false) {
            if let Some(b2_repo) = &self.b2_repo {
                let mut backup_args = vec!["sync".into(), "--delete".into()];

                if let Some(threads) = &self.threads {
                    backup_args.extend(["--threads".into(), threads.to_string()])
                }

                backup_args.extend([expand_env_vars(&self.repo), b2_repo.to_string()]);

                BackupBuilder { backup_args }.exec("b2");
            }
        }

        self
    }

    pub fn run(&self, _config: &VivoConfig) {
        self.backup().check().forget().b2_sync();
    }
}
