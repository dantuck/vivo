use log::debug;
use std::{env, fs, path::Path, process};
use vivo::{
    build_cli, config_path_from, decrypt_sops_file, secrets_path_from,
    BackupConfig, VivoConfig,
};

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

    let tmp_path = env::temp_dir().join("vivo-secrets-init.tmp");
    if let Err(e) = fs::write(&tmp_path, SECRETS_TEMPLATE) {
        eprintln!("error: could not write secrets template: {e}");
        return;
    }

    let output = process::Command::new("sops")
        .arg("-e")
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

fn cmd_doctor(config_path: &str, secrets_path: &str) {
    let exit_code = vivo::doctor::run_doctor(config_path, secrets_path);
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

fn main() {
    env_logger::init();

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
                _ => unreachable!(),
            }
            return;
        }
        Some(("doctor", _)) => {
            cmd_doctor(&config_path, &secrets_path);
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
