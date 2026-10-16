//! `vivo schedule`: runs vivo daily without the user writing launchd, systemd
//! or cron config by hand.
//!
//! macOS gets a launchd agent, Linux a systemd user timer (falling back to a
//! marked block in the user's crontab when systemd isn't running). The
//! generated entries run plain `vivo [task]` with a `PATH` that includes the
//! directories of the tools vivo shells out to, because schedulers don't
//! inherit the user's shell environment.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::{env, fs};

use serde::{Deserialize, Serialize};

pub const LABEL: &str = "dev.vivo.backup";
const CRON_BEGIN: &str = "# >>> vivo backup (managed by `vivo schedule`) >>>";
const CRON_END: &str = "# <<< vivo backup <<<";
const TOOLS: &[&str] = &["restic", "sops", "age-keygen", "mc", "aws", "rclone", "b2"];
const DEFAULT_PATH_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/bin"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    Launchd,
    Systemd,
    Cron,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::Launchd => "launchd",
            Backend::Systemd => "systemd timer",
            Backend::Cron => "cron",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Schedule {
    pub vivo_bin: PathBuf,
    pub hour: u8,
    pub minute: u8,
    /// Task to run; `None` runs the config's default task.
    pub task: Option<String>,
    pub path_env: String,
    /// The paths this invocation resolved (config, secrets, age key, state
    /// dir), pinned so the job uses exactly the files the user's shell does.
    pub pinned_env: Vec<(String, String)>,
    pub log: PathBuf,
}

/// Parses `HH:MM` (24 hour).
pub fn parse_time(s: &str) -> Result<(u8, u8), String> {
    let bad = || format!("'{s}' is not a time of day — use 24-hour HH:MM, e.g. 02:00");
    let (h, m) = s.trim().split_once(':').ok_or_else(bad)?;
    let (h, m): (u8, u8) = (h.parse().map_err(|_| bad())?, m.parse().map_err(|_| bad())?);
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok((h, m))
}

/// Directories to put on the scheduled job's `PATH`: where each tool vivo may
/// call lives right now, then the usual system locations.
pub fn path_for_tools(current_path: &str) -> String {
    let mut dirs: Vec<String> = Vec::new();
    let mut push = |d: String| {
        if !d.is_empty() && !dirs.contains(&d) {
            dirs.push(d);
        }
    };
    for tool in TOOLS {
        for dir in env::split_paths(current_path) {
            if dir.join(tool).is_file() {
                push(dir.to_string_lossy().into_owned());
                break;
            }
        }
    }
    for d in DEFAULT_PATH_DIRS {
        push((*d).to_string());
    }
    dirs.join(":")
}

impl Schedule {
    /// `config_path` is the config this invocation resolved. It and the other
    /// resolved locations are written into the job explicitly, rather than
    /// guessing which environment variables the user happened to set.
    pub fn new(task: Option<String>, hour: u8, minute: u8, config_path: &str) -> Result<Schedule, String> {
        let vivo_bin = env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|e| format!("could not locate the vivo binary: {e}"))?;
        Ok(Schedule {
            vivo_bin,
            hour,
            minute,
            task,
            path_env: path_for_tools(&env::var("PATH").unwrap_or_default()),
            pinned_env: [
                ("VIVO_BACKUP_CONFIG", config_path.to_string()),
                ("VIVO_BACKUP_SECRETS", crate::secrets_path_from()),
                ("SOPS_AGE_KEY_FILE", crate::age_keys_path()),
                ("VIVO_STATE_DIR", crate::state::state_dir().to_string_lossy().into_owned()),
            ]
            .map(|(k, v)| (k.to_string(), v))
            .to_vec(),
            log: crate::state::log_path(),
        })
    }

    pub fn time(&self) -> String {
        format!("{:02}:{:02}", self.hour, self.minute)
    }

    /// The command line the job runs: the binary, then the task if any.
    fn command(&self) -> Vec<String> {
        std::iter::once(self.vivo_bin.to_string_lossy().into_owned()).chain(self.task.clone()).collect()
    }

    /// `PATH` first, then the extras.
    fn env_pairs(&self) -> Vec<(&str, &str)> {
        std::iter::once(("PATH", self.path_env.as_str()))
            .chain(self.pinned_env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .collect()
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn render_plist(s: &Schedule) -> String {
    let program: String = s.command().iter().map(|a| format!("    <string>{}</string>\n", xml_escape(a))).collect();
    let log = xml_escape(&s.log.to_string_lossy());
    let env: String = s
        .env_pairs()
        .iter()
        .map(|(k, v)| format!("<key>{}</key><string>{}</string>", xml_escape(k), xml_escape(v)))
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{program}  </array>
  <key>EnvironmentVariables</key>
  <dict>{env}</dict>
  <key>StartCalendarInterval</key>
  <dict><key>Hour</key><integer>{hour}</integer><key>Minute</key><integer>{minute}</integer></dict>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        hour = s.hour,
        minute = s.minute,
    )
}

/// systemd splits `ExecStart` on whitespace, so quote anything that has some.
fn systemd_quote(s: &str) -> String {
    if s.contains(char::is_whitespace) || s.contains('"') {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

pub fn render_service(s: &Schedule) -> String {
    let exec = s.command().iter().map(|a| systemd_quote(a)).collect::<Vec<_>>().join(" ");
    let env: String = s
        .env_pairs()
        .iter()
        .map(|(k, v)| format!("Environment=\"{k}={}\"\n", v.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!(
        "[Unit]\nDescription=vivo backup\n\n[Service]\nType=oneshot\n{env}ExecStart={exec}\nStandardOutput=append:{log}\nStandardError=append:{log}\n",
        log = s.log.display(),
    )
}

pub fn render_timer(s: &Schedule) -> String {
    format!(
        "[Unit]\nDescription=Daily vivo backup\n\n[Timer]\nOnCalendar=*-*-* {:02}:{:02}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        s.hour, s.minute
    )
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn render_cron_block(s: &Schedule) -> String {
    let cmd = s.command().iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
    let log = shell_quote(&s.log.to_string_lossy());
    let env: String = s.env_pairs().iter().map(|(k, v)| format!("{k}={} ", shell_quote(v))).collect();
    format!("{CRON_BEGIN}\n{} {} * * * {env}{cmd} >> {log} 2>&1\n{CRON_END}\n", s.minute, s.hour)
}

/// `crontab` text with any vivo block removed.
pub fn strip_cron_block(crontab: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for line in crontab.lines() {
        if line == CRON_BEGIN {
            inside = true;
        } else if line == CRON_END {
            inside = false;
        } else if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

fn home() -> PathBuf {
    PathBuf::from(env::var("HOME").unwrap_or_default())
}

pub fn plist_path() -> PathBuf {
    home().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
}

fn systemd_dir() -> PathBuf {
    crate::xdg_config_home().join("systemd/user")
}

fn quiet(program: &str, args: &[&str]) -> Option<std::process::Output> {
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .ok()
}

fn run_ok(program: &str, args: &[&str]) -> Result<(), String> {
    let out = quiet(program, args).ok_or_else(|| format!("could not run {program}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("`{program} {}` failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn systemd_available() -> bool {
    quiet("systemctl", &["--user", "show-environment"]).is_some_and(|o| o.status.success())
}

pub fn detect_backend() -> Backend {
    if cfg!(target_os = "macos") {
        Backend::Launchd
    } else if systemd_available() {
        Backend::Systemd
    } else {
        Backend::Cron
    }
}

fn uid() -> Result<String, String> {
    let out = quiet("id", &["-u"]).ok_or("could not run `id -u`")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn read_crontab() -> String {
    quiet("crontab", &["-l"])
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn write_crontab(text: &str) -> Result<(), String> {
    use std::io::Write;
    let mut child = Command::new("crontab")
        .arg("-")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run crontab: {e}"))?;
    child
        .stdin
        .take()
        .ok_or("could not open crontab's stdin")?
        .write_all(text.as_bytes())
        .map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("crontab exited with {status}"))
    }
}

fn write_file(path: &Path, text: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

/// Installs (or replaces) the daily schedule. Returns the file or entry that was written.
pub fn install(s: &Schedule, backend: Backend) -> Result<String, String> {
    if let Some(dir) = s.log.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let written: Result<String, String> = match backend {
        Backend::Launchd => {
            let path = plist_path();
            let domain = format!("gui/{}", uid()?);
            // Replace a loaded copy; failing here just means there wasn't one.
            let _ = run_ok("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
            write_file(&path, &render_plist(s))?;
            run_ok("launchctl", &["bootstrap", &domain, &path.to_string_lossy()])?;
            Ok(path.display().to_string())
        }
        Backend::Systemd => {
            let dir = systemd_dir();
            write_file(&dir.join("vivo.service"), &render_service(s))?;
            write_file(&dir.join("vivo.timer"), &render_timer(s))?;
            run_ok("systemctl", &["--user", "daemon-reload"])?;
            run_ok("systemctl", &["--user", "enable", "--now", "vivo.timer"])?;
            Ok(dir.join("vivo.timer").display().to_string())
        }
        Backend::Cron => {
            let mut text = strip_cron_block(&read_crontab());
            text.push_str(&render_cron_block(s));
            write_crontab(&text)?;
            Ok("your crontab".to_string())
        }
    };
    let written = written?;
    // Only a convenience for `status`; the schedule itself is already live.
    let _ = write_record(&Record { backend, at: s.time(), task: s.task.clone() });
    Ok(written)
}

/// Removes the schedule from every backend. Returns what was removed.
pub fn remove() -> Result<Vec<Backend>, String> {
    let mut removed = Vec::new();

    let plist = plist_path();
    if plist.exists() {
        if let Ok(uid) = uid() {
            let _ = run_ok("launchctl", &["bootout", &format!("gui/{uid}/{LABEL}")]);
        }
        fs::remove_file(&plist).map_err(|e| format!("could not remove {}: {e}", plist.display()))?;
        removed.push(Backend::Launchd);
    }

    let dir = systemd_dir();
    if dir.join("vivo.timer").exists() {
        let _ = run_ok("systemctl", &["--user", "disable", "--now", "vivo.timer"]);
        for f in ["vivo.timer", "vivo.service"] {
            let _ = fs::remove_file(dir.join(f));
        }
        let _ = run_ok("systemctl", &["--user", "daemon-reload"]);
        removed.push(Backend::Systemd);
    }

    let crontab = read_crontab();
    if crontab.contains(CRON_BEGIN) {
        write_crontab(&strip_cron_block(&crontab))?;
        removed.push(Backend::Cron);
    }
    let _ = fs::remove_file(record_path());
    Ok(removed)
}

/// What `install` recorded, so `installed` can report the time and task
/// without parsing the files it rendered.
#[derive(Debug, Serialize, Deserialize)]
struct Record {
    backend: Backend,
    at: String,
    task: Option<String>,
}

fn record_path() -> PathBuf {
    crate::state::state_dir().join("schedule.json")
}

fn write_record(record: &Record) -> Result<(), String> {
    let text = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
    write_file(&record_path(), &text)
}

fn read_record() -> Option<Record> {
    serde_json::from_str(&fs::read_to_string(record_path()).ok()?).ok()
}

#[derive(Debug)]
pub struct Installed {
    pub backend: Backend,
    /// The time of day and task it was scheduled with, when recorded.
    pub at: Option<String>,
    pub task: Option<String>,
}

/// The installed schedule, if any. Existence comes from the scheduler's own
/// artifact; the details from the record written at install time.
pub fn installed() -> Option<Installed> {
    let backend = if plist_path().exists() {
        Backend::Launchd
    } else if systemd_dir().join("vivo.timer").exists() {
        Backend::Systemd
    // vivo only ever installs a cron entry where there is no launchd.
    } else if !cfg!(target_os = "macos") && read_crontab().contains(CRON_BEGIN) {
        Backend::Cron
    } else {
        return None;
    };
    let record = read_record().filter(|r| r.backend == backend);
    Some(Installed {
        backend,
        at: record.as_ref().map(|r| r.at.clone()),
        task: record.and_then(|r| r.task),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(task: Option<&str>) -> Schedule {
        Schedule {
            vivo_bin: PathBuf::from("/usr/local/bin/vivo"),
            hour: 2,
            minute: 5,
            task: task.map(str::to_string),
            path_env: "/opt/homebrew/bin:/usr/bin".to_string(),
            pinned_env: vec![("VIVO_BACKUP_CONFIG".to_string(), "/etc/my vivo.kdl".to_string())],
            log: PathBuf::from("/home/u/.local/state/vivo/vivo.log"),
        }
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_time("02:00"), Ok((2, 0)));
        assert_eq!(parse_time(" 23:59 "), Ok((23, 59)));
        assert!(parse_time("24:00").is_err());
        assert!(parse_time("2am").is_err());
        assert!(parse_time("12:60").is_err());
    }

    #[test]
    fn plist_has_time_path_and_task() {
        let plist = render_plist(&sample(Some("docs & more")));
        assert!(plist.contains("<key>Hour</key><integer>2</integer><key>Minute</key><integer>5</integer>"));
        assert!(plist.contains("<string>/usr/local/bin/vivo</string>"));
        assert!(plist.contains("<string>docs &amp; more</string>"));
        assert!(plist.contains("/opt/homebrew/bin:/usr/bin"));
        assert!(plist.contains("<key>VIVO_BACKUP_CONFIG</key><string>/etc/my vivo.kdl</string>"));
        assert!(plist.contains("vivo.log"));
    }

    #[test]
    fn systemd_units_quote_and_schedule() {
        let mut s = sample(Some("my task"));
        s.vivo_bin = PathBuf::from("/opt/my apps/vivo");
        assert!(render_service(&s).contains("ExecStart=\"/opt/my apps/vivo\" \"my task\""));
        assert!(render_service(&s).contains("Environment=\"VIVO_BACKUP_CONFIG=/etc/my vivo.kdl\""));
        assert!(render_timer(&s).contains("OnCalendar=*-*-* 02:05:00"));
        assert!(render_timer(&s).contains("Persistent=true"));
    }

    #[test]
    fn cron_block_roundtrips_without_touching_other_lines() {
        let existing = "MAILTO=me\n0 * * * * other-job\n";
        let with = format!("{existing}{}", render_cron_block(&sample(None)));
        assert!(with.contains("5 2 * * * PATH='/opt/homebrew/bin:/usr/bin' VIVO_BACKUP_CONFIG='/etc/my vivo.kdl' '/usr/local/bin/vivo' >>"));
        assert_eq!(strip_cron_block(&with), existing);
        // Re-installing replaces rather than duplicates.
        let again = format!("{}{}", strip_cron_block(&with), render_cron_block(&sample(None)));
        assert_eq!(again.matches(CRON_BEGIN).count(), 1);
    }

    #[test]
    fn path_lists_tool_dirs_first_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("restic"), "").unwrap();
        fs::write(dir.path().join("sops"), "").unwrap();
        let path = path_for_tools(&dir.path().to_string_lossy());
        let parts: Vec<&str> = path.split(':').collect();
        assert_eq!(parts[0], dir.path().to_string_lossy());
        assert_eq!(parts.iter().filter(|p| **p == "/usr/bin").count(), 1);
    }

    #[test]
    fn record_roundtrips() {
        let r = Record { backend: Backend::Systemd, at: "02:05".into(), task: Some("docs".into()) };
        let back: Record = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!((back.backend, back.at.as_str(), back.task.as_deref()), (Backend::Systemd, "02:05", Some("docs")));
        assert!(serde_json::to_string(&r).unwrap().contains("\"systemd\""));
    }
}
