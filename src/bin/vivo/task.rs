//! `vivo task`.

use vivo::config_editor::update_file;
use vivo::ensure_path;

use crate::util::{get_or_prompt_opt, prompt, require_or_prompt, interactive, CmdResult};

pub fn cmd_task_add(config_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    let name = require_or_prompt(matches, "name", "Task name:")?;
    let repo = require_or_prompt(
        matches,
        "repo",
        "Restic repo path (e.g. $HOME/.local/share/restic/main):",
    )?;
    let directory = get_or_prompt_opt(matches, "dir", "Directory to back up:")?;
    let exclude_file = get_or_prompt_opt(matches, "exclude-file", "Exclude file path:")?;

    update_file(config_path, |kdl| {
        vivo::add_task(
            kdl,
            vivo::TaskSpec {
                name: name.clone(),
                repo: Some(repo.clone()),
                directory: directory.clone(),
                exclude_file,
            },
        )
    })?;
    ensure_path(&repo);
    if let Some(dir) = &directory {
        ensure_path(dir);
    }
    println!("Added task '{name}'.");
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
