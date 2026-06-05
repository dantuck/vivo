#[allow(unused_imports)]
use std::collections::HashMap;
#[allow(unused_imports)]
use std::fs;
#[allow(unused_imports)]
use std::path::{Path, PathBuf};
#[allow(unused_imports)]
use std::process::Command;

use crate::backup_config::task::Task;
#[allow(unused_imports)]
use crate::backup_config::BackupConfig;
#[allow(unused_imports)]
use crate::config::Secrets;
#[allow(unused_imports)]
use crate::doctor;

pub struct MountEntry {
    pub label: String,
    pub task_name: String,
    pub repo_url: String,
    pub credentials_profile: Option<String>,
}

pub fn normalize_repo_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("rustfs:") {
        format!("s3:{rest}")
    } else {
        url.to_string()
    }
}

pub fn build_entries(tasks: &[Task]) -> Vec<MountEntry> {
    let mut entries = Vec::new();
    for task in tasks {
        if let Some(repo) = task.backup_repo() {
            entries.push(MountEntry {
                label: format!("{} (local)", task.name),
                task_name: task.name.clone(),
                repo_url: repo.to_string(),
                credentials_profile: None,
            });
        }
        for (url, creds) in task.backup_remotes() {
            entries.push(MountEntry {
                label: format!("{} \u{2192} {}", task.name, url),
                task_name: task.name.clone(),
                repo_url: url.to_string(),
                credentials_profile: Some(creds.to_string()),
            });
        }
    }
    entries
}

pub fn run(_config_path: &str, _secrets_path: &str, _mount_path: Option<&str>) -> Result<(), String> {
    todo!("implemented in Task 5")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task_kdl(name: &str, repo: &str, remotes: &[(&str, &str)]) -> String {
        let remote_lines: String = remotes
            .iter()
            .map(|(url, creds)| {
                format!(
                    "        remote \"{url}\" {{\n            credentials \"{creds}\"\n        }}"
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            r#"default-task "{name}"
tasks {{
    task "{name}" {{
        backup {{
            repo "{repo}"
            directory "/tmp"
{remote_lines}
        }}
    }}
}}"#
        )
    }

    fn parse_tasks(kdl: &str) -> Vec<crate::backup_config::task::Task> {
        let config: crate::BackupConfig = knuffel::parse("<test>", kdl).unwrap();
        config.tasks
    }

    #[test]
    fn build_entries_local_only() {
        let kdl = make_task_kdl("backup", "/tmp/repo", &[]);
        let tasks = parse_tasks(&kdl);
        let entries = build_entries(&tasks);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].label, "backup (local)");
        assert_eq!(entries[0].repo_url, "/tmp/repo");
        assert!(entries[0].credentials_profile.is_none());
    }

    #[test]
    fn build_entries_with_remotes() {
        let kdl = make_task_kdl(
            "backup",
            "/tmp/repo",
            &[("s3:https://s3.example.com/bucket", "aws")],
        );
        let tasks = parse_tasks(&kdl);
        let entries = build_entries(&tasks);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].label, "backup (local)");
        assert_eq!(entries[1].label, "backup \u{2192} s3:https://s3.example.com/bucket");
        assert_eq!(entries[1].credentials_profile.as_deref(), Some("aws"));
    }

    #[test]
    fn build_entries_skips_tasks_without_backup() {
        let kdl = r#"default-task "cmd"
tasks {
    task "cmd" {
        command "echo hi"
    }
}"#;
        let tasks = parse_tasks(kdl);
        let entries = build_entries(&tasks);
        assert_eq!(entries.len(), 0);
    }

    #[test]
    fn normalize_repo_url_translates_rustfs() {
        assert_eq!(
            normalize_repo_url("rustfs:http://nas:9000/backup"),
            "s3:http://nas:9000/backup"
        );
    }

    #[test]
    fn normalize_repo_url_passes_through_s3() {
        assert_eq!(
            normalize_repo_url("s3:https://s3.amazonaws.com/bucket"),
            "s3:https://s3.amazonaws.com/bucket"
        );
    }

    #[test]
    fn normalize_repo_url_passes_through_b2() {
        assert_eq!(
            normalize_repo_url("b2:my-bucket:restic/main"),
            "b2:my-bucket:restic/main"
        );
    }

    #[test]
    fn normalize_repo_url_passes_through_local() {
        assert_eq!(
            normalize_repo_url("/home/user/.local/share/restic/main"),
            "/home/user/.local/share/restic/main"
        );
    }
}
