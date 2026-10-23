//! `vivo task`.

use vivo::config_editor::update_file;
use vivo::ensure_path;

use crate::util::{get_or_prompt_opt, prompt, require_or_prompt, interactive, CmdResult, Stop};

pub fn cmd_task_add(config_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    let name = require_or_prompt(matches, "name", "Task name:")?;

    // `--dir` names one folder; on a terminal, ask for as many as the task needs.
    let folders: Vec<String> = match matches.get_one::<String>("dir") {
        Some(dir) => vec![dir.clone()],
        None if interactive() => prompt(vivo::folders::ask_folders("Which folder should it back up?", None))?,
        None => Vec::new(),
    };
    let repo = match matches.get_one::<String>("repo") {
        Some(repo) => repo.clone(),
        None if interactive() => {
            // Suggest a spot beside the existing repositories; an unreadable config just means no convention yet.
            let config = vivo::BackupConfig::from_file(config_path).ok();
            let existing: Vec<&str> =
                config.iter().flat_map(|c| &c.tasks).filter_map(|t| t.backup().map(|b| b.repo())).collect();
            let suggestion = vivo::config_editor::suggest_repo_path(&existing, &name);
            prompt(
                inquire::Text::new("Where should this task's backup repository be stored?")
                    .with_default(&suggestion)
                    .with_help_message("A local copy of the backups, created for you; remotes are off-site copies of it")
                    .prompt(),
            )?
        }
        None => return Err(Stop::Failed("--repo is required in non-interactive mode".to_string())),
    };
    let exclude_file = get_or_prompt_opt(matches, "exclude-file", "Exclude file path:")?;

    let mut plan = vivo::templates::custom_plan(folders.clone());
    plan.exclude_file = exclude_file;
    update_file(config_path, |kdl| vivo::templates::apply_plan(kdl, &name, &repo, &plan))?;

    // Only the repository is created. A folder to back up that doesn't exist is
    // most likely a typo or an unmounted drive: say so rather than invent it.
    ensure_path(&repo);
    for line in vivo::sources::report(&vivo::sources::check_folders(&folders)) {
        eprintln!("warning: {line}");
    }
    println!("Added task '{name}'.");
    if plan.guards_photos() {
        println!("A Photos library is among those folders, so it won't be backed up while Photos is open.");
    }
    Ok(())
}

pub fn cmd_task_list(config_path: &str) -> CmdResult {
    let config = vivo::BackupConfig::from_file(config_path)?;

    for task in &config.tasks {
        match task.description() {
            Some(desc) => println!("{} — {desc}", task.name),
            None => println!("{}", task.name),
        }
    }
    Ok(())
}

pub fn cmd_task_remove(config_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    let name = matches.get_one::<String>("name").expect("name is required");

    if interactive() {
        let confirmed = prompt(
            inquire::Confirm::new(&format!("Remove task '{name}'?"))
                .with_default(false)
                .prompt(),
        )?;
        if !confirmed {
            return Ok(());
        }
    }

    update_file(config_path, |kdl| vivo::remove_task(kdl, name))?;
    println!("Removed task '{name}'.");
    Ok(())
}
