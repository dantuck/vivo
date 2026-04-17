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
                .subcommand(Command::new("show").about("Decrypt and print secrets")),
        )
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
        secrets_path_from(self.config_file.as_ref())
    }
}

pub fn config_path_from(config_file: Option<&PathBuf>) -> String {
    config_file
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            if let Ok(v) = env::var("VIVO_BACKUP_CONFIG") {
                v
            } else if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
                format!("{xdg}/vivo/backup.kdl")
            } else {
                format!("{}/.config/vivo/backup.kdl", env::var("HOME").unwrap())
            }
        })
}

pub fn secrets_path_from(config_file: Option<&PathBuf>) -> String {
    config_file
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| {
            if let Ok(v) = env::var("VIVO_BACKUP_SECRETS") {
                v
            } else if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
                format!("{xdg}/vivo/secrets.yaml")
            } else {
                format!("{}/.config/vivo/secrets.yaml", env::var("HOME").unwrap())
            }
        })
}
