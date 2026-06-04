# TUI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign `vivo manage` into a full task-detail view with pre-filled edit prompts for task fields and remotes, making multi-remote support obvious.

**Architecture:** Six sequential tasks — expose model fields, add two config_editor functions (TDD), extend TUI app state, rewrite the right pane UI, and wire up the new `e`/`o` key bindings. No new dependencies; uses existing `kdl`, `ratatui`, `inquire`, `crossterm`.

**Tech Stack:** Rust, kdl 6.7.1 (KDL config editing), ratatui 0.30 (TUI), inquire 0.7 (terminal prompts), crossterm 0.28 (raw-mode control)

---

## File Map

| File | Change |
|------|--------|
| `src/backup_config/backup.rs` | Add public accessors: `repo()`, `directory()`, `exclude_file()`, `files_from()` |
| `src/backup_config/task.rs` | Add public delegation methods: `backup_repo()`, `backup_directory()`, `backup_exclude_file()`, `backup_files_from()` |
| `src/config_editor.rs` | Add `EditTaskSpec`, `edit_task()`, `edit_remote()`, two private helpers |
| `src/tui/app.rs` | Extend `TaskEntry` with 5 new fields; update `App::new()` |
| `src/tui/ui.rs` | Replace `draw_remotes` with `draw_task_detail` + `draw_task_fields` + `draw_remotes_list`; update `draw_help` |
| `src/tui/events.rs` | Add `handle_edit`, `edit_task_prompt`, `edit_remote_prompt`; rename current `handle_edit` → `handle_open_editor`; bind `e` and `o` |

---

## Task 1: Expose Backup and Task fields

**Files:**
- Modify: `src/backup_config/backup.rs`
- Modify: `src/backup_config/task.rs`

- [ ] **Step 1: Add public accessors to `Backup`**

In `src/backup_config/backup.rs`, add to the existing `impl Backup` block (after the `remotes` method):

```rust
pub fn repo(&self) -> &str {
    &self.repo
}

pub fn directory(&self) -> Option<&str> {
    self.directory.as_deref()
}

pub fn exclude_file(&self) -> Option<&str> {
    self.exclude_file.as_deref()
}

pub fn files_from(&self) -> Option<&str> {
    self.files_from.as_deref()
}
```

- [ ] **Step 2: Add delegation methods to `Task`**

In `src/backup_config/task.rs`, add to the existing `impl Task` block (after `backup_remotes`):

```rust
pub fn backup_repo(&self) -> Option<&str> {
    self.backup.as_ref().map(|b| b.repo())
}

pub fn backup_directory(&self) -> Option<&str> {
    self.backup.as_ref().and_then(|b| b.directory())
}

pub fn backup_exclude_file(&self) -> Option<&str> {
    self.backup.as_ref().and_then(|b| b.exclude_file())
}

pub fn backup_files_from(&self) -> Option<&str> {
    self.backup.as_ref().and_then(|b| b.files_from())
}
```

- [ ] **Step 3: Build and confirm it compiles**

```bash
cd vivo-cli && cargo build 2>&1
```

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
git add src/backup_config/backup.rs src/backup_config/task.rs
git commit -m "feat(tui): expose backup fields via public accessors"
```

---

## Task 2: Add `edit_task` to config_editor (TDD)

**Files:**
- Modify: `src/config_editor.rs`

- [ ] **Step 1: Add `EditTaskSpec` and two private helpers**

In `src/config_editor.rs`, add after the `RemoteSpec` struct and before `first_arg`:

```rust
pub struct EditTaskSpec {
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
}

fn update_str_child(doc: &mut KdlDocument, name: &str, value: &str) {
    if let Some(node) = doc.nodes_mut().iter_mut().find(|n| n.name().value() == name) {
        if let Some(entry) = node.entries_mut().iter_mut().find(|e| e.name().is_none()) {
            *entry = str_entry(value);
        }
    } else {
        doc.nodes_mut().push(str_node(name, value));
    }
}

fn upsert_or_remove_child(doc: &mut KdlDocument, name: &str, value: Option<&str>) {
    doc.nodes_mut().retain(|n| n.name().value() != name);
    if let Some(v) = value {
        if !v.is_empty() {
            doc.nodes_mut().push(str_node(name, v));
        }
    }
}
```

- [ ] **Step 2: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block in `src/config_editor.rs`:

```rust
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
```

- [ ] **Step 3: Run tests to confirm they all fail**

```bash
cd vivo-cli && cargo test edit_task 2>&1
```

Expected: compile error — `edit_task` not defined yet.

- [ ] **Step 4: Implement `edit_task`**

Add after `remove_task` in `src/config_editor.rs`:

```rust
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
```

- [ ] **Step 5: Run tests and confirm they pass**

```bash
cd vivo-cli && cargo test edit_task 2>&1
```

Expected: all 6 `edit_task_*` tests pass.

- [ ] **Step 6: Run full test suite to check for regressions**

```bash
cd vivo-cli && cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/config_editor.rs
git commit -m "feat(config): add edit_task with rename and reference updates"
```

---

## Task 3: Add `edit_remote` to config_editor (TDD)

**Files:**
- Modify: `src/config_editor.rs`

- [ ] **Step 1: Write the failing tests**

Add to the `#[cfg(test)] mod tests` block:

```rust
#[test]
fn edit_remote_updates_url_and_credentials() {
    let result = edit_remote(
        WITH_REMOTE_KDL,
        "backup",
        "s3:http://example.com/b",
        RemoteSpec {
            url: "rustfs:http://nas:9000/bucket".to_string(),
            credentials: "local".to_string(),
        },
    )
    .unwrap();
    assert!(result.contains(r#"remote "rustfs:http://nas:9000/bucket""#));
    assert!(result.contains(r#"credentials "local""#));
    assert!(!result.contains(r#"remote "s3:http://example.com/b""#));
    assert!(!result.contains(r#"credentials "aws""#));
}

#[test]
fn edit_remote_errors_when_url_not_found() {
    let err = edit_remote(
        WITH_REMOTE_KDL,
        "backup",
        "s3:http://other.com/b",
        RemoteSpec {
            url: "rustfs:http://nas:9000/bucket".to_string(),
            credentials: "local".to_string(),
        },
    )
    .unwrap_err();
    assert!(err.contains("not found"));
}

#[test]
fn edit_remote_errors_when_task_not_found() {
    let err = edit_remote(
        WITH_REMOTE_KDL,
        "ghost",
        "s3:http://example.com/b",
        RemoteSpec {
            url: "rustfs:http://nas:9000/bucket".to_string(),
            credentials: "local".to_string(),
        },
    )
    .unwrap_err();
    assert!(err.contains("not found"));
}
```

- [ ] **Step 2: Run tests to confirm they fail**

```bash
cd vivo-cli && cargo test edit_remote 2>&1
```

Expected: compile error — `edit_remote` not defined yet.

- [ ] **Step 3: Implement `edit_remote`**

Add after `add_remote` in `src/config_editor.rs`:

```rust
pub fn edit_remote(
    kdl: &str,
    task_name: &str,
    old_url: &str,
    spec: RemoteSpec,
) -> Result<String, String> {
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

    let remote = backup
        .ensure_children()
        .nodes_mut()
        .iter_mut()
        .find(|n| n.name().value() == "remote" && first_arg(n) == Some(old_url))
        .ok_or_else(|| format!("remote '{old_url}' not found on task '{task_name}'"))?;

    // Update URL argument
    if let Some(entry) = remote.entries_mut().iter_mut().find(|e| e.name().is_none()) {
        *entry = str_entry(&spec.url);
    }

    // Update credentials child
    let remote_children = remote.ensure_children();
    update_str_child(remote_children, "credentials", &spec.credentials);

    Ok(doc.to_string())
}
```

- [ ] **Step 4: Run tests and confirm they pass**

```bash
cd vivo-cli && cargo test edit_remote 2>&1
```

Expected: all 3 `edit_remote_*` tests pass.

- [ ] **Step 5: Run full test suite**

```bash
cd vivo-cli && cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/config_editor.rs
git commit -m "feat(config): add edit_remote"
```

---

## Task 4: Extend `TaskEntry` in `tui/app.rs`

**Files:**
- Modify: `src/tui/app.rs`

- [ ] **Step 1: Add fields to `TaskEntry`**

Replace the current `TaskEntry` struct definition:

```rust
pub struct TaskEntry {
    pub name: String,
    pub description: Option<String>,
    pub repo: Option<String>,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
    pub remotes: Vec<RemoteEntry>,
}
```

- [ ] **Step 2: Update `App::new()` to populate the new fields**

Replace the `tasks` mapping inside `App::new()`:

```rust
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
```

- [ ] **Step 3: Build and confirm it compiles**

```bash
cd vivo-cli && cargo build 2>&1
```

Expected: no errors (the new fields are only in `app.rs`; `ui.rs` and `events.rs` will get compile errors in later tasks — that's fine, fix them now if needed or proceed to Task 5 next).

- [ ] **Step 4: Commit**

```bash
git add src/tui/app.rs
git commit -m "feat(tui): extend TaskEntry with task detail fields"
```

---

## Task 5: Rewrite right pane UI

**Files:**
- Modify: `src/tui/ui.rs`

- [ ] **Step 1: Update imports**

Replace the current import block at the top of `src/tui/ui.rs`:

```rust
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use super::app::{App, Pane};
```

(No changes needed — all needed types are already imported.)

- [ ] **Step 2: Replace `draw_remotes` with `draw_task_detail` and helpers**

Delete the existing `draw_remotes` function entirely and replace it with these three functions:

```rust
fn draw_task_detail(f: &mut Frame, app: &App, area: Rect) {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.as_str())
        .unwrap_or("—");
    let title = format!(" Task: {task_name} ");

    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(focused_border(app.focused_pane == Pane::Remotes));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Min(0)])
        .split(inner);

    draw_task_fields(f, app, chunks[0]);
    draw_remotes_list(f, app, chunks[1]);
}

fn draw_task_fields(f: &mut Frame, app: &App, area: Rect) {
    let task = match app.tasks.get(app.selected_task) {
        Some(t) => t,
        None => return,
    };

    let label = Style::default().fg(Color::DarkGray);
    let dim = Style::default().fg(Color::DarkGray).add_modifier(Modifier::DIM);

    let field = |name: &'static str, val: Option<&str>| -> Line<'static> {
        let value_span = match val {
            Some(v) if !v.is_empty() => Span::raw(v.to_string()),
            _ => Span::styled("(none)", dim),
        };
        Line::from(vec![
            Span::styled(format!("{name:<14}"), label),
            value_span,
        ])
    };

    let lines = vec![
        field("Name", Some(task.name.as_str())),
        field("Description", task.description.as_deref()),
        field("Repo", task.repo.as_deref()),
        field("Directory", task.directory.as_deref()),
        field("Exclude file", task.exclude_file.as_deref()),
        field("Files from", task.files_from.as_deref()),
    ];

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_remotes_list(f: &mut Frame, app: &App, area: Rect) {
    let remotes = app.current_remotes();
    let count = remotes.len();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    let header_style = if app.focused_pane == Pane::Remotes {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    f.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            format!("Remotes ({count}):"),
            header_style,
        )])),
        chunks[0],
    );

    let items: Vec<ListItem> = remotes
        .iter()
        .map(|r| ListItem::new(format!("  {}  [{}]", r.url, r.credentials)))
        .collect();

    let list = List::new(items)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ListState::default();
    if !remotes.is_empty() && app.focused_pane == Pane::Remotes {
        state.select(Some(app.selected_remote));
    }
    f.render_stateful_widget(list, chunks[1], &mut state);
}
```

- [ ] **Step 3: Update `draw` to call `draw_task_detail`**

In the `draw` function, replace the `draw_remotes` call:

```rust
pub fn draw(f: &mut Frame, app: &App) {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(f.area());

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(outer[0]);

    draw_tasks(f, app, panes[0]);
    draw_task_detail(f, app, panes[1]);
    draw_help(f, app, outer[1]);
}
```

- [ ] **Step 4: Update `draw_help` to show pane-specific hints**

Replace the `draw_help` function:

```rust
fn draw_help(f: &mut Frame, app: &App, area: Rect) {
    let default_hint = match app.focused_pane {
        Pane::Tasks => {
            "[a] add  [d] delete  [e] edit task  [o] open in $EDITOR  [Tab] switch pane  [q] quit"
        }
        Pane::Remotes => {
            "[a] add remote  [d] delete  [e] edit remote  [Tab] switch pane  [q] quit"
        }
    };
    let status = app.status_message.as_deref().unwrap_or(default_hint);
    let para = Paragraph::new(Line::from(vec![Span::raw(format!(" {status}"))]))
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(para, area);
}
```

- [ ] **Step 5: Build and confirm it compiles**

```bash
cd vivo-cli && cargo build 2>&1
```

Expected: no errors.

- [ ] **Step 6: Run `vivo manage` and verify the layout visually**

```bash
cd vivo-cli && cargo run -- manage 2>&1
```

Expected: right pane shows all 6 task fields (Name, Description, Repo, Directory, Exclude file, Files from), with `(none)` for empty fields. Below that, `Remotes (n):` header and the remotes list. Help bar shows `[a] add  [d] delete  [e] edit task  [o] open in $EDITOR  [Tab] switch pane  [q] quit`. Tab switches pane; remotes become highlighted.

- [ ] **Step 7: Commit**

```bash
git add src/tui/ui.rs
git commit -m "feat(tui): rewrite right pane as full task detail view"
```

---

## Task 6: Wire up `e` and `o` keys in `events.rs`

**Files:**
- Modify: `src/tui/events.rs`

- [ ] **Step 1: Add imports**

At the top of `src/tui/events.rs`, add `EditTaskSpec` to the config_editor import:

```rust
use crate::config_editor::{EditTaskSpec, RemoteSpec, TaskSpec};
```

- [ ] **Step 2: Update the key dispatch in `handle_key`**

Replace the current `handle_key` function:

```rust
pub fn handle_key(app: &mut App, key: KeyEvent) {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Tab => {
            app.focused_pane = match app.focused_pane {
                Pane::Tasks => Pane::Remotes,
                Pane::Remotes => Pane::Tasks,
            };
            app.status_message = None;
        }
        KeyCode::Up => navigate_up(app),
        KeyCode::Down => navigate_down(app),
        KeyCode::Char('a') => handle_add(app),
        KeyCode::Char('d') => handle_delete(app),
        KeyCode::Char('e') => handle_edit(app),
        KeyCode::Char('o') => handle_open_editor(app),
        _ => {}
    }
}
```

- [ ] **Step 3: Rename the old `handle_edit` to `handle_open_editor`**

Find the existing `handle_edit` function (the one that opens `$EDITOR`) and rename it:

```rust
fn handle_open_editor(app: &mut App) {
    suspend_tui();
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let _ = process::Command::new(&editor).arg(&app.config_path).status();
    resume_tui();
    app.reload();
}
```

- [ ] **Step 4: Add the new `handle_edit` dispatcher**

Add after `handle_delete`:

```rust
fn handle_edit(app: &mut App) {
    suspend_tui();
    let result = match app.focused_pane {
        Pane::Tasks => edit_task_prompt(app),
        Pane::Remotes => edit_remote_prompt(app),
    };
    resume_tui();
    match result {
        Ok(msg) => {
            app.status_message = Some(msg);
            app.reload();
        }
        Err(e) => app.status_message = Some(format!("error: {e}")),
    }
}
```

- [ ] **Step 5: Add `edit_task_prompt`**

Add after `handle_edit`:

```rust
fn edit_task_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;
    let old_name = task.name.clone();

    let name = inquire::Text::new("Task name:")
        .with_initial_value(&task.name)
        .prompt()
        .map_err(|e| e.to_string())?;

    if name.is_empty() {
        return Err("task name cannot be empty".to_string());
    }

    let desc_default = task.description.clone().unwrap_or_default();
    let desc_raw = inquire::Text::new("Description (blank = none):")
        .with_initial_value(&desc_default)
        .prompt()
        .map_err(|e| e.to_string())?;
    let description = if desc_raw.is_empty() { None } else { Some(desc_raw) };

    let (repo, directory, exclude_file, files_from) = if task.repo.is_some() {
        let repo = inquire::Text::new("Repo path:")
            .with_initial_value(task.repo.as_deref().unwrap_or(""))
            .prompt()
            .map_err(|e| e.to_string())?;

        let dir_raw = inquire::Text::new("Directory (blank = none):")
            .with_initial_value(task.directory.as_deref().unwrap_or(""))
            .prompt()
            .map_err(|e| e.to_string())?;
        let directory = if dir_raw.is_empty() { None } else { Some(dir_raw) };

        let excl_raw = inquire::Text::new("Exclude file (blank = none):")
            .with_initial_value(task.exclude_file.as_deref().unwrap_or(""))
            .prompt()
            .map_err(|e| e.to_string())?;
        let exclude_file = if excl_raw.is_empty() { None } else { Some(excl_raw) };

        let ff_raw = inquire::Text::new("Files from (blank = none):")
            .with_initial_value(task.files_from.as_deref().unwrap_or(""))
            .prompt()
            .map_err(|e| e.to_string())?;
        let files_from = if ff_raw.is_empty() { None } else { Some(ff_raw) };

        (Some(repo), directory, exclude_file, files_from)
    } else {
        (None, None, None, None)
    };

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_task(
        &kdl,
        &old_name,
        EditTaskSpec { name: name.clone(), description, repo, directory, exclude_file, files_from },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    Ok(format!("Updated task '{name}'."))
}
```

- [ ] **Step 6: Add `edit_remote_prompt`**

Add after `edit_task_prompt`:

```rust
fn edit_remote_prompt(app: &App) -> Result<String, String> {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or("no task selected")?;
    let remote = app
        .current_remotes()
        .get(app.selected_remote)
        .ok_or("no remote selected")?;
    let old_url = remote.url.clone();

    let url = inquire::Text::new("Remote URL:")
        .with_initial_value(&remote.url)
        .prompt()
        .map_err(|e| e.to_string())?;

    let credentials = inquire::Text::new("Credentials profile:")
        .with_initial_value(&remote.credentials)
        .prompt()
        .map_err(|e| e.to_string())?;

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_remote(
        &kdl,
        &task_name,
        &old_url,
        RemoteSpec { url, credentials },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    Ok("Updated remote.".to_string())
}
```

- [ ] **Step 7: Build and run full test suite**

```bash
cd vivo-cli && cargo test 2>&1
```

Expected: all tests pass.

- [ ] **Step 8: Run `vivo manage` and test the edit flows manually**

```bash
cd vivo-cli && cargo run -- manage
```

Test checklist:
- Press `e` on a task → pre-filled prompts appear for all fields → after saving, right pane reflects changes
- Press `e` on a task and change its name → `default-task` and any `calls` references update in the KDL
- Tab to Remotes, press `e` on a remote → pre-filled URL and credentials prompts → remote updates in place
- Press `o` → raw KDL opens in `$EDITOR`
- Press `a` in Tasks pane → add task prompt (unchanged behaviour)
- Press `a` in Remotes pane → add remote prompt (unchanged behaviour)
- Press `d` in Tasks pane → delete task confirmation (unchanged behaviour)
- Press `d` in Remotes pane → delete remote confirmation (unchanged behaviour)

- [ ] **Step 9: Commit**

```bash
git add src/tui/events.rs
git commit -m "feat(tui): add edit task/remote prompts, bind e and o keys"
```
