use std::collections::{HashMap, HashSet};

use log::info;

use crate::backup_config::backup::Backup;
use crate::config::VivoConfig;
use crate::ui;

#[derive(knuffel::Decode, Debug)]
pub struct Task {
    #[knuffel(argument)]
    pub name: String,
    #[knuffel(child, unwrap(argument))]
    description: Option<String>,
    #[knuffel(child)]
    backup: Option<Backup>,
    #[knuffel(children(name = "command"))]
    commands: Vec<CommandItem>,
    #[knuffel(children(name = "calls"))]
    calls: Vec<CallRef>,
}

#[derive(knuffel::Decode, Debug)]
struct CommandItem {
    #[knuffel(argument)]
    cmd: String,
}

#[derive(knuffel::Decode, Debug)]
struct CallRef {
    #[knuffel(argument)]
    name: String,
}

fn run_command(cmd: &str) {
    ui::info(&format!("Running: {cmd}"));
    match std::process::Command::new("sh").args(["-c", cmd]).status() {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("warning: command exited with status {s}: {cmd}"),
        Err(e) => eprintln!("warning: failed to run command '{cmd}': {e}"),
    }
}

fn run_call(
    name: &str,
    seen: &mut HashSet<String>,
    config: &VivoConfig,
    tasks: &[Task],
    credentials: &HashMap<String, HashMap<String, String>>,
) {
    if seen.contains(name) {
        eprintln!("warning: circular reference detected, skipping task '{name}'");
        return;
    }
    seen.insert(name.to_string());

    match tasks.iter().find(|t| t.name == name) {
        Some(task) => {
            task.run(config, tasks, credentials);
            ui::info(&format!("Task [{name}] completed."));
        }
        None => eprintln!("error: task '{name}' not found"),
    }
}

impl Task {
    pub fn run(
        &self,
        config: &VivoConfig,
        tasks: &[Task],
        credentials: &HashMap<String, HashMap<String, String>>,
    ) {
        info!("Running task [{}]", self.name);
        ui::section_header(&format!("Running task [{}]", self.name));

        if let Some(description) = &self.description {
            println!("Description: {description}");
        }

        if let Some(backup) = &self.backup {
            backup.run(config, credentials);
        }

        for command in &self.commands {
            run_command(&command.cmd);
        }

        let seen = HashSet::from([self.name.clone()]);
        for call in &self.calls {
            run_call(&call.name, &mut seen.clone(), config, tasks, credentials);
        }
    }
}
