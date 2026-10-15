use std::collections::{HashMap, HashSet};

use log::info;

use kdl::KdlNode;

use crate::backup_config::backup::Backup;
use crate::backup_config::kdl_decode::{named, ConfigError, Source};
use crate::config::VivoConfig;
use crate::ui;

#[derive(Debug)]
pub struct Task {
    pub name: String,
    description: Option<String>,
    backup: Option<Backup>,
    commands: Vec<CommandItem>,
    calls: Vec<CallRef>,
}

#[derive(Debug)]
struct CommandItem {
    cmd: String,
}

#[derive(Debug)]
struct CallRef {
    name: String,
}

impl Task {
    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(children, &["description", "backup", "command", "calls"])?;
        Ok(Task {
            name: src.argument(node)?,
            description: src
                .optional(children, "description")?
                .map(|n| src.string_value(n))
                .transpose()?,
            backup: src
                .optional(children, "backup")?
                .map(|n| Backup::decode(src, n))
                .transpose()?,
            commands: named(children, "command")
                .map(|n| src.leaf_argument(n).map(|cmd| CommandItem { cmd }))
                .collect::<Result<_, _>>()?,
            calls: named(children, "calls")
                .map(|n| src.leaf_argument(n).map(|name| CallRef { name }))
                .collect::<Result<_, _>>()?,
        })
    }
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
) -> bool {
    if seen.contains(name) {
        eprintln!("warning: circular reference detected, skipping task '{name}'");
        return true;
    }
    seen.insert(name.to_string());
    let ok = match tasks.iter().find(|t| t.name == name) {
        Some(task) => {
            let ok = task.run_inner(config, tasks, credentials, seen);
            ui::info(&format!("Task [{name}] {}.", if ok { "completed" } else { "finished with errors" }));
            ok
        }
        None => {
            eprintln!("error: task '{name}' not found");
            false
        }
    };
    seen.remove(name);
    ok
}

impl Task {
    pub fn backup(&self) -> Option<&Backup> {
        self.backup.as_ref()
    }

    pub fn backup_remotes(&self) -> Vec<(&str, &str)> {
        match &self.backup {
            Some(b) => b.remotes().iter().map(|r| (r.url.as_str(), r.credentials.as_str())).collect(),
            None => vec![],
        }
    }

    pub fn backup_remote_entries(&self) -> Vec<&crate::backup_config::backup::Remote> {
        match &self.backup {
            Some(b) => b.remotes().iter().collect(),
            None => vec![],
        }
    }

    pub fn backup_repo(&self) -> Option<&str> {
        self.backup.as_ref().map(|b| b.repo())
    }

    pub fn backup_directory(&self) -> Option<&str> {
        self.backup.as_ref().and_then(|b| b.directory())
    }

    pub fn backup_exclude_file(&self) -> Option<&str> {
        self.backup.as_ref().and_then(|b| b.exclude_file())
    }

    pub fn backup_files_from(&self) -> Option<&str> {
        self.backup.as_ref().and_then(|b| b.files_from())
    }

    pub fn call_names(&self) -> Vec<&str> {
        self.calls.iter().map(|c| c.name.as_str()).collect()
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn run(
        &self,
        config: &VivoConfig,
        tasks: &[Task],
        credentials: &HashMap<String, HashMap<String, String>>,
    ) -> bool {
        // Seed seen with self so any call chain that loops back to the root is caught.
        let mut seen = HashSet::from([self.name.clone()]);
        self.run_inner(config, tasks, credentials, &mut seen)
    }

    fn run_inner(
        &self,
        config: &VivoConfig,
        tasks: &[Task],
        credentials: &HashMap<String, HashMap<String, String>>,
        seen: &mut HashSet<String>,
    ) -> bool {
        let mut ok = true;
        info!("Running task [{}]", self.name);
        ui::section_header(&format!("Running task [{}]", self.name));

        if let Some(description) = &self.description {
            println!("Description: {description}");
        }

        if let Some(backup) = &self.backup {
            match backup.run(config, credentials) {
                Ok(0) => {}
                Ok(failed_syncs) => {
                    eprintln!("error: {failed_syncs} remote sync(s) failed");
                    ok = false;
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    return false;
                }
            }
        }

        for command in &self.commands {
            run_command(&command.cmd);
        }

        for call in &self.calls {
            ok &= run_call(&call.name, seen, config, tasks, credentials);
        }
        ok
    }
}
