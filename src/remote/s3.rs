use std::collections::HashMap;
use std::process::Command;

use super::objects::S3Store;
use super::{recovery_sibling, ObjectStore, RemoteBackend, AWS_CREDENTIALS};

pub struct S3Backend {
    url: String,
}

impl S3Backend {
    pub fn from_url(url: &str) -> Result<Self, String> {
        if !url.starts_with("s3:") {
            return Err(format!("not an s3 URL: '{url}'"));
        }
        Ok(S3Backend { url: url.to_string() })
    }
}

impl S3Backend {
    /// `(endpoint, bucket, path)` of `s3:[scheme://]host/bucket[/path]` — the
    /// scheme defaults to https, as restic's does.
    fn parts(&self) -> Result<(String, String, String), String> {
        let rest = self.url.strip_prefix("s3:").unwrap_or(&self.url);
        let (scheme, after) = rest.split_once("://").unwrap_or(("https", rest));
        let (host, path) = after
            .split_once('/')
            .ok_or_else(|| format!("s3 URL has no bucket: '{}'", self.url))?;
        let (bucket, sub) = path.split_once('/').unwrap_or((path, ""));
        if bucket.is_empty() {
            return Err(format!("s3 URL has no bucket: '{}'", self.url));
        }
        Ok((format!("{scheme}://{host}"), bucket.to_string(), sub.trim_matches('/').to_string()))
    }
}

impl RemoteBackend for S3Backend {
    fn name(&self) -> &str {
        "s3"
    }

    fn restic_url(&self) -> String {
        self.url.clone()
    }

    fn credential_vars(&self) -> [(&'static str, &'static str); 2] {
        AWS_CREDENTIALS
    }

    /// `bucket/path` keeps it in `path.vivo-recovery/` (or `vivo-recovery/` for a
    /// repository at the bucket root; `restic copy` never deletes there).
    fn recovery_store(&self) -> Result<Box<dyn ObjectStore>, String> {
        let (endpoint, bucket, path) = self.parts()?;
        let prefix = if path.is_empty() { "vivo-recovery".to_string() } else { recovery_sibling(&path) };
        Ok(Box::new(S3Store::new(&endpoint, &bucket, &prefix, false)))
    }

    fn check_installed(&self) -> Result<(), String> {
        match Command::new("restic").arg("version").output() {
            Err(e) => Err(format!("failed to run restic: {e}")),
            Ok(o) if !o.status.success() => Err(format!(
                "restic check failed (exit {}); try: restic version",
                o.status.code().unwrap_or(-1)
            )),
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
            println!("[dry-run] would copy from {local_repo} to {}", self.url);
            return Ok(());
        }

        let restic_password = std::env::var("RESTIC_PASSWORD").unwrap_or_default();
        let mut cmd = Command::new("restic");
        cmd.args(["-r", &self.url, "copy", "--from-repo", local_repo])
            .envs(env)
            .env("RESTIC_FROM_PASSWORD", restic_password);
        let (status, stderr) = super::run_streaming(cmd, "restic")?;

        if !status.success() {
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
    fn recovery_copy_location_with_and_without_scheme() {
        let store = |url| S3Backend::from_url(url).unwrap().recovery_store();
        assert_eq!(store("s3:https://h/bkt").unwrap().describe(), "s3://bkt/vivo-recovery (https://h)");
        assert_eq!(store("s3:http://h:9000/bkt/p").unwrap().describe(), "s3://bkt/p.vivo-recovery (http://h:9000)");
        assert_eq!(
            store("s3:s3.amazonaws.com/bkt/p").unwrap().describe(),
            "s3://bkt/p.vivo-recovery (https://s3.amazonaws.com)"
        );
        assert!(store("s3:https://h").is_err());
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
