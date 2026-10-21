//! The optional `steps { }` block of a `backup`: how often the slow phases
//! (`check`, `forget`, `sync`) should run, and how thorough `check` is.
//!
//! ```kdl
//! steps {
//!     check { every "weekly"; subset "5%" }
//!     forget { every "weekly" }
//!     sync { every "daily" }
//! }
//! ```
//!
//! A step with no `every` runs on every run, as it always has.
//!
//! `sync` may also have `window "01:00-06:00"` (a window that crosses midnight
//! works too): a sync only starts inside it, and is stopped when it closes.

use chrono::{NaiveTime, Timelike};
use kdl::KdlNode;

use crate::backup_config::kdl_decode::{ConfigError, Source};
use crate::step::Step;

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// Scheduled jobs fire a few seconds apart from day to day; without slack a
/// "daily" step would skip every other run. The slack is capped at an hour.
fn slack(interval: i64) -> i64 {
    (interval / 10).min(HOUR)
}

/// `hourly`, `daily`, `weekly`, `monthly` (30 days), or `<n>h`, `<n>d`, `<n>w`.
pub fn parse_every(text: &str) -> Result<i64, String> {
    match text {
        "hourly" => return Ok(HOUR),
        "daily" => return Ok(DAY),
        "weekly" => return Ok(7 * DAY),
        "monthly" => return Ok(30 * DAY),
        _ => {}
    }
    let bad = || format!("invalid interval '{text}' (use hourly, daily, weekly, monthly, or e.g. 12h, 3d, 2w)");
    let unit = text.chars().last().ok_or_else(bad)?;
    let secs = match unit {
        'h' => HOUR,
        'd' => DAY,
        'w' => 7 * DAY,
        _ => return Err(bad()),
    };
    match text[..text.len() - 1].parse::<i64>() {
        Ok(n) if n > 0 => Ok(n * secs),
        _ => Err(bad()),
    }
}

/// restic's `--read-data-subset`: a percentage (`5%`), a fraction (`1/10`) or a size (`2G`).
pub fn valid_subset(text: &str) -> bool {
    let number = |s: &str| !s.is_empty() && s.parse::<f64>().is_ok_and(|n| n > 0.0);
    if let Some(percent) = text.strip_suffix('%') {
        return number(percent) && percent.parse::<f64>().is_ok_and(|n| n <= 100.0);
    }
    if let Some((n, t)) = text.split_once('/') {
        return matches!((n.parse::<u32>(), t.parse::<u32>()), (Ok(n), Ok(t)) if n >= 1 && n <= t);
    }
    match text.strip_suffix(['k', 'K', 'm', 'M', 'g', 'G', 't', 'T']) {
        Some(size) => number(size),
        None => false,
    }
}

/// A daily span of local time, `start` up to but not including `end`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    start: (u8, u8),
    end: (u8, u8),
}

impl Window {
    /// `HH:MM-HH:MM`; the end may be earlier than the start (overnight).
    pub fn parse(text: &str) -> Result<Window, String> {
        let bad = || format!("invalid window '{text}' (use HH:MM-HH:MM, e.g. 01:00-06:00)");
        let (start, end) = text.split_once('-').ok_or_else(bad)?;
        let (start, end) = (crate::schedule::parse_time(start).map_err(|_| bad())?, crate::schedule::parse_time(end).map_err(|_| bad())?);
        if start == end {
            return Err(format!("window '{text}' is empty — its start and end are the same"));
        }
        Ok(Window { start, end })
    }

    fn secs((hour, minute): (u8, u8)) -> u32 {
        u32::from(hour) * 3600 + u32::from(minute) * 60
    }

    /// How long until the window closes, or `None` if `now` is outside it.
    pub fn remaining(&self, now: NaiveTime) -> Option<std::time::Duration> {
        let (start, end, now) = (Self::secs(self.start), Self::secs(self.end), now.num_seconds_from_midnight());
        let inside = if start < end { (start..end).contains(&now) } else { now >= start || now < end };
        inside.then(|| std::time::Duration::from_secs(u64::from((end + 86400 - now) % 86400)))
    }
}

impl std::fmt::Display for Window {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:02}:{:02}-{:02}:{:02}", self.start.0, self.start.1, self.end.0, self.end.1)
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct StepPolicy {
    /// Minimum seconds between runs; `None` means every run.
    every: Option<i64>,
    /// The `every` text as written, for messages.
    every_text: Option<String>,
    /// `check` only: passed as `--read-data-subset`.
    subset: Option<String>,
    /// `sync` only: when a sync may run.
    window: Option<Window>,
}

impl StepPolicy {
    fn decode(src: &Source, node: &KdlNode, allowed: &[&str]) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(children, allowed)?;
        let mut policy = StepPolicy::default();
        if let Some(n) = src.optional(children, "every")? {
            let text = src.string_value(n)?;
            policy.every = Some(parse_every(&text).map_err(|e| src.error(n, e))?);
            policy.every_text = Some(text);
        }
        if let Some(n) = src.optional(children, "subset")? {
            let text = src.string_value(n)?;
            if !valid_subset(&text) {
                return Err(src.error(n, format!("invalid subset '{text}' (use e.g. 5%, 1/10 or 2G)")));
            }
            policy.subset = Some(text);
        }
        if let Some(n) = src.optional(children, "window")? {
            policy.window = Some(Window::parse(&src.string_value(n)?).map_err(|e| src.error(n, e))?);
        }
        Ok(policy)
    }

    pub fn window(&self) -> Option<Window> {
        self.window
    }

    pub fn every_secs(&self) -> Option<i64> {
        self.every
    }

    pub fn every_text(&self) -> Option<&str> {
        self.every_text.as_deref()
    }

    pub fn subset(&self) -> Option<&str> {
        self.subset.as_deref()
    }

    /// Whether the step should run, given when it last succeeded.
    pub fn is_due(&self, last: Option<i64>, now: i64) -> bool {
        match (self.every, last) {
            (Some(every), Some(last)) => now - last >= every - slack(every),
            _ => true,
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Steps {
    check: StepPolicy,
    forget: StepPolicy,
    sync: StepPolicy,
}

impl Steps {
    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(children, &["check", "forget", "sync"])?;
        let policy = |name: &str, allowed: &[&str]| -> Result<StepPolicy, ConfigError> {
            match src.optional(children, name)? {
                Some(n) => StepPolicy::decode(src, n, allowed),
                None => Ok(StepPolicy::default()),
            }
        };
        Ok(Steps {
            check: policy("check", &["every", "subset"])?,
            forget: policy("forget", &["every"])?,
            sync: policy("sync", &["every", "window"])?,
        })
    }

    /// One line describing the configured cadence, e.g.
    /// `check weekly (5%) · forget weekly · sync daily 01:00-06:00`; `None` if
    /// no step has any setting.
    pub fn summary(&self) -> Option<String> {
        let parts: Vec<String> = [("check", &self.check), ("forget", &self.forget), ("sync", &self.sync)]
            .into_iter()
            .filter_map(|(name, policy)| {
                let mut words = vec![name.to_string()];
                words.extend(policy.every_text().map(str::to_string));
                words.extend(policy.subset().map(|s| format!("({s})")));
                words.extend(policy.window().map(|w| w.to_string()));
                (words.len() > 1).then(|| words.join(" "))
            })
            .collect();
        (!parts.is_empty()).then(|| parts.join(" \u{b7} "))
    }

    /// The policy of a step; `backup` has none and always runs.
    pub fn policy(&self, step: &Step) -> Option<&StepPolicy> {
        match step {
            Step::Backup => None,
            Step::Check => Some(&self.check),
            Step::Forget => Some(&self.forget),
            Step::Sync => Some(&self.sync),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_intervals() {
        assert_eq!(parse_every("daily"), Ok(DAY));
        assert_eq!(parse_every("weekly"), Ok(7 * DAY));
        assert_eq!(parse_every("12h"), Ok(12 * HOUR));
        assert_eq!(parse_every("3d"), Ok(3 * DAY));
        assert_eq!(parse_every("2w"), Ok(14 * DAY));
        for bad in ["", "d", "0d", "-1d", "1.5d", "fortnightly", "5"] {
            assert!(parse_every(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn validates_subsets() {
        for ok in ["5%", "0.5%", "100%", "1/10", "10/10", "2G", "500M"] {
            assert!(valid_subset(ok), "{ok}");
        }
        for bad in ["", "%", "0%", "101%", "0/10", "11/10", "G", "5", "abc"] {
            assert!(!valid_subset(bad), "{bad}");
        }
    }

    #[test]
    fn due_when_never_run_or_no_cadence() {
        let daily = StepPolicy { every: Some(DAY), ..Default::default() };
        assert!(daily.is_due(None, 1000));
        assert!(StepPolicy::default().is_due(Some(1000), 1001));
    }

    #[test]
    fn due_respects_interval_with_slack() {
        let daily = StepPolicy { every: Some(DAY), ..Default::default() };
        let t = 10 * DAY;
        assert!(!daily.is_due(Some(t), t + 12 * HOUR));
        // yesterday's run was a few seconds later than today's start: still due
        assert!(daily.is_due(Some(t), t + DAY - 5));
        assert!(!daily.is_due(Some(t), t + DAY - 2 * HOUR));
    }

    fn at(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    #[test]
    fn window_parses_and_prints() {
        assert_eq!(Window::parse("01:00-06:00").unwrap().to_string(), "01:00-06:00");
        assert_eq!(Window::parse("22:30-06:15").unwrap().to_string(), "22:30-06:15");
        for bad in ["", "01:00", "01:00-", "-06:00", "25:00-06:00", "01:00-06:00-07:00", "03:00-03:00", "1am-6am"] {
            assert!(Window::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn window_says_how_long_is_left() {
        let w = Window::parse("01:00-06:00").unwrap();
        assert_eq!(w.remaining(at(0, 59)), None);
        assert_eq!(w.remaining(at(1, 0)), Some(std::time::Duration::from_secs(5 * 3600)));
        assert_eq!(w.remaining(at(5, 59)), Some(std::time::Duration::from_secs(60)));
        assert_eq!(w.remaining(at(6, 0)), None);
        assert_eq!(w.remaining(at(12, 0)), None);
    }

    #[test]
    fn overnight_window_wraps_midnight() {
        let w = Window::parse("22:00-06:00").unwrap();
        assert_eq!(w.remaining(at(21, 59)), None);
        assert_eq!(w.remaining(at(22, 0)), Some(std::time::Duration::from_secs(8 * 3600)));
        assert_eq!(w.remaining(at(23, 0)), Some(std::time::Duration::from_secs(7 * 3600)));
        assert_eq!(w.remaining(at(0, 0)), Some(std::time::Duration::from_secs(6 * 3600)));
        assert_eq!(w.remaining(at(5, 59)), Some(std::time::Duration::from_secs(60)));
        assert_eq!(w.remaining(at(6, 0)), None);
    }

    #[test]
    fn summary_lists_only_configured_steps() {
        let parse = |body: &str| {
            let src = format!("default-task \"t\"\ntasks {{ task \"t\" {{ backup {{ repo \"/r\"\n{body} }} }} }}");
            crate::BackupConfig::parse("t", &src).unwrap().tasks[0].backup().unwrap().steps().summary()
        };
        assert_eq!(parse(""), None);
        assert_eq!(parse("steps { }"), None);
        assert_eq!(
            parse("steps { check { every \"weekly\"; subset \"5%\" }\nsync { every \"daily\"; window \"01:00-06:00\" } }").as_deref(),
            Some("check weekly (5%) \u{b7} sync daily 01:00-06:00")
        );
        assert_eq!(parse("steps { forget { every \"3d\" } }").as_deref(), Some("forget 3d"));
    }
}
