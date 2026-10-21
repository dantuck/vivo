//! `vivo schedule`: install, update, remove and inspect automatic backups,
//! one schedule per task.

use std::collections::HashMap;

use clap::ArgMatches;
use vivo::schedule::{self, Schedule, When};

use crate::util::{interactive, prompt, CmdResult, Stop};

const DEFAULT_TIME: &str = "02:00";

/// Where the schedule runs the job, for the "what happens next" message.
fn describe(s: &Schedule) -> String {
    format!(
        "runs {} {}",
        s.task.as_deref().map_or("your default task".to_string(), |t| format!("task '{t}'")),
        s.when
    )
}

fn validate_task(config_path: &str, task: &str) -> Result<(), String> {
    let config = vivo::BackupConfig::from_file(config_path)?;
    if config.tasks.iter().any(|t| t.name == task) {
        Ok(())
    } else {
        Err(format!("no task named '{task}' in {config_path}"))
    }
}

/// Installs the schedule for `task` and prints the follow-up notes.
fn install(config_path: &str, task: Option<String>, when: When, print_only: bool) -> CmdResult {
    if let Some(task) = &task {
        validate_task(config_path, task)?;
    }
    let sched = Schedule::new(task, when, config_path)?;
    let backend = schedule::detect_backend();

    if print_only {
        println!("vivo would install a {} entry that {}:\n", backend.name(), describe(&sched));
        match backend {
            schedule::Backend::Launchd => println!("{}", schedule::render_plist(&sched)),
            schedule::Backend::Systemd => {
                println!("{}\n{}", schedule::render_service(&sched), schedule::render_timer(&sched))
            }
            schedule::Backend::Cron => println!("{}", schedule::render_cron_block(&sched)),
        }
        return Ok(());
    }

    if sched.vivo_bin.to_string_lossy().contains("/target/") {
        eprintln!(
            "warning: scheduling a development build ({}). Install vivo first if you want this to keep working.",
            sched.vivo_bin.display()
        );
    }

    // An older unnamed schedule for this same task would now run it twice.
    if let Some(task) = &sched.task {
        let duplicate = schedule::installed_all().iter().any(|u| u.id.is_none() && u.task.as_ref() == Some(task));
        if duplicate {
            schedule::remove_unit(None)?;
            println!("Replaced the older whole-config schedule for task '{task}'.");
        }
    }

    let written = schedule::install(&sched, backend)?;
    println!("Scheduled: vivo {} (via {}, {written}).", describe(&sched), backend.name());
    println!("  Output is logged to {}", sched.log.display());
    println!("  Check on it any time with: vivo status");
    if cfg!(target_os = "macos") {
        println!(
            "  If you back up Documents, Desktop or other protected folders, grant {} Full Disk Access\n  (System Settings → Privacy & Security → Full Disk Access) or those files will be skipped.",
            sched.vivo_bin.display()
        );
        println!("  A Mac that is asleep at {} runs the backup when it wakes.", sched.time());
    }
    Ok(())
}

/// Asks for a time of day, re-asking until it parses.
fn ask_time(label: &str) -> Result<String, inquire::InquireError> {
    inquire::Text::new(label)
        .with_default(DEFAULT_TIME)
        .with_validator(|input: &str| {
            Ok(match schedule::parse_time(input) {
                Ok(_) => inquire::validator::Validation::Valid,
                Err(e) => inquire::validator::Validation::Invalid(e.into()),
            })
        })
        .prompt()
}

/// `HH:MM` plus an optional weekday, as a [`When`].
fn when_at(time: &str, weekday: Option<&str>) -> Result<When, String> {
    let (hour, minute) = schedule::parse_time(time)?;
    Ok(match weekday {
        Some(day) => When::Weekly { day: schedule::parse_day(day)?, hour, minute },
        None => When::Daily { hour, minute },
    })
}

pub fn cmd_schedule_add(config_path: &str, args: &ArgMatches) -> CmdResult {
    let task = args.get_one::<String>("task").cloned();
    let print_only = args.get_flag("print");
    let weekday = args.get_one::<String>("weekly").map(String::as_str);
    let time = match args.get_one::<String>("at") {
        Some(t) => t.clone(),
        None if interactive() && !print_only => {
            prompt(ask_time("What time should the backup run? (24-hour HH:MM)"))?
        }
        None => DEFAULT_TIME.to_string(),
    };
    let when = when_at(&time, weekday)?;

    // The config is the source of truth: record the schedule there so a later
    // `vivo schedule apply` keeps it.
    if let (Some(task), false) = (&task, print_only) {
        validate_task(config_path, task)?;
        vivo::config_editor::update_file(config_path, |kdl| {
            vivo::config_editor::set_task_schedule(kdl, task, Some(&when.to_string()))
        })?;
        println!("Set `schedule \"{when}\"` on task '{task}' in {config_path}.");
    }
    install(config_path, task, when, print_only)
}

/// Makes the installed schedules match the config's `schedule` nodes.
pub fn cmd_schedule_apply(config_path: &str, args: &ArgMatches) -> CmdResult {
    let print_only = args.get_flag("print");
    let config = vivo::BackupConfig::from_file(config_path)?;
    let declared: Vec<(String, When)> =
        config.tasks.iter().filter_map(|t| Some((t.name.clone(), t.schedule()?))).collect();

    let mut slugs: HashMap<String, &str> = HashMap::new();
    for (name, _) in &declared {
        if let Some(other) = slugs.insert(schedule::slug(name), name) {
            return Err(format!(
                "tasks '{other}' and '{name}' would share the schedule name '{}' — rename one",
                schedule::slug(name)
            )
            .into());
        }
    }

    if declared.is_empty() {
        println!("No task has a `schedule` node. Add one to a task, e.g. `schedule \"daily 02:00\"`,");
        println!("or run `vivo schedule add --task <task> --at 02:00`.");
    }
    for (name, when) in &declared {
        install(config_path, Some(name.clone()), *when, print_only)?;
    }

    if print_only {
        return Ok(());
    }
    for unit in schedule::installed_all() {
        let Some(id) = unit.id.as_deref() else { continue };
        if !slugs.contains_key(id) {
            schedule::remove_unit(Some(id))?;
            println!("Removed the schedule for '{}' (no longer in the config).", unit.task.as_deref().unwrap_or(id));
        }
    }
    Ok(())
}

pub fn cmd_schedule_remove(config_path: &str, args: &ArgMatches) -> CmdResult {
    if let Some(task) = args.get_one::<String>("task") {
        let removed = schedule::remove_unit(Some(&schedule::slug(task)))?;
        // Keep the config in step, or `apply` would bring it back. A task that
        // is gone from the config simply has no node to drop.
        let _ = vivo::config_editor::update_file(config_path, |kdl| {
            vivo::config_editor::set_task_schedule(kdl, task, None)
        });
        if removed.is_empty() {
            println!("Task '{task}' had no schedule installed.");
        } else {
            println!("Removed the schedule for task '{task}'.");
        }
        return Ok(());
    }

    let removed = schedule::remove_all()?;
    if removed.is_empty() {
        println!("No vivo schedule was installed.");
    } else {
        for (id, backends) in &removed {
            let names: Vec<&str> = backends.iter().map(|b| b.name()).collect();
            println!("Removed the schedule for {} ({}).", id.as_deref().unwrap_or("the default task"), names.join(", "));
        }
        println!("Backups will no longer run automatically. `schedule` nodes in the config are kept; `vivo schedule apply` reinstalls them.");
    }
    Ok(())
}

pub fn cmd_schedule_status(config_path: &str) -> CmdResult {
    let units = schedule::installed_all();
    if units.is_empty() {
        println!("Not scheduled. Run `vivo schedule add` to back up automatically.");
    }
    for u in &units {
        println!(
            "{}: {} via {}",
            u.task.as_deref().map_or("default task".to_string(), |t| format!("task '{t}'")),
            u.when.map_or("on an unrecorded schedule".to_string(), |w| w.to_string()),
            u.backend.name(),
        );
    }
    if !units.is_empty() {
        println!("Logs: {}", vivo::state::log_path().display());
    }

    // Compare with the config; an unreadable config just skips the comparison.
    if let Ok(config) = vivo::BackupConfig::from_file(config_path) {
        for t in &config.tasks {
            let Some(when) = t.schedule() else { continue };
            let slug = schedule::slug(&t.name);
            match units.iter().find(|u| u.id.as_deref() == Some(&slug)) {
                None => println!("! task '{}' is set to run {when} but is not installed — run `vivo schedule apply`", t.name),
                Some(u) if u.when.is_some_and(|w| w != when) => {
                    println!("! task '{}' is installed for {} but the config says {when} — run `vivo schedule apply`", t.name, u.when.map_or_else(String::new, |w| w.to_string()))
                }
                Some(_) => {}
            }
        }
        for u in units.iter().filter(|u| u.id.is_some()) {
            let declared = config.tasks.iter().any(|t| Some(schedule::slug(&t.name)) == u.id && t.schedule().is_some());
            if !declared {
                println!("! '{}' is installed but no longer in the config — run `vivo schedule apply` to remove it", u.task.as_deref().or(u.id.as_deref()).unwrap_or(""));
            }
        }
    }
    Ok(())
}

/// Offered at the end of `vivo init`. Returns true if a schedule was installed.
pub fn offer_schedule(config_path: &str) -> bool {
    if schedule::installed().is_some() {
        return true;
    }
    println!();
    let wanted = inquire::Confirm::new("Back up automatically every day?")
        .with_default(true)
        .with_help_message("You can change or remove this later with `vivo schedule`")
        .prompt()
        .unwrap_or(false);
    if !wanted {
        return false;
    }
    let Ok(time) = ask_time("What time? (24-hour HH:MM)") else { return false };
    let Ok(when) = when_at(&time, None) else { return false };
    match install(config_path, None, when, false) {
        Ok(()) => true,
        Err(Stop::Failed(e)) => {
            eprintln!("error: {e}");
            false
        }
        Err(Stop::Cancelled) => false,
    }
}
