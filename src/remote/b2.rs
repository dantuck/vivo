use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use super::RemoteBackend;

fn is_auth_error(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    lower.contains("unauthorized") || lower.contains("invalid credentials") || lower.contains("401")
}

pub struct B2Backend {
    pub(super) bucket: String,
    pub(super) path: String,
}

impl B2Backend {
    pub fn from_url(url: &str) -> Result<Self, String> {
        let without_prefix = url
            .strip_prefix("b2:")
            .ok_or_else(|| format!("not a b2 URL: '{url}'"))?;
        let (bucket, path) = without_prefix
            .split_once(':')
            .unwrap_or((without_prefix, ""));
        Ok(B2Backend {
            bucket: bucket.to_string(),
            path: path.to_string(),
        })
    }
}

impl B2Backend {
    fn execute_sync(&self, local_repo: &str, env: &HashMap<String, String>) -> Result<(), String> {
        let remote_url = format!("b2://{}/{}", self.bucket, self.path);
        let mut child = Command::new("b2")
            .args([
                "sync",
                "--delete",
                "--replace-newer",
                "--compare-versions",
                "size",
                local_repo,
                &remote_url,
            ])
            .envs(env)
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to run b2: {e}"))?;

        let stderr_reader = BufReader::new(child.stderr.take().unwrap());
        let stderr_thread = std::thread::spawn(move || {
            let mut collected = String::new();
            for line in stderr_reader.lines().map_while(Result::ok) {
                eprintln!("{line}");
                collected.push_str(&line);
                collected.push('\n');
            }
            collected
        });

        let status = child.wait().map_err(|e| format!("failed to wait for b2: {e}"))?;
        let stderr = stderr_thread.join().unwrap_or_default();

        if !status.success() {
            return Err(format!("b2 sync to {remote_url} failed: {stderr}"));
        }
        Ok(())
    }
}

impl RemoteBackend for B2Backend {
    fn name(&self) -> &str {
        "b2"
    }

    fn check_installed(&self) -> Result<(), String> {
        match Command::new("b2").arg("version").output() {
            Err(e) => Err(format!("failed to run b2: {e}")),
            Ok(o) if !o.status.success() => {
                Err("b2 not found — install with: pip install b2".to_string())
            }
            Ok(_) => Ok(()),
        }
    }

    fn sync(
        &self,
        local_repo: &str,
        dry_run: bool,
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        if dry_run {
            println!(
                "[dry-run] would sync {} to b2://{}/{}",
                local_repo, self.bucket, self.path
            );
            return Ok(());
        }

        super::verify_restic_repo(local_repo)?;

        match self.execute_sync(local_repo, env) {
            Ok(()) => Ok(()),
            Err(ref e) if is_auth_error(e) => {
                eprintln!("\n[!] B2 authentication failed — starting re-authorization...\n");
                let secrets_path = crate::config::secrets_path_from();
                let new_creds = crate::import_b2_credentials(&secrets_path)
                    .map_err(|e| format!("re-authorization failed: {e}"))?;
                eprintln!();
                self.execute_sync(local_repo, &new_creds)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bucket_and_path() {
        let b = B2Backend::from_url("b2:my-bucket:restic/path").unwrap();
        assert_eq!(b.bucket, "my-bucket");
        assert_eq!(b.path, "restic/path");
    }

    #[test]
    fn accepts_bucket_only() {
        let b = B2Backend::from_url("b2:my-bucket").unwrap();
        assert_eq!(b.bucket, "my-bucket");
        assert_eq!(b.path, "");
    }

    #[test]
    fn name_returns_b2() {
        let b = B2Backend::from_url("b2:bucket:path").unwrap();
        assert_eq!(b.name(), "b2");
    }

    #[test]
    fn dry_run_returns_ok_without_b2_installed() {
        let b = B2Backend::from_url("b2:bucket:path").unwrap();
        let result = b.sync("/nonexistent", true, &HashMap::new());
        // dry_run=true must not shell out to b2 or check the data dir
        assert!(result.is_ok());
    }
}
