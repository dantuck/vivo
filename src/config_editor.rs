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
}
