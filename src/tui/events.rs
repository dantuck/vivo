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
    let repo = ask!(inquire::Text::new("Restic repo path:").prompt());
    let dir_raw = ask!(inquire::Text::new("Directory to back up (blank to skip):")
        .with_help_message("Leave blank to skip")
        .prompt());
    let directory = if dir_raw.is_empty() { None } else { Some(dir_raw) };

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::add_task(
        &kdl,
        TaskSpec { name: name.clone(), repo: repo.clone(), directory: directory.clone(), exclude_file: None },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;
    ensure_path(&repo);
    if let Some(dir) = &directory {
        ensure_path(dir);
    }
    Ok(format!("Added task '{name}'."))
}

fn restic_url(url: &str) -> String {
    if url.starts_with("rustfs:") {
        url.replacen("rustfs:", "s3:", 1)
    } else {
        url.to_string()
    }
}

pub(crate) fn repo_needs_init(output: &str) -> bool {
    output.contains("Is there a repository")
}

fn offer_repo_init(url: &str, profile: &str, secrets_path: &str) -> Result<String, String> {
    // Decrypt secrets to get credentials for this profile
    let decrypted = match crate::backup_config::decrypt_sops_file(secrets_path) {
        Ok(d) => d,
        Err(_) => return Ok(String::new()), // No sops key available — skip silently
    };
    let secrets = match crate::backup_config::parse_secrets(&decrypted) {
        Ok(s) => s,
        Err(_) => return Ok(String::new()),
    };

    env::set_var("RESTIC_PASSWORD", &secrets.restic_password);

    if let Some(creds) = secrets.credentials.get(profile) {
        for (k, v) in creds {
            env::set_var(k, v);
        }
    }

    let rurl = restic_url(url);

    // Check if a repo exists at this location
    let output = match process::Command::new("restic")
        .args(["-r", &rurl, "snapshots", "--no-lock", "--no-cache"])
        .output()
    {
        Ok(o) => o,
        Err(_) => return Ok(String::new()), // restic not available — skip silently
    };

    if output.status.success() {
        return Ok(String::new()); // Repo already exists
    }

    let combined = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    if !repo_needs_init(&combined) {
        return Ok(String::new()); // Different error (bad creds, network) — skip
    }

    // No repo found — offer to initialize
    let init = match inquire::Confirm::new(&format!(
        "No repository found at '{url}'. Initialize it now?"
    ))
    .with_default(true)
    .prompt()
    {
        Ok(v) => v,
        Err(_) => return Ok(String::new()), // Cancelled — skip silently
    };

    if !init {
        return Ok(String::new());
    }

    println!("\nInitializing repository at {rurl}...");
    let status = process::Command::new("restic")
        .args(["-r", &rurl, "init"])
        .status()
        .map_err(|e| format!("restic init failed: {e}"))?;

    if status.success() {
        Ok("Repository initialized.".to_string())
    } else {
        Err(format!("restic init failed (exit {}).", status.code().unwrap_or(-1)))
    }
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
        RemoteSpec { url: url.clone(), credentials: credentials.clone() },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;

    let init_msg = offer_repo_init(&url, &credentials, &secrets_path)?;
    let msg = if init_msg.is_empty() {
        format!("Added remote '{url}' to task '{task_name}'.")
    } else {
        format!("Added remote '{url}' to task '{task_name}'. {init_msg}")
    };
    Ok(msg)
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

    let (status, restic_output) = if url.starts_with("b2:") {
        let path = url.trim_start_matches("b2:").trim_start_matches('/');
        let s = process::Command::new("b2")
            .args(["ls", path])
            .status()
            .map_err(|e| format!("could not run b2: {e}"))?;
        (s, String::new())
    } else {
        let rurl = restic_url(&url);
        let out = process::Command::new("restic")
            .args(["-r", &rurl, "snapshots", "--no-lock", "--no-cache"])
            .output()
            .map_err(|e| format!("could not run restic: {e}"))?;
        print!("{}", String::from_utf8_lossy(&out.stdout));
        print!("{}", String::from_utf8_lossy(&out.stderr));
        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        (out.status, combined)
    };

    println!();
    let mut summary = if status.success() {
        "Remote test passed.".to_string()
    } else {
        format!("Remote test failed (exit {}).", status.code().unwrap_or(-1))
    };
    println!("{summary}");

    if !status.success() && repo_needs_init(&restic_output) {
        let secrets_path = crate::config::secrets_path_from();
        match offer_repo_init(&url, &profile, &secrets_path) {
            Ok(msg) if !msg.is_empty() => {
                summary = msg.clone();
                println!("{msg}");
            }
            Err(e) => println!("Init failed: {e}"),
            _ => {}
        }
    }

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

    let kdl = std::fs::read_to_string(&app.config_path).map_err(|e| e.to_string())?;
    let new_kdl = crate::config_editor::edit_remote(
        &kdl,
        &task_name,
        &old_url,
        RemoteSpec { url: url.clone(), credentials: credentials.clone() },
    )?;
    std::fs::write(&app.config_path, new_kdl).map_err(|e| e.to_string())?;

    let init_msg = offer_repo_init(&url, &credentials, &secrets_path)?;
    let msg = if init_msg.is_empty() {
        "Updated remote.".to_string()
    } else {
        format!("Updated remote. {init_msg}")
    };
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_needs_init_detects_missing_repo() {
        let output = "Fatal: unable to open config file: Stat: The specified key does not exist.\nIs there a repository at the following location?\ns3:https://example.com/bucket\n";
        assert!(repo_needs_init(output));
    }

    #[test]
    fn repo_needs_init_false_for_other_errors() {
        let output = "Fatal: unable to open config file: Forbidden\n";
        assert!(!repo_needs_init(output));
    }

    #[test]
    fn repo_needs_init_false_for_success() {
        let output = "snapshot abc123 ...\n";
        assert!(!repo_needs_init(output));
    }
}
