use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};

use super::RemoteBackend;

pub struct RustfsBackend {
    pub(crate) endpoint: String,
    pub(crate) bucket: String,
    pub(super) subpath: String,
}

pub(super) enum SyncTool {
    Mc,
    Aws,
    Rclone,
}

pub(super) fn is_bucket_already_exists_error(stderr: &str) -> bool {
    stderr.contains("BucketAlreadyOwnedByYou") || stderr.contains("BucketAlreadyExists")
}

pub(super) fn detect_tool() -> Result<(SyncTool, Option<&'static str>), String> {
    if Command::new("mc")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok((SyncTool::Mc, None));
    }
    if Command::new("aws")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok((SyncTool::Aws, Some("mc not found — using aws (install mc from https://min.io/docs/minio/linux/reference/minio-mc.html for best rustfs compatibility)")));
    }
    if Command::new("rclone")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
    {
        return Ok((SyncTool::Rclone, Some("mc not found — using rclone (install mc from https://min.io/docs/minio/linux/reference/minio-mc.html for best rustfs compatibility)")));
    }
    Err(
        "no sync tool found — install mc (https://min.io/docs/minio/linux/reference/minio-mc.html), \
         aws CLI (https://aws.amazon.com/cli/), or rclone (https://rclone.org)"
            .to_string(),
    )
}

pub(crate) fn percent_encode_credential(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '+' => out.push_str("%2B"),
            '/' => out.push_str("%2F"),
            '=' => out.push_str("%3D"),
            ':' => out.push_str("%3A"),
            '@' => out.push_str("%40"),
            '#' => out.push_str("%23"),
            '?' => out.push_str("%3F"),
            '&' => out.push_str("%26"),
            '%' => out.push_str("%25"),
            _ => out.push(c),
        }
    }
    out
}

impl RustfsBackend {
    fn s3_dest(&self) -> String {
        if self.subpath.is_empty() {
            format!("s3://{}", self.bucket)
        } else {
            format!("s3://{}/{}", self.bucket, self.subpath)
        }
    }

    fn rclone_dest(&self) -> String {
        if self.subpath.is_empty() {
            format!(":s3:{}", self.bucket)
        } else {
            format!(":s3:{}/{}", self.bucket, self.subpath)
        }
    }

    fn mc_dest(&self) -> String {
        if self.subpath.is_empty() {
            format!("vivo-sync/{}", self.bucket)
        } else {
            format!("vivo-sync/{}/{}", self.bucket, self.subpath)
        }
    }

    fn ensure_bucket_mc(&self, mc_env: &HashMap<String, String>) -> Result<(), String> {
        let dest = format!("vivo-sync/{}", self.bucket);
        self.run_sync_command("mc", &["mb", "--ignore-existing", &dest], mc_env)
    }

    fn ensure_bucket_aws(&self, env: &HashMap<String, String>) -> Result<(), String> {
        let output = Command::new("aws")
            .args(["s3", "mb", &format!("s3://{}", self.bucket), "--endpoint-url", &self.endpoint])
            .envs(env)
            .output()
            .map_err(|e| format!("failed to run aws s3 mb: {e}"))?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !is_bucket_already_exists_error(&stderr) {
                return Err(format!("aws s3 mb failed for bucket '{}': {stderr}", self.bucket));
            }
        }
        Ok(())
    }

    fn ensure_bucket_rclone(&self, rclone_env: &HashMap<String, String>) -> Result<(), String> {
        let dest = format!(":s3:{}", self.bucket);
        self.run_sync_command(
            "rclone",
            &["mkdir", &dest, "--s3-provider", "Other", "--s3-endpoint", &self.endpoint],
            rclone_env,
        )
    }

    fn run_sync_command(
        &self,
        cmd_name: &str,
        args: &[&str],
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        let mut child = Command::new(cmd_name)
            .args(args)
            .envs(env)
            .stdout(Stdio::inherit())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to run {cmd_name}: {e}"))?;

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

        let status = child
            .wait()
            .map_err(|e| format!("failed to wait for {cmd_name}: {e}"))?;
        let stderr = stderr_thread.join().unwrap_or_default();

        if !status.success() {
            return Err(format!("{cmd_name} sync to {} failed: {stderr}", self.endpoint));
        }
        Ok(())
    }

    fn sync_mc(&self, local_repo: &str, env: &HashMap<String, String>) -> Result<(), String> {
        let key = env.get("AWS_ACCESS_KEY_ID").map(String::as_str).unwrap_or("");
        let secret = env.get("AWS_SECRET_ACCESS_KEY").map(String::as_str).unwrap_or("");

        // Build MC_HOST_vivo-sync URL — credentials percent-encoded to avoid argv exposure
        // and handle special chars in base64-like secret keys
        let (scheme, host) = self.endpoint.split_once("://").unwrap_or(("https", &self.endpoint));
        let mc_host = format!("{}://{}:{}@{}", scheme, percent_encode_credential(key), percent_encode_credential(secret), host);

        let dest = self.mc_dest();
        let mut mc_env = HashMap::new();
        mc_env.insert("MC_HOST_vivo-sync".to_string(), mc_host);

        self.ensure_bucket_mc(&mc_env)?;
        self.run_sync_command(
            "mc",
            &["mirror", "--remove", "--overwrite", local_repo, &dest],
            &mc_env,
        )
    }

    fn sync_aws(&self, local_repo: &str, env: &HashMap<String, String>) -> Result<(), String> {
        self.ensure_bucket_aws(env)?;
        let dest = self.s3_dest();
        self.run_sync_command(
            "aws",
            &["s3", "sync", "--delete", "--size-only", local_repo, &dest, "--endpoint-url", &self.endpoint],
            env,
        )
    }

    fn sync_rclone(&self, local_repo: &str, env: &HashMap<String, String>) -> Result<(), String> {
        let dest = self.rclone_dest();
        let key = env.get("AWS_ACCESS_KEY_ID").cloned().unwrap_or_default();
        let secret = env.get("AWS_SECRET_ACCESS_KEY").cloned().unwrap_or_default();

        let mut rclone_env = HashMap::new();
        rclone_env.insert("RCLONE_S3_ACCESS_KEY_ID".to_string(), key);
        rclone_env.insert("RCLONE_S3_SECRET_ACCESS_KEY".to_string(), secret);

        self.ensure_bucket_rclone(&rclone_env)?;
        self.run_sync_command(
            "rclone",
            &[
                "sync", "--delete-during", "--size-only",
                local_repo, &dest,
                "--s3-provider", "Other",
                "--s3-endpoint", &self.endpoint,
            ],
            &rclone_env,
        )
    }

    pub fn from_url(url: &str) -> Result<Self, String> {
        if !url.starts_with("rustfs:") {
            return Err(format!("not a rustfs URL: '{url}'"));
        }

        // Strip the "rustfs:" prefix to get the inner URL (e.g. "https://host/bucket")
        let inner = &url["rustfs:".len()..];

        // Require a scheme with "://"
        let scheme_sep = inner
            .find("://")
            .ok_or_else(|| format!("rustfs URL missing scheme (expected https:// or http://): '{url}'"))?;
        let after_scheme = &inner[scheme_sep + 3..];

        // Find the first '/' after the host (which separates host from path)
        let slash_pos = after_scheme
            .find('/')
            .ok_or_else(|| format!("rustfs URL missing bucket (no path after host): '{url}'"))?;

        let endpoint = inner[..scheme_sep + 3 + slash_pos].to_string();

        // Everything after the slash is the path: "bucket" or "bucket/subpath"
        let path = &after_scheme[slash_pos + 1..];

        if path.is_empty() {
            return Err(format!("rustfs URL missing bucket (empty path): '{url}'"));
        }

        let (bucket, subpath) = match path.find('/') {
            Some(pos) => (path[..pos].to_string(), path[pos + 1..].to_string()),
            None => (path.to_string(), String::new()),
        };

        if bucket.is_empty() {
            return Err(format!("rustfs URL missing bucket (empty bucket): '{url}'"));
        }

        Ok(RustfsBackend { endpoint, bucket, subpath })
    }
}

impl RemoteBackend for RustfsBackend {
    fn name(&self) -> &str {
        "rustfs"
    }

    fn check_installed(&self) -> Result<(), String> {
        detect_tool().map(|_| ())
    }

    fn sync(
        &self,
        local_repo: &str,
        dry_run: bool,
        env: &HashMap<String, String>,
    ) -> Result<(), String> {
        if dry_run {
            println!(
                "[dry-run] would sync {} to rustfs:{}/{}",
                local_repo, self.endpoint, self.bucket
            );
            return Ok(());
        }

        super::verify_restic_repo(local_repo)?;

        let (tool, warning) = detect_tool()?;
        if let Some(msg) = warning {
            eprintln!("[warn] {msg}");
        }

        match tool {
            SyncTool::Mc => self.sync_mc(local_repo, env),
            SyncTool::Aws => self.sync_aws(local_repo, env),
            SyncTool::Rclone => self.sync_rclone(local_repo, env),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_https_url() {
        let b = RustfsBackend::from_url("rustfs:https://rustfs.cinnamon-trout.ts.net/filecabinet").unwrap();
        assert_eq!(b.endpoint, "https://rustfs.cinnamon-trout.ts.net");
        assert_eq!(b.bucket, "filecabinet");
        assert_eq!(b.subpath, "");
    }

    #[test]
    fn parses_http_with_port() {
        let b = RustfsBackend::from_url("rustfs:http://nas:9000/backup").unwrap();
        assert_eq!(b.endpoint, "http://nas:9000");
        assert_eq!(b.bucket, "backup");
        assert_eq!(b.subpath, "");
    }

    #[test]
    fn parses_url_with_subpath() {
        let b = RustfsBackend::from_url("rustfs:http://nas:9000/bucket/restic/repo").unwrap();
        assert_eq!(b.endpoint, "http://nas:9000");
        assert_eq!(b.bucket, "bucket");
        assert_eq!(b.subpath, "restic/repo");
    }

    #[test]
    fn rejects_missing_bucket() {
        assert!(RustfsBackend::from_url("rustfs:https://host/").is_err());
    }

    #[test]
    fn rejects_missing_scheme() {
        assert!(RustfsBackend::from_url("rustfs:host/bucket").is_err());
    }

    #[test]
    fn rejects_non_rustfs_prefix() {
        assert!(RustfsBackend::from_url("s3:http://host/bucket").is_err());
    }

    #[test]
    fn name_returns_rustfs() {
        let b = RustfsBackend::from_url("rustfs:https://host/bucket").unwrap();
        assert_eq!(b.name(), "rustfs");
    }

    #[test]
    fn dry_run_returns_ok_without_tools() {
        let b = RustfsBackend::from_url("rustfs:https://host/bucket").unwrap();
        let result = b.sync("/tmp/repo", true, &HashMap::new());
        assert!(result.is_ok());
    }

    #[test]
    fn is_bucket_already_exists_tolerates_owned_by_you() {
        assert!(is_bucket_already_exists_error("BucketAlreadyOwnedByYou"));
    }

    #[test]
    fn is_bucket_already_exists_tolerates_already_exists() {
        assert!(is_bucket_already_exists_error("make_bucket_failed: BucketAlreadyExists"));
    }

    #[test]
    fn is_bucket_already_exists_rejects_other_errors() {
        assert!(!is_bucket_already_exists_error("AccessDenied: permission denied"));
    }

    #[test]
    fn is_bucket_already_exists_rejects_empty() {
        assert!(!is_bucket_already_exists_error(""));
    }

    #[test]
    fn check_installed_fails_when_no_tools_on_path() {
        let b = RustfsBackend::from_url("rustfs:https://host/bucket").unwrap();
        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "");
        let result = b.check_installed();
        std::env::set_var("PATH", &original_path);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("mc") && msg.contains("aws") && msg.contains("rclone"));
    }
}
