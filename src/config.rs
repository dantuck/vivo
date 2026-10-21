use clap::{arg, command, value_parser, Arg, ArgAction, Command};
use serde::Deserialize;
use std::collections::HashMap;
use std::str::FromStr;
use std::{env, path::PathBuf};

use crate::step::Step;

#[derive(Debug)]
pub struct VivoConfig {
    pub task_name: Option<String>,
    pub config_file: Option<PathBuf>,
    pub dry_run: bool,
    pub start_step: Step,
    /// Run every step, ignoring each backup's `steps` cadence.
    pub force: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct Secrets {
    pub restic_password: String,
    /// Extra restic repository passwords by name, for backups that set
    /// `password-profile`. A backup without one uses `restic_password`.
    #[serde(default)]
    pub passwords: HashMap<String, String>,
    #[serde(default)]
    pub credentials: HashMap<String, HashMap<String, String>>,
}

impl Secrets {
    /// The repository password for a backup's `password-profile`; `None` if
    /// that profile isn't in the secrets.
    pub fn password_for(&self, profile: Option<&str>) -> Option<&str> {
        match profile {
            None => Some(&self.restic_password),
            Some(name) => self.passwords.get(name).map(String::as_str),
        }
    }

    /// A copy whose `restic_password` is the one for `profile`, for code that
    /// takes a single password (remote tests, doctor).
    pub fn for_profile(&self, profile: Option<&str>) -> Result<Secrets, String> {
        let password = self.password_for(profile).ok_or_else(|| missing_profile(profile.unwrap_or_default()))?;
        Ok(Secrets { restic_password: password.to_string(), ..self.clone() })
    }
}

pub fn missing_profile(name: &str) -> String {
    format!("password profile '{name}' is not in the secrets — add it under `passwords:` with `vivo secrets edit`")
}

fn parse_step(s: &str) -> Result<Step, String> {
    Step::from_str(s)
}

pub fn build_cli() -> Command {
    command!()
        .arg(arg!([task_name] "Optional task name to operate on"))
        .arg(
            arg!(-c --config <FILE> "Sets a custom config file")
                .required(false)
                .value_parser(value_parser!(PathBuf)),
        )
        .arg(arg!(-d --debug ... "Turn debugging information on"))
        .arg(
            Arg::new("dry-run")
                .long("dry-run")
                .help("Perform a dry run without making any changes")
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new("start-step")
                .long("start-step")
                .short('S')
                .help("Start at step: backup, check, forget, sync")
                .value_parser(parse_step)
                .required(false),
        )
        .arg(
            Arg::new("force")
                .long("force")
                .help("Run every step, ignoring the `steps` cadence in the config")
                .action(ArgAction::SetTrue),
        )
        .subcommand(Command::new("init").about("Set up vivo for first use"))
        .subcommand(
            Command::new("config")
                .about("Manage backup configuration")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(Command::new("init").about("Create a new config file"))
                .subcommand(Command::new("edit").about("Open config in $EDITOR"))
                .subcommand(Command::new("show").about("Print config to stdout")),
        )
        .subcommand(
            Command::new("secrets")
                .about("Manage encrypted secrets")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(Command::new("init").about("Create and encrypt a new secrets file"))
                .subcommand(Command::new("edit").about("Edit secrets with sops"))
                .subcommand(Command::new("show").about("Print secrets with values masked"))
                .subcommand(Command::new("import-b2").about("Authorize with b2 and import credentials into secrets"))
                .subcommand(
                    Command::new("import-s3")
                        .about("Import S3-compatible credentials (AWS S3, RustFS, MinIO) into secrets")
                        .arg(
                            Arg::new("key-id")
                                .long("key-id")
                                .value_name("KEY_ID")
                                .help("AWS_ACCESS_KEY_ID / access key ID"),
                        )
                        .arg(
                            Arg::new("key")
                                .long("key")
                                .value_name("KEY")
                                .help("AWS_SECRET_ACCESS_KEY / secret access key"),
                        )
                        .arg(
                            Arg::new("profile")
                                .long("profile")
                                .short('p')
                                .value_name("PROFILE")
                                .help("Credential profile name to use in backup config (default: s3)"),
                        ),
                ),
        )
        .subcommand(
            Command::new("doctor")
                .about("Check system health: tools, config, secrets, and remote connectivity")
                .arg(
                    Arg::new("fix")
                        .long("fix")
                        .action(ArgAction::SetTrue)
                        .help("Auto-install missing tools"),
                ),
        )
        .subcommand(
            Command::new("schedule")
                .about("Run backups automatically (launchd, systemd or cron), one schedule per task")
                .subcommand(
                    Command::new("add")
                        .about("Schedule a task (writes `schedule` into its config) or, with no --task, the default task")
                        .arg(Arg::new("at").long("at").value_name("HH:MM").help("Time of day, 24-hour (default: 02:00)"))
                        .arg(
                            Arg::new("weekly")
                                .long("weekly")
                                .value_name("DAY")
                                .help("Run once a week on DAY (sun, mon, ...) instead of every day"),
                        )
                        .arg(Arg::new("task").long("task").short('t').value_name("TASK").help("Task to run (default: the config's default task)"))
                        .arg(
                            Arg::new("print")
                                .long("print")
                                .action(ArgAction::SetTrue)
                                .help("Show what would be installed without installing it"),
                        ),
                )
                .subcommand(
                    Command::new("apply")
                        .about("Install, update and remove schedules so they match the `schedule` nodes in the config")
                        .arg(
                            Arg::new("print")
                                .long("print")
                                .action(ArgAction::SetTrue)
                                .help("Show what would be installed without changing anything"),
                        ),
                )
                .subcommand(
                    Command::new("remove")
                        .about("Remove one task's schedule (and its `schedule` config node), or every schedule")
                        .arg(Arg::new("task").long("task").short('t').value_name("TASK").help("Only this task (default: remove all schedules)")),
                )
                .subcommand(Command::new("status").about("Show installed schedules and whether they match the config")),
        )
        .subcommand(
            Command::new("status")
                .about("Show when each task last backed up and synced, and flag problems")
                .arg(
                    Arg::new("max-age")
                        .long("max-age")
                        .value_name("HOURS")
                        .value_parser(value_parser!(u32))
                        .default_value("48")
                        .help("Warn when the newest snapshot or sync is older than this"),
                ),
        )
        .subcommand(
            Command::new("recover")
                .about("Rebuild config and secrets on a new machine from a remote's recovery copy")
                .arg(Arg::new("from").long("from").value_name("URL").help("Remote URL from your old config"))
                .arg(Arg::new("key-id").long("key-id").value_name("ID").help("Access key ID for that remote"))
                .arg(Arg::new("key").long("key").value_name("KEY").help("Secret access key for that remote"))
                .arg(
                    Arg::new("force")
                        .long("force")
                        .action(ArgAction::SetTrue)
                        .help("Replace an existing config/secrets file"),
                ),
        )
        .subcommand(
            Command::new("snapshots")
                .about("List the snapshots in a task's backup repository")
                .arg(Arg::new("task").value_name("TASK").help("Task to list (default: all)"))
                .arg(
                    Arg::new("remote")
                        .long("remote")
                        .action(ArgAction::SetTrue)
                        .help("List the off-site copies instead of the local repository"),
                ),
        )
        .subcommand(
            Command::new("restore")
                .about("Restore files from a backup (guided on a terminal)")
                .arg(Arg::new("task").value_name("TASK").help("Task to restore"))
                .arg(
                    Arg::new("snapshot")
                        .long("snapshot")
                        .short('s')
                        .value_name("ID")
                        .help("Snapshot ID to restore (default: latest)"),
                )
                .arg(
                    Arg::new("from")
                        .long("from")
                        .value_name("local|URL")
                        .help("Where to read from: 'local' or a remote URL (default: local, else the first remote)"),
                )
                .arg(
                    Arg::new("target")
                        .long("target")
                        .value_name("DIR")
                        .help("Directory to restore into (default: ~/vivo-restore/<task>)"),
                )
                .arg(
                    Arg::new("path")
                        .long("path")
                        .short('p')
                        .value_name("PATH")
                        .action(ArgAction::Append)
                        .help("Only restore this path inside the snapshot (repeatable)"),
                ),
        )
        .subcommand(Command::new("update").about("Update vivo to the latest release"))
        .subcommand(Command::new("manage").about("Open the interactive config manager (TUI)"))
        .subcommand(
            Command::new("mount")
                .about("Mount a backup repository read-only as a FUSE filesystem (pick from a menu, or name it with --task)")
                .arg(
                    Arg::new("path")
                        .help("Mount point directory (default: auto temp dir under /tmp)")
                        .required(false),
                )
                .arg(Arg::new("task").long("task").short('t').value_name("TASK").help("Mount this task's repository without prompting"))
                .arg(
                    Arg::new("from")
                        .long("from")
                        .value_name("local|URL")
                        .requires("task")
                        .help("With --task: the local repository (default) or a remote URL"),
                )
                .arg(Arg::new("host").long("host").value_name("HOST").help("Only show snapshots taken on this host"))
                .arg(
                    Arg::new("snapshot-path")
                        .long("snapshot-path")
                        .value_name("PATH")
                        .action(ArgAction::Append)
                        .help("Only show snapshots that include this path (repeatable)"),
                ),
        )
        .subcommand(
            Command::new("verify")
                .about("Check that a backup repository is intact by reading data back (default: a 5% sample of the local repository)")
                .arg(Arg::new("task").value_name("TASK").help("Task to verify (default: all)"))
                .arg(
                    Arg::new("from")
                        .long("from")
                        .value_name("local|URL")
                        .conflicts_with("remotes")
                        .help("Verify the local repository (default) or this remote URL"),
                )
                .arg(
                    Arg::new("remotes")
                        .long("remotes")
                        .action(ArgAction::SetTrue)
                        .help("Verify every off-site copy instead of the local repository"),
                )
                .arg(Arg::new("sample").long("sample").value_name("5%|1/10|2G").help("Read back this much data (default: 5%)"))
                .arg(Arg::new("quick").long("quick").action(ArgAction::SetTrue).help("Check structure and indexes only; read no data"))
                .arg(Arg::new("full").long("full").action(ArgAction::SetTrue).help("Read back all data (slow; downloads everything from a remote)")),
        )
        .subcommand(
            Command::new("task")
                .about("Manage backup tasks")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("add")
                        .about("Add a new backup task")
                        .arg(Arg::new("name").long("name").short('n').value_name("NAME").help("Task name"))
                        .arg(Arg::new("repo").long("repo").short('r').value_name("PATH").help("Restic repo path"))
                        .arg(Arg::new("dir").long("dir").short('d').value_name("PATH").help("Directory to back up"))
                        .arg(Arg::new("exclude-file").long("exclude-file").value_name("PATH").help("Exclude file path")),
                )
                .subcommand(Command::new("list").about("List backup tasks"))
                .subcommand(
                    Command::new("remove")
                        .about("Remove a backup task")
                        .arg(
                            Arg::new("name")
                                .index(1)
                                .required(true)
                                .value_name("NAME")
                                .help("Task name to remove"),
                        ),
                ),
        )
        .subcommand(
            Command::new("remote")
                .about("Manage backup remotes")
                .subcommand_required(true)
                .arg_required_else_help(true)
                .subcommand(
                    Command::new("add")
                        .about("Add a remote to a task (guided when --url/--credentials are omitted)")
                        .arg(Arg::new("task").long("task").short('t').value_name("TASK").help("Task name"))
                        .arg(Arg::new("url").long("url").short('u').value_name("URL").help("Remote URL"))
                        .arg(
                            Arg::new("credentials")
                                .long("credentials")
                                .short('c')
                                .value_name("PROFILE")
                                .help("Credentials profile name from secrets"),
                        ),
                )
                .subcommand(
                    Command::new("list")
                        .about("List remotes (all tasks, or filter with --task)")
                        .arg(
                            Arg::new("task")
                                .long("task")
                                .short('t')
                                .value_name("TASK")
                                .help("Filter by task name"),
                        ),
                )
                .subcommand(
                    Command::new("test")
                        .about("Test remote connectivity and credentials (all remotes, or filter with --task)")
                        .arg(
                            Arg::new("task")
                                .long("task")
                                .short('t')
                                .value_name("TASK")
                                .help("Only test this task's remotes"),
                        ),
                )
                .subcommand(
                    Command::new("remove")
                        .about("Remove a remote from a task")
                        .arg(Arg::new("task").long("task").short('t').value_name("TASK").required(true))
                        .arg(Arg::new("url").long("url").short('u').value_name("URL").required(true)),
                ),
        )
}

pub fn xdg_config_home() -> PathBuf {
    env::var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env::var("HOME").expect("HOME environment variable must be set"))
                .join(".config")
        })
}

/// `$XDG_STATE_HOME`, else `~/.local/state`.
pub fn xdg_state_home() -> PathBuf {
    env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env::var("HOME").expect("HOME environment variable must be set"))
                .join(".local/state")
        })
}

impl VivoConfig {
    pub fn from_matches(matches: &clap::ArgMatches) -> Self {
        VivoConfig {
            task_name: matches.get_one::<String>("task_name").cloned(),
            config_file: matches.get_one::<PathBuf>("config").cloned(),
            dry_run: matches.get_flag("dry-run"),
            start_step: matches
                .get_one::<Step>("start-step")
                .cloned()
                .unwrap_or_default(),
            // Naming a start step is an explicit request to run from there.
            force: matches.get_flag("force") || matches.contains_id("start-step"),
        }
    }

    pub fn get_config_path(&self) -> String {
        config_path_from(self.config_file.as_ref())
    }

    pub fn get_secrets_path(&self) -> String {
        secrets_path_from()
    }
}

pub fn config_path_from(config_file: Option<&PathBuf>) -> String {
    config_file
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            env::var("VIVO_BACKUP_CONFIG").unwrap_or_else(|_| {
                xdg_config_home()
                    .join("vivo/backup.kdl")
                    .to_string_lossy()
                    .into_owned()
            })
        })
}

pub fn secrets_path_from() -> String {
    env::var("VIVO_BACKUP_SECRETS").unwrap_or_else(|_| {
        xdg_config_home()
            .join("vivo/secrets.yaml")
            .to_string_lossy()
            .into_owned()
    })
}
