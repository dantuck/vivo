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

pub const FIELD_NAMES: &[&str] =
    &["Name", "Description", "Repo", "Directory", "Exclude file", "Files from"];

pub struct TaskEntry {
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
    pub remotes: Vec<RemoteEntry>,
    pub calls: Vec<String>,
}

pub struct RemoteEntry {
    pub url: String,
    pub credentials: String,
    pub mc_max_workers: Option<u32>,
    pub mc_limit_upload: Option<String>,
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
                    remotes: t
                        .remotes()
                        .iter()
                        .map(|r| RemoteEntry {
                            url: r.url.clone(),
                            credentials: r.credentials.clone(),
                            mc_max_workers: r.mc_max_workers,
                            mc_limit_upload: r.mc_limit_upload.clone(),
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
            Pane::Fields => FIELD_NAMES.len(),
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
