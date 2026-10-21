use std::collections::HashMap;
use std::process::{Command, Stdio};

use super::objects::S3Store;
use super::{recovery_sibling, ObjectStore, RemoteBackend, AWS_CREDENTIALS};

pub struct RustfsBackend {
    pub(crate) endpoint: String,
    pub(crate) bucket: String,
    pub(super) subpath: String,
    pub(crate) mc_max_workers: Option<u32>,
    pub(crate) mc_limit_upload: Option<String>,
    /// `limit-upload`, in KiB/s: `mc --limit-upload` or `rclone --bwlimit`.
    pub(crate) limit_upload_kib: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
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
        let mut cmd = Command::new(cmd_name);
        cmd.args(args).envs(env);
        let (status, stderr) = super::run_streaming(cmd, cmd_name)?;

        if !status.success() {
            return Err(format!("{cmd_name} sync to {} failed: {stderr}", self.endpoint));
        }
        Ok(())
    }

    fn mc_mirror_args(&self) -> Vec<String> {
        let mut extra = Vec::new();
        if let Some(p) = self.mc_max_workers {
            extra.push("--max-workers".to_string());
            extra.push(p.to_string());
        }
        if let Some(ref limit) = self.mc_limit_upload {
            extra.push("--limit-upload".to_string());
            extra.push(limit.clone());
        } else if let Some(kib) = self.limit_upload_kib {
            extra.push("--limit-upload".to_string());
            extra.push(format!("{kib}KiB"));
        }
        extra
    }

    /// Builds the `MC_HOST_vivo-sync` environment for mc — credentials are
    /// percent-encoded to avoid argv exposure and to handle special chars in
    /// base64-like secret keys.
    pub(super) fn mc_env(&self, env: &HashMap<String, String>) -> HashMap<String, String> {
        let key = env.get("AWS_ACCESS_KEY_ID").map(String::as_str).unwrap_or("");
        let secret = env.get("AWS_SECRET_ACCESS_KEY").map(String::as_str).unwrap_or("");

        let (scheme, host) = self.endpoint.split_once("://").unwrap_or(("https", &self.endpoint));
        let mc_host = format!("{}://{}:{}@{}", scheme, percent_encode_credential(key), percent_encode_credential(secret), host);

        let mut mc_env = HashMap::new();
        mc_env.insert("MC_HOST_vivo-sync".to_string(), mc_host);
        mc_env
    }

    pub(super) fn rclone_env(env: &HashMap<String, String>) -> HashMap<String, String> {
        let mut rclone_env = HashMap::new();
        rclone_env.insert(
            "RCLONE_S3_ACCESS_KEY_ID".to_string(),
            env.get("AWS_ACCESS_KEY_ID").cloned().unwrap_or_default(),
        );
        rclone_env.insert(
            "RCLONE_S3_SECRET_ACCESS_KEY".to_string(),
            env.get("AWS_SECRET_ACCESS_KEY").cloned().unwrap_or_default(),
        );
        rclone_env
    }

    /// Lists the bucket with whichever sync tool is installed (15s timeout).
    /// `Ok(false)` means the listing failed or timed out: the bucket may not
    /// exist yet, or the credentials/endpoint are wrong. `Err` means no sync
    /// tool is installed.
    pub fn bucket_reachable(&self, env: &HashMap<String, String>) -> Result<bool, String> {
        let mut cmd = match detect_tool()?.0 {
            SyncTool::Mc => {
                let mut c = Command::new("mc");
                c.args(["ls", &format!("vivo-sync/{}", self.bucket)]).envs(self.mc_env(env));
                c
            }
            SyncTool::Aws => {
                let mut c = Command::new("aws");
                c.args(["s3", "ls", &format!("s3://{}", self.bucket), "--endpoint-url", &self.endpoint])
                    .envs(env);
                c
            }
            SyncTool::Rclone => {
                let mut c = Command::new("rclone");
                c.args(["lsd", &format!(":s3:{}", self.bucket), "--s3-provider", "Other", "--s3-endpoint", &self.endpoint])
                    .envs(Self::rclone_env(env));
                c
            }
        };
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        crate::doctor::run_with_timeout(&mut cmd, std::time::Duration::from_secs(15))
    }

    /// Creates the bucket if it doesn't exist, using whichever sync tool is
    /// installed. Doubles as a credentials/endpoint check: it fails with the
    /// tool's own error on a bad key, unreachable host, or TLS problem.
    pub fn ensure_bucket(&self, env: &HashMap<String, String>) -> Result<(), String> {
        match detect_tool()?.0 {
            SyncTool::Mc => self.ensure_bucket_mc(&self.mc_env(env)),
            SyncTool::Aws => self.ensure_bucket_aws(env),
            SyncTool::Rclone => self.ensure_bucket_rclone(&Self::rclone_env(env)),
        }
    }

    fn sync_mc(&self, local_repo: &str, env: &HashMap<String, String>) -> Result<(), String> {
        let dest = self.mc_dest();
        let mc_env = self.mc_env(env);

        self.ensure_bucket_mc(&mc_env)?;

        let extra = self.mc_mirror_args();
        let mut args = vec!["mirror", "--remove", "--overwrite"];
        let extra_str: Vec<&str> = extra.iter().map(String::as_str).collect();
        args.extend_from_slice(&extra_str);
        args.push(local_repo);
        args.push(&dest);

        self.run_sync_command("mc", &args, &mc_env)
    }

    fn sync_aws(&self, local_repo: &str, env: &HashMap<String, String>) -> Result<(), String> {
        if self.limit_upload_kib.is_some() {
            eprintln!("[warn] limit-upload is ignored: the aws CLI has no bandwidth option (install mc or rclone)");
        }
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
        let rclone_env = Self::rclone_env(env);

        self.ensure_bucket_rclone(&rclone_env)?;
        let mut args: Vec<String> = [
            "sync", "--delete-during", "--size-only",
            local_repo, &dest,
            "--s3-provider", "Other",
            "--s3-endpoint", &self.endpoint,
        ]
        .map(String::from)
        .to_vec();
        if let Some(kib) = self.limit_upload_kib {
            args.extend(["--bwlimit".to_string(), format!("{kib}k")]);
        }
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        self.run_sync_command("rclone", &args, &rclone_env)
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

        Ok(RustfsBackend { endpoint, bucket, subpath, mc_max_workers: None, mc_limit_upload: None, limit_upload_kib: None })
    }

    /// A backend for `bucket` on `endpoint`, for callers that only need its
    /// credentials/environment helpers.
    pub(super) fn for_endpoint(endpoint: &str, bucket: &str) -> Self {
        RustfsBackend {
            endpoint: endpoint.to_string(),
            bucket: bucket.to_string(),
            subpath: String::new(),
            mc_max_workers: None,
            mc_limit_upload: None,
            limit_upload_kib: None,
        }
    }

    pub fn from_remote(remote: &crate::backup_config::backup::Remote) -> Result<Self, String> {
        let mut backend = Self::from_url(&remote.url)?;
        backend.mc_max_workers = remote.mc_max_workers;
        backend.mc_limit_upload = remote.mc_limit_upload.clone();
        backend.limit_upload_kib = remote.limit_upload_kib();
        Ok(backend)
    }
}

impl RemoteBackend for RustfsBackend {
    fn name(&self) -> &str {
        "rustfs"
    }

    fn check_installed(&self) -> Result<(), String> {
        detect_tool().map(|_| ())
    }

    fn restic_url(&self) -> String {
        match self.subpath.as_str() {
            "" => format!("s3:{}/{}", self.endpoint, self.bucket),
            sub => format!("s3:{}/{}/{sub}", self.endpoint, self.bucket),
        }
    }

    fn credential_vars(&self) -> [(&'static str, &'static str); 2] {
        AWS_CREDENTIALS
    }

    /// `bucket/sub` keeps it in `sub.vivo-recovery/`. A repository at the
    /// bucket root can't have a sibling (the mirror deletes anything extra in
    /// the bucket), so that gets a separate `bucket-vivo-recovery` bucket.
    fn recovery_store(&self) -> Result<Box<dyn ObjectStore>, String> {
        let sub = self.subpath.trim_matches('/');
        Ok(Box::new(if sub.is_empty() {
            S3Store::new(&self.endpoint, &format!("{}-vivo-recovery", self.bucket), "", true)
        } else {
            S3Store::new(&self.endpoint, &self.bucket, &recovery_sibling(sub), false)
        }))
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
    #[test]
    fn recovery_copy_gets_its_own_bucket_at_the_root_and_a_sibling_prefix_otherwise() {
        let store = |url| RustfsBackend::from_url(url).unwrap().recovery_store().unwrap().describe();
        assert_eq!(store("rustfs:https://h/filecabinet"), "s3://filecabinet-vivo-recovery/ (https://h)");
        assert_eq!(store("rustfs:https://h/bkt/sub/dir"), "s3://bkt/sub/dir.vivo-recovery (https://h)");
    }

    #[test]
    fn restic_url_is_the_s3_form() {
        let url = |u| RustfsBackend::from_url(u).unwrap().restic_url();
        assert_eq!(url("rustfs:http://nas:9000/backup"), "s3:http://nas:9000/backup");
        assert_eq!(url("rustfs:https://h/bkt/sub"), "s3:https://h/bkt/sub");
    }

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
        let _path_guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original_path = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "");
        let result = b.check_installed();
        std::env::set_var("PATH", &original_path);
        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("mc") && msg.contains("aws") && msg.contains("rclone"));
    }

    #[test]
    fn mc_mirror_args_empty_when_no_options() {
        let b = RustfsBackend::from_url("rustfs:https://host/bucket").unwrap();
        assert!(b.mc_mirror_args().is_empty());
    }

    #[test]
    fn mc_mirror_args_parallel_only() {
        let b = RustfsBackend {
            endpoint: "https://host".to_string(),
            bucket: "bucket".to_string(),
            subpath: String::new(),
            mc_max_workers: Some(4),
            mc_limit_upload: None,
            limit_upload_kib: None,
        };
        let args = b.mc_mirror_args();
        assert_eq!(args, vec!["--max-workers", "4"]);
    }

    #[test]
    fn mc_mirror_args_limit_upload_only() {
        let b = RustfsBackend {
            endpoint: "https://host".to_string(),
            bucket: "bucket".to_string(),
            subpath: String::new(),
            mc_max_workers: None,
            mc_limit_upload: Some("5MiB".to_string()),
            limit_upload_kib: None,
        };
        let args = b.mc_mirror_args();
        assert_eq!(args, vec!["--limit-upload", "5MiB"]);
    }

    #[test]
    fn mc_mirror_args_both_options() {
        let b = RustfsBackend {
            endpoint: "https://host".to_string(),
            bucket: "bucket".to_string(),
            subpath: String::new(),
            mc_max_workers: Some(2),
            mc_limit_upload: Some("10MiB".to_string()),
            limit_upload_kib: None,
        };
        let args = b.mc_mirror_args();
        assert_eq!(args, vec!["--max-workers", "2", "--limit-upload", "10MiB"]);
    }

    #[test]
    fn from_url_sets_mc_fields_to_none() {
        let b = RustfsBackend::from_url("rustfs:https://host/bucket").unwrap();
        assert!(b.mc_max_workers.is_none());
        assert!(b.mc_limit_upload.is_none());
    }

    #[test]
    fn mc_mirror_args_use_limit_upload_unless_mc_limit_upload_is_set() {
        let mut b = RustfsBackend::from_url("rustfs:http://h/b").unwrap();
        assert!(b.mc_mirror_args().is_empty());
        b.limit_upload_kib = Some(20480);
        assert_eq!(b.mc_mirror_args(), ["--limit-upload", "20480KiB"]);
        b.mc_max_workers = Some(2);
        assert_eq!(b.mc_mirror_args(), ["--max-workers", "2", "--limit-upload", "20480KiB"]);
        b.mc_limit_upload = Some("5MiB".into());
        assert_eq!(b.mc_mirror_args(), ["--max-workers", "2", "--limit-upload", "5MiB"]);
    }
}
