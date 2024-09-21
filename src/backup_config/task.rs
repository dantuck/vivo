use std::collections::HashSet;

use log::info;

use crate::ui;
use crate::VivoConfig;

use super::backup::Backup;

#[derive(knuffel::Decode, Debug)]
pub struct Task {
    #[knuffel(argument)]
    pub name: String,
    #[knuffel(child, unwrap(argument))]
    description: Option<String>,
    #[knuffel(child)]
    backup: Option<Backup>,
    #[knuffel(child, unwrap(children(name = "task")))]
    subtasks: Option<Vec<SubTask>>,
}

#[derive(knuffel::Decode, Debug)]
struct SubTask {
    #[knuffel(argument)]
    name: String,
}

impl SubTask {
    fn run(&self, seen: &mut HashSet<String>, config: &VivoConfig, tasks: &Vec<Task>) {
        // Check for circular reference
        if seen.contains(&self.name) {
            println!(
                "Circular reference detected. Skipping subtask {}",
                self.name
            );
            return;
        }

        // Mark this subtask as seen
        seen.insert(self.name.clone());

        // Execute the subtask logic
        println!("Running subtask: {}", self.name);
        tasks.iter().find(|task| task.name == self.name).map(|t| {
            t.run(config, tasks);
            ui::info(&format!("Subtask [{}] completed.", t.name));
        });
    }
}

impl Task {
    pub fn run(&self, config: &VivoConfig, tasks: &Vec<Task>) {
        info!("Running task [{}]", self.name);
        ui::section_header(&format!("Running task [{}]", self.name));
        if let Some(description) = &self.description {
            println!("Description: {}", description);
        }

        if let Some(backup) = &self.backup {
            backup.run(config);
        }

        let mut seen = HashSet::from([self.name.to_string()]);

        self.subtasks.as_ref().map(|subtasks| {
            subtasks
                .iter()
                .for_each(|subtask| subtask.run(&mut seen, config, tasks));
        });

        // ui::info(&format!("Backup task [{}] completed.", self.name));
    }
}
