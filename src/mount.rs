use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use inquire::Select;

use crate::doctor;

pub fn mount_point_path(mount_path: Option<&str>, task_name: &str) -> Result<(PathBuf, bool), String> {
    match mount_path {
        Some(p) => {
            let path = PathBuf::from(p);
            fs::create_dir_all(&path)
                .map_err(|e| format!("could not create mount point: {e}"))?;
            Ok((path, false))
        }
        None => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let path = PathBuf::from(format!("/tmp/vivo-mount-{task_name}-{ts}"));
            fs::create_dir_all(&path)
                .map_err(|e| format!("could not create temp mount point: {e}"))?;
            Ok((path, true))
        }
    }
}

pub fn check_mount_point_valid(path: &Path) -> doctor::CheckResult {
    let label = format!("mount point ({})", path.display());
    if !path.exists() {
        return doctor::CheckResult {
            label,
            status: doctor::CheckStatus::Warn,
            detail: Some("directory does not exist (will be created)".to_string()),
        };
    }
    match fs::read_dir(path) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                doctor::CheckResult {
                    label,
                    status: doctor::CheckStatus::Warn,
                    detail: Some("directory is not empty".to_string()),
                }
            } else {
                doctor::CheckResult {
                    label,
                    status: doctor::CheckStatus::Ok,
                    detail: None,
                }
            }
        }
        Err(e) => doctor::CheckResult {
            label,
            status: doctor::CheckStatus::Warn,
            detail: Some(format!("cannot read directory: {e}")),
        },
    }
}

pub fn check_repo_accessible(
    repo_url: &str,
    credentials: &HashMap<String, String>,
    restic_password: &str,
) -> doctor::CheckResult {
    let label = format!("repo {repo_url}");
    let timeout = std::time::Duration::from_secs(15);
    let mut cmd = Command::new("restic");
    cmd.args(["snapshots", "--repo", repo_url, "--no-lock"])
        .envs(credentials)
        .env("RESTIC_PASSWORD", restic_password)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    match doctor::run_with_timeout(&mut cmd, timeout) {
        Ok(true) => doctor::CheckResult { label, status: doctor::CheckStatus::Ok, detail: None },
        Ok(false) => doctor::CheckResult {
            label,
            status: doctor::CheckStatus::Warn,
            detail: Some("repo check timed out or failed — mount will attempt anyway".to_string()),
        },
        Err(e) => doctor::CheckResult {
            label,
            status: doctor::CheckStatus::Warn,
            detail: Some(format!("could not check repo: {e}")),
        },
    }
}

pub fn run_preflight(
    repo_url: &str,
    credentials: &HashMap<String, String>,
    restic_password: &str,
    explicit_mount_path: Option<&Path>,
) -> bool {
    let required = vec![
        doctor::check_tool_present("restic", "version", "install from https://restic.net"),
        doctor::check_fuse(),
    ];
    let mut warn_only = vec![check_repo_accessible(repo_url, credentials, restic_password)];
    if let Some(p) = explicit_mount_path {
        warn_only.push(check_mount_point_valid(p));
    }

    let mut required_failed = false;
    for r in &required {
        doctor::print_result(r);
        if matches!(r.status, doctor::CheckStatus::Fail) {
            required_failed = true;
        }
    }
    for r in &warn_only {
        doctor::print_result(r);
    }
    println!();
    !required_failed
}

pub fn run(config_path: &str, secrets_path: &str, mount_path: Option<&str>) -> Result<(), String> {
    let (config, secrets) = crate::repos::load(config_path, secrets_path)?;

    let mut targets = Vec::new();
    for task in crate::repos::backup_tasks(&config, None)? {
        targets.extend(crate::repos::targets_for(task, &secrets));
    }
    if targets.is_empty() {
        return Err("No backup tasks configured. Run `vivo task add` first.".to_string());
    }

    let labels: Vec<String> = targets.iter().map(|t| t.label()).collect();
    let selected = Select::new("Select repository to mount:", labels)
        .raw_prompt()
        .map_err(|_| "cancelled".to_string())?;
    let target = &targets[selected.index];

    let restic_url = &target.restic_url;
    let check_path = mount_path.map(Path::new);
    if !run_preflight(restic_url, &target.env, &secrets.restic_password, check_path) {
        return Err("pre-flight checks failed — fix the issues above and try again".to_string());
    }

    let (mount_point, owned) = mount_point_path(mount_path, &target.task)?;
    let mount_point_str = mount_point.to_string_lossy().to_string();

    println!("Mounting {restic_url} at {mount_point_str}");
    println!();
    println!("  Browse your backups:");
    println!("    {mount_point_str}/snapshots/latest/  (most recent)");
    println!("    {mount_point_str}/snapshots/         (all snapshots)");
    println!("    {mount_point_str}/hosts/             (by hostname)");
    println!();
    println!("Press Ctrl+C to unmount.");
    println!("  (close any open files in the mount first to avoid 'device busy' errors)");

    // Suppress default Ctrl+C exit so we can clean up the temp dir after restic exits.
    // restic also receives the signal and handles FUSE unmounting itself.
    ctrlc::set_handler(|| {}).map_err(|e| format!("could not set Ctrl+C handler: {e}"))?;

    let mut child = Command::new("restic")
        .args(["mount", "--repo", restic_url, &mount_point_str])
        .envs(&target.env)
        .spawn()
        .map_err(|e| format!("could not start restic: {e}"))?;

    let _ = child.wait();

    // restic may have failed to unmount if files were open (EBUSY). Attempt a lazy
    // unmount so the kernel detaches the FUSE mount even with open file handles.
    lazy_unmount(&mount_point_str);
    println!("Unmounted {mount_point_str}.");

    if owned {
        let _ = fs::remove_dir(&mount_point);
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn lazy_unmount(mount_point: &str) {
    // -u unmount, -z lazy (detach now, clean up when last handle closes)
    let _ = Command::new("fusermount")
        .args(["-u", "-z", mount_point])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(target_os = "macos")]
fn lazy_unmount(mount_point: &str) {
    let _ = Command::new("umount")
        .arg(mount_point)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn lazy_unmount(_mount_point: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mount_point_path_explicit_creates_dir() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("mount");
        let (path, owned) = mount_point_path(Some(subdir.to_str().unwrap()), "backup").unwrap();
        assert_eq!(path, subdir);
        assert!(!owned, "explicit path should not be owned");
        assert!(path.exists());
    }

    #[test]
    fn mount_point_path_auto_creates_temp() {
        let (path, owned) = mount_point_path(None, "mytask").unwrap();
        assert!(owned, "auto path should be owned");
        assert!(path.to_string_lossy().contains("vivo-mount-mytask-"));
        assert!(path.exists());
        let _ = std::fs::remove_dir(&path);
    }

    #[test]
    fn check_mount_point_valid_ok_for_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let r = check_mount_point_valid(dir.path());
        assert!(matches!(r.status, doctor::CheckStatus::Ok));
    }

    #[test]
    fn check_mount_point_valid_warns_for_nonempty_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("file.txt"), "x").unwrap();
        let r = check_mount_point_valid(dir.path());
        assert!(matches!(r.status, doctor::CheckStatus::Warn));
    }

    #[test]
    fn check_mount_point_valid_warns_for_missing_dir() {
        let r = check_mount_point_valid(Path::new("/tmp/__vivo_no_such_dir_xyz__"));
        assert!(matches!(r.status, doctor::CheckStatus::Warn));
    }

    #[test]
    fn check_repo_accessible_warns_for_nonexistent_repo() {
        let r = check_repo_accessible(
            "/tmp/__vivo_no_such_repo__",
            &HashMap::new(),
            "testpassword",
        );
        assert!(matches!(r.status, doctor::CheckStatus::Warn));
    }

    #[test]
    fn run_preflight_fails_when_restic_missing() {
        // Override PATH to empty so restic is not found
        let _path_guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", "");
        let ok = run_preflight("/tmp/norepo", &HashMap::new(), "pw", None);
        std::env::set_var("PATH", &original);
        assert!(!ok, "preflight should fail when restic is not on PATH");
    }
}
