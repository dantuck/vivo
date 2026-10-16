//! `vivo init`: the guided first-run setup.

use vivo::VivoConfig;

use crate::config::{cmd_config_init_interactive};
use crate::onboard::run_backup;
use crate::secrets::{cmd_secrets_init, setup_restic_password};
use crate::util::interactive;

/// Offers to create the age key `sops` encrypts vivo's secrets with.
/// Returns true if a key was created.
pub fn maybe_create_age_key() -> bool {
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
pub fn ensure_prerequisites(interactive: bool) -> bool {
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
pub fn first_backup_config(config_path: &str, dry_run: bool) -> VivoConfig {
    VivoConfig {
        task_name: None,
        config_file: Some(std::path::PathBuf::from(config_path)),
        dry_run,
        start_step: Default::default(),
    }
}

/// Offers a dry run, then the first real backup. Returns true if the real backup ran and succeeded.
pub fn offer_first_backup(config_path: &str, secrets_path: &str) -> bool {
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
pub fn cmd_init(config_path: &str, secrets_path: &str) -> bool {
    vivo::ui::print_banner();
    println!("Checking prerequisites...");

    let interactive = interactive();
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

    let (mut password_set, mut remote_added, mut backed_up, mut scheduled) = (false, false, false, false);
    if interactive {
        println!();
        password_set = setup_restic_password(secrets_path);
        // Remotes and the first backup need the real password, so they wait for it.
        if password_set {
            remote_added = offer_first_remote(config_path, secrets_path);
            backed_up = offer_first_backup(config_path, secrets_path);
            scheduled = crate::schedule::offer_schedule(config_path);
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
    if !scheduled {
        steps.push("Run it every day:           vivo schedule add");
    }
    steps.push("See if it's working:         vivo status   (and `vivo doctor` for the setup itself)");
    steps.push("Get files back:              vivo restore");
    for (i, step) in steps.iter().enumerate() {
        println!("  {}. {step}", i + 1);
    }
    println!(
        "\nKeep copies of your restic password and your age key ({}) — backups can't be restored without them.",
        vivo::age_keys_path()
    );
    true
}

/// Offers to add a first remote right after init. Returns true if one was added.
pub fn offer_first_remote(config_path: &str, secrets_path: &str) -> bool {
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
