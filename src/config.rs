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
}

#[derive(Debug, Deserialize)]
pub struct Secrets {
    pub restic_password: String,
    #[serde(default)]
    pub credentials: HashMap<String, HashMap<String, String>>,
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
                .subcommand(Command::new("show").about("Decrypt and print secrets"))
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
        .subcommand(Command::new("doctor").about("Check system health: tools, config, secrets, and remote connectivity"))
        .subcommand(Command::new("update").about("Update vivo to the latest release"))
        .subcommand(Command::new("manage").about("Open the interactive config manager (TUI)"))
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
                        .about("Add a remote to a task")
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
