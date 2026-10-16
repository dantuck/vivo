//! Running a backup, and guiding the user through whatever stops it from loading.

use std::path::Path;
use std::process;

use vivo::{decrypt_sops_file, BackupConfig, VivoConfig};

use crate::init::cmd_init;
use crate::secrets::{cmd_secrets_init, setup_restic_password};
use crate::util::{interactive, open_with_sops};

/// Reacts to a `LoadConfigError`: guides the user through a fix when stdin
/// is a TTY, otherwise prints an actionable error. Exits the process on
/// unrecoverable or failed-repair cases so callers can just `return` after
/// calling this (the process either already exited, or repair guidance ran).
pub fn handle_load_error(err: &vivo::LoadConfigError, config_path: &str, secrets_path: &str) {
    use vivo::LoadConfigError as E;

    if interactive() {
        if repair_load_error(err, config_path, secrets_path) {
            return;
        }
    } else {
        eprintln!("error: {err}");
        match err {
            E::ConfigMissing { .. } => eprintln!("Run `vivo init` to get started."),
            E::SecretsMissing { .. } => eprintln!("Run `vivo secrets init` to get started."),
            _ => {}
        }
    }
    process::exit(1);
}

/// The interactive half of `handle_load_error`. Returns false if the problem
/// could not be fixed.
pub fn repair_load_error(err: &vivo::LoadConfigError, config_path: &str, secrets_path: &str) -> bool {
    use vivo::LoadConfigError as E;

    match err {
        E::ConfigMissing { path } => {
            println!("No config found at '{path}' — let's set one up.\n");
            cmd_init(config_path, secrets_path)
        }
        E::SecretsMissing { path } => {
            println!("No secrets file found at '{path}' — let's create one.\n");
            let created = cmd_secrets_init(secrets_path);
            if created {
                println!("\nSet your restic password, then re-run vivo:");
                println!("  vivo secrets edit");
            }
            created
        }
        E::SecretsUndecryptable { path, detail } => {
            println!("Secrets file at '{path}' could not be decrypted:");
            println!("  {detail}");
            println!("\nOpening it in `sops` so you can fix it — set restic_password and save.\n");
            let fixed = open_with_sops(path);
            if !fixed {
                eprintln!("error: secrets are still not fixed. Run `vivo secrets edit` to try again.");
            }
            fixed
        }
        E::PasswordNotSet { .. } => {
            println!("Your restic password is still the placeholder — set it before the first backup.\n");
            let set = setup_restic_password(secrets_path);
            if set {
                println!("\nRe-run `vivo` to start your backup.");
            }
            set
        }
        E::Other(msg) => {
            eprintln!("error: {msg}");
            false
        }
    }
}

/// Checks that config and secrets are present and decryptable, guiding the
/// user through fixes if stdin is a TTY. Returns true if it's safe to
/// proceed to the caller's real work.
pub fn ensure_onboarded(config_path: &str, secrets_path: &str) -> bool {
    if !Path::new(config_path).exists() {
        handle_load_error(
            &vivo::LoadConfigError::ConfigMissing { path: config_path.to_string() },
            config_path,
            secrets_path,
        );
        return false;
    }
    if !Path::new(secrets_path).exists() {
        handle_load_error(
            &vivo::LoadConfigError::SecretsMissing { path: secrets_path.to_string() },
            config_path,
            secrets_path,
        );
        return false;
    }
    if let Err(e) = decrypt_sops_file(secrets_path) {
        handle_load_error(
            &vivo::LoadConfigError::SecretsUndecryptable {
                path: secrets_path.to_string(),
                detail: e.trim().to_string(),
            },
            config_path,
            secrets_path,
        );
        return false;
    }
    true
}

/// Loads config and secrets and runs the selected task. Returns false if the
/// task, a remote sync, or anything before them failed. Load problems get
/// guided repair on a terminal and don't count as a failed run.
pub fn run_backup(vivo_config: &VivoConfig, config_path: &str, secrets_path: &str) -> bool {
    match BackupConfig::load_config(vivo_config) {
        Ok((backup_config, secrets)) => {
            let task_name = vivo_config
                .task_name
                .as_deref()
                .unwrap_or(&backup_config.default_task);

            match backup_config.tasks.iter().find(|t| t.name == task_name) {
                Some(task) => task.run(vivo_config, &backup_config.tasks, &secrets.credentials),
                None => {
                    eprintln!("error: task '{task_name}' not found in config");
                    false
                }
            }
        }
        Err(e) => {
            handle_load_error(&e, config_path, secrets_path);
            true
        }
    }
}
