use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::{env, process};

use super::app::{
    App, Pane, TaskEntry, FIELD_EXCLUDE, FIELD_INCLUDE, FIELD_NAMES, FIELD_PASSWORD_PROFILE, FIELD_SCHEDULE,
};
use super::credentials;
use crate::config_editor::{self as editor, EditTaskSpec, RemoteSpec, TaskSpec};
use crate::ensure_path;
use crate::{folders, templates};

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

const START_PHOTOS: &str = "Photos & video (big files: checked weekly, runs nightly)";
const START_FOLDERS: &str = "Other folders (a plain backup you tune yourself)";
const START_GROUP: &str = "Group (only runs other tasks, e.g. back up everything)";

/// Asks where the task's backup repository goes, suggesting a spot beside the
/// existing ones. `Ok(None)` if cancelled.
fn ask_repo(app: &App, name: &str) -> Result<Option<String>, String> {
    let existing: Vec<&str> = app.tasks.iter().filter_map(|t| t.repo.as_deref()).collect();
    let suggestion = editor::suggest_repo_path(&existing, name);
    Ok(Some(ask!(
        inquire::Text::new("Where should its backup repository be stored?")
            .with_default(&suggestion)
            .with_help_message("A local copy of the backups, created for you; remotes are off-site copies of it")
            .prompt(),
        Ok(None)
    )))
}

const HOW_BOTH: &str = "Both: the library and an export (little extra space; recommended)";
const HOW_LIBRARY: &str = "The library as it is (restores into Photos on a Mac)";
const HOW_EXPORT: &str = "An export only: plain files with real names (readable on any system)";

/// The plan for a Photos & video task. When a Photos library is found: both the
/// library and an export of it, either alone, or (with no library) just folders.
/// `Ok(None)` if cancelled.
fn ask_photos_plan() -> Result<Option<templates::Plan>, String> {
    let Some(library) = templates::default_photos_library() else {
        return Ok(ask_photo_folders()?.map(templates::photos_plan));
    };
    let how = ask!(
        inquire::Select::new("How should your Photos library be backed up?", vec![HOW_BOTH, HOW_LIBRARY, HOW_EXPORT])
            .with_help_message("\u{2191}\u{2193} to move, Enter to select. The README compares them")
            .prompt(),
        Ok(None)
    );

    match how {
        HOW_EXPORT => ask_export_plan(&library),
        HOW_BOTH => {
            let Some(folders) = ask_photo_folders()? else { return Ok(None) };
            let Some(dest) = ask_export_dir()? else { return Ok(None) };
            Ok(Some(templates::photos_both_plan(&library, &dest, folders)))
        }
        _ => Ok(ask_photo_folders()?.map(templates::photos_plan)),
    }
}

/// Where the export goes, after saying what it needs. `Ok(None)` if cancelled.
fn ask_export_dir() -> Result<Option<String>, String> {
    if !templates::tool_on_path("osxphotos") {
        println!("\nosxphotos isn't installed here yet. Install it with: {}", templates::OSXPHOTOS_INSTALL);
        println!("The backup says so if it is missing.\n");
    }
    Ok(Some(ask!(
        folders::ask_new_folder(
            "Where should the export go?",
            "osxphotos fills it and keeps it current. On APFS its files are clones, so it takes little extra disk space",
            templates::EXPORT_DEFAULT_DIR,
        ),
        Ok(None)
    )))
}

/// The plan for backing up an export of `library` alone. `Ok(None)` if cancelled.
fn ask_export_plan(library: &str) -> Result<Option<templates::Plan>, String> {
    let Some(dest) = ask_export_dir()? else { return Ok(None) };
    let others = ask!(folders::ask_folders("Any other folder to back up too? (blank for none)", None), Ok(None));
    Ok(Some(templates::photos_export_plan(library, &dest, others)))
}

/// The folders for a Photos & video task: the Pictures folder to start with, and
/// the Photos library too when it isn't already inside a chosen folder.
/// `Ok(None)` if cancelled.
fn ask_photo_folders() -> Result<Option<Vec<String>>, String> {
    let pictures = "$HOME/Pictures";
    let default = folders::folder_exists(pictures).then_some(pictures);
    let mut chosen = ask!(
        folders::ask_folders("Which folder holds your photos and videos?", default),
        Ok(None)
    );

    if let Some(library) = templates::default_photos_library() {
        if !chosen.iter().any(|f| templates::is_within(&library, f)) {
            let add = ask!(
                inquire::Confirm::new("Also back up your Photos library?")
                    .with_default(true)
                    .with_help_message(&format!("{library}: Photos must be closed while it runs, and vivo checks"))
                    .prompt(),
                Ok(None)
            );
            if add {
                chosen.push(library);
            }
        }
    }
    Ok(Some(chosen))
}

/// Says, on the plain terminal the prompts run on, which of `folders` vivo can't
/// read, and waits: the status line is gone in two seconds, and a backup that
/// can't read its folders is empty. Prints nothing if they are all fine.
fn show_source_problems(folders: &[String]) {
    let lines = crate::sources::report(&crate::sources::check_folders(folders));
    if lines.is_empty() {
        return;
    }
    println!("\nHeads up: this backup won't capture everything yet.");
    for line in &lines {
        println!("  - {line}");
    }
    println!("\nPress Enter to continue...");
    std::io::stdin().read_line(&mut String::new()).ok();
}

/// What a new task was set up with, for the status line.
fn describe_new_task(name: &str, plan: &templates::Plan) -> String {
    let mut parts = vec![match (&plan.output_dir, plan.folders.len()) {
        (Some(dir), n) if n > 1 => format!("{n} folders, including an export of your Photos library ({dir})"),
        (Some(dir), _) => format!("exports your Photos library to {dir}"),
        (None, 0) => "no folders yet (press o to add some)".to_string(),
        (None, 1) => "1 folder".to_string(),
        (None, n) => format!("{n} folders"),
    }];
    if let Some(schedule) = &plan.schedule {
        parts.push(format!("scheduled {schedule}: run `vivo schedule apply` to install it"));
    }
    if plan.soft_photos_guard() {
        parts.push("with Photos open, the library is left out of that run and the rest still backs up".to_string());
    } else if plan.guards_photos() {
        parts.push("Photos library found: it won't be backed up while Photos is open".to_string());
    }
    format!("Added task '{name}': {}.", parts.join("; "))
}

fn add_task_prompt(app: &App) -> Result<String, String> {
    let name = ask!(inquire::Text::new("Task name:").prompt());
    if name.trim().is_empty() {
        return Err("task name cannot be empty".to_string());
    }

    // The default help ("type to filter") invites typing a description, which
    // filters every option away, so it is replaced.
    let start = ask!(inquire::Select::new("Start from:", vec![START_PHOTOS, START_FOLDERS, START_GROUP])
        .with_help_message("\u{2191}\u{2193} to move, Enter to select")
        .prompt());

    if start == START_GROUP {
        editor::update_file(&app.config_path, |kdl| {
            editor::add_task(kdl, TaskSpec { name: name.clone(), repo: None, directory: None, exclude_file: None })
        })?;
        return Ok(format!("Added task '{name}'."));
    }

    let plan = if start == START_PHOTOS {
        let Some(plan) = ask_photos_plan()? else { return Ok("Cancelled.".to_string()) };
        plan
    } else {
        templates::custom_plan(ask!(folders::ask_folders("Which folder should it back up?", None)))
    };
    let Some(repo) = ask_repo(app, &name)? else { return Ok("Cancelled.".to_string()) };

    editor::update_file(&app.config_path, |kdl| templates::apply_plan(kdl, &name, &repo, &plan))?;
    // The repository, and a folder an export fills, are ours to create. A folder
    // being backed up that doesn't exist is a mistake to report, not to fix.
    ensure_path(&repo);
    if let Some(dir) = &plan.output_dir {
        ensure_path(dir);
    }
    show_source_problems(&plan.folders);
    Ok(describe_new_task(&name, &plan))
}

/// Which list of a backup the list editor works on.
#[derive(Clone, Copy)]
enum ListKind {
    /// Folders beyond `directory`: the `include` nodes.
    Include,
    /// Patterns left out: the `exclude` nodes.
    Exclude,
}

impl ListKind {
    fn node(self) -> &'static str {
        match self {
            ListKind::Include => "include",
            ListKind::Exclude => "exclude",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            ListKind::Include => "folder",
            ListKind::Exclude => "pattern",
        }
    }

    fn items(self, backup: &crate::backup_config::backup::Backup) -> Vec<String> {
        match self {
            ListKind::Include => backup.include().to_vec(),
            ListKind::Exclude => backup.exclude().to_vec(),
        }
    }
}

fn is_cancel(e: &inquire::InquireError) -> bool {
    matches!(e, inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted)
}

/// The current items of a task's list, read from the file (the list changes as
/// the user edits it, so the screen's copy would be stale).
fn read_list(config_path: &str, task_name: &str, kind: ListKind) -> Result<Vec<String>, String> {
    let config = crate::BackupConfig::from_file(config_path)?;
    let backup = config
        .tasks
        .iter()
        .find(|t| t.name == task_name)
        .and_then(|t| t.backup())
        .ok_or_else(|| format!("task '{task_name}' has no backup"))?;
    Ok(kind.items(backup))
}

/// Edits the folders ("Also back up") or exclude patterns of a task: pick an
/// entry to remove it, or add one. Every change is saved at once; Esc is done.
fn edit_list_prompt(app: &App, task: &TaskEntry, kind: ListKind) -> Result<String, String> {
    if task.repo.is_none() {
        return Err("set a repo path first".to_string());
    }
    let (add_label, done_label) = (format!("+ Add a {}", kind.noun()), "Done".to_string());
    let mut changed = false;

    loop {
        let items = read_list(&app.config_path, &task.name, kind)?;
        let mut options = items.clone();
        options.extend([add_label.clone(), done_label.clone()]);

        let message = match items.len() {
            0 => format!("No {}s yet.", kind.noun()),
            n => format!("{n} {}{}. Pick one to remove it.", kind.noun(), if n == 1 { "" } else { "s" }),
        };
        let picked = match inquire::Select::new(&message, options)
            .with_help_message("\u{2191}\u{2193} to move, Enter to select, Esc when done")
            .with_page_size(10)
            .prompt()
        {
            Ok(picked) => picked,
            Err(e) if is_cancel(&e) => break,
            Err(e) => return Err(e.to_string()),
        };

        if picked == done_label {
            break;
        }
        if picked == add_label {
            let added = match kind {
                ListKind::Include => folders::ask_folder(
                    "Folder to back up:",
                    "Tab completes. Its parent is often better than listing every subfolder",
                    None,
                ),
                ListKind::Exclude => inquire::Text::new("Pattern to exclude:")
                    .with_help_message("e.g. **/*.tmp, **/Thumbnails, or a path")
                    .prompt()
                    .map(|p| opt(p.trim().to_string())),
            };
            match added {
                Ok(Some(value)) => {
                    editor::update_file(&app.config_path, |kdl| editor::add_backup_node(kdl, &task.name, kind.node(), &value))?;
                    changed = true;
                }
                Ok(None) => {}
                Err(e) if is_cancel(&e) => {}
                Err(e) => return Err(e.to_string()),
            }
            continue;
        }

        match inquire::Confirm::new(&format!("Remove '{picked}'?")).with_default(false).prompt() {
            Ok(true) => {
                editor::update_file(&app.config_path, |kdl| editor::remove_backup_node(kdl, &task.name, kind.node(), &picked))?;
                changed = true;
            }
            Ok(false) => {}
            Err(e) if is_cancel(&e) => {}
            Err(e) => return Err(e.to_string()),
        }
    }

    Ok(if changed { format!("Updated the {}s.", kind.noun()) } else { "No changes.".to_string() })
}

/// Tests a just-saved remote and returns a one-line status for the footer.
/// Never fails the add/edit itself — the remote is already written.
fn offer_remote_test(url: &str, profile: &str, secrets_path: &str, password_profile: Option<&str>) -> String {
    match crate::remote_setup::test_remote(url, profile, secrets_path, password_profile, true) {
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

    let test_msg = offer_remote_test(&url, &credentials, &secrets_path, crate::remote_setup::password_profile_of(&app.config_path, &task_name).as_deref());
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

    let test_msg = offer_remote_test(&url, &credentials, &secrets_path, crate::remote_setup::password_profile_of(&app.config_path, &task_name).as_deref());
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

    let password_profile = selected_task_name(app)
        .ok()
        .and_then(|task| crate::remote_setup::password_profile_of(&app.config_path, &task));
    let result = crate::remote_setup::test_remote(&url, &profile, &secrets_path, password_profile.as_deref(), true);
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

/// Edits the schedule row. Only the config changes: installing it is
/// `vivo schedule apply`, and says so rather than touching the system from a TUI.
fn edit_schedule_prompt(app: &App, task: &TaskEntry) -> Result<String, String> {
    let current = task.schedule.as_deref().unwrap_or("");
    let answer = ask!(text("Schedule (daily HH:MM or weekly <day> HH:MM; blank = none):", current));
    let new = opt(answer);
    editor::update_file(&app.config_path, |kdl| editor::set_task_schedule(kdl, &task.name, new.as_deref()))?;
    Ok(match new {
        Some(w) => format!("Schedule set to '{w}'. Run `vivo schedule apply` to install it."),
        None => "Schedule removed. Run `vivo schedule apply` to uninstall it.".to_string(),
    })
}

/// Edits the password-profile row. The profile only picks which secret to use;
/// it must match what the repository was created with.
fn edit_password_profile_prompt(app: &App, task: &TaskEntry) -> Result<String, String> {
    if task.repo.is_none() {
        return Err("set a repo path first".to_string());
    }
    let current = task.password_profile.as_deref().unwrap_or("");
    let new = opt(ask!(text("Password profile (name under `passwords:` in secrets; blank = global):", current)));
    editor::update_file(&app.config_path, |kdl| editor::set_backup_password_profile(kdl, &task.name, new.as_deref()))?;
    Ok(match new {
        Some(p) => format!("Password profile '{p}' set. Add `passwords: {p}:` with `vivo secrets edit`; it must match the repo's existing password."),
        None => "Password profile removed; the global password is used.".to_string(),
    })
}

fn edit_field_prompt(app: &App) -> Result<String, String> {
    let task = app.tasks.get(app.selected_task).ok_or("no task selected")?;
    match app.selected_field {
        FIELD_INCLUDE => return edit_list_prompt(app, task, ListKind::Include),
        FIELD_EXCLUDE => return edit_list_prompt(app, task, ListKind::Exclude),
        FIELD_SCHEDULE => return edit_schedule_prompt(app, task),
        FIELD_PASSWORD_PROFILE => return edit_password_profile_prompt(app, task),
        _ => {}
    }
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
