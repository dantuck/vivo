use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;
use std::{env, fs, process};

use crate::backup_config::{decrypt_sops_file, parse_secrets, BackupConfig};
use crate::config::{xdg_config_home, Secrets};

pub enum CheckStatus {
    Ok,
    Warn,
    Fail,
}

pub struct CheckResult {
    pub label: String,
    pub status: CheckStatus,
    pub detail: Option<String>,
}

pub fn print_result(r: &CheckResult) {
    let tag = match r.status {
        CheckStatus::Ok   => "  [ok]  ",
        CheckStatus::Warn => "  [warn]",
        CheckStatus::Fail => "  [fail]",
    };
    match &r.detail {
        Some(d) => println!("{tag} {} — {d}", r.label),
        None    => println!("{tag} {}", r.label),
    }
}

pub fn tool_version(name: &str, version_flag: &str) -> Option<String> {
    process::Command::new(name)
        .arg(version_flag)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| {
            String::from_utf8(o.stdout)
                .ok()
                .and_then(|s| s.lines().next().map(str::to_string))
        })
}

pub fn check_tool_present(name: &str, version_flag: &str, install_hint: &str) -> CheckResult {
    match tool_version(name, version_flag) {
        Some(v) => CheckResult {
            label: format!("{name} ({v})"),
            status: CheckStatus::Ok,
            detail: None,
        },
        None => CheckResult {
            label: name.to_string(),
            status: CheckStatus::Fail,
            detail: Some(install_hint.to_string()),
        },
    }
}

pub fn check_age_key() -> CheckResult {
    let path = if let Ok(p) = env::var("SOPS_AGE_KEY_FILE") {
        p
    } else {
        xdg_config_home()
            .join("sops/age/keys.txt")
            .to_string_lossy()
            .into_owned()
    };
    if Path::new(&path).exists() {
        CheckResult {
            label: format!("age key ({path})"),
            status: CheckStatus::Ok,
            detail: None,
        }
    } else {
        CheckResult {
            label: "age key".to_string(),
            status: CheckStatus::Fail,
            detail: Some(format!(
                "not found at {path} — run: age-keygen -o {path}"
            )),
        }
    }
}

pub fn check_config(config_path: &str) -> CheckResult {
    let label = format!("config ({config_path})");
    match fs::read_to_string(config_path) {
        Err(e) => CheckResult {
            label,
            status: CheckStatus::Fail,
            detail: Some(format!("{e} — run `vivo config init`")),
        },
        Ok(content) => match knuffel::parse::<BackupConfig>(config_path, &content) {
            Err(e) => CheckResult {
                label,
                status: CheckStatus::Fail,
                detail: Some(e.to_string()),
            },
            Ok(_) => CheckResult {
                label,
                status: CheckStatus::Ok,
                detail: None,
            },
        },
    }
}

pub fn check_secrets(secrets_path: &str) -> Option<Secrets> {
    decrypt_sops_file(secrets_path)
        .ok()
        .and_then(|yaml| parse_secrets(&yaml).ok())
}

pub fn check_secrets_present(secrets_path: &str) -> CheckResult {
    let label = format!("secrets ({secrets_path})");
    match decrypt_sops_file(secrets_path) {
        Err(e) => CheckResult {
            label,
            status: CheckStatus::Fail,
            detail: Some(format!("{e} — run `vivo secrets init`")),
        },
        Ok(yaml) => match parse_secrets(&yaml) {
            Err(e) => CheckResult {
                label,
                status: CheckStatus::Fail,
                detail: Some(format!("parse error: {e}")),
            },
            Ok(_) => CheckResult {
                label,
                status: CheckStatus::Ok,
                detail: None,
            },
        },
    }
}

pub fn check_restic_password(secrets: &Secrets) -> CheckResult {
    if secrets.restic_password.is_empty() || secrets.restic_password == "change-me" {
        CheckResult {
            label: "restic_password".to_string(),
            status: CheckStatus::Fail,
            detail: Some("not set — run `vivo secrets edit`".to_string()),
        }
    } else {
        CheckResult {
            label: "restic_password".to_string(),
            status: CheckStatus::Ok,
            detail: None,
        }
    }
}

fn run_with_timeout(cmd: &mut process::Command, timeout: Duration) -> Result<bool, String> {
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            return Ok(false);
        }
        match child.try_wait().map_err(|e| e.to_string())? {
            Some(s) => return Ok(s.success()),
            None => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

pub fn check_remote_connectivity(
    url: &str,
    creds_name: &str,
    credentials: &HashMap<String, HashMap<String, String>>,
    restic_password: &str,
) -> CheckResult {
    let label = format!("remote {url}");
    let creds = match credentials.get(creds_name) {
        None => {
            return CheckResult {
                label,
                status: CheckStatus::Warn,
                detail: Some(format!("credentials profile '{creds_name}' not in secrets")),
            }
        }
        Some(c) => c,
    };

    let timeout = Duration::from_secs(15);

    if url.starts_with("b2:") {
        let bucket = url.strip_prefix("b2:").unwrap_or("").split(':').next().unwrap_or("");
        let mut cmd = process::Command::new("b2");
        cmd.args(["ls", bucket])
            .envs(creds)
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null());
        match run_with_timeout(&mut cmd, timeout) {
            Ok(true) => CheckResult { label, status: CheckStatus::Ok, detail: None },
            Ok(false) => CheckResult {
                label,
                status: CheckStatus::Warn,
                detail: Some("connection timed out or failed — check B2 credentials".to_string()),
            },
            Err(e) => CheckResult { label, status: CheckStatus::Warn, detail: Some(e) },
        }
    } else if url.starts_with("s3:") {
        let mut cmd = process::Command::new("restic");
        cmd.args(["snapshots", "--repo", url, "--no-lock"])
            .envs(creds)
            .env("RESTIC_PASSWORD", restic_password)
            .stdout(process::Stdio::null())
            .stderr(process::Stdio::null());
        match run_with_timeout(&mut cmd, timeout) {
            Ok(true) => CheckResult { label, status: CheckStatus::Ok, detail: None },
            Ok(false) => CheckResult {
                label,
                status: CheckStatus::Warn,
                detail: Some("connection timed out or failed — check S3 credentials and repo init".to_string()),
            },
            Err(e) => CheckResult { label, status: CheckStatus::Warn, detail: Some(e) },
        }
    } else {
        CheckResult {
            label,
            status: CheckStatus::Warn,
            detail: Some("unsupported remote prefix — skipping connectivity check".to_string()),
        }
    }
}

pub fn run_doctor(config_path: &str, secrets_path: &str) -> i32 {
    let mut results: Vec<CheckResult> = Vec::new();
    let mut required_failures = 0u32;
    let mut warnings = 0u32;

    let restic = check_tool_present("restic", "version", "install from https://restic.net");
    let sops   = check_tool_present("sops", "--version", "install from https://github.com/getsops/sops");
    let age    = check_age_key();
    let config = check_config(config_path);
    let secrets_result = check_secrets_present(secrets_path);

    for r in [&restic, &sops, &age, &config, &secrets_result] {
        if matches!(r.status, CheckStatus::Fail) {
            required_failures += 1;
        }
    }

    results.push(restic);
    results.push(sops);
    results.push(age);
    results.push(config);
    results.push(secrets_result);

    let maybe_secrets = check_secrets(secrets_path);
    if let Some(ref s) = maybe_secrets {
        let pw = check_restic_password(s);
        if matches!(pw.status, CheckStatus::Fail) {
            required_failures += 1;
        }
        results.push(pw);
    }

    if let (Ok(content), Some(ref secrets)) = (fs::read_to_string(config_path), maybe_secrets) {
        if let Ok(backup_config) = knuffel::parse::<BackupConfig>(config_path, &content) {
            for (url, creds_name) in backup_config.all_remotes() {
                let r = check_remote_connectivity(
                    url,
                    creds_name,
                    &secrets.credentials,
                    &secrets.restic_password,
                );
                if matches!(r.status, CheckStatus::Warn) {
                    warnings += 1;
                }
                results.push(r);
            }
        }
    }

    for r in &results {
        print_result(r);
    }

    println!();
    match (required_failures, warnings) {
        (0, 0) => println!("All checks passed."),
        (0, w) => println!("{w} warning(s). Run `vivo doctor` again after resolving."),
        (f, 0) => println!("{f} required check(s) failed. Fix the issues above and re-run `vivo doctor`."),
        (f, w) => println!("{f} required check(s) failed, {w} warning(s). Fix required checks first."),
    }

    if required_failures > 0 { 1 } else { 0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn check_tool_present_fails_for_nonexistent() {
        let r = check_tool_present("__no_such_tool_xyz__", "--version", "hint");
        assert!(matches!(r.status, CheckStatus::Fail));
    }

    #[test]
    fn check_age_key_fails_when_missing() {
        env::set_var("SOPS_AGE_KEY_FILE", "/tmp/__vivo_test_no_such_key__.txt");
        let r = check_age_key();
        assert!(matches!(r.status, CheckStatus::Fail));
    }

    #[test]
    fn check_age_key_ok_when_file_exists() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        env::set_var("SOPS_AGE_KEY_FILE", tmp.path().to_str().unwrap());
        let r = check_age_key();
        assert!(matches!(r.status, CheckStatus::Ok));
    }

    #[test]
    fn check_config_fails_for_missing_file() {
        let r = check_config("/tmp/__vivo_no_such_config__.kdl");
        assert!(matches!(r.status, CheckStatus::Fail));
    }

    #[test]
    fn check_config_fails_for_invalid_kdl() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, "this is not valid kdl {{{{").unwrap();
        let r = check_config(f.path().to_str().unwrap());
        assert!(matches!(r.status, CheckStatus::Fail));
    }

    #[test]
    fn check_config_ok_for_valid_kdl() {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        writeln!(f, r#"default-task "backup"
tasks {{
    task "backup" {{
        command "echo hi"
    }}
}}"#).unwrap();
        let r = check_config(f.path().to_str().unwrap());
        assert!(matches!(r.status, CheckStatus::Ok));
    }

    #[test]
    fn check_restic_password_fails_for_placeholder() {
        let secrets = Secrets {
            restic_password: "change-me".to_string(),
            credentials: HashMap::new(),
        };
        let r = check_restic_password(&secrets);
        assert!(matches!(r.status, CheckStatus::Fail));
    }

    #[test]
    fn check_restic_password_ok_for_real_password() {
        let secrets = Secrets {
            restic_password: "hunter2".to_string(),
            credentials: HashMap::new(),
        };
        let r = check_restic_password(&secrets);
        assert!(matches!(r.status, CheckStatus::Ok));
    }
}
