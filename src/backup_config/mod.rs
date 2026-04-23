pub(crate) mod backup;
pub(crate) mod task;

use std::collections::HashMap;
use std::process::Command as SysCommand;
use std::{env, fs};

use colored::*;
use knuffel::parse;

use crate::backup_config::task::Task;
use crate::config::{xdg_config_home, Secrets};
use crate::VivoConfig;

pub fn age_public_key() -> Option<String> {
    let keys_path = if let Ok(p) = env::var("SOPS_AGE_KEY_FILE") {
        p
    } else {
        xdg_config_home()
            .join("sops/age/keys.txt")
            .to_string_lossy()
            .into_owned()
    };
    let contents = fs::read_to_string(&keys_path).ok()?;
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

    pub fn load_config(config: &VivoConfig) -> Result<(BackupConfig, Secrets), String> {
        let config_path = config.get_config_path();
        let config_content = fs::read_to_string(&config_path)
            .map_err(|e| format!("could not read config '{config_path}': {e}"))?;

        let secrets_path = config.get_secrets_path();
        let decrypted_yaml = decrypt_sops_file(&secrets_path).map_err(|_| {
            format!(
                "secrets file must be SOPS-encrypted — run `vivo secrets edit` to fix\n  path: {secrets_path}"
            )
        })?;

        let secrets = parse_secrets(&decrypted_yaml)?;

        println!(
            "[{}] Loaded secrets from {}",
            "i".cyan(),
            secrets_path.cyan()
        );
        env::set_var("RESTIC_PASSWORD", &secrets.restic_password);

        let document =
            parse::<BackupConfig>(&config_path, &config_content).map_err(|e| e.to_string())?;

        println!(
            "[{}] Loaded configuration from {}",
            "i".cyan(),
            config_path.cyan()
        );
        Ok((document, secrets))
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
}
