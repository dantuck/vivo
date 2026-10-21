use std::collections::HashSet;

use log::info;

use kdl::KdlNode;

use crate::backup_config::backup::{Backup, Remote, RunOutcome};
use crate::backup_config::kdl_decode::{named, ConfigError, Source};
use crate::config::{Secrets, VivoConfig};
use crate::schedule::When;
use crate::ui;

#[derive(Debug)]
pub struct Task {
    pub name: String,
    description: Option<String>,
    backup: Option<Backup>,
    /// When `vivo schedule apply` runs this task automatically.
    schedule: Option<When>,
    /// Run before the task's backup; one that fails stops the whole task.
    before: Vec<String>,
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
        src.only(children, &["description", "backup", "schedule", "before", "command", "calls", "on-success", "on-failure"])?;
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
            schedule: src
                .optional(children, "schedule")?
                .map(|n| When::parse(&src.string_value(n)?).map_err(|e| src.error(n, e)))
                .transpose()?,
            before: named(children, "before")
                .map(|n| src.leaf_argument(n))
                .collect::<Result<_, _>>()?,
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
    secrets: &Secrets,
) -> bool {
    if seen.contains(name) {
        eprintln!("warning: circular reference detected, skipping task '{name}'");
        return true;
    }
    seen.insert(name.to_string());
    let ok = match tasks.iter().find(|t| t.name == name) {
        Some(task) => {
            let ok = task.run_inner(config, tasks, secrets, seen);
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

    /// The backup's `password-profile`, if it has one.
    pub fn password_profile(&self) -> Option<&str> {
        self.backup.as_ref().and_then(|b| b.password_profile())
    }

    pub fn schedule(&self) -> Option<When> {
        self.schedule
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
        secrets: &Secrets,
    ) -> bool {
        // Seed seen with self so any call chain that loops back to the root is caught.
        let mut seen = HashSet::from([self.name.clone()]);
        self.run_inner(config, tasks, secrets, &mut seen)
    }

    fn run_inner(
        &self,
        config: &VivoConfig,
        tasks: &[Task],
        secrets: &Secrets,
        seen: &mut HashSet<String>,
    ) -> bool {
        let mut ok = true;
        let mut error = None;
        info!("Running task [{}]", self.name);
        ui::section_header(&format!("Running task [{}]", self.name));

        if let Some(description) = &self.description {
            println!("Description: {description}");
        }

        // A guard that fails means the backup must not happen at all.
        if let Err(e) = self.run_before(config) {
            eprintln!("error: {e}");
            if self.backup.is_some() {
                crate::state::record_run(&self.name, false, &RunOutcome::default(), Some(&e));
            }
            self.run_hooks(config, false, Some(&e));
            return false;
        }

        if let Some(backup) = &self.backup {
            match backup.run(config, &self.name, secrets) {
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
            ok &= self.run_commands_and_calls(config, tasks, secrets, seen);
        }

        self.run_hooks(config, ok, error.as_deref());
        ok
    }

    /// Runs the `before` commands in order, stopping at the first that fails.
    /// A dry run only says what it would run, since these may change things.
    fn run_before(&self, config: &VivoConfig) -> Result<(), String> {
        for command in &self.before {
            if self.is_dry_run(config) {
                ui::info(&format!("[dry-run] would run before: {command}"));
                continue;
            }
            ui::info(&format!("Running before: {command}"));
            match std::process::Command::new("sh").args(["-c", command]).status() {
                Ok(s) if s.success() => {}
                Ok(s) => return Err(format!("before command failed ({s}), so nothing was backed up: {command}")),
                Err(e) => return Err(format!("could not run before command '{command}': {e}")),
            }
        }
        Ok(())
    }

    pub fn before_commands(&self) -> &[String] {
        &self.before
    }

    fn run_commands_and_calls(
        &self,
        config: &VivoConfig,
        tasks: &[Task],
        secrets: &Secrets,
        seen: &mut HashSet<String>,
    ) -> bool {
        let mut ok = true;
        for command in &self.commands {
            run_command(command);
        }

        for call in &self.calls {
            ok &= run_call(call, seen, config, tasks, secrets);
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

#[cfg(test)]
mod tests {
    use crate::backup_config::BackupConfig;
    use crate::config::{Secrets, VivoConfig};
    use crate::step::Step;

    fn vivo_config(dry_run: bool) -> VivoConfig {
        VivoConfig { task_name: None, config_file: None, dry_run, start_step: Step::Backup, force: false }
    }

    /// Runs task "t" from `kdl`, with state written to a throwaway directory.
    fn run_task(kdl: &str, dry_run: bool) -> bool {
        let _guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let state = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("VIVO_STATE_DIR");
        std::env::set_var("VIVO_STATE_DIR", state.path());

        let config = BackupConfig::parse("t", kdl).unwrap();
        let secrets = Secrets { restic_password: "pw".into(), ..Secrets::default() };
        let ok = config.tasks[0].run(&vivo_config(dry_run), &config.tasks, &secrets);

        match previous {
            Some(v) => std::env::set_var("VIVO_STATE_DIR", v),
            None => std::env::remove_var("VIVO_STATE_DIR"),
        }
        ok
    }

    #[test]
    fn before_parses_in_order_and_rejects_a_block() {
        let ok = "default-task \"t\"\ntasks { task \"t\" { before \"a\"\nbefore \"b\" } }";
        let cfg = BackupConfig::parse("t", ok).unwrap();
        assert_eq!(cfg.tasks[0].before_commands(), ["a", "b"]);
        for bad in ["before", "before 3", "before \"a\" { }"] {
            let src = format!("default-task \"t\"\ntasks {{ task \"t\" {{ {bad} }} }}");
            assert!(BackupConfig::parse("t", &src).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_failing_before_command_stops_the_task_and_fires_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let p = |name: &str| dir.path().join(name).display().to_string();
        let kdl = format!(
            "default-task \"t\"\ntasks {{ task \"t\" {{\n before \"exit 1\"\n command \"touch '{after}'\"\n on-failure \"echo \\\"$VIVO_ERROR\\\" > '{err}'\"\n backup {{ repo \"{repo}\"\ndirectory \"{dir}\" }}\n}} }}",
            after = p("command-ran"),
            err = p("error"),
            repo = p("repo"),
            dir = dir.path().display(),
        );
        assert!(!run_task(&kdl, false));
        assert!(!dir.path().join("command-ran").exists(), "later commands must not run");
        assert!(!dir.path().join("repo").exists(), "the backup must not even start");
        let error = std::fs::read_to_string(dir.path().join("error")).unwrap();
        assert!(error.contains("before command failed") && error.contains("exit 1"), "{error}");
    }

    #[test]
    fn before_commands_run_in_order_before_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("log").display().to_string();
        let kdl = format!(
            "default-task \"t\"\ntasks {{ task \"t\" {{ before \"echo 1 >> '{log}'\"\nbefore \"echo 2 >> '{log}'\"\ncommand \"echo 3 >> '{log}'\" }} }}"
        );
        assert!(run_task(&kdl, false));
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "1\n2\n3\n");
    }

    #[test]
    fn a_dry_run_does_not_run_before_commands() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("ran");
        let kdl = format!(
            "default-task \"t\"\ntasks {{ task \"t\" {{ before \"touch '{}'\" }} }}",
            marker.display()
        );
        assert!(run_task(&kdl, true));
        assert!(!marker.exists());
    }
}
