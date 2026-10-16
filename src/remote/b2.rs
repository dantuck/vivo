use std::collections::HashMap;
use std::process::Command;

use super::objects::B2Store;
use super::{recovery_sibling, ObjectStore, RemoteBackend};

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
        let mut cmd = Command::new("b2");
        cmd.args([
            "sync",
            "--delete",
            "--replace-newer",
            "--compare-versions",
            "size",
            local_repo,
            &remote_url,
        ])
        .envs(env);
        let (status, stderr) = super::run_streaming(cmd, "b2")?;

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

    fn restic_url(&self) -> String {
        if self.path.is_empty() {
            format!("b2:{}", self.bucket)
        } else {
            format!("b2:{}:{}", self.bucket, self.path)
        }
    }

    /// restic reads `B2_ACCOUNT_ID`/`B2_ACCOUNT_KEY`, but `b2 account authorize`
    /// (and so vivo's secrets) calls them `B2_APPLICATION_KEY_ID`/`..._KEY`.
    fn restic_env(&self, credentials: &HashMap<String, String>) -> HashMap<String, String> {
        let mut env = credentials.clone();
        for (from, to) in [("B2_APPLICATION_KEY_ID", "B2_ACCOUNT_ID"), ("B2_APPLICATION_KEY", "B2_ACCOUNT_KEY")] {
            if let (Some(v), false) = (credentials.get(from), env.contains_key(to)) {
                env.insert(to.to_string(), v.clone());
            }
        }
        env
    }

    fn credential_vars(&self) -> [(&'static str, &'static str); 2] {
        [
            ("B2_APPLICATION_KEY_ID", "Backblaze application key ID"),
            ("B2_APPLICATION_KEY", "Backblaze application key"),
        ]
    }

    /// `bucket:folder` keeps it in `folder.vivo-recovery/`. A repository at the
    /// bucket root has no safe sibling (`b2 sync --delete` would wipe anything
    /// placed inside it), so that is reported rather than guessed at.
    fn recovery_store(&self) -> Result<Box<dyn ObjectStore>, String> {
        let path = self.path.trim_matches('/');
        if path.is_empty() {
            return Err(format!(
                "b2:{} is at the bucket root, so a recovery copy has nowhere safe to go — use a folder, e.g. b2:{}:backups, or set `recovery false`",
                self.bucket, self.bucket
            ));
        }
        Ok(Box::new(B2Store { bucket: self.bucket.clone(), prefix: recovery_sibling(path) }))
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
    fn recovery_copy_is_a_sibling_folder_and_never_the_bucket_root() {
        let store = |url| B2Backend::from_url(url).unwrap().recovery_store();
        assert_eq!(store("b2:bkt:filecabinet").unwrap().describe(), "b2://bkt/filecabinet.vivo-recovery");
        assert_eq!(store("b2:bkt:a/b/").unwrap().describe(), "b2://bkt/a/b.vivo-recovery");
        assert!(store("b2:bkt").err().unwrap().contains("bucket root"));
        assert!(store("b2:bkt:").is_err());
    }

    #[test]
    fn restic_url_and_env() {
        let b = B2Backend::from_url("b2:bkt:p/q").unwrap();
        assert_eq!(b.restic_url(), "b2:bkt:p/q");
        let creds = HashMap::from([
            ("B2_APPLICATION_KEY_ID".to_string(), "id".to_string()),
            ("B2_APPLICATION_KEY".to_string(), "key".to_string()),
        ]);
        let env = b.restic_env(&creds);
        assert_eq!(env["B2_ACCOUNT_ID"], "id");
        assert_eq!(env["B2_ACCOUNT_KEY"], "key");
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
