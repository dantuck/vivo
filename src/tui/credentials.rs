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
    } else if url.starts_with("s3:") || url.starts_with("rustfs:") {
        CredentialType::S3
    } else {
        CredentialType::Generic
    }
}

pub fn suggest_profile_name(url: &str) -> String {
    let scheme_end = url.find(':').unwrap_or(url.len());
    let scheme = &url[..scheme_end];
    let after_colon = url.get(scheme_end + 1..).unwrap_or("");

    // No embedded URL (e.g. "b2:bucket-name") → use the scheme as the name
    if !after_colon.starts_with("//") && !after_colon.contains("://") {
        return scheme.to_string();
    }

    // Extract hostname: the part between `//` and the next `/`
    let after_slashes = if let Some(pos) = after_colon.find("//") {
        &after_colon[pos + 2..]
    } else {
        after_colon
    };
    let host_with_port = after_slashes.split('/').next().unwrap_or(after_slashes);
    // Strip optional port (e.g. "nas:9000" → "nas")
    let host = host_with_port.split(':').next().unwrap_or(host_with_port);

    // IP address → return as-is
    if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return host.to_string();
    }

    // Return the first DNS label
    host.split('.').next().unwrap_or(host).to_string()
}

pub fn parse_profile_names(yaml: &str) -> Vec<String> {
    match crate::backup_config::parse_secrets(yaml) {
        Ok(secrets) => {
            let mut names: Vec<String> = secrets.credentials.into_keys().collect();
            names.sort();
            names
        }
        Err(_) => vec![],
    }
}

pub fn list_profiles(secrets_path: &str) -> Vec<String> {
    match crate::backup_config::decrypt_sops_file(secrets_path) {
        Ok(decrypted) => parse_profile_names(&decrypted),
        Err(_) => vec![],
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
    }

    #[test]
    fn detect_s3_plus_https_is_generic() {
        assert_eq!(detect_url_type("s3+https://nas.example.com/bucket"), CredentialType::Generic);
    }

    #[test]
    fn detect_generic_unknown() {
        assert_eq!(detect_url_type("sftp:user@host/path"), CredentialType::Generic);
        assert_eq!(detect_url_type("rclone:remote:/path"), CredentialType::Generic);
    }

    #[test]
    fn suggest_b2_bare_bucket() {
        assert_eq!(suggest_profile_name("b2:my-bucket"), "b2");
    }

    #[test]
    fn suggest_s3_amazonaws() {
        assert_eq!(suggest_profile_name("s3:https://s3.amazonaws.com/bucket"), "s3");
    }

    #[test]
    fn suggest_rustfs_with_subdomain() {
        assert_eq!(
            suggest_profile_name("rustfs:https://rustfs.cinnamon-trout.ts.net/bucket"),
            "rustfs"
        );
    }

    #[test]
    fn suggest_url_with_ip() {
        assert_eq!(
            suggest_profile_name("rustfs:https://192.168.1.1/bucket"),
            "192.168.1.1"
        );
    }

    #[test]
    fn suggest_url_with_port() {
        assert_eq!(suggest_profile_name("s3+https://nas:9000/bucket"), "nas");
    }

    #[test]
    fn suggest_single_label_host() {
        assert_eq!(suggest_profile_name("rustfs:https://nas/bucket"), "nas");
    }

    #[test]
    fn parse_profiles_returns_sorted_names() {
        let yaml = "restic_password: secret\ncredentials:\n  nas:\n    FOO: bar\n  aws:\n    AWS_ACCESS_KEY_ID: key\n";
        let profiles = parse_profile_names(yaml);
        assert_eq!(profiles, vec!["aws", "nas"]);
    }

    #[test]
    fn parse_profiles_empty_when_no_credentials_key() {
        let profiles = parse_profile_names("restic_password: secret\n");
        assert!(profiles.is_empty());
    }

    #[test]
    fn parse_profiles_empty_on_invalid_yaml() {
        let profiles = parse_profile_names("not: valid: yaml: [");
        assert!(profiles.is_empty());
    }
}
