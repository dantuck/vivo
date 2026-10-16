mod b2;
mod objects;
mod rustfs;
mod s3;

pub use b2::B2Backend;
pub use rustfs::RustfsBackend;
pub use s3::S3Backend;
pub use objects::ObjectStore;

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};

/// Credential variables of S3-compatible stores, with the prompt for each.
pub(crate) const AWS_CREDENTIALS: [(&str, &str); 2] =
    [("AWS_ACCESS_KEY_ID", "Access key ID"), ("AWS_SECRET_ACCESS_KEY", "Secret access key")];

/// Suffix of the folder (or bucket) next to a repository that holds its recovery copy.
const RECOVERY_SUFFIX: &str = ".vivo-recovery";

pub(crate) fn recovery_sibling(path: &str) -> String {
    format!("{path}{RECOVERY_SUFFIX}")
}

pub trait RemoteBackend {
    fn name(&self) -> &str;
    fn check_installed(&self) -> Result<(), String>;

    /// The URL restic uses to open this remote's repository.
    fn restic_url(&self) -> String;

    /// The environment restic needs, given a credentials profile.
    fn restic_env(&self, credentials: &HashMap<String, String>) -> HashMap<String, String> {
        credentials.clone()
    }

    /// The two credential variables a profile for this remote holds, each
    /// with the prompt used to ask for it.
    fn credential_vars(&self) -> [(&'static str, &'static str); 2];

    /// Where this remote keeps its recovery copy (a sibling of the
    /// repository: sync deletes anything extra inside it), or why it can't.
    fn recovery_store(&self) -> Result<Box<dyn ObjectStore>, String>;

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

/// Returns true if mc, aws, or rclone is available for S3/rustfs sync.
pub fn s3_sync_tool_installed() -> bool {
    rustfs::detect_tool().is_ok()
}

/// Runs `cmd` with stdout inherited, echoing stderr live while also
/// collecting it. `Err` means the process could not be run or waited on;
/// otherwise returns its exit status and everything it wrote to stderr.
pub(crate) fn run_streaming(mut cmd: Command, name: &str) -> Result<(ExitStatus, String), String> {
    let mut child = cmd
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to run {name}: {e}"))?;

    let stderr_reader = BufReader::new(child.stderr.take().expect("stderr is piped"));
    let stderr_thread = std::thread::spawn(move || {
        let mut collected = String::new();
        for line in stderr_reader.lines().map_while(Result::ok) {
            eprintln!("{line}");
            collected.push_str(&line);
            collected.push('\n');
        }
        collected
    });

    let status = child.wait().map_err(|e| format!("failed to wait for {name}: {e}"))?;
    Ok((status, stderr_thread.join().unwrap_or_default()))
}

pub fn from_url(url: &str) -> Result<Box<dyn RemoteBackend>, String> {
    if url.starts_with("b2:") {
        Ok(Box::new(B2Backend::from_url(url)?))
    } else if url.starts_with("s3:") {
        Ok(Box::new(S3Backend::from_url(url)?))
    } else if url.starts_with("rustfs:") {
        Ok(Box::new(RustfsBackend::from_url(url)?))
    } else {
        Err(format!(
            "unsupported remote URL '{url}'. supported prefixes: b2:, s3:, rustfs:"
        ))
    }
}

/// Like [`from_url`], but also applies the per-remote settings (rustfs `mc` tuning).
pub fn from_remote(remote: &crate::backup_config::backup::Remote) -> Result<Box<dyn RemoteBackend>, String> {
    if remote.url.starts_with("rustfs:") {
        Ok(Box::new(RustfsBackend::from_remote(remote)?))
    } else {
        from_url(&remote.url)
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

    #[test]
    fn routes_rustfs_prefix() {
        let b = from_url("rustfs:http://nas:9000/backup").unwrap();
        assert_eq!(b.name(), "rustfs");
    }

    #[test]
    fn rejects_unknown_prefix_mentions_rustfs() {
        let err = from_url("sftp:user@host:/backup").err().unwrap();
        assert!(err.contains("rustfs:"));
    }
}
