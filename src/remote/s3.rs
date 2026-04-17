use std::collections::HashMap;

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
        Ok(())
    }

    fn sync(
        &self,
        _local_repo: &str,
        _dry_run: bool,
        _env: &HashMap<String, String>,
    ) -> Result<(), String> {
        unimplemented!("S3Backend not yet implemented")
    }
}
