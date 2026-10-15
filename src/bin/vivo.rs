use is_terminal::IsTerminal;
use log::debug;
use std::{env, fs, path::Path, process};
use vivo::{
    age_public_key, build_cli, config_path_from, decrypt_sops_file, expand_env_vars,
    import_b2_credentials, secrets_path_from, BackupConfig, VivoConfig,
};

/// Unwraps `r`, or prints `error: <e>` and exits with status 1.
fn or_exit<T, E: std::fmt::Display>(r: Result<T, E>) -> T {
    r.unwrap_or_else(|e| {
        eprintln!("error: {e}");
        process::exit(1)
    })
}

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

fn secrets_template() -> String {
    format!(
        "restic_password: \"{}\"\ncredentials: {{}}\n",
        vivo::backup_config::PLACEHOLDER_PASSWORD
    )
}

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

/// Exits cleanly when the user presses Esc/Ctrl-C in a setup prompt.
fn cancel_setup() -> ! {
    println!("\nSetup cancelled before the backup config was written. Re-run `vivo init` to continue.");
    process::exit(0)
}

/// Guides the user through creating their first backup task interactively:
/// what to back up and where the restic repo lives. Remotes come later in
/// `cmd_init`, once the secrets exist, so they can get credentials and a
/// connection test. Falls back to the static template when stdin isn't a
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
    if let Err(e) = fs::write(&tmp_path, secrets_template()) {
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
    let masked = decrypt_sops_file(secrets_path)
        .and_then(|contents| vivo::backup_config::mask_secrets_yaml(&contents));
    match masked {
        Ok(masked) => print!("{masked}"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Offers to create the age key `sops` encrypts vivo's secrets with.
/// Returns true if a key was created.
fn maybe_create_age_key() -> bool {
    let path = vivo::age_keys_path();
    println!("\nvivo encrypts your secrets with an age key, and none exists yet.");
    let create = inquire::Confirm::new(&format!("Create one at {path}?"))
        .with_default(true)
        .prompt()
        .unwrap_or(false);
    if !create {
        return false;
    }
    match vivo::doctor::generate_age_key(&path) {
        Ok(()) => {
            println!("\nBack up {path} somewhere safe — without it your secrets can't be decrypted.\n");
            true
        }
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

/// Checks restic, sops and the age key. On a terminal it offers to install
/// missing tools with Homebrew and to create the age key. Returns false if
/// something required is still missing.
fn ensure_prerequisites(interactive: bool) -> bool {
    use vivo::doctor::{check_age_key, check_tool_present, print_result, CheckResult, CheckStatus};

    let failed = |r: &CheckResult| matches!(r.status, CheckStatus::Fail);
    let probe = || {
        [
            check_tool_present("restic", "version", "install from https://restic.net"),
            check_tool_present("sops", "--version", "install from https://github.com/getsops/sops"),
            check_tool_present("age-keygen", "--version", "install from https://age-encryption.org"),
            check_age_key(),
        ]
    };
    let mut checks = probe();

    if interactive {
        let [restic, sops, age_cli, key] = &checks;
        let packages: Vec<&str> = [
            ("restic", failed(restic)),
            ("sops", failed(sops)),
            ("age", failed(key) && failed(age_cli)),
        ]
        .into_iter()
        .filter_map(|(pkg, missing)| missing.then_some(pkg))
        .collect();

        if !packages.is_empty() {
            for r in [restic, sops].into_iter().filter(|r| failed(r)) {
                print_result(r);
            }
            match vivo::doctor::fix_core_tools(&packages) {
                Ok(true) => checks = probe(),
                Ok(false) => {}
                Err(e) => eprintln!("error: {e}"),
            }
        }

        let [_, _, age_cli, key] = &checks;
        if failed(key) && !failed(age_cli) && maybe_create_age_key() {
            checks[3] = check_age_key();
        }
    }

    let [restic, sops, _, key] = &checks;
    for r in [restic, sops, key] {
        print_result(r);
    }
    if [restic, sops, key].into_iter().any(failed) {
        eprintln!("\nInstall missing prerequisites and re-run `vivo init`.");
        return false;
    }
    true
}

/// A run of the default task against `config_path`, as `vivo` would do with no arguments.
fn first_backup_config(config_path: &str, dry_run: bool) -> VivoConfig {
    VivoConfig {
        task_name: None,
        config_file: Some(std::path::PathBuf::from(config_path)),
        dry_run,
        start_step: Default::default(),
    }
}

/// Offers a dry run, then the first real backup. Returns true if the real backup ran and succeeded.
fn offer_first_backup(config_path: &str, secrets_path: &str) -> bool {
    println!();
    let preview = inquire::Confirm::new("Preview your first backup with a dry run?")
        .with_default(true)
        .prompt()
        .unwrap_or(false);
    if !preview {
        return false;
    }
    println!();
    run_backup(&first_backup_config(config_path, true), config_path, secrets_path);

    println!();
    let run = inquire::Confirm::new("Run the first real backup now?")
        .with_default(false)
        .with_help_message("Large directories can take a while; you can also run `vivo` later")
        .prompt()
        .unwrap_or(false);
    run && run_backup(&first_backup_config(config_path, false), config_path, secrets_path)
}

/// Runs the full guided setup: prerequisites, what to back up, the restic
/// password, an off-site remote, and a first backup. Returns true only if
/// every required step succeeded.
fn cmd_init(config_path: &str, secrets_path: &str) -> bool {
    vivo::ui::print_banner();
    println!("Checking prerequisites...");

    let interactive = std::io::stdin().is_terminal();
    if !ensure_prerequisites(interactive) {
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

    let (mut password_set, mut remote_added, mut backed_up) = (false, false, false);
    if interactive {
        println!();
        password_set = setup_restic_password(secrets_path);
        // Remotes and the first backup need the real password, so they wait for it.
        if password_set {
            remote_added = offer_first_remote(config_path, secrets_path);
            backed_up = offer_first_backup(config_path, secrets_path);
        }
    }

    println!();
    println!("Setup complete. Next steps:");
    let mut steps = vec![];
    if !password_set {
        steps.push("Set your restic password:   vivo secrets edit");
    }
    if !remote_added {
        steps.push("Add an off-site remote:     vivo remote add  (RustFS, S3 or B2)");
    }
    if !backed_up {
        steps.push("Run your first backup:      vivo  (preview first with: vivo --dry-run)");
    }
    steps.push("Run it on a schedule:       see \"Scheduling\" in the README (launchd, cron, systemd)");
    steps.push("Check everything:           vivo doctor");
    for (i, step) in steps.iter().enumerate() {
        println!("  {}. {step}", i + 1);
    }
    println!(
        "\nKeep copies of your restic password and your age key ({}) — backups can't be restored without them.",
        vivo::age_keys_path()
    );
    true
}

/// Prompts for the restic repository password if the secrets file still has
/// the placeholder. Returns true if a real password is set afterwards.
fn setup_restic_password(secrets_path: &str) -> bool {
    let Some(secrets) = decrypt_sops_file(secrets_path)
        .ok()
        .and_then(|d| vivo::parse_secrets(&d).ok())
    else {
        return false;
    };
    if !vivo::backup_config::is_placeholder_password(&secrets.restic_password) {
        return true;
    }

    let password = match inquire::Password::new("Restic repository password:")
        .with_help_message("Encrypts your backups. Keep a copy somewhere safe — it can't be recovered.")
        .prompt()
    {
        Ok(p) if !p.is_empty() => p,
        _ => return false,
    };
    match vivo::set_restic_password(secrets_path, &password) {
        Ok(()) => {
            println!("Saved restic password to {secrets_path}");
            true
        }
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
}

/// Offers to add a first remote right after init. Returns true if one was added.
fn offer_first_remote(config_path: &str, secrets_path: &str) -> bool {
    println!();
    let wanted = inquire::Confirm::new("Add an off-site remote now (RustFS, S3, or B2)?")
        .with_default(true)
        .prompt()
        .unwrap_or(false);
    if !wanted {
        return false;
    }
    match vivo::remote_setup::guided_add(config_path, secrets_path, None) {
        Ok(added) => added.is_some(),
        Err(e) => {
            eprintln!("error: {e}");
            false
        }
    }
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
        vivo::LoadConfigError::PasswordNotSet { .. } => {
            if interactive {
                println!("Your restic password is still the placeholder — set it before the first backup.\n");
                if setup_restic_password(secrets_path) {
                    println!("\nRe-run `vivo` to start your backup.");
                } else {
                    process::exit(1);
                }
            } else {
                eprintln!("error: {err}");
                process::exit(1);
            }
        }
        vivo::LoadConfigError::Other(msg) => {
            eprintln!("error: {msg}");
            process::exit(1);
        }
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
    let config = or_exit(vivo::BackupConfig::from_file(config_path));

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

    let kdl = or_exit(fs::read_to_string(config_path));

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

fn cmd_remote_add(config_path: &str, secrets_path: &str, matches: &clap::ArgMatches) {
    let interactive = std::io::stdin().is_terminal();

    // No URL or credentials given on a terminal: walk through the guided flow
    // (backend, validated URL, credential profile, then a connection test).
    if interactive
        && matches.get_one::<String>("url").is_none()
        && matches.get_one::<String>("credentials").is_none()
    {
        if !ensure_onboarded(config_path, secrets_path) {
            return;
        }
        let task = matches.get_one::<String>("task").map(String::as_str);
        if let Err(e) = vivo::remote_setup::guided_add(config_path, secrets_path, task) {
            eprintln!("error: {e}");
            process::exit(1);
        }
        return;
    }

    let task = match matches.get_one::<String>("task") {
        Some(t) => t.clone(),
        None if !interactive => {
            eprintln!("error: --task is required in non-interactive mode");
            process::exit(1);
        }
        None => {
            let default_suggestion = vivo::BackupConfig::from_file(config_path)
                .map(|cfg| cfg.default_task)
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
    or_exit(vivo::remote_setup::validate_url(&url));
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

    match vivo::add_remote(&kdl, &task, vivo::RemoteSpec { url: url.clone(), credentials: credentials.clone(), mc_max_workers: None, mc_limit_upload: None }) {
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

    if interactive {
        vivo::remote_setup::test_after_save(&url, &credentials, secrets_path);
    }
}

fn cmd_remote_test(config_path: &str, secrets_path: &str, matches: &clap::ArgMatches) {
    if !ensure_onboarded(config_path, secrets_path) {
        process::exit(1);
    }
    let config = or_exit(vivo::BackupConfig::from_file(config_path));
    let secrets = or_exit(vivo::remote_setup::load_secrets(secrets_path));

    let remotes = match matches.get_one::<String>("task") {
        Some(t) => config.remotes_for_task(t),
        None => config.all_remotes(),
    };
    if remotes.is_empty() {
        println!("No remotes configured. Add one with `vivo remote add`.");
        return;
    }

    let interactive = std::io::stdin().is_terminal();
    let mut failed = 0;
    for (url, profile) in &remotes {
        println!("{url}  (credentials: {profile})");
        match vivo::remote_setup::test_remote_with_secrets(url, profile, &secrets, interactive) {
            Ok(msg) => println!("  ok: {msg}\n"),
            Err(e) => {
                eprintln!("  failed: {e}\n");
                failed += 1;
            }
        }
    }
    if failed > 0 {
        eprintln!("{failed} of {} remote(s) failed.", remotes.len());
        process::exit(1);
    }
}

fn cmd_remote_list(config_path: &str, matches: &clap::ArgMatches) {
    let config = or_exit(vivo::BackupConfig::from_file(config_path));

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

/// Loads config and secrets and runs the selected task. Returns false if the
/// task, a remote sync, or anything before them failed. Load problems get
/// guided repair on a terminal and don't count as a failed run.
fn run_backup(vivo_config: &VivoConfig, config_path: &str, secrets_path: &str) -> bool {
    match BackupConfig::load_config(vivo_config) {
        Ok((backup_config, secrets)) => {
            let task_name = vivo_config
                .task_name
                .as_deref()
                .unwrap_or(&backup_config.default_task);

            match backup_config.tasks.iter().find(|t| t.name == task_name) {
                Some(task) => task.run(vivo_config, &backup_config.tasks, &secrets.credentials),
                None => {
                    eprintln!("error: task '{task_name}' not found in config");
                    false
                }
            }
        }
        Err(e) => {
            handle_load_error(&e, config_path, secrets_path);
            true
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
                Some(("add", args)) => cmd_remote_add(&config_path, &secrets_path, args),
                Some(("list", args)) => cmd_remote_list(&config_path, args),
                Some(("test", args)) => cmd_remote_test(&config_path, &secrets_path, args),
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

    let ok = run_backup(&vivo_config, &config_path, &secrets_path);

    if let Some(ref latest) = update_notice {
        vivo::update::print_update_notice(latest);
    }

    // Non-zero exit lets cron/launchd/systemd see a failed backup or remote sync.
    if !ok {
        process::exit(1);
    }
}
