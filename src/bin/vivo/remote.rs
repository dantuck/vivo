//! `vivo remote`.

use std::process;

use vivo::config_editor::update_file;

use crate::onboard::ensure_onboarded;
use crate::util::{interactive, prompt, require_or_prompt, CmdResult, Stop};

pub fn cmd_remote_add(config_path: &str, secrets_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    let interactive = interactive();

    // No URL or credentials given on a terminal: walk through the guided flow
    // (backend, validated URL, credential profile, then a connection test).
    if interactive
        && matches.get_one::<String>("url").is_none()
        && matches.get_one::<String>("credentials").is_none()
    {
        if ensure_onboarded(config_path, secrets_path) {
            let task = matches.get_one::<String>("task").map(String::as_str);
            vivo::remote_setup::guided_add(config_path, secrets_path, task)?;
        }
        return Ok(());
    }

    let task = match matches.get_one::<String>("task") {
        Some(t) => t.clone(),
        None if !interactive => {
            return Err(Stop::Failed("--task is required in non-interactive mode".into()));
        }
        None => {
            let default_suggestion = vivo::BackupConfig::from_file(config_path)
                .map(|cfg| cfg.default_task)
                .unwrap_or_default();

            prompt(
                inquire::Text::new("Task name:")
                    .with_default(&default_suggestion)
                    .prompt(),
            )?
        }
    };

    let url = require_or_prompt(matches, "url", "Remote URL (e.g. rustfs:http://nas:9000/bucket):")?;
    vivo::remote_setup::validate_url(&url)?;
    let credentials = require_or_prompt(
        matches,
        "credentials",
        "Credentials profile name (must exist in secrets):",
    )?;

    update_file(config_path, |kdl| {
        vivo::add_remote(
            kdl,
            &task,
            vivo::RemoteSpec {
                url: url.clone(),
                credentials: credentials.clone(),
                mc_max_workers: None,
                mc_limit_upload: None,
            },
        )
    })?;
    println!("Added remote '{url}' to task '{task}'.");

    if interactive {
        vivo::remote_setup::test_after_save(&url, &credentials, secrets_path);
    }
    Ok(())
}

pub fn cmd_remote_test(config_path: &str, secrets_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    if !ensure_onboarded(config_path, secrets_path) {
        process::exit(1);
    }
    let config = vivo::BackupConfig::from_file(config_path)?;
    let secrets = vivo::remote_setup::load_secrets(secrets_path)?;

    let remotes = match matches.get_one::<String>("task") {
        Some(t) => config.remotes_for_task(t),
        None => config.all_remotes(),
    };
    if remotes.is_empty() {
        println!("No remotes configured. Add one with `vivo remote add`.");
        return Ok(());
    }

    let interactive = interactive();
    let mut failed = 0;
    for (url, profile) in &remotes {
        println!("{url}  (credentials: {profile})");
        match vivo::remote_setup::test_remote_with_secrets(url, profile, &secrets, interactive) {
            Ok(msg) => println!("  ok: {msg}\n"),
            Err(e) => {
                eprintln!("  failed: {e}\n");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        return Err(Stop::Failed(format!("{failed} of {} remote(s) failed.", remotes.len())));
    }
    Ok(())
}

pub fn cmd_remote_list(config_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    let config = vivo::BackupConfig::from_file(config_path)?;

    if let Some(task_filter) = matches.get_one::<String>("task") {
        let remotes = config.remotes_for_task(task_filter);
        if remotes.is_empty() {
            println!("No remotes for task '{task_filter}'.");
        }
        for (url, creds) in remotes {
            println!("{url}  [{creds}]");
        }
    } else {
        for task in &config.tasks {
            let remotes = task.backup_remotes();
            if !remotes.is_empty() {
                println!("{}:", task.name);
                for (url, creds) in remotes {
                    println!("  {url}  [{creds}]");
                }
            }
        }
    }
    Ok(())
}

pub fn cmd_remote_remove(config_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    let task = matches.get_one::<String>("task").expect("task is required");
    let url = matches.get_one::<String>("url").expect("url is required");

    if interactive() {
        let confirmed = prompt(
            inquire::Confirm::new(&format!("Remove remote '{url}' from task '{task}'?"))
                .with_default(false)
                .prompt(),
        )?;
        if !confirmed {
            return Ok(());
        }
    }

    update_file(config_path, |kdl| vivo::remove_remote(kdl, task, url))?;
    println!("Removed remote '{url}' from task '{task}'.");
    Ok(())
}
