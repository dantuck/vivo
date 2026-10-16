mod config;
mod init;
mod onboard;
mod recover;
mod remote;
mod restore;
mod schedule;
mod secrets;
mod snapshots;
mod status;
mod task;
mod util;

use log::debug;
use std::process;
use vivo::{build_cli, config_path_from, secrets_path_from, VivoConfig};

use config::{cmd_config_edit, cmd_config_init, cmd_config_show};
use init::cmd_init;
use onboard::{ensure_onboarded, run_backup};
use recover::cmd_recover;
use restore::cmd_restore;
use schedule::{cmd_schedule_add, cmd_schedule_remove, cmd_schedule_status};
use snapshots::cmd_snapshots;
use status::cmd_status;
use remote::{cmd_remote_add, cmd_remote_list, cmd_remote_remove, cmd_remote_test};
use secrets::{
    cmd_secrets_edit, cmd_secrets_import_b2, cmd_secrets_import_s3, cmd_secrets_init,
    cmd_secrets_show,
};
use task::{cmd_task_add, cmd_task_list, cmd_task_remove};
use util::finish;

fn cmd_doctor(config_path: &str, secrets_path: &str, fix: bool) {
    let exit_code = vivo::doctor::run_doctor(config_path, secrets_path, fix);
    std::process::exit(exit_code);
}

fn cmd_update(dry_run: bool) {
    if let Err(e) = vivo::update::apply_update(dry_run) {
        let msg = e.to_string();
        if msg.contains("Permission denied") || msg.contains("Access is denied") {
            eprintln!("error: cannot replace binary — try: sudo vivo update");
        } else {
            eprintln!("error: update failed: {msg}");
        }
        std::process::exit(1);
    }
}

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
        vivo::ui::print_banner();
    }

    let matches = build_cli().get_matches();

    debug!("args parsed");

    let config_path = config_path_from(matches.get_one("config"));
    let secrets_path = secrets_path_from();

    match matches.subcommand() {
        Some(("init", _)) => {
            if !cmd_init(&config_path, &secrets_path) {
                process::exit(1);
            }
            return;
        }
        Some(("config", sub)) => {
            match sub.subcommand() {
                Some(("init", _)) => {
                    if !cmd_config_init(&config_path) {
                        process::exit(1);
                    }
                }
                Some(("edit", _)) => cmd_config_edit(&config_path),
                Some(("show", _)) => cmd_config_show(&config_path),
                _ => unreachable!(),
            }
            return;
        }
        Some(("secrets", sub)) => {
            match sub.subcommand() {
                Some(("init", _)) => {
                    if !cmd_secrets_init(&secrets_path) {
                        process::exit(1);
                    }
                }
                Some(("edit", _)) => cmd_secrets_edit(&secrets_path),
                Some(("show", _)) => cmd_secrets_show(&secrets_path),
                Some(("import-b2", _)) => cmd_secrets_import_b2(&secrets_path),
                Some(("import-s3", args)) => finish(cmd_secrets_import_s3(&secrets_path, args)),
                _ => unreachable!(),
            }
            return;
        }
        Some(("task", sub)) => {
            match sub.subcommand() {
                Some(("add", args)) => finish(cmd_task_add(&config_path, args)),
                Some(("list", _)) => finish(cmd_task_list(&config_path)),
                Some(("remove", args)) => finish(cmd_task_remove(&config_path, args)),
                _ => unreachable!(),
            }
            return;
        }
        Some(("remote", sub)) => {
            match sub.subcommand() {
                Some(("add", args)) => finish(cmd_remote_add(&config_path, &secrets_path, args)),
                Some(("list", args)) => finish(cmd_remote_list(&config_path, args)),
                Some(("test", args)) => finish(cmd_remote_test(&config_path, &secrets_path, args)),
                Some(("remove", args)) => finish(cmd_remote_remove(&config_path, args)),
                _ => unreachable!(),
            }
            return;
        }
        Some(("schedule", sub)) => {
            match sub.subcommand() {
                Some(("add", args)) => finish(cmd_schedule_add(&config_path, args)),
                Some(("remove", _)) => finish(cmd_schedule_remove()),
                Some(("status", _)) | None => finish(cmd_schedule_status()),
                _ => unreachable!(),
            }
            return;
        }
        Some(("status", args)) => {
            finish(cmd_status(&config_path, &secrets_path, args));
            return;
        }
        Some(("snapshots", args)) => {
            finish(cmd_snapshots(&config_path, &secrets_path, args));
            return;
        }
        Some(("recover", args)) => {
            finish(cmd_recover(&config_path, &secrets_path, args));
            return;
        }
        Some(("restore", args)) => {
            finish(cmd_restore(&config_path, &secrets_path, args));
            return;
        }
        Some(("mount", sub)) => {
            if !ensure_onboarded(&config_path, &secrets_path) {
                return;
            }
            let mount_path = sub.get_one::<String>("path").map(String::as_str);
            if let Err(e) = vivo::mount::run(&config_path, &secrets_path, mount_path) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            return;
        }
        Some(("manage", _)) => {
            if !ensure_onboarded(&config_path, &secrets_path) {
                return;
            }
            if let Err(e) = vivo::tui::run(&config_path) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            return;
        }
        Some(("doctor", sub)) => {
            let fix = sub.get_flag("fix");
            cmd_doctor(&config_path, &secrets_path, fix);
            return;
        }
        Some(("update", _)) => {
            let dry_run = matches.get_flag("dry-run");
            cmd_update(dry_run);
            return;
        }
        _ => {}
    }

    let vivo_config = VivoConfig::from_matches(&matches);
    debug!("{:?}", vivo_config);

    let update_notice = vivo::update::maybe_check_update();

    let ok = run_backup(&vivo_config, &config_path, &secrets_path);

    if let Some(ref latest) = update_notice {
        vivo::update::print_update_notice(latest);
    }

    // Non-zero exit lets cron/launchd/systemd see a failed backup or remote sync.
    if !ok {
        process::exit(1);
    }
}
