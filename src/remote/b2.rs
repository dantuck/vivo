use std::collections::HashMap;
use std::process::Command;

use super::RemoteBackend;

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
            .ok_or_else(|| format!("invalid b2 URL '{url}'. expected: b2:<bucket>:<path>"))?;
        Ok(B2Backend {
            bucket: bucket.to_string(),
            path: path.to_string(),
        })
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

        let remote_url = format!("b2://{}/{}", self.bucket, self.path);
        let output = Command::new("b2")
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
            .output()
            .map_err(|e| format!("failed to run b2: {e}"))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("b2 sync to {remote_url} failed: {stderr}"));
        }
        Ok(())
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
    fn rejects_missing_colon_separator() {
        assert!(B2Backend::from_url("b2:my-bucket").is_err());
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
