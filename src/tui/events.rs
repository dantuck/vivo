use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{env, process};

use super::app::{App, Pane, FIELD_NAMES};
use super::credentials;
use crate::config_editor::{EditTaskSpec, RemoteSpec, TaskSpec};

fn ensure_path(path: &str) {
    let expanded = crate::backup_config::backup::expand_env_vars(path);
    if !expanded.is_empty() {
        let _ = std::fs::create_dir_all(&expanded);
    }
}

macro_rules! ask {
    ($expr:expr) => {
        match $expr {
            Ok(v) => v,
            Err(
                inquire::InquireError::OperationCanceled
                | inquire::InquireError::OperationInterrupted,
            ) => return Ok("Cancelled.".to_string()),
            Err(e) => return Err(e.to_string()),
        }
    };
}

pub fn handle_key(app: &mut App, key: KeyEvent) {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        match key.code {
            KeyCode::Up => {
                handle_move_call_up(app);
                return;
            }
            KeyCode::Down => {
                handle_move_call_down(app);
                return;
            }
            _ => {}
        }
    }

    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Tab => {
            app.focused_pane = match app.focused_pane {
                Pane::Tasks => Pane::Fields,
                Pane::Fields => Pane::Remotes,
                Pane::Remotes => Pane::Calls,
                Pane::Calls => Pane::Tasks,
            };
            app.status_message = None;
        }
        KeyCode::Enter => {
            if app.focused_pane == Pane::Fields {
                handle_edit_field(app);
            }
        }
        KeyCode::Up => navigate_up(app),
        KeyCode::Down => navigate_down(app),
        KeyCode::Char('a') => handle_add(app),
        KeyCode::Char('d') => handle_delete(app),
        KeyCode::Char('e') => handle_edit(app),
        KeyCode::Char('o') => handle_open_editor(app),
        KeyCode::Char('t') => handle_test_remote(app),
        KeyCode::Char('s') => handle_set_default(app),
        _ => {}
    }
}

fn navigate_up(app: &mut App) {
    match app.focused_pane {
        Pane::Tasks => {
            if app.selected_task > 0 {
                app.selected_task -= 1;
                app.selected_remote = 0;
                app.selected_field = 0;
                app.selected_call = 0;
            }
        }
        Pane::Fields => {
            if app.selected_field > 0 {
                app.selected_field -= 1;
            }
        }
        Pane::Remotes => {
            if app.selected_remote > 0 {
                app.selected_remote -= 1;
            }
        }
        Pane::Calls => {
            if app.selected_call > 0 {
                app.selected_call -= 1;
            }
        }
    }
}

fn navigate_down(app: &mut App) {
    match app.focused_pane {
        Pane::Tasks => {
            if app.selected_task + 1 < app.tasks.len() {
                app.selected_task += 1;
                app.selected_remote = 0;
                app.selected_field = 0;
                app.selected_call = 0;
            }
        }
        Pane::Fields => {
            if app.selected_field + 1 < FIELD_NAMES.len() {
                app.selected_field += 1;
            }
        }
        Pane::Remotes => {
            let max = app.current_remotes().len().saturating_sub(1);
            if app.selected_remote < max {
                app.selected_remote += 1;
            }
        }
        Pane::Calls => {
            let max = app.current_calls().len().saturating_sub(1);
            if app.selected_call < max {
                app.selected_call += 1;
            }
        }
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

/// Suspend the TUI, run `f`, resume, then handle the result.
/// Sets `needs_clear` so the render loop does a full repaint on return.
/// If `reload` is true, reloads config on success. Empty Ok messages are silent.
fn run_prompt(
    app: &mut App,
    f: impl FnOnce(&App) -> Result<String, String>,
    reload: bool,
) {
    suspend_tui();
    let result = f(app);
    resume_tui();
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
    let repo_raw = ask!(inquire::Text::new("Restic repo path (blank for calls-only task):")
        .with_help_message("Leave blank to create a task that only calls other tasks")
        .prompt());
    let repo = if repo_raw.is_empty() { None } else { Some(repo_raw) };

    let directory = if repo.is_some() {
        let dir_raw = ask!(inquire::Text::new("Directory to back up (blank to skip):")
            .with_help_message("Leave blank to skip")
            .prompt());
        if dir_raw.is_empty() { None } else { Some(dir_raw) }
    } else {
        None
    };

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::add_task(
        &kdl,
        TaskSpec { name: name.clone(), repo: repo.clone(), directory: directory.clone(), exclude_file: None },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    if let Some(r) = &repo {
        ensure_path(r);
    }
    if let Some(dir) = &directory {
        ensure_path(dir);
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

fn add_remote_prompt(app: &App) -> Result<String, String> {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or("no task selected")?;

    let url = ask!(inquire::Text::new("Remote URL (e.g. rustfs:http://nas:9000/bucket):").prompt());
    if url.starts_with("rustfs:") && !crate::remote::s3_sync_tool_installed()
    {
        eprintln!(
            "[warn] no S3 sync tool found (mc, aws, or rclone) — \
             sync to this remote will fail until one is installed. \
             Install mc: https://min.io/docs/minio/linux/reference/minio-mc.html"
        );
    }
    let secrets_path = crate::config::secrets_path_from();
    let credentials = match credentials::select_or_create_profile(&url, &secrets_path)? {
        Some(p) => p,
        None => return Ok("Cancelled.".to_string()),
    };

    let (mc_max_workers, mc_limit_upload) = if url.starts_with("rustfs:") {
        let raw_workers = ask!(inquire::Text::new(
            "mc --max-workers (leave blank for default):"
        )
        .with_placeholder("e.g. 4")
        .prompt());
        let p = if raw_workers.trim().is_empty() {
            None
        } else {
            Some(
                raw_workers
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| format!("'{}' is not a valid integer", raw_workers.trim()))?,
            )
        };
        let raw_limit = ask!(inquire::Text::new(
            "mc --limit-upload rate (leave blank for no limit):"
        )
        .with_placeholder("e.g. 5MiB")
        .prompt());
        let l = if raw_limit.trim().is_empty() {
            None
        } else {
            Some(raw_limit.trim().to_string())
        };
        (p, l)
    } else {
        (None, None)
    };

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::add_remote(
        &kdl,
        &task_name,
        RemoteSpec { url: url.clone(), credentials: credentials.clone(), mc_max_workers, mc_limit_upload },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;

    let test_msg = offer_remote_test(&url, &credentials, &secrets_path);
    Ok(format!("Added remote '{url}' to task '{task_name}'. {test_msg}"))
}

fn add_call_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;
    let task_name = task.name.clone();

    let options: Vec<String> = app
        .tasks
        .iter()
        .filter(|t| t.name != task_name && !task.calls.contains(&t.name))
        .map(|t| t.name.clone())
        .collect();

    if options.is_empty() {
        return Ok("No other tasks to add.".to_string());
    }

    let call_name = ask!(inquire::Select::new("Call which task?", options).prompt());

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::add_call(&kdl, &task_name, &call_name)?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
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

fn handle_move_call_up(app: &mut App) {
    if app.focused_pane != Pane::Calls || app.selected_call == 0 {
        return;
    }
    let idx = app.selected_call;
    run_prompt(
        app,
        move |a| {
            let task_name = a
                .tasks
                .get(a.selected_task)
                .map(|t| t.name.clone())
                .ok_or("no task selected")?;
            let kdl = std::fs::read_to_string(&a.config_path).map_err(|e| e.to_string())?;
            let new_kdl = crate::config_editor::move_call_up(&kdl, &task_name, idx)?;
            std::fs::write(&a.config_path, new_kdl).map_err(|e| e.to_string())?;
            Ok(String::new())
        },
        true,
    );
    let failed = app.status_message.as_deref().map(|s| s.starts_with("error:")).unwrap_or(false);
    if !failed && app.selected_call > 0 {
        app.selected_call -= 1;
    }
}

fn handle_move_call_down(app: &mut App) {
    if app.focused_pane != Pane::Calls {
        return;
    }
    let calls_len = app.current_calls().len();
    if app.selected_call + 1 >= calls_len {
        return;
    }
    let idx = app.selected_call;
    run_prompt(
        app,
        move |a| {
            let task_name = a
                .tasks
                .get(a.selected_task)
                .map(|t| t.name.clone())
                .ok_or("no task selected")?;
            let kdl = std::fs::read_to_string(&a.config_path).map_err(|e| e.to_string())?;
            let new_kdl = crate::config_editor::move_call_down(&kdl, &task_name, idx)?;
            std::fs::write(&a.config_path, new_kdl).map_err(|e| e.to_string())?;
            Ok(String::new())
        },
        true,
    );
    let failed = app.status_message.as_deref().map(|s| s.starts_with("error:")).unwrap_or(false);
    if !failed {
        app.selected_call += 1;
    }
}

fn delete_task_prompt(app: &App) -> Result<String, String> {
    let name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or("no task selected")?;

    let ok = ask!(inquire::Confirm::new(&format!("Remove task '{name}'?"))
        .with_default(false)
        .prompt());
    if !ok {
        return Ok("Cancelled.".to_string());
    }

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::remove_task(&kdl, &name)?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    Ok(format!("Removed task '{name}'."))
}

fn delete_remote_prompt(app: &App) -> Result<String, String> {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or("no task selected")?;
    let url = app
        .current_remotes()
        .get(app.selected_remote)
        .map(|r| r.url.clone())
        .ok_or("no remote selected")?;

    let ok = ask!(inquire::Confirm::new(&format!(
        "Remove remote '{url}' from task '{task_name}'?"
    ))
    .with_default(false)
    .prompt());
    if !ok {
        return Ok("Cancelled.".to_string());
    }

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::remove_remote(&kdl, &task_name, &url)?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    Ok(format!("Removed remote '{url}'."))
}

fn delete_call_prompt(app: &App) -> Result<String, String> {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or("no task selected")?;
    let call_name = app
        .current_calls()
        .get(app.selected_call)
        .cloned()
        .ok_or("no call selected")?;

    let ok = ask!(inquire::Confirm::new(&format!(
        "Remove call to '{call_name}' from task '{task_name}'?"
    ))
    .with_default(false)
    .prompt());
    if !ok {
        return Ok("Cancelled.".to_string());
    }

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::remove_call(&kdl, &task_name, app.selected_call)?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
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
        Pane::Tasks => run_prompt(app, edit_task_prompt, true),
        Pane::Remotes => {
            if app.current_remotes().is_empty() {
                return;
            }
            run_prompt(app, edit_remote_prompt, true);
        }
        Pane::Calls => {}
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
    let task_name = match app.tasks.get(app.selected_task) {
        Some(t) => t.name.clone(),
        None => return,
    };
    if task_name == app.default_task {
        app.set_status("Already the default task.".to_string());
        return;
    }
    run_prompt(
        app,
        move |a| {
            let kdl = std::fs::read_to_string(&a.config_path).map_err(|e| e.to_string())?;
            let new_kdl = crate::config_editor::set_default_task(&kdl, &task_name)?;
            std::fs::write(&a.config_path, new_kdl).map_err(|e| e.to_string())?;
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

fn edit_field_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;
    let old_name = task.name.clone();

    let mut name = task.name.clone();
    let mut description = task.description.clone();
    let mut repo = task.repo.clone();
    let mut directory = task.directory.clone();
    let mut exclude_file = task.exclude_file.clone();
    let mut files_from = task.files_from.clone();

    if app.selected_field >= 3 && task.repo.is_none() {
        return Err("set a repo path first".to_string());
    }

    match app.selected_field {
        0 => {
            let v = ask!(inquire::Text::new("Task name:").with_initial_value(&name).prompt());
            if v.trim().is_empty() {
                return Err("task name cannot be empty".to_string());
            }
            name = v;
        }
        1 => {
            let v = ask!(inquire::Text::new("Description (blank = none):")
                .with_initial_value(description.as_deref().unwrap_or(""))
                .prompt());
            description = if v.is_empty() { None } else { Some(v) };
        }
        2 => {
            let v = ask!(inquire::Text::new("Repo path (blank = none):")
                .with_initial_value(repo.as_deref().unwrap_or(""))
                .prompt());
            repo = if v.is_empty() { None } else { Some(v) };
        }
        3 => {
            let v = ask!(inquire::Text::new("Directory (blank = none):")
                .with_initial_value(directory.as_deref().unwrap_or(""))
                .prompt());
            directory = if v.is_empty() { None } else { Some(v) };
        }
        4 => {
            let v = ask!(inquire::Text::new("Exclude file (blank = none):")
                .with_initial_value(exclude_file.as_deref().unwrap_or(""))
                .prompt());
            exclude_file = if v.is_empty() { None } else { Some(v) };
        }
        5 => {
            let v = ask!(inquire::Text::new("Files from (blank = none):")
                .with_initial_value(files_from.as_deref().unwrap_or(""))
                .prompt());
            files_from = if v.is_empty() { None } else { Some(v) };
        }
        _ => return Ok(String::new()),
    }

    let repo_path = repo.clone();
    let dir_path = directory.clone();
    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_task(
        &kdl,
        &old_name,
        EditTaskSpec { name: name.clone(), description, repo, directory, exclude_file, files_from },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    if let Some(r) = repo_path { ensure_path(&r); }
    if let Some(d) = dir_path { ensure_path(&d); }
    Ok(format!("Updated '{name}'."))
}

fn edit_task_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;
    let old_name = task.name.clone();

    let name = ask!(inquire::Text::new("Task name:")
        .with_initial_value(&task.name)
        .prompt());

    if name.is_empty() {
        return Err("task name cannot be empty".to_string());
    }

    let desc_default = task.description.clone().unwrap_or_default();
    let desc_raw = ask!(inquire::Text::new("Description (blank = none):")
        .with_initial_value(&desc_default)
        .prompt());
    let description = if desc_raw.is_empty() { None } else { Some(desc_raw) };

    let (repo, directory, exclude_file, files_from) = if task.repo.is_some() {
        let repo = ask!(inquire::Text::new("Repo path:")
            .with_initial_value(task.repo.as_deref().unwrap_or(""))
            .prompt());

        let dir_raw = ask!(inquire::Text::new("Directory (blank = none):")
            .with_initial_value(task.directory.as_deref().unwrap_or(""))
            .prompt());
        let directory = if dir_raw.is_empty() { None } else { Some(dir_raw) };

        let excl_raw = ask!(inquire::Text::new("Exclude file (blank = none):")
            .with_initial_value(task.exclude_file.as_deref().unwrap_or(""))
            .prompt());
        let exclude_file = if excl_raw.is_empty() { None } else { Some(excl_raw) };

        let ff_raw = ask!(inquire::Text::new("Files from (blank = none):")
            .with_initial_value(task.files_from.as_deref().unwrap_or(""))
            .prompt());
        let files_from = if ff_raw.is_empty() { None } else { Some(ff_raw) };

        (Some(repo), directory, exclude_file, files_from)
    } else {
        (None, None, None, None)
    };

    let repo_path = repo.clone();
    let dir_path = directory.clone();
    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_task(
        &kdl,
        &old_name,
        EditTaskSpec { name: name.clone(), description, repo, directory, exclude_file, files_from },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    if let Some(r) = repo_path { ensure_path(&r); }
    if let Some(d) = dir_path { ensure_path(&d); }
    Ok(format!("Updated task '{name}'."))
}

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

    let url = ask!(inquire::Text::new("Remote URL:")
        .with_initial_value(&remote.url)
        .prompt());
    if url.trim().is_empty() {
        return Err("remote URL cannot be empty".to_string());
    }

    let secrets_path = crate::config::secrets_path_from();
    let credentials = match credentials::select_or_create_profile(&url, &secrets_path)? {
        Some(p) => p,
        None => return Ok("Cancelled.".to_string()),
    };

    let (mc_max_workers, mc_limit_upload) = if url.starts_with("rustfs:") {
        // Pre-fill from the existing remote entry
        let existing_workers = remote.mc_max_workers.map(|p| p.to_string()).unwrap_or_default();
        let existing_limit = remote.mc_limit_upload.clone().unwrap_or_default();

        let raw_workers = ask!(inquire::Text::new(
            "mc --max-workers (leave blank for default):"
        )
        .with_initial_value(&existing_workers)
        .prompt());
        let p = if raw_workers.trim().is_empty() {
            None
        } else {
            Some(
                raw_workers
                    .trim()
                    .parse::<u32>()
                    .map_err(|_| format!("'{}' is not a valid integer", raw_workers.trim()))?,
            )
        };

        let raw_limit = ask!(inquire::Text::new(
            "mc --limit-upload rate (leave blank for no limit):"
        )
        .with_initial_value(&existing_limit)
        .prompt());
        let l = if raw_limit.trim().is_empty() {
            None
        } else {
            Some(raw_limit.trim().to_string())
        };
        (p, l)
    } else {
        (None, None)
    };

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_remote(
        &kdl,
        &task_name,
        &old_url,
        RemoteSpec { url: url.clone(), credentials: credentials.clone(), mc_max_workers, mc_limit_upload },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;

    let test_msg = offer_remote_test(&url, &credentials, &secrets_path);
    Ok(format!("Updated remote. {test_msg}"))
}
