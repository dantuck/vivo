use std::collections::HashSet;

use log::info;

use kdl::{KdlNode, KdlValue};

use crate::backup_config::backup::{Backup, LeftOut, Remote, RunOutcome};
use crate::backup_config::kdl_decode::{named, ConfigError, Source};
use crate::config::{Secrets, VivoConfig};
use crate::notice::{Notice, RunError};
use crate::schedule::When;
use crate::ui;

/// A `before` check: a command that must succeed before the task's backup,
/// and optionally why, in words, for when it doesn't.
#[derive(Debug, Clone, PartialEq)]
pub struct Before {
    pub command: String,
    /// `reason="Photos is open"`: what a failure means, in plain words.
    reason: Option<String>,
    /// `fix="Quit Photos (Cmd+Q)"`: what to do about it.
    fix: Option<String>,
    /// `skip="**/*.photoslibrary"`: instead of stopping the task when this check
    /// fails, leave this pattern out of that run's backup and carry on.
    skip: Option<String>,
    /// `quiet=#true` hides the command's own output (`#false` shows it). Unset:
    /// hidden for a `! pgrep -x Name` check, whose output is just a process ID,
    /// and shown for anything else, such as an export that reports its progress.
    quiet: Option<bool>,
}

/// `Name` from a `! pgrep -x Name` check (a trailing redirect is fine): the
/// usual "don't back up while this app is running" guard.
fn running_app(command: &str) -> Option<String> {
    let rest = command.trim().strip_prefix('!')?.trim_start().strip_prefix("pgrep")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let args = rest.split('>').next()?;
    let name = args.split_whitespace().rfind(|word| !word.starts_with('-'))?;
    // a plain process name only: anything with quotes or patterns is left alone
    name.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '_' | '-')).then(|| name.to_string())
}

impl Before {
    pub(crate) fn decode(src: &Source, node: &KdlNode) -> Result<Self, ConfigError> {
        if node.children().is_some() {
            return Err(src.error(node, "'before' does not take a child block"));
        }
        let (mut command, mut reason, mut fix, mut skip, mut quiet) = (None, None, None, None, None);
        for entry in node.entries() {
            let name = entry.name().map(|n| n.value());
            if name == Some("quiet") {
                match entry.value() {
                    KdlValue::Bool(b) => quiet = Some(*b),
                    _ => return Err(src.error(node, "'quiet' must be #true or #false")),
                }
                continue;
            }
            let text = match entry.value() {
                KdlValue::String(s) => s.clone(),
                _ => return Err(src.error(node, "'before' takes strings")),
            };
            match name {
                None if command.is_some() => return Err(src.error(node, "'before' takes exactly one command")),
                None => command = Some(text),
                Some("reason") => reason = Some(text),
                Some("fix") => fix = Some(text),
                Some("skip") => skip = Some(text),
                Some(other) => {
                    return Err(src.error(node, format!("'before' has no property '{other}' (expected: reason, fix, skip, quiet)")))
                }
            }
        }
        let command = command.ok_or_else(|| src.error(node, "'before' needs a command"))?;
        Ok(Before { command, reason, fix, skip, quiet })
    }

    /// What a failure means: the configured `reason`, or for a `! pgrep -x Name`
    /// check, that Name is running. `None` for any other command.
    /// Whether the command's own output is hidden.
    fn is_quiet(&self) -> bool {
        self.quiet.unwrap_or_else(|| running_app(&self.command).is_some())
    }

    pub fn reason(&self) -> Option<String> {
        self.reason.clone().or_else(|| running_app(&self.command).map(|app| format!("{app} is running")))
    }

    /// What to do about it: the configured `fix`, or for a `! pgrep -x Name`
    /// check, to quit Name. `None` for any other command.
    pub fn fix(&self) -> Option<String> {
        self.fix.clone().or_else(|| {
            running_app(&self.command).map(|app| format!("Quit {app} (an app can keep running with no window open)"))
        })
    }

    /// What to do about a failed check, ending with the command to run again.
    fn to_continue(&self, task: &str) -> String {
        match self.fix() {
            Some(fix) => format!("{fix}, then run `vivo {task}` again"),
            None => format!("Make that check pass, then run `vivo {task}` again"),
        }
    }

    /// A soft check that failed: what to leave out of this run, an informational
    /// notice saying so, and the sentence for `vivo status`.
    fn left_out(&self, pattern: &str, task: &str, status: &str) -> (LeftOut, Notice) {
        let reason = self.reason().unwrap_or_else(|| format!("a `before` check failed ({status})"));
        let notice = Notice::new(format!("Part of '{task}' is being left out this time."))
            .row("Blocked by", reason.clone())
            .row("Left out", format!("{pattern} (this run only)"))
            .row("Still backed up", "everything else in this task")
            .row("To include it", self.to_continue(task));
        (LeftOut { pattern: pattern.to_string(), warning: format!("{reason}, so {pattern} was left out of this backup") }, notice)
    }

    /// The explanation for a failed check: a one-line summary for `vivo status`
    /// and hooks, and a notice for the console. `status` is how the command
    /// ended, when it did not simply report failure.
    fn blocked(&self, task: &str, has_backup: bool, status: &str) -> RunError {
        let headline = if has_backup { format!("Backup of '{task}' did not run.") } else { format!("Task '{task}' did not run.") };
        let reason = self.reason();

        let notice = Notice::new(headline)
            .row("Blocked by", reason.clone().unwrap_or_else(|| format!("a `before` check failed ({status})")))
            .row("Check", format!("{} (a `before` line in your config)", self.command))
            .row("To continue", self.to_continue(task));

        let summary = match reason {
            Some(reason) => format!("{reason}, so nothing was backed up"),
            None => format!("before command failed ({status}), so nothing was backed up: {}", self.command),
        };
        RunError::with_notice(summary, notice)
    }
}

#[derive(Debug)]
pub struct Task {
    pub name: String,
    description: Option<String>,
    backup: Option<Backup>,
    /// When `vivo schedule apply` runs this task automatically.
    schedule: Option<When>,
    /// Run before the task's backup; one that fails stops the whole task.
    before: Vec<Before>,
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
                .map(|n| Before::decode(src, n))
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
        let leave_out = match self.run_before(config) {
            Ok(leave_out) => leave_out,
            Err(e) => {
                e.present();
                if self.backup.is_some() {
                    crate::state::record_run(&self.name, false, &RunOutcome::default(), Some(&e.summary));
                }
                self.run_hooks(config, false, Some(&e.summary));
                return false;
            }
        };

        if let Some(backup) = &self.backup {
            match backup.run(config, &self.name, secrets, &leave_out) {
                Ok(outcome) => {
                    let failed = outcome.failed_syncs();
                    if failed > 0 {
                        eprintln!("error: {failed} remote sync(s) failed");
                        ok = false;
                    }
                    // restic's own output scrolls by; say again what it was worth knowing.
                    for warning in &outcome.warnings {
                        eprintln!("warning: {warning}");
                    }
                    crate::state::record_run(&self.name, outcome.dry_run, &outcome, None);
                }
                Err(e) => {
                    e.present();
                    crate::state::record_run(&self.name, self.is_dry_run(config), &RunOutcome::default(), Some(&e.summary));
                    ok = false;
                    error = Some(e.summary);
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
    /// A `! pgrep -x Name` check runs quietly (see [`Before::is_quiet`]); a failure
    /// of any check is explained as a [`Notice`].
    fn run_before(&self, config: &VivoConfig) -> Result<Vec<LeftOut>, RunError> {
        let mut leave_out = Vec::new();
        for guard in &self.before {
            let command = &guard.command;
            if self.is_dry_run(config) {
                ui::info(&format!("[dry-run] would run before: {command}"));
                continue;
            }
            ui::info(&format!("Running before: {command}"));
            let mut check = std::process::Command::new("sh");
            check.args(["-c", command]);
            if guard.is_quiet() {
                // `pgrep` prints a process ID; the notice says all there is to say.
                check.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
            }
            match check.status() {
                Ok(s) if s.success() => {}
                Ok(s) => match &guard.skip {
                    // a soft check: carry on without that part, and say so
                    Some(pattern) => {
                        let (left, notice) = guard.left_out(pattern, &self.name, &s.to_string());
                        notice.print();
                        leave_out.push(left);
                    }
                    None => return Err(guard.blocked(&self.name, self.backup.is_some(), &s.to_string())),
                },
                Err(e) => return Err(format!("could not run before command '{command}': {e}").into()),
            }
        }
        Ok(leave_out)
    }

    pub fn before_commands(&self) -> Vec<&str> {
        self.before.iter().map(|b| b.command.as_str()).collect()
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
    use super::Before;
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
    fn before_takes_an_optional_reason_and_nothing_else() {
        let src = |node: &str| format!("default-task \"t\"\ntasks {{ task \"t\" {{ {node} }} }}");
        let cfg = BackupConfig::parse("t", &src("before \"! pgrep -x Photos\" reason=\"Photos is open\"")).unwrap();
        assert_eq!(cfg.tasks[0].before_commands(), ["! pgrep -x Photos"]);
        assert_eq!(cfg.tasks[0].before[0].reason().as_deref(), Some("Photos is open"));
        // the reason may come first, as KDL properties may
        assert!(BackupConfig::parse("t", &src("before reason=\"r\" \"cmd\"")).is_ok());

        for bad in [
            "before \"a\" colour=\"red\"",
            "before \"a\" \"b\"",
            "before reason=\"only a reason\"",
            "before \"a\" reason=3",
            "before \"a\" quiet=\"yes\"",
            "before \"a\" reason=\"r\" { }",
        ] {
            assert!(BackupConfig::parse("t", &src(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_pgrep_guard_implies_its_reason() {
        let reason = |command: &str| Before { command: command.to_string(), reason: None, fix: None, skip: None, quiet: None }.reason();
        for (command, app) in [
            ("! pgrep -x Photos", "Photos"),
            ("!pgrep -x Photos", "Photos"),
            ("! pgrep Photos", "Photos"),
            ("! pgrep -ix Photos > /dev/null", "Photos"),
            ("  ! pgrep -x Final-Cut.Pro >/dev/null 2>&1", "Final-Cut.Pro"),
        ] {
            assert_eq!(reason(command).as_deref(), Some(format!("{app} is running").as_str()), "{command}");
        }
        // anything else says nothing, and the written reason always wins
        for command in ["pgrep -x Photos", "! pgrep", "! pgrepx Photos", "! pgrep -f 'a b'", "exit 1", "test -f /x"] {
            assert_eq!(reason(command), None, "{command}");
        }
        let written = Before { command: "! pgrep -x Photos".into(), reason: Some("Photos is open".into()), fix: None, skip: None, quiet: None };
        assert_eq!(written.reason().as_deref(), Some("Photos is open"));
    }

    #[test]
    fn the_console_explains_what_is_blocking_and_what_to_do() {
        // the check already in a config written before `reason` existed
        let plain = Before { command: "! pgrep -x Photos".into(), reason: None, fix: None, skip: None, quiet: None };
        let err = plain.blocked("photos", true, "exit status: 1");
        assert_eq!(err.summary, "Photos is running, so nothing was backed up");
        assert_eq!(
            err.notice.unwrap().render(200),
            "[i] Backup of 'photos' did not run.\n\
             \n\
             \x20   Blocked by   Photos is running\n\
             \x20   Check        ! pgrep -x Photos (a `before` line in your config)\n\
             \x20   To continue  Quit Photos (an app can keep running with no window open), then run `vivo photos` again\n"
        );

        // written down, both are the user's own words
        let written = Before {
            command: "! pgrep -x Photos".into(),
            reason: Some("Photos is open".into()),
            fix: Some("Quit Photos (Cmd+Q)".into()),
            skip: None,
            quiet: None,
        };
        let shown = written.blocked("photos", true, "exit status: 1").notice.unwrap().render(200);
        assert!(shown.contains("Blocked by   Photos is open\n"), "{shown}");
        assert!(shown.contains("To continue  Quit Photos (Cmd+Q), then run `vivo photos` again\n"), "{shown}");

        // a check that says nothing about itself still explains where it came from
        let opaque = Before { command: "test -f /ready".into(), reason: None, fix: None, skip: None, quiet: None };
        let err = opaque.blocked("docs", false, "exit status: 1");
        assert_eq!(err.summary, "before command failed (exit status: 1), so nothing was backed up: test -f /ready");
        let shown = err.notice.unwrap().render(200);
        assert!(shown.starts_with("[i] Task 'docs' did not run.\n"), "a task with no backup is not 'a backup': {shown}");
        assert!(shown.contains("Blocked by   a `before` check failed (exit status: 1)"), "{shown}");
        assert!(shown.contains("To continue  Make that check pass, then run `vivo docs` again"), "{shown}");
    }

    #[test]
    fn output_is_hidden_only_for_process_checks_unless_asked() {
        let q = |command: &str, quiet: Option<bool>| Before { command: command.into(), reason: Some("r".into()), fix: None, skip: None, quiet }.is_quiet();
        assert!(q("! pgrep -x Photos", None), "a pgrep check only prints a process ID");
        assert!(!q("osxphotos export /x", None), "an export's progress should be seen, even with a reason");
        assert!(q("osxphotos export /x", Some(true)));
        assert!(!q("! pgrep -x Photos", Some(false)));
        let src = "default-task \"t\"\ntasks { task \"t\" { before \"c\" quiet=#true } }";
        assert_eq!(BackupConfig::parse("t", src).unwrap().tasks[0].before[0].quiet, Some(true));
    }

    #[test]
    fn a_soft_check_names_what_to_leave_out_when_it_fails() {
        let src = "default-task \"t\"\ntasks { task \"t\" { before \"exit 1\" reason=\"Photos is open\" fix=\"Quit Photos\" skip=\"**/*.photoslibrary\" } }";
        let cfg = BackupConfig::parse("t", src).unwrap();
        assert_eq!(cfg.tasks[0].before[0].skip.as_deref(), Some("**/*.photoslibrary"));

        // a failing soft check does not stop the task: it hands back what to leave out
        let left = cfg.tasks[0].run_before(&vivo_config(false)).unwrap();
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].pattern, "**/*.photoslibrary");
        assert_eq!(left[0].warning, "Photos is open, so **/*.photoslibrary was left out of this backup");

        // a passing one leaves nothing out; a dry run does not run it at all
        let ok = BackupConfig::parse("t", &src.replace("exit 1", "true")).unwrap();
        assert!(ok.tasks[0].run_before(&vivo_config(false)).unwrap().is_empty());
        assert!(cfg.tasks[0].run_before(&vivo_config(true)).unwrap().is_empty());
    }

    #[test]
    fn a_soft_check_is_explained_as_information() {
        let check = Before {
            command: "! pgrep -x Photos".into(),
            reason: Some("Photos is open".into()),
            fix: Some("Quit Photos (Cmd+Q)".into()),
            skip: Some("**/*.photoslibrary".into()),
            quiet: None,
        };
        let (left, notice) = check.left_out("**/*.photoslibrary", "photos", "exit status: 1");
        assert_eq!(left.pattern, "**/*.photoslibrary");
        assert_eq!(
            notice.render(200),
            "[i] Part of 'photos' is being left out this time.\n\
             \n\
             \x20   Blocked by       Photos is open\n\
             \x20   Left out         **/*.photoslibrary (this run only)\n\
             \x20   Still backed up  everything else in this task\n\
             \x20   To include it    Quit Photos (Cmd+Q), then run `vivo photos` again\n"
        );
    }

    #[test]
    fn before_takes_a_fix_too() {
        let src = "default-task \"t\"\ntasks { task \"t\" { before \"c\" reason=\"r\" fix=\"f\" } }";
        let cfg = BackupConfig::parse("t", src).unwrap();
        assert_eq!(cfg.tasks[0].before[0].fix().as_deref(), Some("f"));
    }

    #[test]
    fn a_failed_check_with_a_reason_says_it_in_words() {
        let dir = tempfile::tempdir().unwrap();
        let err = dir.path().join("error").display().to_string();
        let kdl = format!(
            "default-task \"t\"\ntasks {{ task \"t\" {{\n before \"echo 743; exit 1\" reason=\"Photos is open\"\n on-failure \"echo \\\"$VIVO_ERROR\\\" > '{err}'\"\n}} }}"
        );
        assert!(!run_task(&kdl, false));
        let said = std::fs::read_to_string(dir.path().join("error")).unwrap();
        // the hook and `vivo status` get one short line; the explanation is for the console
        assert_eq!(said.trim(), "Photos is open, so nothing was backed up");
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
