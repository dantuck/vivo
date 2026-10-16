//! `vivo schedule`: install, remove and inspect the daily automatic backup.

use clap::ArgMatches;
use vivo::schedule::{self, Schedule};

use crate::util::{interactive, prompt, CmdResult, Stop};

const DEFAULT_TIME: &str = "02:00";

/// Where the schedule runs the job, for the "what happens next" message.
fn describe(s: &Schedule) -> String {
    format!(
        "runs {} every day at {}",
        s.task.as_deref().map_or("your default task".to_string(), |t| format!("task '{t}'")),
        s.time()
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

/// Installs the schedule at `time` (`HH:MM`) and prints the follow-up notes.
fn install(config_path: &str, task: Option<String>, time: &str, print_only: bool) -> CmdResult {
    let (hour, minute) = schedule::parse_time(time)?;
    if let Some(task) = &task {
        validate_task(config_path, task)?;
    }
    let sched = Schedule::new(task, hour, minute, config_path)?;
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

pub fn cmd_schedule_add(config_path: &str, args: &ArgMatches) -> CmdResult {
    let task = args.get_one::<String>("task").cloned();
    let print_only = args.get_flag("print");
    let time = match args.get_one::<String>("at") {
        Some(t) => t.clone(),
        None if interactive() && !print_only => {
            prompt(ask_time("What time should the daily backup run? (24-hour HH:MM)"))?
        }
        None => DEFAULT_TIME.to_string(),
    };
    install(config_path, task, &time, print_only)
}

pub fn cmd_schedule_remove() -> CmdResult {
    let removed = schedule::remove()?;
    if removed.is_empty() {
        println!("No vivo schedule was installed.");
    } else {
        let names: Vec<&str> = removed.iter().map(|b| b.name()).collect();
        println!("Removed the vivo schedule ({}). Backups will no longer run automatically.", names.join(", "));
    }
    Ok(())
}

pub fn cmd_schedule_status() -> CmdResult {
    match schedule::installed() {
        Some(i) => println!(
            "Scheduled daily{}{} via {}. Logs: {}",
            i.at.map(|t| format!(" at {t}")).unwrap_or_default(),
            i.task.map(|t| format!(" (task '{t}')")).unwrap_or_default(),
            i.backend.name(),
            vivo::state::log_path().display()
        ),
        None => println!("Not scheduled. Run `vivo schedule add` to back up automatically."),
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
    match install(config_path, None, &time, false) {
        Ok(()) => true,
        Err(Stop::Failed(e)) => {
            eprintln!("error: {e}");
            false
        }
        Err(Stop::Cancelled) => false,
    }
}
