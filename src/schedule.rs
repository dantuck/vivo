//! `vivo schedule`: runs vivo on a schedule without the user writing launchd,
//! systemd or cron config by hand.
//!
//! There is one scheduler unit per scheduled task (named from the task, see
//! [`slug`]), plus the original unnamed unit that runs the default task. A
//! task's schedule is declared in the config as `schedule "daily 02:00"` or
//! `schedule "weekly sun 03:00"`, and `vivo schedule apply` makes the installed
//! units match.
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

const DAYS: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// When a scheduled job fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Daily { hour: u8, minute: u8 },
    /// `day` counts from Sunday = 0.
    Weekly { day: u8, hour: u8, minute: u8 },
}

const DAY_NAMES: [&str; 7] = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];

/// A weekday as `mon` or `monday` (any case), counted from Sunday = 0.
pub fn parse_day(s: &str) -> Result<u8, String> {
    let lower = s.trim().to_lowercase();
    DAYS.iter()
        .position(|d| *d == lower)
        .or_else(|| DAY_NAMES.iter().position(|d| *d == lower))
        .map(|i| i as u8)
        .ok_or_else(|| format!("'{s}' is not a weekday — use sun, mon, tue, wed, thu, fri or sat"))
}

impl When {
    /// `daily HH:MM`, `weekly <day> HH:MM`, or a bare `HH:MM` (daily).
    pub fn parse(s: &str) -> Result<When, String> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        match parts.as_slice() {
            [time] | ["daily", time] => {
                let (hour, minute) = parse_time(time)?;
                Ok(When::Daily { hour, minute })
            }
            ["weekly", day, time] => {
                let (hour, minute) = parse_time(time)?;
                Ok(When::Weekly { day: parse_day(day)?, hour, minute })
            }
            _ => Err(format!("'{s}' is not a schedule — use `daily HH:MM` or `weekly <day> HH:MM`")),
        }
    }

    pub fn hm(&self) -> (u8, u8) {
        match *self {
            When::Daily { hour, minute } | When::Weekly { hour, minute, .. } => (hour, minute),
        }
    }

    /// The weekday (Sunday = 0) for a weekly schedule.
    fn day(&self) -> Option<u8> {
        match *self {
            When::Weekly { day, .. } => Some(day),
            When::Daily { .. } => None,
        }
    }
}

impl std::fmt::Display for When {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (hour, minute) = self.hm();
        match self.day() {
            Some(day) => write!(f, "weekly {} {hour:02}:{minute:02}", DAYS[day as usize]),
            None => write!(f, "daily {hour:02}:{minute:02}"),
        }
    }
}

/// A task name as it appears in unit names: lowercase letters, digits and
/// single dashes. Two task names can map to the same slug; `vivo schedule
/// apply` rejects that.
pub fn slug(task: &str) -> String {
    let mut out = String::new();
    for c in task.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() { "task".to_string() } else { out.to_string() }
}

#[derive(Debug, Clone)]
pub struct Schedule {
    pub vivo_bin: PathBuf,
    pub when: When,
    /// Task to run; `None` runs the config's default task.
    pub task: Option<String>,
    pub path_env: String,
    /// The paths this invocation resolved (config, secrets, age key, state
    /// dir), pinned so the job uses exactly the files the user's shell does.
    pub pinned_env: Vec<(String, String)>,
    pub log: PathBuf,
}

/// launchd label of a unit (`None`: the default-task unit).
fn label(id: Option<&str>) -> String {
    match id {
        None => LABEL.to_string(),
        Some(id) => format!("{LABEL}.{id}"),
    }
}

/// systemd unit basename (`.service` / `.timer` are appended).
fn unit_name(id: Option<&str>) -> String {
    match id {
        None => "vivo".to_string(),
        Some(id) => format!("vivo-{id}"),
    }
}

const CRON_BEGIN_PREFIX: &str = "# >>> vivo backup";
const CRON_END_PREFIX: &str = "# <<< vivo backup";

fn cron_begin(id: Option<&str>) -> String {
    match id {
        None => format!("{CRON_BEGIN_PREFIX} (managed by `vivo schedule`) >>>"),
        Some(id) => format!("{CRON_BEGIN_PREFIX} [{id}] (managed by `vivo schedule`) >>>"),
    }
}

fn cron_end(id: Option<&str>) -> String {
    match id {
        None => format!("{CRON_END_PREFIX} <<<"),
        Some(id) => format!("{CRON_END_PREFIX} [{id}] <<<"),
    }
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
    pub fn new(task: Option<String>, when: When, config_path: &str) -> Result<Schedule, String> {
        let vivo_bin = env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|e| format!("could not locate the vivo binary: {e}"))?;
        Ok(Schedule {
            vivo_bin,
            when,
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
        let (hour, minute) = self.when.hm();
        format!("{hour:02}:{minute:02}")
    }

    /// The unit's id: `None` for the default-task unit, else the task's slug.
    pub fn id(&self) -> Option<String> {
        self.task.as_deref().map(slug)
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
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key>
  <array>
{program}  </array>
  <key>EnvironmentVariables</key>
  <dict>{env}</dict>
  <key>StartCalendarInterval</key>
  <dict>{weekday}<key>Hour</key><integer>{hour}</integer><key>Minute</key><integer>{minute}</integer></dict>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
        label = xml_escape(&label(s.id().as_deref())),
        weekday = s.when.day().map(|d| format!("<key>Weekday</key><integer>{d}</integer>")).unwrap_or_default(),
        hour = s.when.hm().0,
        minute = s.when.hm().1,
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
        "[Unit]\nDescription=vivo backup{what}\n\n[Service]\nType=oneshot\n{env}ExecStart={exec}\nStandardOutput=append:{log}\nStandardError=append:{log}\n",
        log = s.log.display(),
        what = s.task.as_deref().map(|t| format!(" ({t})")).unwrap_or_default(),
    )
}

pub fn render_timer(s: &Schedule) -> String {
    let (hour, minute) = s.when.hm();
    let day = s.when.day().map(|d| format!("{} ", ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"][d as usize]));
    format!(
        "[Unit]\nDescription=Scheduled vivo backup ({when})\n\n[Timer]\nOnCalendar={day}*-*-* {hour:02}:{minute:02}:00\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n",
        when = s.when,
        day = day.unwrap_or_default(),
    )
}

fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

pub fn render_cron_block(s: &Schedule) -> String {
    let cmd = s.command().iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
    let log = shell_quote(&s.log.to_string_lossy());
    let env: String = s.env_pairs().iter().map(|(k, v)| format!("{k}={} ", shell_quote(v))).collect();
    let id = s.id();
    let (hour, minute) = s.when.hm();
    let day = s.when.day().map_or("*".to_string(), |d| d.to_string());
    format!(
        "{}\n{minute} {hour} * * {day} {env}{cmd} >> {log} 2>&1\n{}\n",
        cron_begin(id.as_deref()),
        cron_end(id.as_deref())
    )
}

/// `crontab` text with the block of unit `id` removed.
pub fn strip_cron_block(crontab: &str, id: Option<&str>) -> String {
    let (begin, end) = (cron_begin(id), cron_end(id));
    let mut out = String::new();
    let mut inside = false;
    for line in crontab.lines() {
        if line == begin {
            inside = true;
        } else if line == end {
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

fn launch_agents_dir() -> PathBuf {
    home().join("Library/LaunchAgents")
}

pub fn plist_path(id: Option<&str>) -> PathBuf {
    launch_agents_dir().join(format!("{}.plist", label(id)))
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

/// Installs (or replaces) the unit for `s`. Returns the file or entry that was written.
pub fn install(s: &Schedule, backend: Backend) -> Result<String, String> {
    let id = s.id();
    let id = id.as_deref();
    if let Some(dir) = s.log.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    }
    let written: Result<String, String> = match backend {
        Backend::Launchd => {
            let path = plist_path(id);
            let domain = format!("gui/{}", uid()?);
            // Replace a loaded copy; failing here just means there wasn't one.
            let _ = run_ok("launchctl", &["bootout", &format!("{domain}/{}", label(id))]);
            write_file(&path, &render_plist(s))?;
            run_ok("launchctl", &["bootstrap", &domain, &path.to_string_lossy()])?;
            Ok(path.display().to_string())
        }
        Backend::Systemd => {
            let dir = systemd_dir();
            let name = unit_name(id);
            write_file(&dir.join(format!("{name}.service")), &render_service(s))?;
            write_file(&dir.join(format!("{name}.timer")), &render_timer(s))?;
            run_ok("systemctl", &["--user", "daemon-reload"])?;
            // `enable --now` leaves a running timer on its old schedule.
            run_ok("systemctl", &["--user", "enable", &format!("{name}.timer")])?;
            run_ok("systemctl", &["--user", "restart", &format!("{name}.timer")])?;
            Ok(dir.join(format!("{name}.timer")).display().to_string())
        }
        Backend::Cron => {
            let mut text = strip_cron_block(&read_crontab(), id);
            text.push_str(&render_cron_block(s));
            write_crontab(&text)?;
            Ok("your crontab".to_string())
        }
    };
    let written = written?;
    // Only a convenience for `status`; the schedule itself is already live.
    let _ = write_record(id, &Record { backend, at: s.when.to_string(), task: s.task.clone() });
    Ok(written)
}

/// Removes one unit (`None`: the default-task unit) from every backend.
/// Returns what was removed.
pub fn remove_unit(id: Option<&str>) -> Result<Vec<Backend>, String> {
    let mut removed = Vec::new();

    let plist = plist_path(id);
    if plist.exists() {
        if let Ok(uid) = uid() {
            let _ = run_ok("launchctl", &["bootout", &format!("gui/{uid}/{}", label(id))]);
        }
        fs::remove_file(&plist).map_err(|e| format!("could not remove {}: {e}", plist.display()))?;
        removed.push(Backend::Launchd);
    }

    let dir = systemd_dir();
    let name = unit_name(id);
    let timer = format!("{name}.timer");
    if dir.join(&timer).exists() {
        let _ = run_ok("systemctl", &["--user", "disable", "--now", &timer]);
        for f in [timer, format!("{name}.service")] {
            let _ = fs::remove_file(dir.join(f));
        }
        let _ = run_ok("systemctl", &["--user", "daemon-reload"]);
        removed.push(Backend::Systemd);
    }

    let crontab = read_crontab();
    if crontab.lines().any(|l| l == cron_begin(id)) {
        write_crontab(&strip_cron_block(&crontab, id))?;
        removed.push(Backend::Cron);
    }
    let _ = fs::remove_file(record_path(id));
    Ok(removed)
}

/// A removed unit's id and the backends it was removed from.
pub type RemovedUnit = (Option<String>, Vec<Backend>);

/// Removes every vivo unit: the default-task one and all per-task ones.
/// Returns each removed unit's id and backends.
pub fn remove_all() -> Result<Vec<RemovedUnit>, String> {
    let mut out = Vec::new();
    for unit in installed_all() {
        let backends = remove_unit(unit.id.as_deref())?;
        if !backends.is_empty() {
            out.push((unit.id, backends));
        }
    }
    Ok(out)
}

/// What `install` recorded, so `installed_all` can report the time and task
/// without parsing the files it rendered. `at` is a [`When`] (older records
/// hold a bare `HH:MM`, which parses as daily).
#[derive(Debug, Serialize, Deserialize)]
struct Record {
    backend: Backend,
    at: String,
    task: Option<String>,
}

fn record_path(id: Option<&str>) -> PathBuf {
    let file = match id {
        None => "schedule.json".to_string(),
        Some(id) => format!("schedule-{id}.json"),
    };
    crate::state::state_dir().join(file)
}

fn write_record(id: Option<&str>, record: &Record) -> Result<(), String> {
    let text = serde_json::to_string_pretty(record).map_err(|e| e.to_string())?;
    write_file(&record_path(id), &text)
}

fn read_record(id: Option<&str>) -> Option<Record> {
    serde_json::from_str(&fs::read_to_string(record_path(id)).ok()?).ok()
}

#[derive(Debug)]
pub struct Installed {
    /// `None` for the default-task unit, else the task's slug.
    pub id: Option<String>,
    pub backend: Backend,
    /// When it fires and the task it runs, when recorded.
    pub when: Option<When>,
    pub task: Option<String>,
}

/// File names in `dir` of the form `<prefix><id><suffix>`, as the ids.
fn ids_in(dir: &Path, prefix: &str, suffix: &str) -> Vec<String> {
    let Ok(entries) = fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| n.strip_prefix(prefix)?.strip_suffix(suffix).map(str::to_string))
        .filter(|id| !id.is_empty())
        .collect()
}

/// Every installed unit, default-task unit first, then per-task units by id.
/// Existence comes from the scheduler's own artifacts; the details from the
/// records written at install time.
pub fn installed_all() -> Vec<Installed> {
    let mut found: Vec<(Option<String>, Backend)> = Vec::new();

    if plist_path(None).exists() {
        found.push((None, Backend::Launchd));
    }
    for id in ids_in(&launch_agents_dir(), &format!("{LABEL}."), ".plist") {
        found.push((Some(id), Backend::Launchd));
    }

    let dir = systemd_dir();
    if dir.join("vivo.timer").exists() {
        found.push((None, Backend::Systemd));
    }
    for id in ids_in(&dir, "vivo-", ".timer") {
        found.push((Some(id), Backend::Systemd));
    }

    // vivo only ever installs a cron entry where there is no launchd.
    if !cfg!(target_os = "macos") {
        for line in read_crontab().lines() {
            if line == cron_begin(None) {
                found.push((None, Backend::Cron));
            } else if let Some(id) = line
                .strip_prefix(&format!("{CRON_BEGIN_PREFIX} ["))
                .and_then(|rest| rest.strip_suffix("] (managed by `vivo schedule`) >>>"))
            {
                found.push((Some(id.to_string()), Backend::Cron));
            }
        }
    }

    found.sort_by(|a, b| a.0.cmp(&b.0));
    found
        .into_iter()
        .map(|(id, backend)| {
            let record = read_record(id.as_deref()).filter(|r| r.backend == backend);
            Installed {
                id,
                backend,
                when: record.as_ref().and_then(|r| When::parse(&r.at).ok()),
                task: record.and_then(|r| r.task),
            }
        })
        .collect()
}

/// Whether any schedule is installed.
pub fn installed() -> Option<Installed> {
    installed_all().into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(task: Option<&str>) -> Schedule {
        Schedule {
            vivo_bin: PathBuf::from("/usr/local/bin/vivo"),
            when: When::Daily { hour: 2, minute: 5 },
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
        assert_eq!(strip_cron_block(&with, None), existing);
        // Re-installing replaces rather than duplicates.
        let again = format!("{}{}", strip_cron_block(&with, None), render_cron_block(&sample(None)));
        assert_eq!(again.matches(&cron_begin(None)).count(), 1);
    }

    #[test]
    fn parses_and_prints_when() {
        assert_eq!(When::parse("02:00"), Ok(When::Daily { hour: 2, minute: 0 }));
        assert_eq!(When::parse("daily 23:59"), Ok(When::Daily { hour: 23, minute: 59 }));
        assert_eq!(When::parse("weekly Sunday 03:30"), Ok(When::Weekly { day: 0, hour: 3, minute: 30 }));
        assert_eq!(When::parse("  weekly   sat  1:05 ").unwrap().to_string(), "weekly sat 01:05");
        assert_eq!(When::parse("daily 02:00").unwrap().to_string(), "daily 02:00");
        for bad in ["", "daily", "weekly 03:00", "weekly funday 03:00", "monthly 1 03:00", "weekly mon 25:00", "daily 2am"] {
            assert!(When::parse(bad).is_err(), "{bad}");
        }
        // what we write into the config must read back the same
        let w = When::Weekly { day: 4, hour: 1, minute: 2 };
        assert_eq!(When::parse(&w.to_string()), Ok(w));
    }

    #[test]
    fn slugs_are_safe_unit_names() {
        assert_eq!(slug("photos"), "photos");
        assert_eq!(slug("My Photos & Videos!"), "my-photos-videos");
        assert_eq!(slug("--a__b--"), "a-b");
        assert_eq!(slug("///"), "task");
        assert_eq!(slug("filecabinet"), slug("FileCabinet"));
    }

    fn weekly_photos() -> Schedule {
        let mut s = sample(Some("My Photos"));
        s.when = When::Weekly { day: 0, hour: 3, minute: 30 };
        s
    }

    #[test]
    fn per_task_units_have_their_own_names() {
        let s = weekly_photos();
        assert_eq!(s.id().as_deref(), Some("my-photos"));
        assert_eq!(label(s.id().as_deref()), "dev.vivo.backup.my-photos");
        assert_eq!(label(None), LABEL);
        assert_eq!(unit_name(s.id().as_deref()), "vivo-my-photos");
        assert_eq!(unit_name(None), "vivo");
        assert!(plist_path(s.id().as_deref()).ends_with("LaunchAgents/dev.vivo.backup.my-photos.plist"));
    }

    #[test]
    fn weekly_schedule_renders_for_every_backend() {
        let s = weekly_photos();
        let plist = render_plist(&s);
        assert!(plist.contains("<key>Label</key><string>dev.vivo.backup.my-photos</string>"));
        assert!(plist.contains("<key>Weekday</key><integer>0</integer><key>Hour</key><integer>3</integer><key>Minute</key><integer>30</integer>"));
        assert!(render_timer(&s).contains("OnCalendar=Sun *-*-* 03:30:00"));
        assert!(render_service(&s).contains("Description=vivo backup (My Photos)"));
        let cron = render_cron_block(&s);
        assert!(cron.contains("\n30 3 * * 0 "));
        assert!(cron.starts_with("# >>> vivo backup [my-photos] (managed by `vivo schedule`) >>>\n"));
        // daily output has no weekday
        assert!(!render_plist(&sample(None)).contains("Weekday"));
        assert!(render_timer(&sample(None)).contains("OnCalendar=*-*-* 02:05:00"));
    }

    #[test]
    fn cron_blocks_are_removed_per_unit() {
        let daily = render_cron_block(&sample(None));
        let photos = render_cron_block(&weekly_photos());
        let docs = render_cron_block(&sample(Some("docs")));
        let all = format!("MAILTO=me\n{daily}{photos}{docs}");

        let without_photos = strip_cron_block(&all, Some("my-photos"));
        assert_eq!(without_photos, format!("MAILTO=me\n{daily}{docs}"));
        let without_default = strip_cron_block(&all, None);
        assert_eq!(without_default, format!("MAILTO=me\n{photos}{docs}"));
        assert_eq!(strip_cron_block(&without_default, Some("docs")), format!("MAILTO=me\n{photos}"));
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
        // records written before per-task schedules hold a bare time
        assert_eq!(When::parse(&r.at), Ok(When::Daily { hour: 2, minute: 5 }));
        let back: Record = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!((back.backend, back.at.as_str(), back.task.as_deref()), (Backend::Systemd, "02:05", Some("docs")));
        assert!(serde_json::to_string(&r).unwrap().contains("\"systemd\""));
    }
}
