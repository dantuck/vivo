pub(crate) mod backup;
pub(crate) mod task;

use std::process::Command as SysCommand;
use std::{env, fs};

use colored::*;
use knuffel::parse;

use crate::backup_config::task::Task;
use crate::config::Secrets;
use crate::VivoConfig;

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

        let secrets: Secrets = serde_yml::from_str(&decrypted_yaml)
            .map_err(|e| format!("failed to parse secrets: {e}"))?;

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
