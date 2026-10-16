use std::collections::{HashMap, HashSet};

use log::info;

use kdl::KdlNode;

use crate::backup_config::backup::{Backup, Remote, RunOutcome};
use crate::backup_config::kdl_decode::{named, ConfigError, Source};
use crate::config::VivoConfig;
use crate::ui;

#[derive(Debug)]
pub struct Task {
    pub name: String,
    description: Option<String>,
    backup: Option<Backup>,
    commands: Vec<String>,
    calls: Vec<String>,
    /// Run after the task finishes with no errors (skipped on dry runs).
    on_success: Vec<String>,
    /// Run after the task finishes with errors (skipped on dry runs).
    on_failure: Vec<String>,
}

impl Task {
    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        let children = src.children(node);
        src.only(children, &["description", "backup", "command", "calls", "on-success", "on-failure"])?;
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
                .map(|n| src.leaf_argument(n))
                .collect::<Result<_, _>>()?,
            calls: named(children, "calls")
                .map(|n| src.leaf_argument(n))
                .collect::<Result<_, _>>()?,
            on_success: named(children, "on-success")
                .map(|n| src.leaf_argument(n))
                .collect::<Result<_, _>>()?,
            on_failure: named(children, "on-failure")
                .map(|n| src.leaf_argument(n))
                .collect::<Result<_, _>>()?,
        })
    }
}

fn run_command(cmd: &str) {
    run_command_with_env(cmd, &[]);
}

fn run_command_with_env(cmd: &str, env: &[(&str, &str)]) {
    ui::info(&format!("Running: {cmd}"));
    match std::process::Command::new("sh").args(["-c", cmd]).envs(env.iter().copied()).status() {
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

    /// `(url, credentials profile)` of each remote of the task's backup.
    pub fn backup_remotes(&self) -> Vec<(&str, &str)> {
        self.remotes().iter().map(|r| (r.url.as_str(), r.credentials.as_str())).collect()
    }

    pub fn remotes(&self) -> &[Remote] {
        self.backup.as_ref().map_or(&[], |b| b.remotes())
    }

    pub fn call_names(&self) -> &[String] {
        &self.calls
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
        let mut error = None;
        info!("Running task [{}]", self.name);
        ui::section_header(&format!("Running task [{}]", self.name));

        if let Some(description) = &self.description {
            println!("Description: {description}");
        }

        if let Some(backup) = &self.backup {
            match backup.run(config, &self.name, credentials) {
                Ok(outcome) => {
                    let failed = outcome.failed_syncs();
                    if failed > 0 {
                        eprintln!("error: {failed} remote sync(s) failed");
                        ok = false;
                    }
                    crate::state::record_run(&self.name, outcome.dry_run, &outcome, None);
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    crate::state::record_run(&self.name, self.is_dry_run(config), &RunOutcome::default(), Some(&e));
                    ok = false;
                    error = Some(e);
                }
            }
        }

        // A failed backup step stops the task: later commands and calls would
        // run against a repo that did not get its snapshot.
        if error.is_none() {
            ok &= self.run_commands_and_calls(config, tasks, credentials, seen);
        }

        self.run_hooks(config, ok, error.as_deref());
        ok
    }

    fn run_commands_and_calls(
        &self,
        config: &VivoConfig,
        tasks: &[Task],
        credentials: &HashMap<String, HashMap<String, String>>,
        seen: &mut HashSet<String>,
    ) -> bool {
        let mut ok = true;
        for command in &self.commands {
            run_command(command);
        }

        for call in &self.calls {
            ok &= run_call(call, seen, config, tasks, credentials);
        }
        ok
    }

    /// True for `--dry-run` or the task's own `dry-run true`.
    fn is_dry_run(&self, config: &VivoConfig) -> bool {
        config.dry_run || self.backup.as_ref().is_some_and(|b| b.dry_run())
    }

    /// Runs `on-success` / `on-failure` commands. They see `VIVO_TASK`,
    /// `VIVO_STATUS` (`success` or `failure`) and `VIVO_ERROR` (may be empty).
    fn run_hooks(&self, config: &VivoConfig, ok: bool, error: Option<&str>) {
        let hooks = if ok { &self.on_success } else { &self.on_failure };
        if self.is_dry_run(config) || hooks.is_empty() {
            return;
        }
        let env = [
            ("VIVO_TASK", self.name.as_str()),
            ("VIVO_STATUS", if ok { "success" } else { "failure" }),
            ("VIVO_ERROR", error.unwrap_or("")),
        ];
        for hook in hooks {
            run_command_with_env(hook, &env);
        }
    }
}
