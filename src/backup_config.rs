pub(crate) mod backup;
pub(crate) mod task;

use std::process::Command as SysCommand;
use std::{env, fs};

use colored::*;

use knuffel::parse;

use crate::{backup_config::task::Task, config::Secrets, VivoConfig};

#[derive(knuffel::Decode, Debug)]
pub struct BackupConfig {
    #[knuffel(child, unwrap(argument))]
    pub default_task: String,
    #[knuffel(child, unwrap(children(name = "task")))]
    pub tasks: Vec<Task>,
}

fn decrypt_sops_file(file_path: &str) -> Result<String, String> {
    // Use sops to decrypt the file
    let output = SysCommand::new("sops")
        .arg("-d")
        .arg(file_path)
        .output()
        .expect("error");

    // Check if the command was successful
    if !output.status.success() {
        return Err(format!("Failed to decrypt file: {}", file_path).into());
    }

    // Convert the decrypted bytes into a string
    let decrypted_content = String::from_utf8(output.stdout).expect("error");
    Ok(decrypted_content)
}

impl BackupConfig {
    pub fn load_config(config: &VivoConfig) -> Result<BackupConfig, String> {
        let config_path = config.get_config_path();
        let config_content = fs::read_to_string(&config_path).map_err(|e| {
            format!(
                "Could not read configuration file: {}. Error: {}",
                config_path, e
            )
        })?;

        let secrets_path = config.get_secrets_path();
        let decrypted_yaml = decrypt_sops_file(&secrets_path)?;

        let secrets: Secrets = serde_yml::from_str(&decrypted_yaml).expect("error");

        println!(
            "[{}] Loaded secrets from {}",
            "i".cyan(),
            secrets_path.cyan()
        );
        env::set_var("RESTIC_PASSWORD", secrets.restic_password);

        let document =
            parse::<BackupConfig>(&config_path, &config_content).map_err(|e| e.to_string())?;

        println!(
            "[{}] Loaded configuration from {}",
            "i".cyan(),
            config_path.cyan()
        );
        Ok(document)
    }
}
