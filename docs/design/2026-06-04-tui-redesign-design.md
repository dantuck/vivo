# TUI Redesign — Full Detail View + Task/Remote Editing

**Date:** 2026-06-04  
**Status:** Approved

## Overview

Redesign `vivo manage` from a two-pane viewer (Tasks | Remotes) into a two-pane manager (Tasks | Task Detail) that shows all task fields and supports editing tasks and remotes via pre-filled terminal prompts. Also makes multi-remote support visible and obvious.

## Layout

Two panes — same structure as today, different right pane content.

**Left pane (30%) — Tasks list:**
- Navigable list of task names
- Focused when border is highlighted

**Right pane (70%) — Task detail:**
- Header: `Task: <name>`
- Field grid (label/value pairs):
  - Name, Description, Repo, Directory, Exclude file, Files from
  - Empty optional fields shown as `(none)` in dim text
- Separator line
- `Remotes (n)` section header
- List of remotes: `<url>  [<credentials>]`
  - When Remotes pane is focused: list is navigable with highlight
  - When Tasks pane is focused: list is dimmed, not navigable

**Help bar (bottom, 1 line):**
- Tasks pane: `[a] add  [d] delete  [e] edit task  [o] open in $EDITOR  [Tab] switch pane  [q] quit`
- Remotes pane: `[a] add remote  [d] delete  [e] edit remote  [Tab] switch pane  [q] quit`

## Key Bindings

| Key | Tasks pane | Remotes pane |
|-----|-----------|--------------|
| `↑`/`↓` | Navigate tasks | Navigate remotes |
| `a` | Add task (existing) | Add remote (existing) |
| `d` | Delete task (existing) | Delete remote (existing) |
| `e` | Edit task fields (new) | Edit selected remote (new) |
| `o` | Open raw KDL in `$EDITOR` | Open raw KDL in `$EDITOR` |
| `Tab` | Switch to Remotes pane | Switch to Tasks pane |
| `q` / `Esc` | Quit | Quit |

## Edit Flows

### Edit task (`e` in Tasks pane)

For tasks with a `backup` block, all 6 fields are editable. For command-only tasks (no `backup` block), only name and description are prompted — repo/directory/exclude-file/files-from are shown as `(not a backup task)` in the detail view and skipped in the edit prompt. Adding a backup block to a command-only task is out of scope; use `o` for that.

1. TUI suspends (raw mode off, leave alternate screen)
2. `inquire` prompts appear in order, each pre-filled with the current value:
   - Task name (always)
   - Description (always)
   - Repo path (backup tasks only)
   - Directory (backup tasks only; blank = none)
   - Exclude file (backup tasks only; blank = none)
   - Files from (backup tasks only; blank = none)
3. User edits fields, presses Enter on each to confirm (or Enter to keep current)
4. `config_editor::edit_task` writes updated KDL to disk
5. If name changed: `default-task` node and all `calls` references updated in same write
6. TUI resumes, config reloaded
7. Status message: `Updated task '<name>'.`

### Edit remote (`e` in Remotes pane)

1. TUI suspends
2. `inquire` prompts pre-filled with current remote values:
   - Remote URL
   - Credentials profile name
3. `config_editor::edit_remote` finds remote by old URL, updates URL and credentials in place
4. TUI resumes, config reloaded
5. Status message: `Updated remote.`

### Error handling

- If `inquire` prompt is cancelled (Ctrl+C or Esc): return `Ok("Cancelled.")` — no write, no error shown
- If KDL write fails: show `error: <message>` in status bar
- If rename target name already exists: return error `task '<name>' already exists`
- Blank required field (name): return error `task name cannot be empty`, display in status bar, no write

## Components

### `config_editor.rs` — new functions

```rust
pub struct EditTaskSpec {
    pub name: String,
    pub description: Option<String>,
    pub repo: String,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
}

pub fn edit_task(kdl: &str, old_name: &str, spec: EditTaskSpec) -> Result<String, String>
pub fn edit_remote(kdl: &str, task_name: &str, old_url: &str, spec: RemoteSpec) -> Result<String, String>
```

`edit_task` behaviour:
- Finds the `task "<old_name>"` node, updates its argument and child nodes
- If `spec.name != old_name`: also rewrites `default-task` argument and all `calls "<old_name>"` arguments throughout the document
- Optional fields: if `None`, removes the child node if present; if `Some(v)`, upserts it

`edit_remote` behaviour:
- Finds the `remote "<old_url>"` node within the task's backup block
- Updates its argument (URL) and `credentials` child node in place

### `backup_config/task.rs` + `backup_config/backup.rs` — expose fields

`Task` needs public accessors for:
- `description: Option<String>` (already has `description()`)
- `backup_repo()`, `backup_directory()`, `backup_exclude_file()`, `backup_files_from()` — delegating to `Backup`

`Backup` needs public accessors (or `pub` fields) for:
- `repo`, `directory`, `exclude_file`, `files_from`

### `tui/app.rs` — extend `TaskEntry`

```rust
pub struct TaskEntry {
    pub name: String,
    pub description: Option<String>,
    pub repo: String,
    pub directory: Option<String>,
    pub exclude_file: Option<String>,
    pub files_from: Option<String>,
    pub remotes: Vec<RemoteEntry>,
}
```

`App::new()` populates all fields from the config. Tasks without a `backup` block show `(none)` for repo and related fields — repo stored as empty string in that case.

### `tui/ui.rs` — rewrite right pane

- Rename `draw_remotes` → `draw_task_detail`
- Top: `Paragraph` widget with field grid using `Line` / `Span` for label/value colouring
- Bottom: `List` widget for remotes — `highlight_style` applied only when `app.focused_pane == Pane::Remotes`
- Help bar: dynamically built based on `app.focused_pane`

### `tui/events.rs` — key changes

- `KeyCode::Char('e')` → `handle_edit(app)` (dispatches by pane)
- `KeyCode::Char('o')` → `handle_open_editor(app)` (current `e` behaviour)
- New: `edit_task_prompt(app)` — pre-filled inquire for all 6 task fields
- New: `edit_remote_prompt(app)` — pre-filled inquire for URL + credentials

## Testing

New unit tests in `config_editor.rs`:
- `edit_task_updates_name_and_references` — rename updates `default-task` and `calls`
- `edit_task_preserves_unchanged_fields` — partial edits don't touch other fields
- `edit_task_removes_optional_field_when_none` — clearing directory removes the node
- `edit_task_errors_on_duplicate_name` — rename to existing name returns error
- `edit_remote_updates_url_and_credentials`
- `edit_remote_errors_when_not_found`

Existing tests for `add_task`, `remove_task`, `add_remote`, `remove_remote` remain unchanged.
