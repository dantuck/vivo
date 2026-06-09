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

const CONFIG_TEMPLATE: &str = r#"default-task "backup"

tasks {
    task "backup" {
        description "Main backup task"
        backup {
            repo "$HOME/.local/share/restic/main"
            directory "$HOME"
            exclude-file "$HOME/.config/vivo/excludes"
            retention {
                daily 7
                weekly 5
                monthly 12
                yearly 2
            }
            // Add remotes here, e.g.:
            // remote "rustfs:http://your-nas:9000/backup" {
            //     credentials "rustfs"
            // }
            // remote "s3:https://s3.amazonaws.com/my-bucket" {
            //     credentials "aws"
            // }
            // remote "b2:my-bucket:restic/main" {
            //     credentials "b2"
            // }
        }
    }
}
"#;

const SECRETS_TEMPLATE: &str = "restic_password: \"change-me\"\ncredentials: {}\n";

fn open_in_editor(path: &str) {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    if let Err(e) = process::Command::new(&editor).arg(path).status() {
        eprintln!("error: could not open editor '{editor}': {e}");
    }
}

fn open_with_sops(path: &str) {
    if let Err(e) = process::Command::new("sops").arg(path).status() {
        eprintln!("error: could not run sops: {e}");
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

fn cmd_config_init(config_path: &str) {
    match create_with_template(config_path, CONFIG_TEMPLATE) {
        Ok(true) => println!("Created config: {config_path}"),
        Ok(false) => println!("Config already exists: {config_path}"),
        Err(e) => eprintln!("error: {e}"),
    }
}

fn cmd_config_edit(config_path: &str) {
    if let Err(e) = create_with_template(config_path, CONFIG_TEMPLATE) {
        eprintln!("error: {e}");
        return;
    }
    open_in_editor(config_path);
}

fn cmd_config_show(config_path: &str) {
    match fs::read_to_string(config_path) {
        Ok(contents) => print!("{contents}"),
        Err(e) => eprintln!("error: could not read config '{config_path}': {e}"),
    }
}

fn cmd_secrets_init(secrets_path: &str) {
    if Path::new(secrets_path).exists() {
        println!("Secrets file already exists: {secrets_path}");
        return;
    }
    if let Err(e) = ensure_parent_dirs(secrets_path) {
        eprintln!("error: {e}");
        return;
    }

    let Some(recipient) = age_public_key() else {
        eprintln!("error: no age key found — run: age-keygen -o ~/.config/sops/age/keys.txt");
        return;
    };

    let tmp_path = env::temp_dir().join("vivo-secrets-init.tmp");
    if let Err(e) = fs::write(&tmp_path, SECRETS_TEMPLATE) {
        eprintln!("error: could not write secrets template: {e}");
        return;
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
        }
        Ok(o) => eprintln!("error: sops encryption failed: {}", String::from_utf8_lossy(&o.stderr)),
        Err(e) => eprintln!("error: could not run sops: {e}"),
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

fn cmd_init(config_path: &str, secrets_path: &str) {
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
        return;
    }

    println!();
    cmd_config_init(config_path);
    cmd_secrets_init(secrets_path);

    println!();
    println!("Setup complete. Next steps:");
    println!("  1. Edit your backup config:  vivo config edit");
    println!("  2. Set your restic password: vivo secrets edit");
    println!("  3. Run a dry-run backup:     vivo --dry-run");
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
            cmd_init(&config_path, &secrets_path);
            return;
        }
        Some(("config", sub)) => {
            match sub.subcommand() {
                Some(("init", _)) => cmd_config_init(&config_path),
                Some(("edit", _)) => cmd_config_edit(&config_path),
                Some(("show", _)) => cmd_config_show(&config_path),
                _ => unreachable!(),
            }
            return;
        }
        Some(("secrets", sub)) => {
            match sub.subcommand() {
                Some(("init", _)) => cmd_secrets_init(&secrets_path),
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
            let mount_path = sub.get_one::<String>("path").map(String::as_str);
            if let Err(e) = vivo::mount::run(&config_path, &secrets_path, mount_path) {
                eprintln!("error: {e}");
                process::exit(1);
            }
            return;
        }
        Some(("manage", _)) => {
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
        Err(e) => eprintln!("error: {e}"),
    }

    if let Some(ref latest) = update_notice {
        vivo::update::print_update_notice(latest);
    }
}
