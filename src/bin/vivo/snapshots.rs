//! `vivo snapshots`: what is in each backup repository.

use clap::ArgMatches;
use vivo::repos::{self, RepoTarget, Snapshot};
use vivo::state::{ago, now};

use crate::util::CmdResult;

/// Prints one repository's snapshots as a table, newest last.
fn print_snapshots(snapshots: &[Snapshot]) {
    println!("  {:<10} {:<17} {:<14} PATHS", "ID", "DATE", "AGE");
    let now = now();
    for s in snapshots {
        let age = s.unix_time().map_or_else(String::new, |t| ago(now - t));
        println!("  {:<10} {:<17} {:<14} {}", s.short_id, s.local_time(), age, s.paths.join(", "));
    }
}

/// Prints one repository's result; false if it could not be read.
fn show(target: &RepoTarget, snapshots: Option<Result<Vec<Snapshot>, String>>) -> bool {
    println!("\n{}", target.label());
    match snapshots {
        None => println!("  (no backups yet — run `vivo` to make the first one)"),
        Some(Ok(s)) if s.is_empty() => println!("  (no snapshots)"),
        Some(Ok(s)) => print_snapshots(&s),
        Some(Err(e)) => {
            eprintln!("  error: {e}");
            return false;
        }
    }
    true
}

pub fn cmd_snapshots(config_path: &str, secrets_path: &str, args: &ArgMatches) -> CmdResult {
    let (config, secrets) = repos::load(config_path, secrets_path)?;
    let remote = args.get_flag("remote");
    let tasks = repos::backup_tasks(&config, args.get_one::<String>("task").map(String::as_str))?;

    let targets: Vec<RepoTarget> = tasks
        .into_iter()
        .flat_map(|task| repos::targets_for(task, &secrets))
        .filter(|t| t.is_local != remote)
        .collect();
    if targets.is_empty() {
        return Err("no remotes are configured — add one with `vivo remote add`".to_string().into());
    }

    let results = repos::list_all(&targets, false);
    let mut ok = true;
    for (target, snapshots) in targets.iter().zip(results) {
        ok &= show(target, snapshots);
    }
    if ok {
        Ok(())
    } else {
        Err("could not read every repository".to_string().into())
    }
}
