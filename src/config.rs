use clap::{arg, command, value_parser, Arg, ArgAction, Command};
use serde::Deserialize;
use std::{env, path::PathBuf};

#[derive(Debug)]
pub struct VivoConfig {
    pub task_name: Option<String>,
    config_file: Option<PathBuf>,
    // subcommand: Option<SubcommandConfig>,
    pub dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub struct Secrets {
    pub restic_password: String,
}

// #[derive(Debug)]
// struct SubcommandConfig {
//     list: bool,
// }

impl VivoConfig {
    pub fn from_args() -> Self {
        let matches = command!() // requires `cargo` feature
            .arg(arg!([task_name] "Optional task name to operate on"))
            .arg(
                arg!(
                    -c --config <FILE> "Sets a custom config file"
                )
                // We don't have syntax yet for optional options, so manually calling `required`
                .required(false)
                .value_parser(value_parser!(PathBuf)),
            )
            .arg(arg!(
                -d --debug ... "Turn debugging information on"
            ))
            .arg(
                Arg::new("dry-run")
                    .long("dry-run")
                    .help("Perform a dry run without making any changes")
                    .action(clap::ArgAction::SetTrue),
            )
            .subcommand(
                Command::new("test")
                    .about("does testing things")
                    .arg(arg!(-l --list "lists test values").action(ArgAction::SetTrue)),
            )
            .get_matches();

        let task_name = matches.get_one::<String>("task_name").cloned();
        let config_file = matches.get_one::<PathBuf>("config").cloned();
        // let debug = matches.get_one::<u8>("debug").copied().unwrap_or(0);
        let dry_run = matches.get_one::<bool>("dry-run").copied().unwrap_or(false);

        // let subcommand = matches
        //     .subcommand_matches("test")
        //     .map(|sub_matches| SubcommandConfig {
        //         list: sub_matches.get_flag("list"),
        //     });

        VivoConfig {
            task_name,
            config_file,
            // subcommand,
            dry_run,
        }
    }

    pub fn get_config_path(&self) -> String {
        self.config_file.clone().map_or_else(
            || {
                if let Ok(backup_config) = env::var("VIVO_BACKUP_CONFIG") {
                    backup_config
                } else if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME") {
                    format!("{}/vivo/backup.kdl", xdg_config_home)
                } else {
                    // Fall back to user's home directory
                    format!("{}/.config/vivo/backup.kdl", env::var("HOME").unwrap())
                }
            },
            |path| path.to_string_lossy().into_owned(),
        )
    }

    pub fn get_secrets_path(&self) -> String {
        self.config_file.clone().map_or_else(
            || {
                if let Ok(backup_config) = env::var("VIVO_BACKUP_SECRETS") {
                    backup_config
                } else if let Ok(xdg_config_home) = env::var("XDG_CONFIG_HOME")
                {
                    // Both XDG_CONFIG_HOME and DOTS exist
                    format!("{}/vivo/secrets.yaml", xdg_config_home)
                } else {
                    // Fall back to user's home directory
                    format!("{}/.config/vivo/secrets.yaml", env::var("HOME").unwrap())
                }
            },
            |path| path.to_string_lossy().into_owned(),
        )
    }
}
