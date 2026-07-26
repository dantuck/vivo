use is_terminal::IsTerminal;
use log::debug;
use std::{env, fs, path::Path, process};
use vivo::{
    age_public_key, build_cli, config_path_from, decrypt_sops_file, expand_env_vars,
    import_b2_credentials, secrets_path_from, BackupConfig, VivoConfig,
};

fn ensure_path(path: &str) {
    let expanded = expand_env_vars(path);
    if !expanded.is_empty() {
        let _ = fs::create_dir_all(&expanded);
    }
}

const DEFAULT_EXCLUDES_PATH: &str = "$HOME/.config/vivo/excludes";

fn config_template() -> String {
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
/// `default-task` must be present (knuffel requires it) but is set for real
/// once a task exists.
const SKELETON_CONFIG_TEMPLATE: &str = "default-task \"\"\n\ntasks {\n}\n";

const SECRETS_TEMPLATE: &str = "restic_password: \"change-me\"\ncredentials: {}\n";

const EXCLUDES_TEMPLATE: &str = "\
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
fn ensure_default_excludes_file() -> bool {
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

fn open_in_editor(path: &str) {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    if let Err(e) = process::Command::new(&editor).arg(path).status() {
        eprintln!("error: could not open editor '{editor}': {e}");
    }
}

/// Opens `path` in `sops`. Returns true if `sops` ran and exited successfully.
fn open_with_sops(path: &str) -> bool {
    match process::Command::new("sops")
        .env("SOPS_AGE_KEY_FILE", vivo::age_keys_path())
        .arg(path)
        .status()
    {
        Ok(status) if status.success() => true,
        Ok(status) => {
            eprintln!("error: sops exited with {status}");
            false
        }
        Err(e) => {
            eprintln!("error: could not run sops: {e}");
            false
        }
    }
}

fn ensure_parent_dirs(path: &str) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("could not create directory: {e}"))?;
    }
    Ok(())
}

/// Creates parent dirs and writes `contents` to `path` if it does not already exist.
/// Returns Ok(true) on creation, Ok(false) if already exists, Err on I/O failure.
fn create_with_template(path: &str, contents: &str) -> Result<bool, String> {
    if Path::new(path).exists() {
        return Ok(false);
    }
    ensure_parent_dirs(path)?;
    fs::write(path, contents).map_err(|e| format!("could not write file: {e}"))?;
    Ok(true)
}

/// Guides the user through creating their first backup task interactively:
/// what to back up, where the restic repo lives, and (optionally) a remote
/// sync destination. Falls back to the static template when stdin isn't a
/// TTY. Returns true on success (including "already existed").
fn cmd_config_init_interactive(config_path: &str) -> bool {
    if Path::new(config_path).exists() {
        println!("Config already exists: {config_path}");
        return true;
    }

    if !std::io::stdin().is_terminal() {
        return cmd_config_init(config_path);
    }

    println!("Let's set up your first backup.\n");

    let directory = inquire::Text::new("Which directory would you like to back up?")
        .with_default("$HOME")
        .prompt()
        .unwrap_or_else(|_| process::exit(0));

    let repo = inquire::Text::new("Where should the restic repository be stored?")
        .with_default("$HOME/.local/share/restic/main")
        .prompt()
        .unwrap_or_else(|_| process::exit(0));

    let add_remote = inquire::Confirm::new("Add a remote sync destination now?")
        .with_default(false)
        .with_help_message("You can always add one later with `vivo remote add`")
        .prompt()
        .unwrap_or(false);

    let remote = if add_remote {
        let remote_type = inquire::Select::new(
            "Remote type:",
            vec!["S3-compatible (s3:)", "Backblaze B2 (b2:)", "RustFS/custom (rustfs:)"],
        )
        .prompt()
        .unwrap_or_else(|_| process::exit(0));

        let (default_url, default_creds) = match remote_type {
            "S3-compatible (s3:)" => ("s3:https://s3.amazonaws.com/my-bucket", "aws"),
            "Backblaze B2 (b2:)" => ("b2:my-bucket:restic/main", "b2"),
            _ => ("rustfs:http://your-nas:9000/backup", "rustfs"),
        };

        let url = inquire::Text::new("Remote URL:")
            .with_default(default_url)
            .prompt()
            .unwrap_or_else(|_| process::exit(0));
        let credentials = inquire::Text::new("Credentials profile name:")
            .with_default(default_creds)
            .prompt()
            .unwrap_or_else(|_| process::exit(0));

        Some((url, credentials))
    } else {
        None
    };

    if let Err(e) = create_with_template(config_path, SKELETON_CONFIG_TEMPLATE) {
        eprintln!("error: {e}");
        return false;
    }

    let kdl = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: could not read config '{config_path}': {e}");
            return false;
        }
    };

    let task_name = "backup";
    let build = || -> Result<String, String> {
        let kdl = vivo::add_task(
            &kdl,
            vivo::TaskSpec {
                name: task_name.to_string(),
                repo: Some(repo.clone()),
                directory: Some(directory.clone()),
                exclude_file: Some(DEFAULT_EXCLUDES_PATH.to_string()),
            },
        )?;
        let kdl = if let Some((url, credentials)) = &remote {
            vivo::add_remote(
                &kdl,
                task_name,
                vivo::RemoteSpec {
                    url: url.clone(),
                    credentials: credentials.clone(),
                    mc_max_workers: None,
                    mc_limit_upload: None,
                },
            )?
        } else {
            kdl
        };
        vivo::set_default_task(&kdl, task_name)
    };

    match build() {
        Ok(new_kdl) => {
            if let Err(e) = fs::write(config_path, new_kdl) {
                eprintln!("error: could not write config: {e}");
                return false;
            }
        }
        Err(e) => {
            eprintln!("error: {e}");
            return false;
        }
    }

    ensure_path(&repo);
    ensure_path(&directory);
    println!("\nCreated config: {config_path}");
    ensure_default_excludes_file()
}

/// Creates the config file (and its default excludes file) if missing.
/// Returns true on success (including "already existed").
fn cmd_config_init(config_path: &str) -> bool {
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

fn cmd_config_edit(config_path: &str) {
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

fn cmd_config_show(config_path: &str) {
    match fs::read_to_string(config_path) {
        Ok(contents) => print!("{contents}"),
        Err(e) => eprintln!("error: could not read config '{config_path}': {e}"),
    }
}

/// Creates the encrypted secrets file if missing. Returns true on success
/// (including "already existed").
fn cmd_secrets_init(secrets_path: &str) -> bool {
    if Path::new(secrets_path).exists() {
        println!("Secrets file already exists: {secrets_path}");
        return true;
    }
    if let Err(e) = ensure_parent_dirs(secrets_path) {
        eprintln!("error: {e}");
        return false;
    }

    let Some(recipient) = age_public_key() else {
        eprintln!("error: no age key found — run: age-keygen -o ~/.config/sops/age/keys.txt");
        return false;
    };

    let tmp_path = env::temp_dir().join("vivo-secrets-init.tmp");
    if let Err(e) = fs::write(&tmp_path, SECRETS_TEMPLATE) {
        eprintln!("error: could not write secrets template: {e}");
        return false;
    }

    let output = process::Command::new("sops")
        .arg("-e")
        .arg("--age")
        .arg(&recipient)
        .arg("--output")
        .arg(secrets_path)
        .arg(&tmp_path)
        .output();
    let _ = fs::remove_file(&tmp_path);

    match output {
        Ok(o) if o.status.success() => {
            println!("Created encrypted secrets: {secrets_path}");
            println!("Run `vivo secrets edit` to set your restic_password and credentials.");
            true
        }
        Ok(o) => {
            eprintln!("error: sops encryption failed: {}", String::from_utf8_lossy(&o.stderr));
            false
        }
        Err(e) => {
            eprintln!("error: could not run sops: {e}");
            false
        }
    }
}

fn cmd_secrets_import_b2(secrets_path: &str) {
    if !Path::new(secrets_path).exists() {
        eprintln!("Secrets file not found. Run `vivo secrets init` first.");
        return;
    }
    match import_b2_credentials(secrets_path) {
        Ok(_) => println!("B2 credentials imported successfully."),
        Err(e) => eprintln!("error: {e}"),
    }
}

fn cmd_secrets_import_s3(secrets_path: &str, matches: &clap::ArgMatches) {
    if !Path::new(secrets_path).exists() {
        eprintln!("Secrets file not found. Run `vivo secrets init` first.");
        return;
    }

    let interactive = std::io::stdin().is_terminal();

    let key_id = require_or_prompt(matches, "key-id", "Access key ID (AWS_ACCESS_KEY_ID):", interactive);
    let key = require_or_prompt(matches, "key", "Secret access key (AWS_SECRET_ACCESS_KEY):", interactive);
    let profile = matches
        .get_one::<String>("profile")
        .cloned()
        .unwrap_or_else(|| {
            if interactive {
                inquire::Text::new("Profile name:")
                    .with_default("s3")
                    .prompt()
                    .unwrap_or_else(|_| process::exit(0))
            } else {
                "s3".to_string()
            }
        });

    match vivo::update_s3_in_secrets(secrets_path, &profile, &key_id, &key) {
        Ok(()) => println!("S3 credentials saved to profile '{profile}'."),
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn cmd_secrets_edit(secrets_path: &str) {
    if !Path::new(secrets_path).exists() {
        eprintln!("Secrets file not found. Run `vivo secrets init` first.");
        return;
    }
    open_with_sops(secrets_path);
}

fn cmd_secrets_show(secrets_path: &str) {
    match decrypt_sops_file(secrets_path) {
        Ok(contents) => print!("{contents}"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Runs the full guided setup: prerequisite checks, then config/secrets
/// creation. Returns true only if every step succeeded.
fn cmd_init(config_path: &str, secrets_path: &str) -> bool {
    vivo::ui::print_banner();
    println!("Checking prerequisites...");

    let checks = [
        vivo::doctor::check_tool_present("restic", "version", "install from https://restic.net"),
        vivo::doctor::check_tool_present("sops", "--version", "install from https://github.com/getsops/sops"),
        vivo::doctor::check_age_key(),
    ];

    let mut ok = true;
    for r in &checks {
        vivo::doctor::print_result(r);
        if matches!(r.status, vivo::doctor::CheckStatus::Fail) {
            ok = false;
        }
    }

    if !ok {
        eprintln!("\nInstall missing prerequisites and re-run `vivo init`.");
        return false;
    }

    // FUSE is only needed for `vivo mount`, so it doesn't block init — but
    // offer to install it now rather than just telling the user about it later.
    let fuse = vivo::doctor::check_fuse();
    vivo::doctor::print_result(&fuse);
    if matches!(fuse.status, vivo::doctor::CheckStatus::Fail) {
        println!("  (only needed for `vivo mount`)");
        match vivo::doctor::fix_fuse() {
            Ok(true) => {
                let fuse = vivo::doctor::check_fuse();
                vivo::doctor::print_result(&fuse);
            }
            Ok(false) => {}
            Err(e) => eprintln!("error: {e}"),
        }
    }

    println!();
    let config_ok = cmd_config_init_interactive(config_path);
    let secrets_ok = cmd_secrets_init(secrets_path);
    if !config_ok || !secrets_ok {
        return false;
    }

    println!();
    println!("Setup complete. Next steps:");
    println!("  1. Edit your backup config:  vivo config edit");
    println!("  2. Set your restic password: vivo secrets edit");
    println!("  3. Run a dry-run backup:     vivo --dry-run");
    true
}

/// Reacts to a `LoadConfigError`: guides the user through a fix when stdin
/// is a TTY, otherwise prints an actionable error. Exits the process on
/// unrecoverable or failed-repair cases so callers can just `return` after
/// calling this (the process either already exited, or repair guidance ran).
fn handle_load_error(err: &vivo::LoadConfigError, config_path: &str, secrets_path: &str) {
    let interactive = std::io::stdin().is_terminal();
    match err {
        vivo::LoadConfigError::ConfigMissing { path } => {
            if interactive {
                println!("No config found at '{path}' — let's set one up.\n");
                if !cmd_init(config_path, secrets_path) {
                    process::exit(1);
                }
            } else {
                eprintln!("error: {err}");
                eprintln!("Run `vivo init` to get started.");
                process::exit(1);
            }
        }
        vivo::LoadConfigError::SecretsMissing { path } => {
            if interactive {
                println!("No secrets file found at '{path}' — let's create one.\n");
                if cmd_secrets_init(secrets_path) {
                    println!("\nSet your restic password, then re-run vivo:");
                    println!("  vivo secrets edit");
                } else {
                    process::exit(1);
                }
            } else {
                eprintln!("error: {err}");
                eprintln!("Run `vivo secrets init` to get started.");
                process::exit(1);
            }
        }
        vivo::LoadConfigError::SecretsUndecryptable { path, detail } => {
            if interactive {
                println!("Secrets file at '{path}' could not be decrypted:");
                println!("  {detail}");
                println!("\nOpening it in `sops` so you can fix it — set restic_password and save.\n");
                if !open_with_sops(path) {
                    eprintln!(
                        "error: secrets are still not fixed. Run `vivo secrets edit` to try again."
                    );
                    process::exit(1);
                }
            } else {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        vivo::LoadConfigError::Other(msg) => eprintln!("error: {msg}"),
    }
}

/// Checks that config and secrets are present and decryptable, guiding the
/// user through fixes if stdin is a TTY. Returns true if it's safe to
/// proceed to the caller's real work.
fn ensure_onboarded(config_path: &str, secrets_path: &str) -> bool {
    if !Path::new(config_path).exists() {
        handle_load_error(
            &vivo::LoadConfigError::ConfigMissing { path: config_path.to_string() },
            config_path,
            secrets_path,
        );
        return false;
    }
    if !Path::new(secrets_path).exists() {
        handle_load_error(
            &vivo::LoadConfigError::SecretsMissing { path: secrets_path.to_string() },
            config_path,
            secrets_path,
        );
        return false;
    }
    if let Err(e) = decrypt_sops_file(secrets_path) {
        handle_load_error(
            &vivo::LoadConfigError::SecretsUndecryptable {
                path: secrets_path.to_string(),
                detail: e.trim().to_string(),
            },
            config_path,
            secrets_path,
        );
        return false;
    }
    true
}

fn cmd_doctor(config_path: &str, secrets_path: &str, fix: bool) {
    let exit_code = vivo::doctor::run_doctor(config_path, secrets_path, fix);
    std::process::exit(exit_code);
}

fn cmd_update(dry_run: bool) {
    if let Err(e) = vivo::update::apply_update(dry_run) {
        let msg = e.to_string();
        if msg.contains("Permission denied") || msg.contains("Access is denied") {
            eprintln!("error: cannot replace binary — try: sudo vivo update");
        } else {
            eprintln!("error: update failed: {msg}");
        }
        std::process::exit(1);
    }
}

fn require_or_prompt(
    matches: &clap::ArgMatches,
    flag: &str,
    prompt: &str,
    interactive: bool,
) -> String {
    if let Some(v) = matches.get_one::<String>(flag) {
        return v.clone();
    }
    if !interactive {
        eprintln!("error: --{flag} is required in non-interactive mode");
        process::exit(1);
    }
    inquire::Text::new(prompt)
        .prompt()
        .unwrap_or_else(|_| process::exit(0))
}

fn get_or_prompt_opt(
    matches: &clap::ArgMatches,
    flag: &str,
    prompt: &str,
    interactive: bool,
) -> Option<String> {
    if let Some(v) = matches.get_one::<String>(flag) {
        return Some(v.clone());
    }
    if !interactive {
        return None;
    }
    let val = inquire::Text::new(prompt)
        .with_help_message("Leave blank to skip")
        .prompt()
        .unwrap_or_else(|_| process::exit(0));
    if val.is_empty() { None } else { Some(val) }
}

fn cmd_task_add(config_path: &str, matches: &clap::ArgMatches) {
    let interactive = std::io::stdin().is_terminal();
    let name = require_or_prompt(matches, "name", "Task name:", interactive);
    let repo = require_or_prompt(
        matches,
        "repo",
        "Restic repo path (e.g. $HOME/.local/share/restic/main):",
        interactive,
    );
    let directory = get_or_prompt_opt(matches, "dir", "Directory to back up:", interactive);
    let exclude_file =
        get_or_prompt_opt(matches, "exclude-file", "Exclude file path:", interactive);

    let kdl = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: could not read config '{config_path}': {e}");
            process::exit(1);
        }
    };

    match vivo::add_task(&kdl, vivo::TaskSpec { name: name.clone(), repo: Some(repo.clone()), directory: directory.clone(), exclude_file }) {
        Ok(new_kdl) => {
            if let Err(e) = fs::write(config_path, new_kdl) {
                eprintln!("error: could not write config: {e}");
                process::exit(1);
            }
            ensure_path(&repo);
            if let Some(dir) = &directory {
                ensure_path(dir);
            }
            println!("Added task '{name}'.");
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn cmd_task_list(config_path: &str) {
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: could not read config '{config_path}': {e}");
            process::exit(1);
        }
    };
    let config: vivo::BackupConfig = match knuffel::parse(config_path, &content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    for task in &config.tasks {
        if let Some(desc) = task.description() {
            println!("{} — {}", task.name, desc);
        } else {
            println!("{}", task.name);
        }
    }
}

fn cmd_task_remove(config_path: &str, matches: &clap::ArgMatches) {
    let name = matches
        .get_one::<String>("name")
        .expect("name is required")
        .clone();

    if std::io::stdin().is_terminal() {
        let confirmed = inquire::Confirm::new(&format!("Remove task '{name}'?"))
            .with_default(false)
            .prompt()
            .unwrap_or(false);
        if !confirmed {
            return;
        }
    }

    let kdl = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    match vivo::remove_task(&kdl, &name) {
        Ok(new_kdl) => {
            if let Err(e) = fs::write(config_path, new_kdl) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            println!("Removed task '{name}'.");
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn cmd_remote_add(config_path: &str, matches: &clap::ArgMatches) {
    let interactive = std::io::stdin().is_terminal();

    let task = match matches.get_one::<String>("task") {
        Some(t) => t.clone(),
        None if !interactive => {
            eprintln!("error: --task is required in non-interactive mode");
            process::exit(1);
        }
        None => {
            let default_suggestion = fs::read_to_string(config_path)
                .ok()
                .and_then(|c| {
                    knuffel::parse::<vivo::BackupConfig>(config_path, &c)
                        .ok()
                        .map(|cfg| cfg.default_task)
                })
                .unwrap_or_default();

            inquire::Text::new("Task name:")
                .with_default(&default_suggestion)
                .prompt()
                .unwrap_or_else(|_| process::exit(0))
        }
    };

    let url = require_or_prompt(
        matches,
        "url",
        "Remote URL (e.g. rustfs:http://nas:9000/bucket):",
        interactive,
    );
    let credentials = require_or_prompt(
        matches,
        "credentials",
        "Credentials profile name (must exist in secrets):",
        interactive,
    );

    let kdl = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    match vivo::add_remote(&kdl, &task, vivo::RemoteSpec { url: url.clone(), credentials, mc_max_workers: None, mc_limit_upload: None }) {
        Ok(new_kdl) => {
            if let Err(e) = fs::write(config_path, new_kdl) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            println!("Added remote '{url}' to task '{task}'.");
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn cmd_remote_list(config_path: &str, matches: &clap::ArgMatches) {
    let content = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };
    let config: vivo::BackupConfig = match knuffel::parse(config_path, &content) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    if let Some(task_filter) = matches.get_one::<String>("task") {
        let remotes = config.remotes_for_task(task_filter);
        if remotes.is_empty() {
            println!("No remotes for task '{task_filter}'.");
        } else {
            for (url, creds) in remotes {
                println!("{url}  [{creds}]");
            }
        }
    } else {
        for task in &config.tasks {
            let remotes = task.backup_remotes();
            if !remotes.is_empty() {
                println!("{}:", task.name);
                for (url, creds) in remotes {
                    println!("  {url}  [{creds}]");
                }
            }
        }
    }
}

fn cmd_remote_remove(config_path: &str, matches: &clap::ArgMatches) {
    let task = matches.get_one::<String>("task").expect("task is required").clone();
    let url = matches.get_one::<String>("url").expect("url is required").clone();

    if std::io::stdin().is_terminal() {
        let confirmed =
            inquire::Confirm::new(&format!("Remove remote '{url}' from task '{task}'?"))
                .with_default(false)
                .prompt()
                .unwrap_or(false);
        if !confirmed {
            return;
        }
    }

    let kdl = match fs::read_to_string(config_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    };

    match vivo::remove_remote(&kdl, &task, &url) {
        Ok(new_kdl) => {
            if let Err(e) = fs::write(config_path, new_kdl) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            println!("Removed remote '{url}' from task '{task}'.");
        }
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--help" || a == "-h" || a == "help") {
        vivo::ui::print_banner();
    }

    let matches = build_cli().get_matches();

    debug!("args parsed");

    let config_path = config_path_from(matches.get_one("config"));
    let secrets_path = secrets_path_from();

    match matches.subcommand() {
        Some(("init", _)) => {
            if !cmd_init(&config_path, &secrets_path) {
                process::exit(1);
            }
            return;
        }
        Some(("config", sub)) => {
            match sub.subcommand() {
                Some(("init", _)) => {
                    if !cmd_config_init(&config_path) {
                        process::exit(1);
                    }
                }
                Some(("edit", _)) => cmd_config_edit(&config_path),
                Some(("show", _)) => cmd_config_show(&config_path),
                _ => unreachable!(),
            }
            return;
        }
        Some(("secrets", sub)) => {
            match sub.subcommand() {
                Some(("init", _)) => {
                    if !cmd_secrets_init(&secrets_path) {
                        process::exit(1);
                    }
                }
                Some(("edit", _)) => cmd_secrets_edit(&secrets_path),
                Some(("show", _)) => cmd_secrets_show(&secrets_path),
                Some(("import-b2", _)) => cmd_secrets_import_b2(&secrets_path),
                Some(("import-s3", args)) => cmd_secrets_import_s3(&secrets_path, args),
                _ => unreachable!(),
            }
            return;
        }
        Some(("task", sub)) => {
            match sub.subcommand() {
                Some(("add", args)) => cmd_task_add(&config_path, args),
                Some(("list", _)) => cmd_task_list(&config_path),
                Some(("remove", args)) => cmd_task_remove(&config_path, args),
                _ => unreachable!(),
            }
            return;
        }
        Some(("remote", sub)) => {
            match sub.subcommand() {
                Some(("add", args)) => cmd_remote_add(&config_path, args),
                Some(("list", args)) => cmd_remote_list(&config_path, args),
                Some(("remove", args)) => cmd_remote_remove(&config_path, args),
                _ => unreachable!(),
            }
            return;
        }
        Some(("mount", sub)) => {
            if !ensure_onboarded(&config_path, &secrets_path) {
                return;
            }
            let mount_path = sub.get_one::<String>("path").map(String::as_str);
            if let Err(e) = vivo::mount::run(&config_path, &secrets_path, mount_path) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            return;
        }
        Some(("manage", _)) => {
            if !ensure_onboarded(&config_path, &secrets_path) {
                return;
            }
            if let Err(e) = vivo::tui::run(&config_path) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            return;
        }
        Some(("doctor", sub)) => {
            let fix = sub.get_flag("fix");
            cmd_doctor(&config_path, &secrets_path, fix);
            return;
        }
        Some(("update", _)) => {
            let dry_run = matches.get_flag("dry-run");
            cmd_update(dry_run);
            return;
        }
        _ => {}
    }

    let vivo_config = VivoConfig::from_matches(&matches);
    debug!("{:?}", vivo_config);

    let update_notice = vivo::update::maybe_check_update();

    match BackupConfig::load_config(&vivo_config) {
        Ok((backup_config, secrets)) => {
            let task_name = vivo_config
                .task_name
                .as_deref()
                .unwrap_or(&backup_config.default_task);

            match backup_config.tasks.iter().find(|t| t.name == task_name) {
                Some(task) => task.run(&vivo_config, &backup_config.tasks, &secrets.credentials),
                None => eprintln!("error: task '{task_name}' not found in config"),
            }
        }
        Err(e) => handle_load_error(&e, &config_path, &secrets_path),
    }

    if let Some(ref latest) = update_notice {
        vivo::update::print_update_notice(latest);
    }
}
