use crossterm::event::{KeyCode, KeyEvent};
use std::{env, process};

use super::app::{App, Pane, FIELD_NAMES};
use super::credentials;
use crate::config_editor::{EditTaskSpec, RemoteSpec, TaskSpec};

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
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
        KeyCode::Tab => {
            app.focused_pane = match app.focused_pane {
                Pane::Tasks => Pane::Fields,
                Pane::Fields => Pane::Remotes,
                Pane::Remotes => Pane::Tasks,
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
    }
}

fn navigate_down(app: &mut App) {
    match app.focused_pane {
        Pane::Tasks => {
            if app.selected_task + 1 < app.tasks.len() {
                app.selected_task += 1;
                app.selected_remote = 0;
                app.selected_field = 0;
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
        },
        true,
    );
}

fn add_task_prompt(app: &App) -> Result<String, String> {
    let name = ask!(inquire::Text::new("Task name:").prompt());
    let repo = ask!(inquire::Text::new("Restic repo path:").prompt());
    let dir_raw = ask!(inquire::Text::new("Directory to back up (blank to skip):")
        .with_help_message("Leave blank to skip")
        .prompt());
    let directory = if dir_raw.is_empty() { None } else { Some(dir_raw) };

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::add_task(
        &kdl,
        TaskSpec { name: name.clone(), repo, directory, exclude_file: None },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    Ok(format!("Added task '{name}'."))
}

fn add_remote_prompt(app: &App) -> Result<String, String> {
    let task_name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or("no task selected")?;

    let url = ask!(inquire::Text::new("Remote URL (e.g. rustfs:http://nas:9000/bucket):").prompt());
    let secrets_path = crate::config::secrets_path_from();
    let credentials = match credentials::select_or_create_profile(&url, &secrets_path)? {
        Some(p) => p,
        None => return Ok("Cancelled.".to_string()),
    };

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::add_remote(
        &kdl,
        &task_name,
        RemoteSpec { url: url.clone(), credentials },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    Ok(format!("Added remote '{url}' to task '{task_name}'."))
}

fn handle_delete(app: &mut App) {
    if app.focused_pane == Pane::Remotes && app.current_remotes().is_empty() {
        return;
    }
    run_prompt(
        app,
        |a| match a.focused_pane {
            Pane::Tasks | Pane::Fields => delete_task_prompt(a),
            Pane::Remotes => delete_remote_prompt(a),
        },
        true,
    );
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

fn test_remote_prompt(app: &App) -> Result<String, String> {
    let remote = app
        .current_remotes()
        .get(app.selected_remote)
        .ok_or("no remote selected")?;
    let url = remote.url.clone();
    let profile = remote.credentials.clone();

    let secrets_path = crate::config::secrets_path_from();
    println!("Decrypting secrets from {secrets_path}...");
    let decrypted =
        crate::backup_config::decrypt_sops_file(&secrets_path).map_err(|e| e.to_string())?;
    let secrets =
        crate::backup_config::parse_secrets(&decrypted).map_err(|e| e.to_string())?;

    env::set_var("RESTIC_PASSWORD", &secrets.restic_password);

    let creds = secrets.credentials.get(&profile).ok_or_else(|| {
        format!("credentials profile '{profile}' not found in secrets")
    })?;
    for (k, v) in creds {
        env::set_var(k, v);
    }

    println!("\nTesting remote: {url}");
    println!("Credentials:    {profile}\n");

    let status = if url.starts_with("b2:") {
        let path = url.trim_start_matches("b2:").trim_start_matches('/');
        process::Command::new("b2")
            .args(["ls", path])
            .status()
            .map_err(|e| format!("could not run b2: {e}"))?
    } else {
        // Translate vivo-specific schemes to restic-native equivalents before invoking restic
        let restic_url = if url.starts_with("rustfs:") {
            url.replacen("rustfs:", "s3:", 1)
        } else {
            url.clone()
        };
        process::Command::new("restic")
            .args(["-r", &restic_url, "snapshots", "--no-lock", "--no-cache"])
            .status()
            .map_err(|e| format!("could not run restic: {e}"))?
    };

    println!();
    let summary = if status.success() {
        "Remote test passed.".to_string()
    } else {
        format!("Remote test failed (exit {}).", status.code().unwrap_or(-1))
    };
    println!("{summary}");

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

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_task(
        &kdl,
        &old_name,
        EditTaskSpec { name: name.clone(), description, repo, directory, exclude_file, files_from },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
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

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_task(
        &kdl,
        &old_name,
        EditTaskSpec { name: name.clone(), description, repo, directory, exclude_file, files_from },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
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
