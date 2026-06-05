# Calls Management in the TUI

**Date:** 2026-06-05
**Status:** Approved

## Overview

Tasks can reference other tasks via `calls "name"` KDL nodes. A task like `daily_backup` may have no backup block at all — just a list of calls that delegate to other tasks. Currently the TUI does not surface `calls` anywhere; this design adds a full-CRUD Calls pane to the task detail view.

## Architecture & Data Model

### `task.rs`

Add a `call_names()` accessor, consistent with `backup_remotes()`:

```rust
pub fn call_names(&self) -> Vec<&str> {
    self.calls.iter().map(|c| c.name.as_str()).collect()
}
```

### `app.rs`

- Add `Pane::Calls` to the `Pane` enum.
- Add `calls: Vec<String>` to `TaskEntry`, populated from `t.call_names()`.
- Add `selected_call: usize` to `App`.
- Tab cycle: Tasks → Fields → Remotes → Calls → Tasks.
- On `reload()`, clamp `selected_call` to `calls.len().saturating_sub(1)`, mirroring `selected_remote`.

### `config_editor.rs`

Add/remove/reorder `calls "name"` child nodes inside the matching `task` block, using the same KDL editing pattern as existing remote add/remove logic.

## UI Layout

`draw_task_detail` in `ui.rs` splits the inner area into three vertical chunks instead of two:

```
┌─ Task: daily_backup ────────────────────┐
│ Name          daily_backup              │  ← Fields (Constraint::Length(6))
│ Description   Perform daily backup...   │
│ Repo          (none)                    │
│ Directory     (none)                    │
│ Exclude file  (none)                    │
│ Files from    (none)                    │
│─────────────────────────────────────────│
│ Remotes (0):                            │  ← Remotes (Constraint::Min(0))
│                                         │
│─────────────────────────────────────────│
│ Calls (2):                              │  ← Calls (Constraint::Min(0))
│   filecabinet                           │
│ > dantuckx                              │
└─────────────────────────────────────────┘
```

The Calls section renders identically to Remotes: a header line (`Calls (n):` in yellow when focused, dark gray otherwise), then a selectable `List`. The selected entry is highlighted with `Modifier::REVERSED` when `Pane::Calls` is focused.

Help bar when `Pane::Calls` is focused:
```
[a] add  [d] delete  [Ctrl+↑↓] reorder  [Tab] switch pane  [q] quit
```

## Interactions & Events

All three operations are handled in `events.rs` when `Pane::Calls` is focused:

### Add (`a`)
Show a picker listing all task names, excluding:
- The current task itself
- Any task already in the calls list

Selecting an entry appends a `calls "name"` node to the task in KDL via `config_editor`, then calls `app.reload()`.

If no eligible tasks remain, show a status message: `"No other tasks to add"`.

### Delete (`d`)
Remove the `calls` node at index `selected_call` from the task in KDL, then reload. No confirmation prompt (matches Remotes behavior).

### Reorder (`Ctrl+↑` / `Ctrl+↓`)
Swap the selected call with the adjacent one (above or below) in KDL, reload, and keep `selected_call` tracking the moved item. No-op when the list has only one entry or when at the boundary.

### Navigation
`↑`/`↓` navigate within the list; bounds are clamped on reload. Tab moves focus to the next pane.

## Edge Cases

| Situation | Behavior |
|-----------|----------|
| All tasks already called | Status: "No other tasks to add"; picker not shown |
| One call in list | Reorder keys are no-ops |
| Delete last call | `selected_call` clamps to 0 |
| Task has no backup block | Fields show `(none)`; Remotes shows `Remotes (0):`; Calls shows entries normally |
