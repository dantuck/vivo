mod b2;
mod s3;

pub use b2::B2Backend;
pub use s3::S3Backend;

use std::collections::HashMap;
use std::path::Path;

pub trait RemoteBackend {
    fn name(&self) -> &str;
    fn check_installed(&self) -> Result<(), String>;
    fn sync(
        &self,
        local_repo: &str,
        dry_run: bool,
        env: &HashMap<String, String>,
    ) -> Result<(), String>;
}

/// Verifies all required restic repository components are present before a
/// destructive sync (e.g. `b2 sync --delete`) that could corrupt the remote
/// if the local repo is incomplete.
pub fn verify_restic_repo(local_repo: &str) -> Result<(), String> {
    let repo = Path::new(local_repo);
    let missing: Vec<&str> = ["config", "data", "index", "keys", "snapshots"]
        .iter()
        .copied()
        .filter(|entry| !repo.join(entry).exists())
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "source repo '{local_repo}' is missing: {} — aborting sync to prevent remote corruption",
            missing.join(", ")
        ));
    }
    Ok(())
}

pub fn from_url(url: &str) -> Result<Box<dyn RemoteBackend>, String> {
    if url.starts_with("b2:") {
        Ok(Box::new(B2Backend::from_url(url)?))
    } else if url.starts_with("s3:") {
        Ok(Box::new(S3Backend::from_url(url)?))
    } else {
        Err(format!(
            "unsupported remote URL '{url}'. supported prefixes: b2:, s3:"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_b2_prefix() {
        let b = from_url("b2:my-bucket:restic").unwrap();
        assert_eq!(b.name(), "b2");
    }

    #[test]
    fn routes_s3_prefix() {
        let b = from_url("s3:http://rustfs.local:9000/backup").unwrap();
        assert_eq!(b.name(), "s3");
    }

    #[test]
    fn rejects_unknown_prefix() {
        let err = from_url("sftp:user@host:/backup").err().unwrap();
        assert!(err.contains("b2:, s3:"));
    }
}
