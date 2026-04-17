use std::collections::HashMap;
use std::path::Path;
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
        Command::new("b2")
            .arg("version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|_| ())
            .ok_or_else(|| "b2 not found — install with: pip install b2".to_string())
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

        if !Path::new(&format!("{}/data", local_repo)).exists() {
            return Err(format!(
                "source repo '{local_repo}' has no data directory — aborting sync to prevent remote deletion"
            ));
        }

        let remote_url = format!("b2://{}/{}", self.bucket, self.path);
        let status = Command::new("b2")
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
            .status()
            .map_err(|e| format!("failed to run b2: {e}"))?;

        if !status.success() {
            return Err(format!("b2 sync to {remote_url} failed"));
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
