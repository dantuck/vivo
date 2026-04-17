use std::collections::HashMap;
use std::process::Command;

use super::RemoteBackend;

pub struct S3Backend {
    pub(super) url: String,
}

impl S3Backend {
    pub fn from_url(url: &str) -> Result<Self, String> {
        if !url.starts_with("s3:") {
            return Err(format!("not an s3 URL: '{url}'"));
        }
        Ok(S3Backend { url: url.to_string() })
    }
}

impl RemoteBackend for S3Backend {
    fn name(&self) -> &str {
        "s3"
    }

    fn check_installed(&self) -> Result<(), String> {
        match Command::new("restic").arg("version").output() {
            Err(e) => Err(format!("failed to run restic: {e}")),
            Ok(o) if !o.status.success() => {
                Err("restic not found — install from https://restic.net".to_string())
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
            println!("[dry-run] would copy {local_repo} to {}", self.url);
            return Ok(());
        }

        let output = Command::new("restic")
            .args(["copy", "--repo", local_repo, "--to", &self.url])
            .envs(env)
            .output()
            .map_err(|e| format!("failed to run restic: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("unable to open config") {
                return Err(format!(
                    "remote restic repo must be initialized first: restic init --repo {}",
                    self.url
                ));
            }
            return Err(format!("restic copy to {} failed: {stderr}", self.url));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stores_full_url() {
        let s = S3Backend::from_url("s3:http://rustfs.local:9000/backup").unwrap();
        assert_eq!(s.url, "s3:http://rustfs.local:9000/backup");
    }

    #[test]
    fn accepts_aws_s3_url() {
        let s = S3Backend::from_url("s3:https://s3.amazonaws.com/my-bucket").unwrap();
        assert_eq!(s.name(), "s3");
    }

    #[test]
    fn rejects_non_s3_prefix() {
        assert!(S3Backend::from_url("b2:bucket:path").is_err());
    }

    #[test]
    fn dry_run_returns_ok_without_restic() {
        let s = S3Backend::from_url("s3:http://localhost:9000/test").unwrap();
        let result = s.sync("/tmp/repo", true, &HashMap::new());
        assert!(result.is_ok());
    }
}
