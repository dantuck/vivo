use crossterm::event::{KeyCode, KeyEvent};
use std::{env, process};

use super::app::{App, Pane};
use crate::config_editor::{RemoteSpec, TaskSpec};

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
        _ => {}
    }
}

fn navigate_up(app: &mut App) {
    match app.focused_pane {
        Pane::Tasks => {
            if app.selected_task > 0 {
                app.selected_task -= 1;
                app.selected_remote = 0;
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

fn handle_add(app: &mut App) {
    suspend_tui();
    let result = match app.focused_pane {
        Pane::Tasks => add_task_prompt(app),
        Pane::Remotes => add_remote_prompt(app),
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

fn add_task_prompt(app: &App) -> Result<String, String> {
    let name = inquire::Text::new("Task name:")
        .prompt()
        .map_err(|e| e.to_string())?;
    let repo = inquire::Text::new("Restic repo path:")
        .prompt()
        .map_err(|e| e.to_string())?;
    let dir_raw = inquire::Text::new("Directory to back up (blank to skip):")
        .with_help_message("Leave blank to skip")
        .prompt()
        .map_err(|e| e.to_string())?;
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

    let url = inquire::Text::new("Remote URL (e.g. rustfs:http://nas:9000/bucket):")
        .prompt()
        .map_err(|e| e.to_string())?;
    let credentials = inquire::Text::new("Credentials profile name:")
        .prompt()
        .map_err(|e| e.to_string())?;

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
    suspend_tui();
    let result = match app.focused_pane {
        Pane::Tasks => delete_task_prompt(app),
        Pane::Remotes => delete_remote_prompt(app),
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

fn delete_task_prompt(app: &App) -> Result<String, String> {
    let name = app
        .tasks
        .get(app.selected_task)
        .map(|t| t.name.clone())
        .ok_or("no task selected")?;

    let ok = inquire::Confirm::new(&format!("Remove task '{name}'?"))
        .with_default(false)
        .prompt()
        .map_err(|e| e.to_string())?;
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

    let ok = inquire::Confirm::new(&format!(
        "Remove remote '{url}' from task '{task_name}'?"
    ))
    .with_default(false)
    .prompt()
    .map_err(|e| e.to_string())?;
    if !ok {
        return Ok("Cancelled.".to_string());
    }

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::remove_remote(&kdl, &task_name, &url)?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    Ok(format!("Removed remote '{url}'."))
}

fn handle_edit(app: &mut App) {
    suspend_tui();
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    let _ = process::Command::new(&editor).arg(&app.config_path).status();
    resume_tui();
    app.reload();
}
