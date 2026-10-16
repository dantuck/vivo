//! `vivo config` and the config-file templates.

use std::path::Path;
use std::fs;

use vivo::config_editor::update_file;
use vivo::{ensure_path, expand_env_vars};

use crate::util::{cancel_setup, create_with_template, interactive, open_in_editor};

pub const DEFAULT_EXCLUDES_PATH: &str = "$HOME/.config/vivo/excludes";

pub fn config_template() -> String {
    format!(
        r#"default-task "backup"

tasks {{
    task "backup" {{
        description "Main backup task"
        backup {{
            repo "$HOME/.local/share/restic/main"
            directory "$HOME"
            exclude-file "{DEFAULT_EXCLUDES_PATH}"
            retention {{
                daily 7
                weekly 5
                monthly 12
                yearly 2
            }}
            // Add remotes here, e.g.:
            // remote "rustfs:http://your-nas:9000/backup" {{
            //     credentials "rustfs"
            // }}
            // remote "s3:https://s3.amazonaws.com/my-bucket" {{
            //     credentials "aws"
            // }}
            // remote "b2:my-bucket:restic/main" {{
            //     credentials "b2"
            // }}
        }}
    }}
}}
"#
    )
}

/// Minimal valid config written before the interactive wizard fills it in.
/// `default-task` must be present (the config parser requires it) but is set for real
/// once a task exists.
pub const SKELETON_CONFIG_TEMPLATE: &str = "default-task \"\"\n\ntasks {\n}\n";

pub const EXCLUDES_TEMPLATE: &str = "\
.DS_Store
.Trash
.cache
node_modules
target
.venv
__pycache__
*.tmp
";

/// Creates the default excludes file referenced by the config template's
/// `exclude-file` entry (DEFAULT_EXCLUDES_PATH), if it doesn't already exist.
pub fn ensure_default_excludes_file() -> bool {
    let path = expand_env_vars(DEFAULT_EXCLUDES_PATH);
    match create_with_template(&path, EXCLUDES_TEMPLATE) {
        Ok(true) => {
            println!("Created excludes file: {path}");
            true
        }
        Ok(false) => true,
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

/// Guides the user through creating their first backup task interactively:
/// what to back up and where the restic repo lives. Remotes come later in
/// `cmd_init`, once the secrets exist, so they can get credentials and a
/// connection test. Falls back to the static template when stdin isn't a
/// TTY. Returns true on success (including "already existed").
pub fn cmd_config_init_interactive(config_path: &str) -> bool {
    if Path::new(config_path).exists() {
        println!("Config already exists: {config_path}");
        return true;
    }

    if !interactive() {
        return cmd_config_init(config_path);
    }

    println!("Let's set up your first backup.\n");

    let home = expand_env_vars("$HOME");
    let directory = loop {
        let input = inquire::Text::new("Which directory would you like to back up?")
            .with_default("$HOME")
            .with_help_message("Add more later with `vivo task add`")
            .prompt()
            .unwrap_or_else(|_| cancel_setup());
        let dir = input.trim();
        let expanded = expand_env_vars(dir);
        if expanded == home {
            println!(
                "  Backing up your whole home directory; common junk is skipped via {DEFAULT_EXCLUDES_PATH}."
            );
        } else if !Path::new(&expanded).is_dir() {
            let keep = inquire::Confirm::new(&format!("'{expanded}' does not exist. Use it anyway?"))
                .with_default(false)
                .prompt()
                .unwrap_or_else(|_| cancel_setup());
            if !keep {
                continue;
            }
        }
        break dir.to_string();
    };

    let repo = inquire::Text::new("Where should the restic repository be stored?")
        .with_default("$HOME/.local/share/restic/main")
        .with_help_message("A local copy of your backups; remotes (next) are off-site copies of it")
        .prompt()
        .unwrap_or_else(|_| cancel_setup());

    if let Err(e) = create_with_template(config_path, SKELETON_CONFIG_TEMPLATE) {
        eprintln!("error: {e}");
        return false;
    }

    let task_name = "backup";
    let written = update_file(config_path, |kdl| {
        let kdl = vivo::add_task(
            kdl,
            vivo::TaskSpec {
                name: task_name.to_string(),
                repo: Some(repo.clone()),
                directory: Some(directory.clone()),
                exclude_file: Some(DEFAULT_EXCLUDES_PATH.to_string()),
            },
        )?;
        vivo::set_default_task(&kdl, task_name)
    });
    if let Err(e) = written {
        eprintln!("error: {e}");
        return false;
    }

    ensure_path(&repo);
    ensure_path(&directory);
    println!("\nCreated config: {config_path}");
    ensure_default_excludes_file()
}

/// Creates the config file (and its default excludes file) if missing.
/// Returns true on success (including "already existed").
pub fn cmd_config_init(config_path: &str) -> bool {
    match create_with_template(config_path, &config_template()) {
        Ok(true) => {
            println!("Created config: {config_path}");
            ensure_default_excludes_file()
        }
        Ok(false) => {
            println!("Config already exists: {config_path}");
            true
        }
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

pub fn cmd_config_edit(config_path: &str) {
    match create_with_template(config_path, &config_template()) {
        Ok(true) => {
            ensure_default_excludes_file();
        }
        Ok(false) => {}
        Err(e) => {
            eprintln!("error: {e}");
            return;
        }
    }
    open_in_editor(config_path);
}

pub fn cmd_config_show(config_path: &str) {
    match fs::read_to_string(config_path) {
        Ok(contents) => print!("{contents}"),
        Err(e) => eprintln!("error: could not read config '{config_path}': {e}"),
    }
}
