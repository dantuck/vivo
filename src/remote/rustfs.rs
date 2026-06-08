use std::collections::HashMap;

use super::RemoteBackend;

pub struct RustfsBackend {
    pub(super) endpoint: String,
    pub(super) bucket: String,
    pub(super) subpath: String,
}

impl RustfsBackend {
    pub fn from_url(url: &str) -> Result<Self, String> {
        if !url.starts_with("rustfs:") {
            return Err(format!("not a rustfs URL: '{url}'"));
        }

        // Strip the "rustfs:" prefix to get the inner URL (e.g. "https://host/bucket")
        let inner = &url["rustfs:".len()..];

        // Require a scheme with "://"
        let after_scheme = inner
            .find("://")
            .map(|i| &inner[i + 3..])
            .ok_or_else(|| format!("rustfs URL missing scheme (expected https:// or http://): '{url}'"))?;

        // Reconstruct the scheme prefix so we can build the endpoint
        let scheme_end = inner.find("://").unwrap() + 3;
        let scheme = &inner[..scheme_end]; // e.g. "https://"

        // Find the first '/' after the host (which separates host from path)
        let slash_pos = after_scheme
            .find('/')
            .ok_or_else(|| format!("rustfs URL missing bucket (no path after host): '{url}'"))?;

        let host = &after_scheme[..slash_pos];
        let endpoint = format!("{scheme}{host}");

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
        todo!()
    }

    fn sync(
        &self,
        _local_repo: &str,
        _dry_run: bool,
        _env: &HashMap<String, String>,
    ) -> Result<(), String> {
        todo!()
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
}
