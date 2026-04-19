use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{fs, io};

use crate::config::xdg_config_home;

pub const GITHUB_REPO_OWNER: &str = "dantuck";
pub const GITHUB_REPO_NAME: &str = "vivo";
const CHECK_INTERVAL_SECS: u64 = 86_400;

fn update_check_path() -> PathBuf {
    xdg_config_home().join("vivo/update-check")
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn is_check_due_at(path: &Path) -> bool {
    let content = fs::read_to_string(path).unwrap_or_default();
    let last: u64 = content.trim().parse().unwrap_or(0);
    now_secs().saturating_sub(last) >= CHECK_INTERVAL_SECS
}

pub(crate) fn record_check_at(path: &Path) -> io::Result<()> {
    if let Some(p) = path.parent() {
        fs::create_dir_all(p)?;
    }
    fs::write(path, now_secs().to_string())
}

fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim_start_matches('v');
    let mut parts = v.splitn(3, '.').map(|p| p.parse::<u64>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

fn is_newer(latest: &str, current: &str) -> bool {
    match (parse_version(latest), parse_version(current)) {
        (Some(l), Some(c)) => l > c,
        _ => false,
    }
}

/// Queries GitHub for the latest release. Returns the version string (without
/// 'v' prefix) if it is newer than the running binary. Network errors are
/// silently ignored.
pub fn check_update_available() -> Option<String> {
    let releases = self_update::backends::github::ReleaseList::configure()
        .repo_owner(GITHUB_REPO_OWNER)
        .repo_name(GITHUB_REPO_NAME)
        .build()
        .ok()?
        .fetch()
        .ok()?;

    let latest = releases.first()?;
    let latest_version = latest.version.trim_start_matches('v').to_string();
    let current = env!("CARGO_PKG_VERSION");

    if is_newer(&latest_version, current) {
        Some(latest_version)
    } else {
        None
    }
}

/// Rate-limited check: queries at most once per 24 hours. Silently ignores
/// all errors.
pub fn maybe_check_update() -> Option<String> {
    let path = update_check_path();
    if !is_check_due_at(&path) {
        return None;
    }
    let result = check_update_available();
    let _ = record_check_at(&path);
    result
}

pub fn print_update_notice(latest: &str) {
    let current = env!("CARGO_PKG_VERSION");
    println!();
    println!("A new version of vivo is available: v{latest} (you have v{current})");
    println!("Run `vivo update` to upgrade.");
}

/// Downloads and applies the latest release. If dry_run is true, prints what
/// would happen without replacing the binary.
pub fn apply_update(dry_run: bool) -> Result<(), Box<dyn std::error::Error>> {
    if dry_run {
        let releases = self_update::backends::github::ReleaseList::configure()
            .repo_owner(GITHUB_REPO_OWNER)
            .repo_name(GITHUB_REPO_NAME)
            .build()?
            .fetch()?;
        let latest = releases.first().ok_or("no releases found")?;
        println!(
            "[dry-run] would update vivo v{} → v{}",
            env!("CARGO_PKG_VERSION"),
            latest.version.trim_start_matches('v')
        );
        return Ok(());
    }

    let status = self_update::backends::github::Update::configure()
        .repo_owner(GITHUB_REPO_OWNER)
        .repo_name(GITHUB_REPO_NAME)
        .bin_name("vivo")
        .show_download_progress(true)
        .current_version(env!("CARGO_PKG_VERSION"))
        .build()?
        .update()?;

    if status.updated() {
        println!("Updated to vivo v{}", status.version());
    } else {
        println!("vivo v{} is already up to date.", env!("CARGO_PKG_VERSION"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_due_when_no_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_check_due_at(&dir.path().join("update-check")));
    }

    #[test]
    fn check_not_due_when_just_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check");
        record_check_at(&path).unwrap();
        assert!(!is_check_due_at(&path));
    }

    #[test]
    fn check_due_when_timestamp_is_old() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update-check");
        let two_days_ago = now_secs().saturating_sub(172_800);
        fs::write(&path, two_days_ago.to_string()).unwrap();
        assert!(is_check_due_at(&path));
    }

    #[test]
    fn parse_version_handles_v_prefix() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
    }

    #[test]
    fn parse_version_returns_none_for_invalid() {
        assert_eq!(parse_version("not-a-version"), None);
    }

    #[test]
    fn is_newer_returns_true_for_higher_minor() {
        assert!(is_newer("0.2.0", "0.1.0"));
    }

    #[test]
    fn is_newer_returns_false_for_same_version() {
        assert!(!is_newer("0.1.0", "0.1.0"));
    }

    #[test]
    fn is_newer_returns_false_for_older() {
        assert!(!is_newer("0.1.0", "0.2.0"));
    }
}
