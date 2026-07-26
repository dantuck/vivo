pub mod backup;
pub(crate) mod task;

use std::collections::HashMap;
use std::path::Path;
use std::process::Command as SysCommand;
use std::{env, fs};

use colored::*;
use knuffel::parse;

use crate::backup_config::task::Task;
use crate::config::{xdg_config_home, Secrets};
use crate::VivoConfig;

/// Resolves the age identity file path, honoring `SOPS_AGE_KEY_FILE` if set,
/// falling back to vivo's XDG-style default. `sops` itself resolves its
/// default key path via Go's `os.UserConfigDir()`, which on macOS is
/// `~/Library/Application Support` (ignoring `XDG_CONFIG_HOME`) — so this
/// path must always be passed explicitly via `SOPS_AGE_KEY_FILE` when
/// invoking `sops`, rather than relying on its own default resolution.
pub fn age_keys_path() -> String {
    env::var("SOPS_AGE_KEY_FILE").unwrap_or_else(|_| {
        xdg_config_home()
            .join("sops/age/keys.txt")
            .to_string_lossy()
            .into_owned()
    })
}

pub fn age_public_key() -> Option<String> {
    let contents = fs::read_to_string(age_keys_path()).ok()?;
    contents
        .lines()
        .find_map(|line| line.strip_prefix("# public key: "))
        .map(str::to_owned)
}

fn update_b2_in_secrets(secrets_path: &str, key_id: &str, key: &str) -> Result<(), String> {
    let decrypted = decrypt_sops_file(secrets_path)?;

    #[derive(serde::Deserialize)]
    struct DataWrapper {
        data: String,
    }
    let inner_yaml = match serde_yml::from_str::<DataWrapper>(&decrypted) {
        Ok(w) => w.data,
        Err(_) => decrypted,
    };

    let mut doc: serde_yml::Value = serde_yml::from_str(&inner_yaml)
        .map_err(|e| format!("could not parse secrets: {e}"))?;

    let credentials = doc
        .get_mut("credentials")
        .and_then(|v| v.as_mapping_mut())
        .ok_or("secrets missing 'credentials' map")?;

    let b2 = credentials
        .entry(serde_yml::Value::String("b2".to_string()))
        .or_insert(serde_yml::Value::Mapping(serde_yml::Mapping::new()));

    let b2_map = b2
        .as_mapping_mut()
        .ok_or("'credentials.b2' is not a map")?;

    b2_map.insert(
        serde_yml::Value::String("B2_APPLICATION_KEY_ID".to_string()),
        serde_yml::Value::String(key_id.to_string()),
    );
    b2_map.insert(
        serde_yml::Value::String("B2_APPLICATION_KEY".to_string()),
        serde_yml::Value::String(key.to_string()),
    );

    let updated_yaml = serde_yml::to_string(&doc)
        .map_err(|e| format!("could not serialize secrets: {e}"))?;

    let recipient =
        age_public_key().ok_or("no age key found — run: age-keygen -o ~/.config/sops/age/keys.txt")?;

    let tmp_path = env::temp_dir().join("vivo-secrets-import.yaml");
    fs::write(&tmp_path, &updated_yaml).map_err(|e| format!("could not write temp file: {e}"))?;

    let result = SysCommand::new("sops")
        .args(["-e", "--age", &recipient, "--output", secrets_path])
        .arg(&tmp_path)
        .output();
    let _ = fs::remove_file(&tmp_path);

    match result {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "sops encryption failed: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(format!("could not run sops: {e}")),
    }
}

pub fn update_s3_in_secrets(
    secrets_path: &str,
    profile: &str,
    key_id: &str,
    key: &str,
) -> Result<(), String> {
    let decrypted = decrypt_sops_file(secrets_path)?;

    #[derive(serde::Deserialize)]
    struct DataWrapper {
        data: String,
    }
    let inner_yaml = match serde_yml::from_str::<DataWrapper>(&decrypted) {
        Ok(w) => w.data,
        Err(_) => decrypted,
    };

    let mut doc: serde_yml::Value = serde_yml::from_str(&inner_yaml)
        .map_err(|e| format!("could not parse secrets: {e}"))?;

    let credentials = doc
        .get_mut("credentials")
        .and_then(|v| v.as_mapping_mut())
        .ok_or("secrets missing 'credentials' map")?;

    let entry = credentials
        .entry(serde_yml::Value::String(profile.to_string()))
        .or_insert(serde_yml::Value::Mapping(serde_yml::Mapping::new()));

    let entry_map = entry
        .as_mapping_mut()
        .ok_or_else(|| format!("'credentials.{profile}' is not a map"))?;

    entry_map.insert(
        serde_yml::Value::String("AWS_ACCESS_KEY_ID".to_string()),
        serde_yml::Value::String(key_id.to_string()),
    );
    entry_map.insert(
        serde_yml::Value::String("AWS_SECRET_ACCESS_KEY".to_string()),
        serde_yml::Value::String(key.to_string()),
    );

    let updated_yaml = serde_yml::to_string(&doc)
        .map_err(|e| format!("could not serialize secrets: {e}"))?;

    let recipient = age_public_key()
        .ok_or("no age key found — run: age-keygen -o ~/.config/sops/age/keys.txt")?;

    let tmp_path = env::temp_dir().join("vivo-secrets-import-s3.yaml");
    fs::write(&tmp_path, &updated_yaml)
        .map_err(|e| format!("could not write temp file: {e}"))?;

    let result = SysCommand::new("sops")
        .args(["-e", "--age", &recipient, "--output", secrets_path])
        .arg(&tmp_path)
        .output();
    let _ = fs::remove_file(&tmp_path);

    match result {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "sops encryption failed: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(format!("could not run sops: {e}")),
    }
}

/// Runs `b2 account authorize` interactively, reads the resulting credentials,
/// persists them to `secrets_path`, and returns the credential map for immediate use.
pub fn import_b2_credentials(secrets_path: &str) -> Result<HashMap<String, String>, String> {
    let status = SysCommand::new("b2")
        .args(["account", "authorize"])
        .status()
        .map_err(|e| format!("could not run b2: {e}"))?;

    if !status.success() {
        return Err("b2 account authorize failed".to_string());
    }

    let output = SysCommand::new("b2")
        .args(["account", "get"])
        .output()
        .map_err(|e| format!("could not run b2: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "b2 account get failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("could not parse b2 output: {e}"))?;

    let key_id = json["applicationKeyId"]
        .as_str()
        .ok_or("applicationKeyId not found in b2 output")?
        .to_string();
    let key = json["applicationKey"]
        .as_str()
        .ok_or("applicationKey not found in b2 output")?
        .to_string();

    update_b2_in_secrets(secrets_path, &key_id, &key)?;

    let mut creds = HashMap::new();
    creds.insert("B2_APPLICATION_KEY_ID".to_string(), key_id);
    creds.insert("B2_APPLICATION_KEY".to_string(), key);
    Ok(creds)
}

#[derive(knuffel::Decode, Debug)]
pub struct BackupConfig {
    #[knuffel(child, unwrap(argument))]
    pub default_task: String,
    #[knuffel(child, unwrap(children(name = "task")))]
    pub tasks: Vec<Task>,
}

pub fn decrypt_sops_file(file_path: &str) -> Result<String, String> {
    let output = SysCommand::new("sops")
        .env("SOPS_AGE_KEY_FILE", age_keys_path())
        .arg("-d")
        .arg(file_path)
        .output()
        .map_err(|e| format!("failed to run sops: {e}"))?;

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into_owned());
    }

    String::from_utf8(output.stdout).map_err(|e| format!("sops output is not valid UTF-8: {e}"))
}

pub fn parse_secrets(decrypted_yaml: &str) -> Result<Secrets, String> {
    #[derive(serde::Deserialize)]
    struct DataWrapper {
        data: String,
    }
    let secrets_yaml = match serde_yml::from_str::<DataWrapper>(decrypted_yaml) {
        Ok(w) => w.data,
        Err(_) => decrypted_yaml.to_string(),
    };
    serde_yml::from_str(&secrets_yaml).map_err(|e| format!("failed to parse secrets: {e}"))
}

impl BackupConfig {
    pub fn all_remotes(&self) -> Vec<(&str, &str)> {
        self.tasks.iter().flat_map(|t| t.backup_remotes()).collect()
    }

    pub fn remotes_for_task(&self, task_name: &str) -> Vec<(&str, &str)> {
        self.tasks
            .iter()
            .filter(|t| t.name == task_name)
            .flat_map(|t| t.backup_remotes())
            .collect()
    }

    /// Loads config and secrets in a single pass, distinguishing the specific
    /// failure so callers (e.g. the bare `vivo` entry point) can offer
    /// targeted guidance without re-deriving these checks or re-invoking
    /// `sops` themselves.
    pub fn load_config(config: &VivoConfig) -> Result<(BackupConfig, Secrets), LoadConfigError> {
        let config_path = config.get_config_path();
        if !Path::new(&config_path).exists() {
            return Err(LoadConfigError::ConfigMissing { path: config_path });
        }
        let config_content = fs::read_to_string(&config_path).map_err(|e| {
            LoadConfigError::Other(format!("could not read config '{config_path}': {e}"))
        })?;

        let secrets_path = config.get_secrets_path();
        if !Path::new(&secrets_path).exists() {
            return Err(LoadConfigError::SecretsMissing { path: secrets_path });
        }
        let decrypted_yaml =
            decrypt_sops_file(&secrets_path).map_err(|e| LoadConfigError::SecretsUndecryptable {
                path: secrets_path.clone(),
                detail: e.trim().to_string(),
            })?;

        let secrets = parse_secrets(&decrypted_yaml).map_err(LoadConfigError::Other)?;

        println!(
            "[{}] Loaded secrets from {}",
            "i".cyan(),
            secrets_path.cyan()
        );
        env::set_var("RESTIC_PASSWORD", &secrets.restic_password);

        let document = parse::<BackupConfig>(&config_path, &config_content)
            .map_err(|e| LoadConfigError::Other(e.to_string()))?;

        println!(
            "[{}] Loaded configuration from {}",
            "i".cyan(),
            config_path.cyan()
        );
        Ok((document, secrets))
    }
}

#[derive(Debug)]
pub enum LoadConfigError {
    ConfigMissing { path: String },
    SecretsMissing { path: String },
    SecretsUndecryptable { path: String, detail: String },
    Other(String),
}

impl std::fmt::Display for LoadConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadConfigError::ConfigMissing { path } => {
                write!(f, "could not read config '{path}': No such file or directory")
            }
            LoadConfigError::SecretsMissing { path } => {
                write!(f, "could not read secrets '{path}': No such file or directory")
            }
            LoadConfigError::SecretsUndecryptable { path, detail } => write!(
                f,
                "secrets file could not be decrypted — run `vivo secrets edit` to fix\n  path: {path}\n  {detail}"
            ),
            LoadConfigError::Other(msg) => write!(f, "{msg}"),
        }
    }
}

fn apply_profile_to_yaml(
    yaml: &str,
    profile: &str,
    credentials: &HashMap<String, String>,
) -> Result<String, String> {
    let mut doc: serde_yml::Value = serde_yml::from_str(yaml)
        .map_err(|e| format!("could not parse secrets: {e}"))?;

    let creds_map = doc
        .get_mut("credentials")
        .and_then(|v| v.as_mapping_mut())
        .ok_or("secrets missing 'credentials' map")?;

    let entry = creds_map
        .entry(serde_yml::Value::String(profile.to_string()))
        .or_insert(serde_yml::Value::Mapping(serde_yml::Mapping::new()));

    let entry_map = entry
        .as_mapping_mut()
        .ok_or_else(|| format!("'credentials.{profile}' is not a map"))?;

    for (k, v) in credentials {
        entry_map.insert(
            serde_yml::Value::String(k.clone()),
            serde_yml::Value::String(v.clone()),
        );
    }

    serde_yml::to_string(&doc).map_err(|e| format!("could not serialize secrets: {e}"))
}

pub fn write_profile_to_secrets(
    secrets_path: &str,
    profile: &str,
    credentials: &HashMap<String, String>,
) -> Result<(), String> {
    let decrypted = decrypt_sops_file(secrets_path)?;

    #[derive(serde::Deserialize)]
    struct DataWrapper {
        data: String,
    }
    let inner_yaml = match serde_yml::from_str::<DataWrapper>(&decrypted) {
        Ok(w) => w.data,
        Err(_) => decrypted,
    };

    let updated_yaml = apply_profile_to_yaml(&inner_yaml, profile, credentials)?;

    let recipient = age_public_key()
        .ok_or("no age key found — run: age-keygen -o ~/.config/sops/age/keys.txt")?;

    let tmp_file = tempfile::Builder::new()
        .prefix("vivo-secrets-")
        .suffix(".yaml")
        .tempfile()
        .map_err(|e| format!("could not create temp file: {e}"))?;
    fs::write(tmp_file.path(), &updated_yaml)
        .map_err(|e| format!("could not write temp file: {e}"))?;

    let result = SysCommand::new("sops")
        .args(["-e", "--age", &recipient, "--output", secrets_path])
        .arg(tmp_file.path())
        .output();
    // tmp_file dropped here, auto-deleted

    match result {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(format!(
            "sops encryption failed: {}",
            String::from_utf8_lossy(&o.stderr)
        )),
        Err(e) => Err(format!("could not run sops: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> BackupConfig {
        knuffel::parse::<BackupConfig>("test", src).unwrap()
    }

    #[test]
    fn all_remotes_returns_url_and_credentials() {
        let cfg = parse(r#"
            default-task "t"
            tasks {
                task "t" {
                    backup {
                        repo "/tmp/repo"
                        directory "/tmp"
                        remote "s3:http://example.com/bucket" {
                            credentials "aws"
                        }
                    }
                }
            }
        "#);
        let remotes = cfg.all_remotes();
        assert_eq!(remotes.len(), 1);
        assert_eq!(remotes[0].0, "s3:http://example.com/bucket");
        assert_eq!(remotes[0].1, "aws");
    }

    #[test]
    fn all_remotes_empty_when_no_backup() {
        let cfg = parse(r#"
            default-task "t"
            tasks {
                task "t" {
                    command "echo hi"
                }
            }
        "#);
        assert!(cfg.all_remotes().is_empty());
    }

    #[test]
    fn all_remotes_collects_across_tasks() {
        let cfg = parse(r#"
            default-task "a"
            tasks {
                task "a" {
                    backup {
                        repo "/tmp/r1"
                        directory "/tmp"
                        remote "s3:http://s3.example.com/b1" {
                            credentials "aws"
                        }
                    }
                }
                task "b" {
                    backup {
                        repo "/tmp/r2"
                        directory "/tmp"
                        remote "b2:bucket:path" {
                            credentials "b2"
                        }
                    }
                }
            }
        "#);
        let remotes = cfg.all_remotes();
        assert_eq!(remotes.len(), 2);
    }

    #[test]
    fn apply_profile_inserts_new_profile() {
        let yaml = "restic_password: s3cr3t\ncredentials:\n  existing:\n    KEY: val\n";
        let mut creds = std::collections::HashMap::new();
        creds.insert("AWS_ACCESS_KEY_ID".to_string(), "kid".to_string());
        creds.insert("AWS_SECRET_ACCESS_KEY".to_string(), "sak".to_string());
        let result = apply_profile_to_yaml(yaml, "new-s3", &creds).unwrap();
        let parsed: serde_yml::Value = serde_yml::from_str(&result).unwrap();
        assert_eq!(
            parsed["credentials"]["new-s3"]["AWS_ACCESS_KEY_ID"].as_str().unwrap(),
            "kid"
        );
    }

    #[test]
    fn apply_profile_overwrites_existing() {
        let yaml = "restic_password: s3cr3t\ncredentials:\n  aws:\n    AWS_ACCESS_KEY_ID: old\n";
        let mut creds = std::collections::HashMap::new();
        creds.insert("AWS_ACCESS_KEY_ID".to_string(), "new".to_string());
        let result = apply_profile_to_yaml(yaml, "aws", &creds).unwrap();
        let parsed: serde_yml::Value = serde_yml::from_str(&result).unwrap();
        assert_eq!(
            parsed["credentials"]["aws"]["AWS_ACCESS_KEY_ID"].as_str().unwrap(),
            "new"
        );
    }

    #[test]
    fn apply_profile_errors_without_credentials_key() {
        let yaml = "restic_password: s3cr3t\n";
        let creds = std::collections::HashMap::new();
        assert!(apply_profile_to_yaml(yaml, "x", &creds).is_err());
    }
}
