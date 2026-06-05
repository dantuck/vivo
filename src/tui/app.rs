use std::time::{Duration, Instant};

use crate::backup_config::BackupConfig;

#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    Tasks,
    Fields,
    Remotes,
    Calls,
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
}

pub struct App {
    pub tasks: Vec<TaskEntry>,
    pub selected_task: usize,
    pub selected_remote: usize,
    pub selected_field: usize,
    pub selected_call: usize,
    pub focused_pane: Pane,
    pub config_path: String,
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
            .map(|t| TaskEntry {
                name: t.name.clone(),
                description: t.description().map(str::to_owned),
                repo: t.backup_repo().map(str::to_owned),
                directory: t.backup_directory().map(str::to_owned),
                exclude_file: t.backup_exclude_file().map(str::to_owned),
                files_from: t.backup_files_from().map(str::to_owned),
                remotes: t
                    .backup_remotes()
                    .into_iter()
                    .map(|(url, creds)| RemoteEntry {
                        url: url.to_string(),
                        credentials: creds.to_string(),
                    })
                    .collect(),
                calls: t.call_names().into_iter().map(str::to_owned).collect(),
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

    pub fn reload(&mut self) {
        if let Ok(content) = std::fs::read_to_string(&self.config_path) {
            if let Ok(config) =
                knuffel::parse::<BackupConfig>(&self.config_path, &content)
            {
                let prev_task = self.selected_task;
                let prev_remote = self.selected_remote;
                let prev_field = self.selected_field;
                let prev_call = self.selected_call;
                let prev_pane = self.focused_pane;
                let prev_msg = self.status_message.take();
                let prev_exp = self.status_expires_at;
                let new = App::new(&config, self.config_path.clone());
                *self = new;
                self.selected_task =
                    prev_task.min(self.tasks.len().saturating_sub(1));
                self.selected_remote =
                    prev_remote.min(self.current_remotes().len().saturating_sub(1));
                self.selected_field = prev_field.min(FIELD_NAMES.len() - 1);
                self.selected_call =
                    prev_call.min(self.current_calls().len().saturating_sub(1));
                self.focused_pane = prev_pane;
                self.status_message = prev_msg;
                self.status_expires_at = prev_exp;
            }
        }
    }
}
