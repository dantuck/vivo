pub mod backup_config;
pub(crate) mod config;
pub mod ui;
pub mod step;
pub mod remote;
pub mod doctor;
pub mod update;
pub mod config_editor;

pub use crate::config::{build_cli, config_path_from, secrets_path_from, xdg_config_home, Secrets, VivoConfig};
pub use crate::backup_config::{age_public_key, decrypt_sops_file, import_b2_credentials, parse_secrets, update_s3_in_secrets, BackupConfig};
pub use config_editor::{add_task, add_remote, remove_task, remove_remote, TaskSpec, RemoteSpec};
