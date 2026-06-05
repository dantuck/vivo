# Set Default Task in TUI

**Date:** 2026-06-05
**Status:** Approved

## Overview

The `default-task` node in the KDL config determines which task runs when no task name is given. Currently there is no TUI action to change it — users must open `$EDITOR`. This design adds an `s` key in the Tasks pane to set the selected task as the default, with a `*` marker in the task list showing which task is currently default.

## Architecture & Data Model

### `config_editor.rs`

Add `set_default_task(kdl: &str, name: &str) -> Result<String, String>`: finds the top-level `default-task` node and replaces its string argument with `name`. Returns `Err` if the task name doesn't exist in the tasks list.

### `app.rs`

Add `pub default_task: String` to `App`, populated from `config.default_task` in `App::new()`. In `reload()`, save and restore `default_task` (no clamping — it's a plain string).

### `ui.rs`

In `draw_tasks`, append ` *` to the list item whose name matches `app.default_task`:

```
 backup-main *
 filecabinet
 dantuckx
```

The star is part of the list item text, so it inherits the selection highlight when that row is focused.

### `events.rs`

Handle `s` key when `Pane::Tasks` is focused:
- If the selected task is already the default: set status `"Already the default task."` (no write, no reload)
- Otherwise: call `config_editor::set_default_task`, write to disk, reload, set status `"Set '{name}' as default task."`

### Help bar

Tasks pane hint updated to include `[s] set default`.

## Edge Cases

| Situation | Behavior |
|-----------|----------|
| Selected task is already default | Status: "Already the default task."; no write |
| Task list is empty | `s` is a no-op (no task selected) |
