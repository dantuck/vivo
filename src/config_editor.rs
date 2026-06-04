use kdl::{KdlDocument, KdlEntry, KdlEntryFormat, KdlNode, KdlValue};

pub struct TaskSpec {
    pub name: String,
    pub repo: String,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
}

pub struct RemoteSpec {
    pub url: String,
    pub credentials: String,
}

pub struct EditTaskSpec {
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
}

fn first_arg(node: &KdlNode) -> Option<&str> {
    node.entries()
        .iter()
        .find(|e| e.name().is_none())
        .and_then(|e| e.value().as_string())
}

fn str_entry(value: &str) -> KdlEntry {
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    let mut entry = KdlEntry::new(KdlValue::String(value.to_string()));
    entry.set_format(KdlEntryFormat {
        value_repr: format!("\"{escaped}\""),
        leading: " ".to_string(),
        ..Default::default()
    });
    entry
}

fn str_node(name: &str, value: &str) -> KdlNode {
    let mut n = KdlNode::new(name);
    n.push(str_entry(value));
    n
}

fn update_str_child(doc: &mut KdlDocument, name: &str, value: &str) {
    if let Some(node) = doc.nodes_mut().iter_mut().find(|n| n.name().value() == name) {
        if let Some(entry) = node.entries_mut().iter_mut().find(|e| e.name().is_none()) {
            *entry = str_entry(value);
        } else {
            node.push(str_entry(value));
        }
    } else {
        doc.nodes_mut().push(str_node(name, value));
    }
}

fn upsert_or_remove_child(doc: &mut KdlDocument, name: &str, value: Option<&str>) {
    if let Some(v) = value {
        if !v.is_empty() {
            if let Some(node) = doc.nodes_mut().iter_mut().find(|n| n.name().value() == name) {
                if let Some(entry) = node.entries_mut().iter_mut().find(|e| e.name().is_none()) {
                    *entry = str_entry(v);
                }
                return;
            }
            doc.nodes_mut().push(str_node(name, v));
        } else {
            doc.nodes_mut().retain(|n| n.name().value() != name);
        }
    } else {
        doc.nodes_mut().retain(|n| n.name().value() != name);
    }
}

pub fn add_task(kdl: &str, spec: TaskSpec) -> Result<String, String> {
    let mut doc: KdlDocument = kdl.parse().map_err(|e| format!("KDL parse error: {e}"))?;

    let tasks = doc.get_mut("tasks").ok_or("config missing 'tasks' block")?;
    let children = tasks.ensure_children();

    if children
        .nodes()
        .iter()
        .any(|n| n.name().value() == "task" && first_arg(n) == Some(spec.name.as_str()))
    {
        return Err(format!("task '{}' already exists", spec.name));
    }

    let mut task = KdlNode::new("task");
    task.push(str_entry(&spec.name));

    let mut backup = KdlNode::new("backup");
    {
        let bc = backup.ensure_children();
        bc.nodes_mut().push(str_node("repo", &spec.repo));
        if let Some(dir) = &spec.directory {
            bc.nodes_mut().push(str_node("directory", dir));
        }
        if let Some(excl) = &spec.exclude_file {
            bc.nodes_mut().push(str_node("exclude-file", excl));
        }
    }
    task.ensure_children().nodes_mut().push(backup);
    children.nodes_mut().push(task);

    Ok(doc.to_string())
}

pub fn remove_task(kdl: &str, name: &str) -> Result<String, String> {
    let mut doc: KdlDocument = kdl.parse().map_err(|e| format!("KDL parse error: {e}"))?;

    // Read default-task BEFORE any mutable borrow of tasks
    let default_task = doc
        .get("default-task")
        .and_then(|n| first_arg(n))
        .map(str::to_owned)
        .unwrap_or_default();

    let tasks = doc.get_mut("tasks").ok_or("config missing 'tasks' block")?;
    let children = tasks.ensure_children();

    let task_count = children
        .nodes()
        .iter()
        .filter(|n| n.name().value() == "task")
        .count();

    if task_count <= 1 {
        return Err("cannot remove the only task — add another task first".to_string());
    }

    if default_task == name {
        return Err(format!(
            "cannot remove '{name}': it is the default-task — update default-task first"
        ));
    }

    let before = children.nodes().len();
    children
        .nodes_mut()
        .retain(|n| !(n.name().value() == "task" && first_arg(n) == Some(name)));

    if children.nodes().len() == before {
        return Err(format!("task '{name}' not found"));
    }

    Ok(doc.to_string())
}

pub fn edit_task(kdl: &str, old_name: &str, spec: EditTaskSpec) -> Result<String, String> {
    if spec.name.is_empty() {
        return Err("task name cannot be empty".to_string());
    }

    let mut doc: KdlDocument = kdl.parse().map_err(|e| format!("KDL parse error: {e}"))?;

    // Check for duplicate name before any mutable borrow
    if spec.name != old_name {
        let duplicate = doc
            .get("tasks")
            .and_then(|t| t.children())
            .map(|c| {
                c.nodes()
                    .iter()
                    .any(|n| n.name().value() == "task" && first_arg(n) == Some(spec.name.as_str()))
            })
            .unwrap_or(false);
        if duplicate {
            return Err(format!("task '{}' already exists", spec.name));
        }
    }

    // Read default-task before mutable borrow
    let default_task_is_old = doc
        .get("default-task")
        .and_then(|n| first_arg(n))
        .map(|s| s == old_name)
        .unwrap_or(false);

    // Update the task node
    {
        let tasks = doc.get_mut("tasks").ok_or("config missing 'tasks' block")?;
        let task = tasks
            .ensure_children()
            .nodes_mut()
            .iter_mut()
            .find(|n| n.name().value() == "task" && first_arg(n) == Some(old_name))
            .ok_or_else(|| format!("task '{old_name}' not found"))?;

        // Update name argument
        if spec.name != old_name {
            if let Some(entry) = task.entries_mut().iter_mut().find(|e| e.name().is_none()) {
                *entry = str_entry(&spec.name);
            }
        }

        let task_children = task.ensure_children();

        // Update description
        task_children.nodes_mut().retain(|n| n.name().value() != "description");
        if let Some(desc) = &spec.description {
            if !desc.is_empty() {
                let pos = task_children
                    .nodes()
                    .iter()
                    .position(|n| n.name().value() == "backup")
                    .unwrap_or(0);
                task_children.nodes_mut().insert(pos, str_node("description", desc));
            }
        }

        // Update backup block fields only if repo is provided
        if let Some(repo) = &spec.repo {
            if let Some(backup) = task_children
                .nodes_mut()
                .iter_mut()
                .find(|n| n.name().value() == "backup")
            {
                let bc = backup.ensure_children();
                update_str_child(bc, "repo", repo);
                upsert_or_remove_child(bc, "directory", spec.directory.as_deref());
                upsert_or_remove_child(bc, "exclude-file", spec.exclude_file.as_deref());
                upsert_or_remove_child(bc, "files-from", spec.files_from.as_deref());
            }
        }
    }

    // Update default-task if renaming
    if spec.name != old_name && default_task_is_old {
        if let Some(node) = doc.get_mut("default-task") {
            if let Some(entry) = node.entries_mut().iter_mut().find(|e| e.name().is_none()) {
                *entry = str_entry(&spec.name);
            }
        }
    }

    // Update calls references if renaming
    if spec.name != old_name {
        if let Some(tasks) = doc.get_mut("tasks") {
            if let Some(children) = tasks.children_mut() {
                for task_node in children.nodes_mut() {
                    if task_node.name().value() != "task" {
                        continue;
                    }
                    if let Some(task_children) = task_node.children_mut() {
                        for child in task_children.nodes_mut() {
                            if child.name().value() == "calls" {
                                if let Some(entry) =
                                    child.entries_mut().iter_mut().find(|e| e.name().is_none())
                                {
                                    if entry.value().as_string() == Some(old_name) {
                                        *entry = str_entry(&spec.name);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(doc.to_string())
}

pub fn add_remote(kdl: &str, task_name: &str, spec: RemoteSpec) -> Result<String, String> {
    let mut doc: KdlDocument = kdl.parse().map_err(|e| format!("KDL parse error: {e}"))?;

    let tasks = doc.get_mut("tasks").ok_or("config missing 'tasks' block")?;
    let task = tasks
        .ensure_children()
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "task" && first_arg(n) == Some(task_name))
        .ok_or_else(|| format!("task '{task_name}' not found"))?;

    let backup = task
        .ensure_children()
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "backup")
        .ok_or_else(|| {
            format!(
                "task '{task_name}' has no backup block — add one with `vivo config edit`"
            )
        })?;

    let backup_children = backup.ensure_children();

    if backup_children
        .nodes()
        .iter()
        .any(|n| n.name().value() == "remote" && first_arg(n) == Some(spec.url.as_str()))
    {
        return Err(format!(
            "remote '{}' already exists on task '{task_name}'",
            spec.url
        ));
    }

    let mut remote = KdlNode::new("remote");
    remote.push(str_entry(&spec.url));
    remote
        .ensure_children()
        .nodes_mut()
        .push(str_node("credentials", &spec.credentials));

    backup_children.nodes_mut().push(remote);

    Ok(doc.to_string())
}

pub fn remove_remote(kdl: &str, task_name: &str, url: &str) -> Result<String, String> {
    let mut doc: KdlDocument = kdl.parse().map_err(|e| format!("KDL parse error: {e}"))?;

    let tasks = doc.get_mut("tasks").ok_or("config missing 'tasks' block")?;
    let task = tasks
        .ensure_children()
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "task" && first_arg(n) == Some(task_name))
        .ok_or_else(|| format!("task '{task_name}' not found"))?;

    let backup = task
        .ensure_children()
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "backup")
        .ok_or_else(|| format!("task '{task_name}' has no backup block"))?;

    let backup_children = backup.ensure_children();
    let before = backup_children.nodes().len();
    backup_children
        .nodes_mut()
        .retain(|n| !(n.name().value() == "remote" && first_arg(n) == Some(url)));

    if backup_children.nodes().len() == before {
        return Err(format!("remote '{url}' not found on task '{task_name}'"));
    }

    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE_KDL: &str = r#"default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "/tmp/repo"
            directory "/tmp"
        }
    }
}
"#;

    #[test]
    fn add_task_appends_new_task() {
        let result = add_task(
            BASE_KDL,
            TaskSpec {
                name: "photos".to_string(),
                repo: "/tmp/photos".to_string(),
                directory: Some("/home/user/Photos".to_string()),
                exclude_file: None,
            },
        )
        .unwrap();
        assert!(result.contains(r#"task "photos""#));
        assert!(result.contains(r#"repo "/tmp/photos""#));
        assert!(result.contains(r#"directory "/home/user/Photos""#));
        assert!(result.contains(r#"task "backup""#));
    }

    #[test]
    fn add_task_includes_exclude_file_when_provided() {
        let result = add_task(
            BASE_KDL,
            TaskSpec {
                name: "docs".to_string(),
                repo: "/tmp/docs".to_string(),
                directory: None,
                exclude_file: Some("/home/user/.vivoexclude".to_string()),
            },
        )
        .unwrap();
        assert!(result.contains(r#"exclude-file "/home/user/.vivoexclude""#));
    }

    #[test]
    fn add_task_rejects_duplicate_name() {
        let err = add_task(
            BASE_KDL,
            TaskSpec {
                name: "backup".to_string(),
                repo: "/tmp/other".to_string(),
                directory: None,
                exclude_file: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("already exists"));
    }

    const TWO_TASKS_KDL: &str = r#"default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "/tmp/r1"
            directory "/tmp"
        }
    }
    task "photos" {
        backup {
            repo "/tmp/r2"
            directory "/tmp"
        }
    }
}
"#;

    #[test]
    fn remove_task_removes_non_default_task() {
        let result = remove_task(TWO_TASKS_KDL, "photos").unwrap();
        assert!(!result.contains(r#"task "photos""#));
        assert!(result.contains(r#"task "backup""#));
    }

    #[test]
    fn remove_task_rejects_default_task() {
        let err = remove_task(TWO_TASKS_KDL, "backup").unwrap_err();
        assert!(err.contains("default"));
    }

    #[test]
    fn remove_task_rejects_only_task() {
        let single_non_default = r#"default-task "main"
tasks {
    task "photos" {
        backup { repo "/tmp/r" }
    }
}
"#;
        let err = remove_task(single_non_default, "photos").unwrap_err();
        assert!(err.contains("only task"));
    }

    #[test]
    fn remove_task_errors_when_not_found() {
        let err = remove_task(TWO_TASKS_KDL, "nonexistent").unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn add_remote_appends_remote_to_backup_block() {
        let result = add_remote(
            BASE_KDL,
            "backup",
            RemoteSpec {
                url: "rustfs:http://nas:9000/bucket".to_string(),
                credentials: "rustfs".to_string(),
            },
        )
        .unwrap();
        assert!(result.contains(r#"remote "rustfs:http://nas:9000/bucket""#));
        assert!(result.contains(r#"credentials "rustfs""#));
    }

    #[test]
    fn add_remote_rejects_duplicate_url() {
        let with_remote = r#"default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "/tmp/repo"
            remote "s3:http://example.com/b" {
                credentials "aws"
            }
        }
    }
}
"#;
        let err = add_remote(
            with_remote,
            "backup",
            RemoteSpec {
                url: "s3:http://example.com/b".to_string(),
                credentials: "aws".to_string(),
            },
        )
        .unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn add_remote_errors_when_task_not_found() {
        let err = add_remote(
            BASE_KDL,
            "nonexistent",
            RemoteSpec { url: "s3:http://x".to_string(), credentials: "c".to_string() },
        )
        .unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn add_remote_errors_when_no_backup_block() {
        let no_backup = r#"default-task "cmd"
tasks {
    task "cmd" {
        command "echo hi"
    }
}
"#;
        let err = add_remote(
            no_backup,
            "cmd",
            RemoteSpec { url: "s3:http://x".to_string(), credentials: "c".to_string() },
        )
        .unwrap_err();
        assert!(err.contains("backup block"));
    }

    const WITH_CALLS_KDL: &str = r#"default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "/tmp/repo"
            directory "/tmp"
        }
    }
    task "secondary" {
        calls "backup"
    }
}
"#;

    #[test]
    fn edit_task_updates_fields() {
        let result = edit_task(
            BASE_KDL,
            "backup",
            EditTaskSpec {
                name: "backup".to_string(),
                description: Some("my backup".to_string()),
                repo: Some("/new/repo".to_string()),
                directory: Some("/new/dir".to_string()),
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap();
        assert!(result.contains(r#"description "my backup""#));
        assert!(result.contains(r#"repo "/new/repo""#));
        assert!(result.contains(r#"directory "/new/dir""#));
    }

    #[test]
    fn edit_task_renames_and_updates_references() {
        let result = edit_task(
            WITH_CALLS_KDL,
            "backup",
            EditTaskSpec {
                name: "main".to_string(),
                description: None,
                repo: Some("/tmp/repo".to_string()),
                directory: Some("/tmp".to_string()),
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap();
        assert!(result.contains(r#"default-task "main""#));
        assert!(result.contains(r#"task "main""#));
        assert!(!result.contains(r#"task "backup""#));
        assert!(result.contains(r#"calls "main""#));
        assert!(!result.contains(r#"calls "backup""#));
    }

    #[test]
    fn edit_task_removes_optional_field_when_none() {
        let with_dir = r#"default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "/tmp/repo"
            directory "/tmp"
        }
    }
}
"#;
        let result = edit_task(
            with_dir,
            "backup",
            EditTaskSpec {
                name: "backup".to_string(),
                description: None,
                repo: Some("/tmp/repo".to_string()),
                directory: None,
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap();
        assert!(!result.contains("directory"));
        assert!(result.contains(r#"repo "/tmp/repo""#));
    }

    #[test]
    fn edit_task_errors_on_duplicate_name() {
        let err = edit_task(
            TWO_TASKS_KDL,
            "photos",
            EditTaskSpec {
                name: "backup".to_string(),
                description: None,
                repo: Some("/tmp/r2".to_string()),
                directory: None,
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[test]
    fn edit_task_errors_on_empty_name() {
        let err = edit_task(
            BASE_KDL,
            "backup",
            EditTaskSpec {
                name: String::new(),
                description: None,
                repo: Some("/tmp/repo".to_string()),
                directory: None,
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("cannot be empty"));
    }

    #[test]
    fn edit_task_skips_backup_fields_when_repo_is_none() {
        let cmd_only = r#"default-task "cmd"
tasks {
    task "cmd" {
        command "echo hi"
    }
}
"#;
        let result = edit_task(
            cmd_only,
            "cmd",
            EditTaskSpec {
                name: "cmd".to_string(),
                description: Some("a command task".to_string()),
                repo: None,
                directory: None,
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap();
        assert!(result.contains(r#"description "a command task""#));
        assert!(!result.contains("backup"));
    }

    #[test]
    fn edit_task_renames_non_default_task_leaves_default_task_unchanged() {
        let result = edit_task(
            WITH_CALLS_KDL,
            "secondary",
            EditTaskSpec {
                name: "mirror".to_string(),
                description: None,
                repo: None,
                directory: None,
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap();
        assert!(result.contains(r#"default-task "backup""#));
        assert!(result.contains(r#"task "mirror""#));
        assert!(!result.contains(r#"task "secondary""#));
    }

    const WITH_REMOTE_KDL: &str = r#"default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "/tmp/repo"
            remote "s3:http://example.com/b" {
                credentials "aws"
            }
        }
    }
}
"#;

    #[test]
    fn remove_remote_removes_by_url() {
        let result = remove_remote(WITH_REMOTE_KDL, "backup", "s3:http://example.com/b").unwrap();
        assert!(!result.contains(r#"remote "s3:http://example.com/b""#));
        assert!(result.contains(r#"task "backup""#));
    }

    #[test]
    fn remove_remote_errors_when_url_not_found() {
        let err = remove_remote(WITH_REMOTE_KDL, "backup", "s3:http://other.com/b").unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn remove_remote_errors_when_task_not_found() {
        let err = remove_remote(WITH_REMOTE_KDL, "ghost", "s3:http://example.com/b").unwrap_err();
        assert!(err.contains("not found"));
    }

    #[test]
    fn edit_task_preserves_remote_order_when_updating_directory() {
        let kdl_with_dir_and_remote = r#"default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "/tmp/repo"
            directory "/old/dir"
            remote "s3:http://example.com/b" {
                credentials "aws"
            }
        }
    }
}
"#;
        let result = edit_task(
            kdl_with_dir_and_remote,
            "backup",
            EditTaskSpec {
                name: "backup".to_string(),
                description: None,
                repo: Some("/tmp/repo".to_string()),
                directory: Some("/new/dir".to_string()),
                exclude_file: None,
                files_from: None,
            },
        )
        .unwrap();
        let dir_pos = result.find("directory").unwrap();
        let remote_pos = result.find("remote").unwrap();
        assert!(
            dir_pos < remote_pos,
            "directory should precede remote in output"
        );
    }
}
