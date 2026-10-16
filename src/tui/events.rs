use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{env, process};

use super::app::{App, Pane, TaskEntry, FIELD_NAMES};
use super::credentials;
use crate::config_editor::{self as editor, EditTaskSpec, RemoteSpec, TaskSpec};
use crate::ensure_path;

/// Unwraps an inquire result inside a prompt function. A cancelled prompt
/// (Esc / Ctrl-C) returns `$cancelled` (default: an `Ok("Cancelled.")` status);
/// any other prompt error becomes `Err`.
macro_rules! ask {
    ($expr:expr) => {
        ask!($expr, Ok("Cancelled.".to_string()))
    };
    ($expr:expr, $cancelled:expr) => {
        match $expr {
            Ok(v) => v,
            Err(
                inquire::InquireError::OperationCanceled
                | inquire::InquireError::OperationInterrupted,
            ) => return $cancelled,
            Err(e) => return Err(e.to_string()),
        }
    };
}

/// Asks "Remove …?" and returns early with a "Cancelled." status on "no".
macro_rules! confirm_remove {
    ($($msg:tt)*) => {
        if !ask!(inquire::Confirm::new(&format!($($msg)*)).with_default(false).prompt()) {
            return Ok("Cancelled.".to_string());
        }
    };
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Up => return handle_move_call(app, -1),
            KeyCode::Down => return handle_move_call(app, 1),
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Tab => {
            app.focused_pane = app.focused_pane.next();
            app.status_message = None;
        }
        KeyCode::Enter if app.focused_pane == Pane::Fields => handle_edit_field(app),
        KeyCode::Up => app.move_selection(-1),
        KeyCode::Down => app.move_selection(1),
        KeyCode::Char('a') => handle_add(app),
        KeyCode::Char('d') => handle_delete(app),
        KeyCode::Char('e') => handle_edit(app),
        KeyCode::Char('o') => handle_open_editor(app),
        KeyCode::Char('t') => handle_test_remote(app),
        KeyCode::Char('s') => handle_set_default(app),
        _ => {}
    }
}

fn suspend_tui() {
    crossterm::terminal::disable_raw_mode().ok();
    crossterm::execute!(std::io::stdout(), crossterm::terminal::LeaveAlternateScreen).ok();
}

fn resume_tui() {
    crossterm::execute!(std::io::stdout(), crossterm::terminal::EnterAlternateScreen).ok();
    crossterm::terminal::enable_raw_mode().ok();
}

/// Suspend the TUI, run `f`, resume, then handle the result. Returns whether
/// `f` succeeded.
/// Sets `needs_clear` so the render loop does a full repaint on return.
/// If `reload` is true, reloads config on success. Empty Ok messages are silent.
fn run_prompt(
    app: &mut App,
    f: impl FnOnce(&App) -> Result<String, String>,
    reload: bool,
) -> bool {
    suspend_tui();
    let result = f(app);
    resume_tui();
    let ok = result.is_ok();
    match result {
        Ok(msg) => {
            if reload {
                app.reload();
            }
            if !msg.is_empty() {
                app.set_status(msg);
            }
        }
        Err(e) => app.set_status(format!("error: {e}")),
    }
    app.needs_clear = true;
    ok
}

fn selected_task_name(app: &App) -> Result<String, String> {
    app.tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or_else(|| "no task selected".to_string())
}

/// Turns a blank answer into `None`.
fn opt(answer: String) -> Option<String> {
    (!answer.trim().is_empty()).then_some(answer)
}

fn text(label: &str, initial: &str) -> Result<String, inquire::InquireError> {
    inquire::Text::new(label).with_initial_value(initial).prompt()
}

fn handle_add(app: &mut App) {
    run_prompt(
        app,
        |a| match a.focused_pane {
            Pane::Tasks | Pane::Fields => add_task_prompt(a),
            Pane::Remotes => add_remote_prompt(a),
            Pane::Calls => add_call_prompt(a),
        },
        true,
    );
}

fn add_task_prompt(app: &App) -> Result<String, String> {
    let name = ask!(inquire::Text::new("Task name:").prompt());
    let repo = opt(ask!(inquire::Text::new("Restic repo path (blank for calls-only task):")
        .with_help_message("Leave blank to create a task that only calls other tasks")
        .prompt()));
    let directory = match repo {
        Some(_) => opt(ask!(inquire::Text::new("Directory to back up (blank to skip):")
            .with_help_message("Leave blank to skip")
            .prompt())),
        None => None,
    };

    editor::update_file(&app.config_path, |kdl| {
        editor::add_task(
            kdl,
            TaskSpec { name: name.clone(), repo: repo.clone(), directory: directory.clone(), exclude_file: None },
        )
    })?;
    for path in repo.iter().chain(&directory) {
        ensure_path(path);
    }
    Ok(format!("Added task '{name}'."))
}

/// Tests a just-saved remote and returns a one-line status for the footer.
/// Never fails the add/edit itself — the remote is already written.
fn offer_remote_test(url: &str, profile: &str, secrets_path: &str) -> String {
    match crate::remote_setup::test_remote(url, profile, secrets_path, true) {
        Ok(msg) => msg,
        Err(e) => format!("Warning: {}", e.lines().next().unwrap_or("remote test failed")),
    }
}

/// Parses the "mc --max-workers" answer; blank means default.
fn parse_workers(raw: &str) -> Result<Option<u32>, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    raw.parse().map(Some).map_err(|_| format!("'{raw}' is not a valid integer"))
}

/// Prompts for a remote's URL, credentials profile and (for rustfs) `mc`
/// tuning, pre-filled from `existing` when editing. `Ok(None)` if cancelled.
fn prompt_remote(
    existing: Option<&super::app::RemoteEntry>,
    secrets_path: &str,
) -> Result<Option<RemoteSpec>, String> {
    let url = ask!(
        match existing {
            Some(r) => text("Remote URL:", &r.url),
            None => inquire::Text::new("Remote URL (e.g. rustfs:http://nas:9000/bucket):").prompt(),
        },
        Ok(None)
    );
    if url.trim().is_empty() {
        return Err("remote URL cannot be empty".to_string());
    }
    let is_rustfs = url.starts_with("rustfs:");
    if is_rustfs && !crate::remote::s3_sync_tool_installed() {
        eprintln!(
            "[warn] no S3 sync tool found (mc, aws, or rclone) — \
             sync to this remote will fail until one is installed. \
             Install mc: https://min.io/docs/minio/linux/reference/minio-mc.html"
        );
    }
    let Some(credentials) = credentials::select_or_create_profile(&url, secrets_path)? else {
        return Ok(None);
    };

    let (mc_max_workers, mc_limit_upload) = if is_rustfs {
        let workers = existing.and_then(|r| r.mc_max_workers).map(|w| w.to_string());
        let raw_workers = ask!(
            inquire::Text::new("mc --max-workers (leave blank for default):")
                .with_placeholder("e.g. 4")
                .with_initial_value(workers.as_deref().unwrap_or(""))
                .prompt(),
            Ok(None)
        );
        let limit = existing.and_then(|r| r.mc_limit_upload.as_deref()).unwrap_or("");
        let raw_limit = ask!(
            inquire::Text::new("mc --limit-upload rate (leave blank for no limit):")
                .with_placeholder("e.g. 5MiB")
                .with_initial_value(limit)
                .prompt(),
            Ok(None)
        );
        (parse_workers(&raw_workers)?, opt(raw_limit.trim().to_string()))
    } else {
        (None, None)
    };

    Ok(Some(RemoteSpec { url, credentials, mc_max_workers, mc_limit_upload }))
}

fn add_remote_prompt(app: &App) -> Result<String, String> {
    let task_name = selected_task_name(app)?;
    let secrets_path = crate::config::secrets_path_from();
    let Some(spec) = prompt_remote(None, &secrets_path)? else {
        return Ok("Cancelled.".to_string());
    };
    let (url, credentials) = (spec.url.clone(), spec.credentials.clone());

    editor::update_file(&app.config_path, |kdl| editor::add_remote(kdl, &task_name, spec))?;

    let test_msg = offer_remote_test(&url, &credentials, &secrets_path);
    Ok(format!("Added remote '{url}' to task '{task_name}'. {test_msg}"))
}

fn edit_remote_prompt(app: &App) -> Result<String, String> {
    let task_name = selected_task_name(app)?;
    let remote = app.current_remotes().get(app.selected_remote).ok_or("no remote selected")?;
    let secrets_path = crate::config::secrets_path_from();
    let Some(spec) = prompt_remote(Some(remote), &secrets_path)? else {
        return Ok("Cancelled.".to_string());
    };
    let (url, credentials) = (spec.url.clone(), spec.credentials.clone());

    editor::update_file(&app.config_path, |kdl| {
        editor::edit_remote(kdl, &task_name, &remote.url, spec)
    })?;

    let test_msg = offer_remote_test(&url, &credentials, &secrets_path);
    Ok(format!("Updated remote. {test_msg}"))
}

fn add_call_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;

    let options: Vec<String> = app
        .tasks
        .iter()
        .filter(|t| t.name != task.name && !task.calls.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();

    if options.is_empty() {
        return Ok("No other tasks to add.".to_string());
    }

    let call_name = ask!(inquire::Select::new("Call which task?", options).prompt());

    editor::update_file(&app.config_path, |kdl| editor::add_call(kdl, &task.name, &call_name))?;
    Ok(format!("Added call to '{call_name}'."))
}

fn handle_delete(app: &mut App) {
    if app.focused_pane == Pane::Remotes && app.current_remotes().is_empty() {
        return;
    }
    if app.focused_pane == Pane::Calls && app.current_calls().is_empty() {
        return;
    }
    run_prompt(
        app,
        |a| match a.focused_pane {
            Pane::Tasks | Pane::Fields => delete_task_prompt(a),
            Pane::Remotes => delete_remote_prompt(a),
            Pane::Calls => delete_call_prompt(a),
        },
        true,
    );
}

/// Moves the selected call up (`delta < 0`) or down (`delta > 0`) in the
/// task's call order, and the selection with it.
fn handle_move_call(app: &mut App, delta: isize) {
    if app.focused_pane != Pane::Calls {
        return;
    }
    let idx = app.selected_call;
    match idx.checked_add_signed(delta) {
        Some(target) if target < app.current_calls().len() => {}
        _ => return,
    }
    let moved = run_prompt(
        app,
        |a| {
            let task_name = selected_task_name(a)?;
            editor::update_file(&a.config_path, |kdl| {
                if delta < 0 {
                    editor::move_call_up(kdl, &task_name, idx)
                } else {
                    editor::move_call_down(kdl, &task_name, idx)
                }
            })?;
            Ok(String::new())
        },
        true,
    );
    if moved {
        app.selected_call = idx.saturating_add_signed(delta);
    }
}

fn delete_task_prompt(app: &App) -> Result<String, String> {
    let name = selected_task_name(app)?;
    confirm_remove!("Remove task '{name}'?");
    editor::update_file(&app.config_path, |kdl| editor::remove_task(kdl, &name))?;
    Ok(format!("Removed task '{name}'."))
}

fn delete_remote_prompt(app: &App) -> Result<String, String> {
    let task_name = selected_task_name(app)?;
    let url = &app.current_remotes().get(app.selected_remote).ok_or("no remote selected")?.url;
    confirm_remove!("Remove remote '{url}' from task '{task_name}'?");
    editor::update_file(&app.config_path, |kdl| editor::remove_remote(kdl, &task_name, url))?;
    Ok(format!("Removed remote '{url}'."))
}

fn delete_call_prompt(app: &App) -> Result<String, String> {
    let task_name = selected_task_name(app)?;
    let call_name = app.current_calls().get(app.selected_call).ok_or("no call selected")?;
    confirm_remove!("Remove call to '{call_name}' from task '{task_name}'?");
    editor::update_file(&app.config_path, |kdl| {
        editor::remove_call(kdl, &task_name, app.selected_call)
    })?;
    Ok(format!("Removed call to '{call_name}'."))
}

fn handle_open_editor(app: &mut App) {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    run_prompt(
        app,
        |a| {
            process::Command::new(&editor).arg(&a.config_path).status().ok();
            Ok(String::new())
        },
        true,
    );
}

fn handle_edit(app: &mut App) {
    match app.focused_pane {
        Pane::Fields => handle_edit_field(app),
        Pane::Tasks => {
            run_prompt(app, edit_task_prompt, true);
        }
        Pane::Remotes if !app.current_remotes().is_empty() => {
            run_prompt(app, edit_remote_prompt, true);
        }
        Pane::Remotes | Pane::Calls => {}
    }
}

fn handle_edit_field(app: &mut App) {
    run_prompt(app, edit_field_prompt, true);
}

fn handle_test_remote(app: &mut App) {
    if app.focused_pane != Pane::Remotes || app.current_remotes().is_empty() {
        return;
    }
    run_prompt(app, test_remote_prompt, false);
}

fn handle_set_default(app: &mut App) {
    if app.focused_pane != Pane::Tasks {
        return;
    }
    let Ok(task_name) = selected_task_name(app) else {
        return;
    };
    if task_name == app.default_task {
        app.set_status("Already the default task.".to_string());
        return;
    }
    run_prompt(
        app,
        |a| {
            editor::update_file(&a.config_path, |kdl| editor::set_default_task(kdl, &task_name))?;
            Ok(format!("Set '{task_name}' as default task."))
        },
        true,
    );
}

fn test_remote_prompt(app: &App) -> Result<String, String> {
    let remote = app
        .current_remotes()
        .get(app.selected_remote)
        .ok_or("no remote selected")?;
    let url = remote.url.clone();
    let profile = remote.credentials.clone();
    let secrets_path = crate::config::secrets_path_from();

    println!("Testing remote: {url}");
    println!("Credentials:    {profile}\n");

    let result = crate::remote_setup::test_remote(&url, &profile, &secrets_path, true);
    let summary = match &result {
        Ok(msg) => {
            println!("{msg}");
            format!("Remote test passed. {msg}")
        }
        Err(e) => {
            println!("{e}");
            format!("Remote test failed: {}", e.lines().next().unwrap_or(""))
        }
    };

    println!("\nPress Enter to return...");
    std::io::stdin().read_line(&mut String::new()).ok();

    Ok(summary)
}

/// The editable fields of the Fields pane, as spec slots, in `FIELD_NAMES` order.
fn field_slot(spec: &mut EditTaskSpec, field: usize) -> Option<&mut Option<String>> {
    match field {
        1 => Some(&mut spec.description),
        2 => Some(&mut spec.repo),
        3 => Some(&mut spec.directory),
        4 => Some(&mut spec.exclude_file),
        5 => Some(&mut spec.files_from),
        _ => None,
    }
}

fn task_spec(task: &TaskEntry) -> EditTaskSpec {
    EditTaskSpec {
        name: task.name.clone(),
        description: task.description.clone(),
        repo: task.repo.clone(),
        directory: task.directory.clone(),
        exclude_file: task.exclude_file.clone(),
        files_from: task.files_from.clone(),
    }
}

fn save_task_edit(app: &App, old_name: &str, spec: EditTaskSpec) -> Result<String, String> {
    let name = spec.name.clone();
    let paths = [spec.repo.clone(), spec.directory.clone()];
    editor::update_file(&app.config_path, |kdl| editor::edit_task(kdl, old_name, spec))?;
    for path in paths.iter().flatten() {
        ensure_path(path);
    }
    Ok(format!("Updated task '{name}'."))
}

fn edit_field_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;
    let mut spec = task_spec(task);

    if app.selected_field >= 3 && task.repo.is_none() {
        return Err("set a repo path first".to_string());
    }

    if app.selected_field == 0 {
        let name = ask!(text("Task name:", &spec.name));
        if name.trim().is_empty() {
            return Err("task name cannot be empty".to_string());
        }
        spec.name = name;
    } else if let Some(slot) = field_slot(&mut spec, app.selected_field) {
        let label = format!("{} (blank = none):", FIELD_NAMES[app.selected_field]);
        *slot = opt(ask!(text(&label, slot.as_deref().unwrap_or(""))));
    } else {
        return Ok(String::new());
    }

    save_task_edit(app, &task.name, spec)
}

fn edit_task_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;
    let mut spec = task_spec(task);

    spec.name = ask!(text("Task name:", &task.name));
    if spec.name.trim().is_empty() {
        return Err("task name cannot be empty".to_string());
    }
    spec.description =
        opt(ask!(text("Description (blank = none):", task.description.as_deref().unwrap_or(""))));

    // Calls-only tasks have no backup settings to edit.
    if let Some(repo) = &task.repo {
        spec.repo = Some(ask!(text("Repo path:", repo)));
        for (field, name) in FIELD_NAMES.iter().enumerate().skip(3) {
            let slot = field_slot(&mut spec, field).expect("field index is in range");
            let label = format!("{name} (blank = none):");
            *slot = opt(ask!(text(&label, slot.as_deref().unwrap_or(""))));
        }
    }

    save_task_edit(app, &task.name, spec)
}
