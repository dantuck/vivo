//! `vivo status`: is the backup actually working?

use clap::ArgMatches;
use colored::Colorize;
use vivo::repos::{self, RepoTarget, Snapshot};
use vivo::state::{ago, now, RecoveryState, State, TaskState};

use crate::util::CmdResult;

const OK: &str = "✓";
const WARN: &str = "!";

struct Report {
    problems: usize,
}

impl Report {
    fn good(&self, label: &str, detail: &str) {
        println!("  {} {:<28} {detail}", OK.green(), label);
    }

    /// Worth reading but not a failure, so it doesn't count as a problem.
    fn note(&self, label: &str, detail: &str) {
        println!("  {} {:<28} {}", WARN.yellow().bold(), label, detail.yellow());
    }

    fn bad(&mut self, label: &str, detail: &str) {
        self.problems += 1;
        println!("  {} {:<28} {}", WARN.red().bold(), label, detail.red());
    }
}

fn judge_age(report: &mut Report, label: &str, what: &str, at: Option<i64>, max_secs: i64) {
    match at {
        None => report.bad(label, &format!("{what}: never")),
        Some(t) if now() - t > max_secs => {
            report.bad(label, &format!("{what} {} — older than expected", ago(now() - t)))
        }
        Some(t) => report.good(label, &format!("{what} {}", ago(now() - t))),
    }
}

/// One task's targets, each with its newest snapshot (local repositories only).
type Checked = (String, Vec<(RepoTarget, Option<Result<Vec<Snapshot>, String>>)>);

fn check_task(report: &mut Report, (task_name, targets): Checked, state: &State, max_secs: i64, sync_every: i64) {
    // A remote that only syncs weekly is not stale after two days.
    let remote_max_secs = max_secs + sync_every;
    println!("\n{}", task_name.bold());
    let recorded: Option<&TaskState> = state.tasks.get(&task_name);

    for (target, latest) in targets {
        if target.is_local {
            match latest {
                None => {
                    report.bad("backup", "no backup has been made yet — run `vivo`");
                    continue;
                }
                Some(Ok(s)) => judge_age(report, "latest backup", "snapshot taken", s.last().and_then(|s| s.unix_time()), max_secs),
                Some(Err(e)) => report.bad("latest backup", &format!("could not read the repository: {e}")),
            }
            continue;
        }
        let Some(r) = recorded.and_then(|t| t.remotes.get(&target.config_url)) else {
            report.bad(&target.config_url, "never synced (or not since status tracking began)");
            continue;
        };
        match &r.error {
            // Paused at the end of its sync window: it resumes by itself, so only
            // an old last good sync is a problem.
            Some(_) if r.interrupted => {
                println!("  {} paused {} when its sync window closed; resumes next run", "·".dimmed(), ago(now() - r.at));
                judge_age(report, &target.config_url, "last complete sync", r.last_ok, remote_max_secs);
            }
            Some(e) => report.bad(
                &target.config_url,
                &format!(
                    "last sync FAILED {}: {e} (last good: {})",
                    ago(now() - r.at),
                    r.last_ok.map_or("never".to_string(), |t| ago(now() - t)),
                ),
            ),
            None => judge_age(report, &target.config_url, "synced", r.last_ok, remote_max_secs),
        }
        match &r.recovery {
            Some(RecoveryState::Current { at, .. }) => report.good("  recovery copy", &format!("saved {}", ago(now() - at))),
            Some(RecoveryState::Stale { at, error, .. }) => report.bad(
                "  recovery copy",
                &format!("not updated since {} — {error}", ago(now() - at)),
            ),
            Some(RecoveryState::Missing { error }) => report.bad("  recovery copy", &format!("none — {error}")),
            None => {}
        }
    }
    if let Some((t, e)) = recorded.and_then(|t| t.error.as_ref().map(|e| (t, e))) {
        report.bad("last run", &format!("{} — {e}", ago(now() - t.last_run)));
    }
    for warning in recorded.map(|t| t.warnings.as_slice()).unwrap_or_default() {
        report.note("last run warning", warning);
    }
}

pub fn cmd_status(config_path: &str, secrets_path: &str, args: &ArgMatches) -> CmdResult {
    let (config, secrets) = repos::load(config_path, secrets_path)?;
    let max_secs = i64::from(*args.get_one::<u32>("max-age").unwrap_or(&48)) * 3600;
    let state = State::load();

    let mut report = Report { problems: 0 };
    let tasks = repos::backup_tasks(&config, None)?;
    if tasks.is_empty() {
        return Err("no tasks with a backup are configured — run `vivo task add`".to_string().into());
    }

    // Reading each local repository is independent, so do them all at once.
    let targets: Vec<RepoTarget> = tasks.iter().flat_map(|t| repos::targets_for(t, &secrets)).collect();
    let local: Vec<RepoTarget> = targets.iter().filter(|t| t.is_local).cloned().collect();
    let mut latest = repos::list_all(&local, true).into_iter();
    for task in tasks {
        let checked = targets
            .iter()
            .filter(|t| t.task == task.name)
            .map(|t| (t.clone(), if t.is_local { latest.next().flatten() } else { None }))
            .collect();
        let sync_every = task
            .backup()
            .and_then(|b| b.steps().policy(&vivo::step::Step::Sync))
            .and_then(|p| p.every_secs())
            .unwrap_or(0);
        check_task(&mut report, (task.name.clone(), checked), &state, max_secs, sync_every);
    }

    println!();
    let units = vivo::schedule::installed_all();
    if units.is_empty() {
        println!("{} Not scheduled — run `vivo schedule add` so backups happen on their own.", WARN.yellow());
    } else {
        let list: Vec<String> = units
            .iter()
            .map(|u| match u.when {
                Some(w) => format!("{} ({w})", u.task.as_deref().unwrap_or("default task")),
                None => u.task.clone().unwrap_or_else(|| "default task".to_string()),
            })
            .collect();
        println!("{} Scheduled via {}: {}.", OK.green(), units[0].backend.name(), list.join(", "));
    }

    println!(
        "\nKeep your age key ({}) and restic password somewhere other than this computer: with them and your remote, `vivo recover` rebuilds everything else.",
        vivo::age_keys_path()
    );

    if report.problems == 0 {
        println!("\nAll good.");
        Ok(())
    } else {
        Err(format!("{} problem(s) found — see above (`vivo doctor` checks the setup itself)", report.problems).into())
    }
}
