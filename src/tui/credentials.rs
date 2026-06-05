// src/tui/credentials.rs

#[derive(Debug, PartialEq)]
pub enum CredentialType {
    B2,
    S3,
    Generic,
}

#[allow(dead_code)]
pub fn detect_url_type(url: &str) -> CredentialType {
    if url.starts_with("b2:") {
        CredentialType::B2
    } else if url.starts_with("s3:") || url.starts_with("rustfs:") || url.starts_with("s3+https:") {
        CredentialType::S3
    } else {
        CredentialType::Generic
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_b2_prefix() {
        assert_eq!(detect_url_type("b2:my-bucket"), CredentialType::B2);
        assert_eq!(detect_url_type("b2:bucket/path"), CredentialType::B2);
    }

    #[test]
    fn detect_s3_prefixes() {
        assert_eq!(detect_url_type("s3:https://s3.amazonaws.com/bucket"), CredentialType::S3);
        assert_eq!(detect_url_type("rustfs:https://nas.example.com/bucket"), CredentialType::S3);
        assert_eq!(detect_url_type("s3+https://nas.example.com/bucket"), CredentialType::S3);
    }

    #[test]
    fn detect_generic_unknown() {
        assert_eq!(detect_url_type("sftp:user@host/path"), CredentialType::Generic);
        assert_eq!(detect_url_type("rclone:remote:/path"), CredentialType::Generic);
    }
}
