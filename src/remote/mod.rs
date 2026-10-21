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
use std::cell::Cell;
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

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

/// Parses an upload rate such as `20MiB`, `5M`, `500k` or `2MB/s` into KiB per
/// second (at least 1). Every unit is binary, so `M`, `MB` and `MiB` all mean
/// 1024 KiB: that is how restic, mc and rclone read them.
pub fn parse_rate_kib(text: &str) -> Result<u64, String> {
    let bad = || format!("invalid rate '{text}' (use e.g. 500k, 5M or 20MiB, per second)");
    let lower = text.trim().to_lowercase();
    let lower = lower.strip_suffix("/s").unwrap_or(&lower);
    let lower = lower.strip_suffix('b').map_or(lower, |l| l.strip_suffix('i').unwrap_or(l));
    let (number, unit_kib) = match lower.chars().last() {
        Some('k') => (&lower[..lower.len() - 1], 1.0),
        Some('m') => (&lower[..lower.len() - 1], 1024.0),
        Some('g') => (&lower[..lower.len() - 1], 1024.0 * 1024.0),
        _ => return Err(bad()),
    };
    match number.trim().parse::<f64>() {
        Ok(n) if n > 0.0 && n.is_finite() => Ok(((n * unit_kib).ceil() as u64).max(1)),
        _ => Err(bad()),
    }
}

/// Text of the error `run_streaming` returns when it stops a transfer at its
/// deadline; callers tell a closed sync window from a real failure by it.
pub const WINDOW_CLOSED: &str = "the sync window closed";

/// How long a transfer gets to exit after SIGTERM before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(30);

thread_local! {
    static DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Makes every `run_streaming` on this thread stop its process at `deadline`
/// (`None`: no limit). Syncs run one after another on one thread.
pub fn set_deadline(deadline: Option<Instant>) {
    DEADLINE.with(|d| d.set(deadline));
}

/// Asks `pid` to exit, as Ctrl+C would. restic, b2, mc, rclone and aws all
/// stop cleanly on SIGTERM, and every sync here resumes where it left off.
fn terminate(pid: u32) {
    let _ = Command::new("kill").args(["-TERM", &pid.to_string()]).stdout(Stdio::null()).stderr(Stdio::null()).status();
}

/// Runs `cmd` with stdout inherited, echoing stderr live while also
/// collecting it. `Err` means the process could not be run or waited on, or
/// that it was stopped at the deadline (see [`WINDOW_CLOSED`]); otherwise
/// returns its exit status and everything it wrote to stderr.
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

    let deadline = DEADLINE.with(Cell::get);
    let mut stopped_at: Option<Instant> = None;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| format!("failed to wait for {name}: {e}"))? {
            break status;
        }
        let now = Instant::now();
        match stopped_at {
            None if deadline.is_some_and(|d| now >= d) => {
                eprintln!("[i] {WINDOW_CLOSED}; stopping {name} (the next run picks up where this one left off)");
                terminate(child.id());
                stopped_at = Some(now);
            }
            Some(t) if now.duration_since(t) >= STOP_GRACE => {
                let _ = child.kill();
            }
            _ => {}
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let stderr = stderr_thread.join().unwrap_or_default();
    if stopped_at.is_some() && !status.success() {
        return Err(format!("{WINDOW_CLOSED}; {name} was stopped before it finished"));
    }
    Ok((status, stderr))
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
    } else if remote.url.starts_with("s3:") {
        Ok(Box::new(S3Backend::from_remote(remote)?))
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

    #[cfg(unix)]
    #[test]
    fn run_streaming_stops_a_process_at_the_deadline() {
        // Other tests replace PATH while they run; wait for them.
        let _path_guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_deadline(Some(Instant::now() + Duration::from_millis(300)));
        let mut cmd = Command::new("sleep");
        cmd.arg("30");
        let started = Instant::now();
        let result = run_streaming(cmd, "sleep");
        set_deadline(None);
        assert!(result.unwrap_err().contains(WINDOW_CLOSED));
        assert!(started.elapsed() < Duration::from_secs(10), "should stop promptly, took {:?}", started.elapsed());
    }

    #[cfg(unix)]
    #[test]
    fn run_streaming_without_a_deadline_runs_to_completion() {
        // Other tests replace PATH while they run; wait for them.
        let _path_guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_deadline(None);
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "echo oops >&2; exit 3"]);
        let (status, stderr) = run_streaming(cmd, "sh").unwrap();
        assert_eq!(status.code(), Some(3));
        assert!(stderr.contains("oops"));
    }

    #[cfg(unix)]
    #[test]
    fn a_process_that_finishes_before_the_deadline_is_not_stopped() {
        // Other tests replace PATH while they run; wait for them.
        let _path_guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_deadline(Some(Instant::now() + Duration::from_secs(60)));
        let result = run_streaming(Command::new("true"), "true");
        set_deadline(None);
        assert!(result.unwrap().0.success());
    }

    #[test]
    fn parses_rates_into_kib_per_second() {
        for (text, kib) in [
            ("500k", 500),
            ("500KiB", 500),
            ("5M", 5120),
            ("5MB/s", 5120),
            ("20MiB", 20480),
            ("20 mib/s", 20480),
            ("1.5M", 1536),
            ("1G", 1_048_576),
            ("0.0001k", 1),
        ] {
            assert_eq!(parse_rate_kib(text), Ok(kib), "{text}");
        }
        for bad in ["", "fast", "M", "0M", "-5M", "5", "5T", "1,5M", "NaNM"] {
            assert!(parse_rate_kib(bad).is_err(), "{bad}");
        }
    }
}
