//! `vivo verify`: check that a backup repository is intact by reading back
//! some (or all) of its data, locally or on the off-site copies.

use clap::ArgMatches;
use vivo::backup_config::steps::valid_subset;
use vivo::repos::{self, RepoTarget};
use vivo::state::{ago, now};

use crate::util::CmdResult;

const DEFAULT_SAMPLE: &str = "5%";

/// How much of the repository's data `restic check` reads back.
#[derive(Debug, PartialEq)]
enum Depth {
    /// Repository structure and indexes only; no pack data.
    Quick,
    /// A subset of the pack data (restic's `--read-data-subset`).
    Sample(String),
    /// Every pack.
    Full,
}

fn check_args(depth: &Depth, no_lock: bool) -> Vec<String> {
    let mut args = vec!["check".to_string()];
    if no_lock {
        args.push("--no-lock".to_string());
    }
    match depth {
        Depth::Quick => {}
        Depth::Sample(subset) => args.push(format!("--read-data-subset={subset}")),
        Depth::Full => args.push("--read-data".to_string()),
    }
    args
}

fn depth_from(args: &ArgMatches) -> Result<Depth, String> {
    let sample = args.get_one::<String>("sample");
    let flags = [args.get_flag("quick"), args.get_flag("full"), sample.is_some()];
    if flags.iter().filter(|f| **f).count() > 1 {
        return Err("choose one of --quick, --sample and --full".to_string());
    }
    if args.get_flag("quick") {
        return Ok(Depth::Quick);
    }
    if args.get_flag("full") {
        return Ok(Depth::Full);
    }
    let subset = sample.map_or(DEFAULT_SAMPLE, String::as_str);
    if !valid_subset(subset) {
        return Err(format!("invalid --sample '{subset}' (use e.g. 5%, 1/10 or 2G)"));
    }
    Ok(Depth::Sample(subset.to_string()))
}

/// Verifies one repository. Returns whether it is intact.
fn verify_one(target: &RepoTarget, depth: &Depth) -> bool {
    println!("\n{}", target.label());
    if !target.exists() {
        println!("  no repository here yet — nothing to verify");
        return false;
    }
    if !target.is_local && *depth != Depth::Quick {
        println!("  reading data back from the off-site copy; this downloads it (hosts may charge for egress)");
    }

    // A read-only check of an off-site copy must not leave a lock file behind
    // for the next sync to trip over.
    let status = target.restic().args(check_args(depth, !target.is_local)).status();
    let ok = match status {
        Ok(s) if s.success() => true,
        Ok(s) => {
            eprintln!("  restic check failed ({s})");
            false
        }
        Err(e) => {
            eprintln!("  could not run restic: {e}");
            false
        }
    };

    if ok {
        match repos::list_snapshots(target, true) {
            Ok(s) => match s.last().and_then(|s| s.unix_time().map(|t| (s, t))) {
                Some((snap, t)) => println!("  latest snapshot {} from {}", snap.short_id, ago(now() - t)),
                None => println!("  the repository has no snapshots"),
            },
            Err(e) => eprintln!("  could not list snapshots: {e}"),
        }
    }
    ok
}

pub fn cmd_verify(config_path: &str, secrets_path: &str, args: &ArgMatches) -> CmdResult {
    let depth = depth_from(args)?;
    let (config, secrets) = repos::load(config_path, secrets_path)?;
    let tasks = repos::backup_tasks(&config, args.get_one::<String>("task").map(String::as_str))?;

    let mut targets: Vec<RepoTarget> = Vec::new();
    for task in tasks {
        let all = repos::targets_for(task, &secrets);
        if args.get_flag("remotes") {
            targets.extend(all.into_iter().filter(|t| !t.is_local));
        } else {
            targets.push(repos::find_source(&all, args.get_one::<String>("from").map(String::as_str))?);
        }
    }
    if targets.is_empty() {
        return Err("nothing to verify: no remote is configured".to_string().into());
    }

    let results: Vec<bool> = targets.iter().map(|t| verify_one(t, &depth)).collect();
    let bad = results.iter().filter(|ok| !**ok).count();
    println!();
    if bad == 0 {
        println!("All {} repositor{} verified.", results.len(), if results.len() == 1 { "y is" } else { "ies are" });
        Ok(())
    } else {
        Err(format!("{bad} of {} repositories failed verification", results.len()).into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_args_by_depth() {
        assert_eq!(check_args(&Depth::Quick, false), ["check"]);
        assert_eq!(check_args(&Depth::Sample("5%".into()), false), ["check", "--read-data-subset=5%"]);
        assert_eq!(check_args(&Depth::Full, true), ["check", "--no-lock", "--read-data"]);
    }

    fn parse(args: &[&str]) -> Result<Depth, String> {
        let cmd = vivo::build_cli();
        let m = cmd.try_get_matches_from(std::iter::once("vivo").chain(["verify"]).chain(args.iter().copied())).unwrap();
        depth_from(m.subcommand_matches("verify").unwrap())
    }

    #[test]
    fn depth_defaults_to_a_small_sample() {
        assert_eq!(parse(&[]), Ok(Depth::Sample("5%".into())));
        assert_eq!(parse(&["--sample", "1/10"]), Ok(Depth::Sample("1/10".into())));
        assert_eq!(parse(&["--quick"]), Ok(Depth::Quick));
        assert_eq!(parse(&["--full"]), Ok(Depth::Full));
    }

    #[test]
    fn depth_rejects_bad_or_conflicting_options() {
        assert!(parse(&["--sample", "lots"]).is_err());
        assert!(parse(&["--quick", "--full"]).is_err());
        assert!(parse(&["--sample", "5%", "--full"]).is_err());
    }
}
