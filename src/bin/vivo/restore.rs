//! `vivo restore`: get files back out of a backup.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use clap::ArgMatches;
use inquire::{Confirm, Select, Text};
use vivo::repos::{self, RepoTarget, Snapshot};
use vivo::state::{ago, now};

use crate::util::{interactive, prompt, CmdResult};

/// Chooses the repository to read from. `--from` wins; otherwise the local
/// repository, or — when it is gone, as on a new machine — an off-site copy.
fn choose_source(targets: Vec<RepoTarget>, from: Option<&str>) -> Result<RepoTarget, crate::util::Stop> {
    if let Some(from) = from {
        return Ok(repos::find_source(&targets, Some(from))?);
    }

    let local = targets.iter().find(|t| t.is_local).filter(|t| t.exists());
    if let Some(local) = local {
        println!("Reading from the local repository {} (use --from <remote URL> to read an off-site copy).", local.restic_url);
        return Ok(local.clone());
    }

    let remotes: Vec<RepoTarget> = targets.into_iter().filter(|t| !t.is_local).collect();
    match remotes.len() {
        0 => Err("there is no local repository yet and no remote is configured — nothing to restore from".to_string().into()),
        1 => {
            println!("No local repository here; reading from {}.", remotes[0].config_url);
            Ok(remotes.into_iter().next().unwrap())
        }
        _ if interactive() => {
            println!("No local repository here — choose an off-site copy to restore from.");
            let labels: Vec<String> = remotes.iter().map(|t| t.config_url.clone()).collect();
            let pick = prompt(Select::new("Restore from:", labels).raw_prompt())?;
            Ok(remotes.into_iter().nth(pick.index).unwrap())
        }
        _ => Err("no local repository; pass --from <remote URL> to choose an off-site copy".to_string().into()),
    }
}

fn describe(s: &Snapshot) -> String {
    let age = s.unix_time().map(|t| ago(now() - t)).unwrap_or_default();
    format!("{}  {}  ({age})  {}", s.short_id, s.local_time(), s.paths.join(", "))
}

fn choose_snapshot(snapshots: &[Snapshot], wanted: Option<&str>) -> Result<String, crate::util::Stop> {
    if let Some(id) = wanted {
        return Ok(id.to_string());
    }
    if !interactive() || snapshots.len() == 1 {
        return Ok("latest".to_string());
    }
    // Newest first, with "latest" as the default top entry.
    let mut labels = vec!["latest (most recent snapshot)".to_string()];
    labels.extend(snapshots.iter().rev().map(describe));
    let pick = prompt(Select::new("Which snapshot?", labels).raw_prompt())?;
    Ok(match pick.index {
        0 => "latest".to_string(),
        i => snapshots[snapshots.len() - i].short_id.clone(),
    })
}

fn default_target(task: &str) -> PathBuf {
    PathBuf::from(vivo::expand_env_vars("$HOME")).join("vivo-restore").join(task)
}

fn is_non_empty_dir(p: &Path) -> bool {
    std::fs::read_dir(p).is_ok_and(|mut d| d.next().is_some())
}

pub fn cmd_restore(config_path: &str, secrets_path: &str, args: &ArgMatches) -> CmdResult {
    let (config, secrets) = repos::load(config_path, secrets_path)?;
    let tasks = repos::backup_tasks(&config, args.get_one::<String>("task").map(String::as_str))?;

    let task = match tasks.len() {
        1 => tasks[0],
        _ if interactive() => {
            let names: Vec<&str> = tasks.iter().map(|t| t.name.as_str()).collect();
            let pick = prompt(Select::new("Which task do you want to restore?", names).raw_prompt())?;
            tasks[pick.index]
        }
        _ => return Err("several tasks have backups; say which one: vivo restore <TASK>".to_string().into()),
    };

    let source = choose_source(
        repos::targets_for(task, &secrets),
        args.get_one::<String>("from").map(String::as_str),
    )?;
    if !source.exists() {
        return Err(format!("{} has no backups yet", source.restic_url).into());
    }

    let snapshots = repos::list_snapshots(&source, false)
        .map_err(|e| format!("could not read {}: {e}", source.config_url))?;
    if snapshots.is_empty() {
        return Err("that repository has no snapshots".to_string().into());
    }
    let snapshot = choose_snapshot(&snapshots, args.get_one::<String>("snapshot").map(String::as_str))?;

    let target = match args.get_one::<String>("target") {
        Some(t) => PathBuf::from(vivo::expand_env_vars(t)),
        None if interactive() => PathBuf::from(vivo::expand_env_vars(&prompt(
            Text::new("Restore into which directory?")
                .with_default(&default_target(&task.name).to_string_lossy())
                .with_help_message("Files are restored next to anything already there; pick a new folder to stay safe")
                .prompt(),
        )?)),
        None => default_target(&task.name),
    };
    if interactive() && is_non_empty_dir(&target) {
        let go = prompt(
            Confirm::new(&format!("{} is not empty; files with the same names will be overwritten. Continue?", target.display()))
                .with_default(false)
                .prompt(),
        )?;
        if !go {
            return Ok(());
        }
    }
    std::fs::create_dir_all(&target).map_err(|e| format!("could not create {}: {e}", target.display()))?;

    println!("\nRestoring snapshot {snapshot} from {} into {} ...\n", source.config_url, target.display());
    let mut cmd = source.restic();
    cmd.args(["restore", &snapshot, "--target"]).arg(&target);
    for path in args.get_many::<String>("path").into_iter().flatten() {
        cmd.args(["--include", path]);
    }
    let status = cmd
        .stdin(Stdio::null())
        .status()
        .map_err(|e| format!("could not run restic: {e}"))?;
    if !status.success() {
        return Err(format!("restic restore failed ({status})").into());
    }

    println!("\nRestored into {}.", target.display());
    let example = snapshots.last().and_then(|s| s.paths.first()).map_or(String::new(), |p| format!(" (e.g. {}{p})", target.display()));
    println!("Files keep their original folder structure beneath it{example}.");
    Ok(())
}
