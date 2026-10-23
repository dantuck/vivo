use std::time::{Duration, Instant};

use crate::backup_config::BackupConfig;

#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    Tasks,
    Fields,
    Remotes,
    Calls,
}

impl Pane {
    /// The pane Tab moves to: Tasks → Fields → Remotes → Calls → Tasks.
    pub fn next(self) -> Pane {
        match self {
            Pane::Tasks => Pane::Fields,
            Pane::Fields => Pane::Remotes,
            Pane::Remotes => Pane::Calls,
            Pane::Calls => Pane::Tasks,
        }
    }
}

/// The fields backed by `EditTaskSpec`, in row order.
pub const FIELD_NAMES: &[&str] =
    &["Name", "Description", "Repo", "Directory", "Exclude file", "Files from"];

/// Rows after those, edited on their own: they are not part of `EditTaskSpec`.
pub const EXTRA_FIELD_NAMES: &[&str] = &["Also back up", "Exclude", "Schedule", "Password profile"];

/// Index of the "Also back up" row: the folders beyond `directory` (`include`).
pub const FIELD_INCLUDE: usize = FIELD_NAMES.len();
/// Index of the "Exclude" row: patterns left out of the backup.
pub const FIELD_EXCLUDE: usize = FIELD_NAMES.len() + 1;
/// Index of the "Schedule" row.
pub const FIELD_SCHEDULE: usize = FIELD_NAMES.len() + 2;
/// Index of the "Password profile" row.
pub const FIELD_PASSWORD_PROFILE: usize = FIELD_NAMES.len() + 3;

/// Number of rows in the Fields pane.
pub const FIELD_COUNT: usize = FIELD_NAMES.len() + EXTRA_FIELD_NAMES.len();

pub struct TaskEntry {
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
    /// `daily 02:00` / `weekly sun 03:30`, as written to the config.
    pub schedule: Option<String>,
    pub password_profile: Option<String>,
    /// Settings with no row of their own: shown read-only, edited in the config file.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub before: Vec<String>,
    pub steps: Option<String>,
    pub remotes: Vec<RemoteEntry>,
    pub calls: Vec<String>,
}

/// `a`, or `a (+2 more)`; `None` for an empty list.
fn summarize(items: &[String]) -> Option<String> {
    let first = items.first()?;
    Some(match items.len() {
        1 => first.clone(),
        n => format!("{first} (+{} more)", n - 1),
    })
}

impl TaskEntry {
    /// The "Also back up" row: the extra folders.
    pub fn include_summary(&self) -> Option<String> {
        summarize(&self.include)
    }

    /// The "Exclude" row: the exclude patterns.
    pub fn exclude_summary(&self) -> Option<String> {
        summarize(&self.exclude)
    }

    /// One line per read-only setting this task has: what has no row to edit it.
    pub fn detail_lines(&self) -> Vec<String> {
        [
            summarize(&self.before).map(|b| format!("Before   {b}")),
            self.steps.as_ref().map(|s| format!("Steps    {s}")),
        ]
        .into_iter()
        .flatten()
        .collect()
    }
}

pub struct RemoteEntry {
    pub url: String,
    pub credentials: String,
    pub mc_max_workers: Option<u32>,
    pub mc_limit_upload: Option<String>,
    pub limit_upload: Option<String>,
}

pub struct App {
    pub tasks: Vec<TaskEntry>,
    pub selected_task: usize,
    pub selected_remote: usize,
    pub selected_field: usize,
    pub selected_call: usize,
    pub focused_pane: Pane,
    pub config_path: String,
    pub default_task: String,
    pub should_quit: bool,
    pub status_message: Option<String>,
    pub status_expires_at: Option<Instant>,
    pub needs_clear: bool,
}

impl App {
    pub fn new(config: &BackupConfig, config_path: String) -> Self {
        let tasks = config
            .tasks
            .iter()
            .map(|t| {
                let backup = t.backup();
                TaskEntry {
                    name: t.name.clone(),
                    description: t.description().map(str::to_owned),
                    repo: backup.map(|b| b.repo().to_owned()),
                    directory: backup.and_then(|b| b.directory()).map(str::to_owned),
                    exclude_file: backup.and_then(|b| b.exclude_file()).map(str::to_owned),
                    files_from: backup.and_then(|b| b.files_from()).map(str::to_owned),
                    schedule: t.schedule().map(|w| w.to_string()),
                    password_profile: t.password_profile().map(str::to_owned),
                    include: backup.map(|b| b.include().to_vec()).unwrap_or_default(),
                    exclude: backup.map(|b| b.exclude().to_vec()).unwrap_or_default(),
                    before: t.before_commands().into_iter().map(str::to_owned).collect(),
                    steps: backup.and_then(|b| b.steps().summary()),
                    remotes: t
                        .remotes()
                        .iter()
                        .map(|r| RemoteEntry {
                            url: r.url.clone(),
                            credentials: r.credentials.clone(),
                            mc_max_workers: r.mc_max_workers,
                            mc_limit_upload: r.mc_limit_upload.clone(),
                            limit_upload: r.limit_upload.clone(),
                        })
                        .collect(),
                    calls: t.call_names().to_vec(),
                }
            })
            .collect();

        App {
            tasks,
            selected_task: 0,
            selected_remote: 0,
            selected_field: 0,
            selected_call: 0,
            focused_pane: Pane::Tasks,
            config_path,
            default_task: config.default_task.clone(),
            should_quit: false,
            status_message: None,
            status_expires_at: None,
            needs_clear: false,
        }
    }

    pub fn set_status(&mut self, msg: String) {
        self.status_expires_at = Some(Instant::now() + Duration::from_secs(2));
        self.status_message = Some(msg);
    }

    pub fn tick_status(&mut self) {
        if let Some(exp) = self.status_expires_at {
            if Instant::now() >= exp {
                self.status_message = None;
                self.status_expires_at = None;
            }
        }
    }

    pub fn current_remotes(&self) -> &[RemoteEntry] {
        self.tasks
            .get(self.selected_task)
            .map(|t| t.remotes.as_slice())
            .unwrap_or(&[])
    }

    pub fn current_calls(&self) -> &[String] {
        self.tasks
            .get(self.selected_task)
            .map(|t| t.calls.as_slice())
            .unwrap_or(&[])
    }

    /// Moves the focused pane's selection by `delta`, staying within the list.
    /// Changing task resets the selection in the task's own panes.
    pub fn move_selection(&mut self, delta: isize) {
        let pane = self.focused_pane;
        let last = self.pane_len(pane).saturating_sub(1);
        let selected = self.selection_mut(pane);
        let moved = selected.saturating_add_signed(delta).min(last);
        if moved == *selected {
            return;
        }
        *selected = moved;
        if pane == Pane::Tasks {
            self.selected_remote = 0;
            self.selected_field = 0;
            self.selected_call = 0;
        }
    }

    fn pane_len(&self, pane: Pane) -> usize {
        match pane {
            Pane::Tasks => self.tasks.len(),
            Pane::Fields => FIELD_COUNT,
            Pane::Remotes => self.current_remotes().len(),
            Pane::Calls => self.current_calls().len(),
        }
    }

    fn selection_mut(&mut self, pane: Pane) -> &mut usize {
        match pane {
            Pane::Tasks => &mut self.selected_task,
            Pane::Fields => &mut self.selected_field,
            Pane::Remotes => &mut self.selected_remote,
            Pane::Calls => &mut self.selected_call,
        }
    }

    /// Re-reads the config file, keeping the focus, status and (clamped) selections.
    pub fn reload(&mut self) {
        let Ok(content) = std::fs::read_to_string(&self.config_path) else {
            return;
        };
        let Ok(config) = BackupConfig::parse(&self.config_path, &content) else {
            return;
        };
        let fresh = App::new(&config, self.config_path.clone());
        self.tasks = fresh.tasks;
        self.default_task = fresh.default_task;

        // Clamp every selection, the task first: the others index into its lists.
        self.selected_task = self.selected_task.min(self.pane_len(Pane::Tasks).saturating_sub(1));
        for pane in [Pane::Remotes, Pane::Calls, Pane::Fields] {
            let last = self.pane_len(pane).saturating_sub(1);
            let selected = self.selection_mut(pane);
            *selected = (*selected).min(last);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KDL: &str = r#"default-task "photos"
tasks {
    task "photos" {
        schedule "weekly sun 03:30"
        before "! pgrep -x Photos"
        before "echo hi"
        backup {
            repo "/r/photos"
            directory "/d"
            password-profile "photos"
            include "/a"
            include "/b"
            include "/c"
            exclude "**/*.tmp"
            steps {
                check { every "weekly"; subset "5%" }
                sync { every "daily"; window "01:00-06:00" }
            }
            remote "rustfs:https://h/p" {
                credentials "rustfs"
                limit-upload "20MiB"
            }
        }
    }
    task "plain" {
        backup { repo "/r/plain"
directory "/d" }
    }
    task "all" { calls "plain" }
}"#;

    fn app() -> App {
        App::new(&BackupConfig::parse("t.kdl", KDL).unwrap(), "t.kdl".to_string())
    }

    #[test]
    fn entries_carry_the_new_settings() {
        let app = app();
        let photos = &app.tasks[0];
        assert_eq!(photos.schedule.as_deref(), Some("weekly sun 03:30"));
        assert_eq!(photos.password_profile.as_deref(), Some("photos"));
        assert_eq!(photos.include, ["/a", "/b", "/c"]);
        assert_eq!(photos.exclude, ["**/*.tmp"]);
        assert_eq!(photos.before.len(), 2);
        assert_eq!(photos.steps.as_deref(), Some("check weekly (5%) \u{b7} sync daily 01:00-06:00"));
        assert_eq!(photos.remotes[0].limit_upload.as_deref(), Some("20MiB"));

        let plain = &app.tasks[1];
        assert!(plain.schedule.is_none() && plain.password_profile.is_none() && plain.steps.is_none());
        assert!(plain.include.is_empty() && plain.before.is_empty());
        // a calls-only task has none of the backup settings
        assert!(app.tasks[2].password_profile.is_none() && app.tasks[2].include.is_empty());
    }

    #[test]
    fn detail_lines_summarize_what_has_no_row() {
        let app = app();
        // include and exclude have rows of their own; only what can't be edited here is listed
        assert_eq!(
            app.tasks[0].detail_lines(),
            ["Before   ! pgrep -x Photos (+1 more)", "Steps    check weekly (5%) \u{b7} sync daily 01:00-06:00"]
        );
        assert!(app.tasks[1].detail_lines().is_empty(), "nothing extra, so no summary block");
    }

    #[test]
    fn list_rows_summarize_their_items() {
        let app = app();
        assert_eq!(app.tasks[0].include_summary().as_deref(), Some("/a (+2 more)"));
        assert_eq!(app.tasks[0].exclude_summary().as_deref(), Some("**/*.tmp"));
        assert_eq!(app.tasks[1].include_summary(), None);
        assert_eq!(app.tasks[1].exclude_summary(), None);
    }

    #[test]
    fn the_fields_pane_reaches_the_new_rows() {
        let mut app = app();
        app.focused_pane = Pane::Fields;
        for _ in 0..20 {
            app.move_selection(1);
        }
        assert_eq!(app.selected_field, FIELD_COUNT - 1);
        assert_eq!(app.selected_field, FIELD_PASSWORD_PROFILE);
        assert_eq!([FIELD_INCLUDE, FIELD_EXCLUDE, FIELD_SCHEDULE], [6, 7, 8]);
        assert_eq!(FIELD_PASSWORD_PROFILE, 9);
        assert_eq!(FIELD_COUNT, 10);
    }
}
