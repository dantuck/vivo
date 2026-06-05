use crate::backup_config::BackupConfig;

#[derive(Clone, Copy, PartialEq)]
pub enum Pane {
    Tasks,
    Remotes,
}

pub struct TaskEntry {
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
    pub remotes: Vec<RemoteEntry>,
}

pub struct RemoteEntry {
    pub url: String,
    pub credentials: String,
}

pub struct App {
    pub tasks: Vec<TaskEntry>,
    pub selected_task: usize,
    pub selected_remote: usize,
    pub focused_pane: Pane,
    pub config_path: String,
    pub should_quit: bool,
    pub status_message: Option<String>,
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
            })
            .collect();

        App {
            tasks,
            selected_task: 0,
            selected_remote: 0,
            focused_pane: Pane::Tasks,
            config_path,
            should_quit: false,
            status_message: None,
            needs_clear: false,
        }
    }

    pub fn current_remotes(&self) -> &[RemoteEntry] {
        self.tasks
            .get(self.selected_task)
            .map(|t| t.remotes.as_slice())
            .unwrap_or(&[])
    }

    pub fn reload(&mut self) {
        if let Ok(content) = std::fs::read_to_string(&self.config_path) {
            if let Ok(config) =
                knuffel::parse::<BackupConfig>(&self.config_path, &content)
            {
                let prev_task = self.selected_task;
                let prev_remote = self.selected_remote;
                let prev_pane = self.focused_pane;
                let prev_msg = self.status_message.take();
                let new = App::new(&config, self.config_path.clone());
                *self = new;
                self.selected_task =
                    prev_task.min(self.tasks.len().saturating_sub(1));
                self.selected_remote =
                    prev_remote.min(self.current_remotes().len().saturating_sub(1));
                self.focused_pane = prev_pane;
                self.status_message = prev_msg;
            }
        }
    }
}
