//! Remembers how each task's last run went, so `vivo status` can answer
//! "is my backup actually working?" without re-running anything.
//!
//! The file lives in `$VIVO_STATE_DIR`, else `$XDG_STATE_HOME/vivo`, else
//! `~/.local/state/vivo`, next to the scheduler's `vivo.log`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::{env, fs};

use serde::{Deserialize, Serialize};

use crate::backup_config::backup::RunOutcome;
use crate::recovery::Outcome;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub tasks: BTreeMap<String, TaskState>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct TaskState {
    /// Unix seconds of the last real (non-dry-run) run.
    pub last_run: i64,
    /// Why the last run was not clean; `None` means it finished with no errors.
    #[serde(default)]
    pub error: Option<String>,
    /// Latest sync attempt per remote URL.
    #[serde(default)]
    pub remotes: BTreeMap<String, RemoteState>,
    /// Unix seconds each `steps`-scheduled step (`check`, `forget`, `sync`)
    /// last completed, keyed by step name.
    #[serde(default)]
    pub steps: BTreeMap<String, i64>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RemoteState {
    /// Unix seconds of the latest sync attempt.
    pub at: i64,
    /// Why that attempt failed; `None` means it succeeded.
    #[serde(default)]
    pub error: Option<String>,
    /// The latest attempt was cut short when its sync window closed. `error`
    /// says so; the sync resumes next run, so it is not a failure.
    #[serde(default)]
    pub interrupted: bool,
    /// Unix seconds of the last successful sync of this remote.
    #[serde(default)]
    pub last_ok: Option<i64>,
    /// The remote's recovery copy; `None` if it is switched off or not yet attempted.
    #[serde(default)]
    pub recovery: Option<RecoveryState>,
}

/// Where a remote's recovery copy stands. `hash` is the config+secrets hash
/// that was last uploaded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum RecoveryState {
    /// The remote has the current files.
    Current { hash: String, at: i64 },
    /// A copy exists, but the latest attempt to update it failed.
    Stale { hash: String, at: i64, error: String },
    /// There is no copy on the remote, and why.
    Missing { error: String },
}

impl RecoveryState {
    /// Hash of the files on the remote, if there is a copy.
    pub fn hash(&self) -> Option<&str> {
        match self {
            RecoveryState::Current { hash, .. } | RecoveryState::Stale { hash, .. } => Some(hash),
            RecoveryState::Missing { .. } => None,
        }
    }

    /// The state after one sync's recovery `outcome` (`None`: not attempted).
    fn after(previous: Option<RecoveryState>, outcome: Option<&Outcome>, at: i64) -> Option<RecoveryState> {
        let copy = |p: &Option<RecoveryState>| match p {
            Some(RecoveryState::Current { hash, at } | RecoveryState::Stale { hash, at, .. }) => Some((hash.clone(), *at)),
            _ => None,
        };
        match outcome {
            None => previous,
            Some(Outcome::Disabled) => None,
            Some(Outcome::Uploaded(hash)) => Some(RecoveryState::Current { hash: hash.clone(), at }),
            Some(Outcome::Unchanged) => copy(&previous).map(|(hash, at)| RecoveryState::Current { hash, at }).or(previous),
            Some(Outcome::Skipped(why)) => Some(RecoveryState::Missing { error: why.clone() }),
            Some(Outcome::Failed(why)) => Some(match copy(&previous) {
                Some((hash, at)) => RecoveryState::Stale { hash, at, error: why.clone() },
                None => RecoveryState::Missing { error: why.clone() },
            }),
        }
    }
}

pub fn state_dir() -> PathBuf {
    if let Ok(dir) = env::var("VIVO_STATE_DIR") {
        return PathBuf::from(dir);
    }
    crate::config::xdg_state_home().join("vivo")
}

pub fn state_path() -> PathBuf {
    state_dir().join("state.json")
}

/// Where scheduled runs write their output.
pub fn log_path() -> PathBuf {
    state_dir().join("vivo.log")
}

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

impl State {
    /// A missing or unreadable file is an empty state: status is advisory and
    /// must never stop a backup.
    pub fn load() -> State {
        fs::read_to_string(state_path())
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<(), String> {
        let path = state_path();
        fs::create_dir_all(state_dir()).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(&tmp, text).map_err(|e| e.to_string())?;
        fs::rename(&tmp, &path).map_err(|e| e.to_string())
    }

    /// Folds one finished run into the state. `error` is a failure that
    /// stopped the task before or during the backup.
    pub fn apply(&mut self, task: &str, outcome: &RunOutcome, error: Option<&str>, at: i64) {
        let entry = self.tasks.entry(task.to_string()).or_default();
        for sync in &outcome.syncs {
            let r = entry.remotes.entry(sync.url.clone()).or_default();
            r.at = at;
            r.error = sync.error.clone();
            r.interrupted = sync.interrupted;
            if sync.error.is_none() {
                r.last_ok = Some(at);
            }
            r.recovery = RecoveryState::after(r.recovery.take(), sync.recovery.as_ref(), at);
        }
        for step in &outcome.steps_done {
            entry.steps.insert(step.as_str().to_string(), at);
        }
        entry.last_run = at;
        entry.error = error.map(str::to_string).or_else(|| {
            let failed = outcome.failed_syncs();
            (failed > 0).then(|| format!("{failed} remote sync(s) failed"))
        });
    }
}

/// Records a finished run. Dry runs are not recorded. Failing to write the
/// state only warns: it must not turn a good backup into a failed one.
pub fn record_run(task: &str, dry_run: bool, outcome: &RunOutcome, error: Option<&str>) {
    if dry_run {
        return;
    }
    let mut state = State::load();
    state.apply(task, outcome, error, now());
    if let Err(e) = state.save() {
        eprintln!("warning: could not record run status: {e}");
    }
}

/// "5 hours", "20 minutes" — a span of time, coarse on purpose.
pub fn span(secs: i64) -> String {
    let secs = secs.max(0);
    let (n, unit) = match secs {
        0..=59 => (secs, "second"),
        60..=3599 => (secs / 60, "minute"),
        3600..=86399 => (secs / 3600, "hour"),
        _ => (secs / 86400, "day"),
    };
    format!("{n} {unit}{}", if n == 1 { "" } else { "s" })
}

/// "3 hours ago", "2 days ago" — coarse on purpose.
pub fn ago(secs: i64) -> String {
    let secs = secs.max(0);
    let (n, unit) = match secs {
        0..=59 => return "just now".to_string(),
        60..=3599 => (secs / 60, "minute"),
        3600..=86399 => (secs / 3600, "hour"),
        _ => (secs / 86400, "day"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup_config::backup::SyncResult;

    fn outcome(syncs: Vec<(&str, Option<&str>)>) -> RunOutcome {
        RunOutcome {
            dry_run: false,
            steps_done: Vec::new(),
            syncs: syncs
                .into_iter()
                .map(|(url, e)| SyncResult { url: url.into(), error: e.map(str::to_string), interrupted: false, recovery: None })
                .collect(),
        }
    }

    #[test]
    fn a_paused_sync_is_recorded_without_counting_as_good() {
        let mut s = State::default();
        s.apply("t", &outcome(vec![("b2:x", None)]), None, 100);

        let mut paused = outcome(vec![("b2:x", Some("the sync window closed; b2:x was not synced"))]);
        paused.syncs[0].interrupted = true;
        s.apply("t", &paused, None, 200);
        let r = &s.tasks["t"].remotes["b2:x"];
        assert!(r.interrupted);
        assert_eq!(r.at, 200);
        assert_eq!(r.last_ok, Some(100), "an incomplete sync is not a good sync");
        assert_eq!(s.tasks["t"].error, None, "and it does not fail the task");

        // the next full sync clears it
        s.apply("t", &outcome(vec![("b2:x", None)]), None, 300);
        let r = &s.tasks["t"].remotes["b2:x"];
        assert!(!r.interrupted && r.error.is_none());
        assert_eq!(r.last_ok, Some(300));
    }

    #[test]
    fn spans_are_coarse() {
        assert_eq!(span(30), "30 seconds");
        assert_eq!(span(60), "1 minute");
        assert_eq!(span(5 * 3600 + 59), "5 hours");
        assert_eq!(span(3 * 86400), "3 days");
    }

    #[test]
    fn records_completed_steps() {
        let mut s = State::default();
        let mut o = outcome(vec![]);
        o.steps_done = vec![crate::step::Step::Check, crate::step::Step::Sync];
        s.apply("t", &o, None, 500);
        assert_eq!(s.tasks["t"].steps["check"], 500);
        assert_eq!(s.tasks["t"].steps["sync"], 500);
        assert!(!s.tasks["t"].steps.contains_key("forget"));

        // a later run that skips check leaves its timestamp alone
        let mut o = outcome(vec![]);
        o.steps_done = vec![crate::step::Step::Sync];
        s.apply("t", &o, None, 900);
        assert_eq!(s.tasks["t"].steps["check"], 500);
        assert_eq!(s.tasks["t"].steps["sync"], 900);
    }

    #[test]
    fn successful_run_is_clean() {
        let mut s = State::default();
        s.apply("t", &outcome(vec![("b2:x", None)]), None, 100);
        let t = &s.tasks["t"];
        assert_eq!(t.error, None);
        assert_eq!(t.remotes["b2:x"].last_ok, Some(100));
    }

    #[test]
    fn failed_sync_keeps_previous_sync_time() {
        let mut s = State::default();
        s.apply("t", &outcome(vec![("b2:x", None)]), None, 100);
        s.apply("t", &outcome(vec![("b2:x", Some("boom"))]), None, 200);
        let t = &s.tasks["t"];
        assert_eq!(t.remotes["b2:x"].last_ok, Some(100));
        assert_eq!(t.remotes["b2:x"].error.as_deref(), Some("boom"));
        assert_eq!(t.error.as_deref(), Some("1 remote sync(s) failed"));
    }

    #[test]
    fn recovery_outcomes_are_tracked_per_remote() {
        let sync = |recovery| RunOutcome {
            dry_run: false,
            syncs: vec![SyncResult { url: "b2:x:p".into(), error: None, interrupted: false, recovery }],
            ..RunOutcome::default()
        };
        let state = |s: &State| s.tasks["t"].remotes["b2:x:p"].recovery.clone();
        let mut s = State::default();

        s.apply("t", &sync(Some(Outcome::Failed("no age".into()))), None, 50);
        assert_eq!(state(&s), Some(RecoveryState::Missing { error: "no age".into() }));

        s.apply("t", &sync(Some(Outcome::Uploaded("h1".into()))), None, 100);
        assert_eq!(state(&s), Some(RecoveryState::Current { hash: "h1".into(), at: 100 }));

        // A failed update keeps the old copy's hash (so it retries) and says why.
        s.apply("t", &sync(Some(Outcome::Failed("boom".into()))), None, 200);
        assert_eq!(state(&s), Some(RecoveryState::Stale { hash: "h1".into(), at: 100, error: "boom".into() }));
        assert_eq!(state(&s).unwrap().hash(), Some("h1"));

        s.apply("t", &sync(Some(Outcome::Unchanged)), None, 300);
        assert_eq!(state(&s), Some(RecoveryState::Current { hash: "h1".into(), at: 100 }));

        // A sync that failed outright leaves the recovery state alone.
        s.apply("t", &sync(None), None, 350);
        assert_eq!(state(&s), Some(RecoveryState::Current { hash: "h1".into(), at: 100 }));

        s.apply("t", &sync(Some(Outcome::Disabled)), None, 400);
        assert_eq!(state(&s), None);
    }

    #[test]
    fn backup_error_is_recorded() {
        let mut s = State::default();
        s.apply("t", &RunOutcome::default(), Some("restic failed"), 50);
        assert_eq!(s.tasks["t"].error.as_deref(), Some("restic failed"));
    }

    #[test]
    fn ago_is_coarse_and_pluralized() {
        assert_eq!(ago(5), "just now");
        assert_eq!(ago(60), "1 minute ago");
        assert_eq!(ago(7200), "2 hours ago");
        assert_eq!(ago(86400 * 3), "3 days ago");
    }
}
