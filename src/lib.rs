pub mod backup_config;
pub(crate) mod config;
pub mod ui;
pub mod step;
pub mod remote;
pub mod doctor;
pub mod mount;
pub mod update;
pub mod config_editor;
pub mod tui;
pub mod remote_setup;
pub mod state;
pub mod repos;
pub mod schedule;
pub mod recovery;

pub use crate::config::{build_cli, config_path_from, secrets_path_from, xdg_config_home, Secrets, VivoConfig};
pub use crate::backup_config::{age_keys_path, age_public_key, decrypt_sops_file, import_b2_credentials, parse_secrets, set_restic_password, update_s3_in_secrets, BackupConfig, LoadConfigError};
pub use crate::backup_config::backup::{ensure_path, expand_env_vars};
pub use config_editor::{add_task, add_remote, remove_task, remove_remote, set_default_task, TaskSpec, RemoteSpec};

/// Serializes tests that temporarily replace `PATH`; the environment is process-wide.
#[cfg(test)]
pub(crate) static PATH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
