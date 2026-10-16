//! `vivo secrets` and the secrets-file template.

use std::path::Path;
use std::{env, fs, process};

use vivo::{age_public_key, decrypt_sops_file, import_b2_credentials};

use crate::util::{
    ensure_parent_dirs, interactive, open_with_sops, prompt, require_or_prompt, CmdResult, Stop,
};

pub fn secrets_template() -> String {
    format!(
        "restic_password: \"{}\"\ncredentials: {{}}\n",
        vivo::backup_config::PLACEHOLDER_PASSWORD
    )
}

/// Creates the encrypted secrets file if missing. Returns true on success
/// (including "already existed").
pub fn cmd_secrets_init(secrets_path: &str) -> bool {
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

pub fn cmd_secrets_import_b2(secrets_path: &str) {
    if !Path::new(secrets_path).exists() {
        eprintln!("Secrets file not found. Run `vivo secrets init` first.");
        return;
    }
    match import_b2_credentials(secrets_path) {
        Ok(_) => println!("B2 credentials imported successfully."),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Imports S3-compatible credentials into the secrets file.
pub fn cmd_secrets_import_s3(secrets_path: &str, matches: &clap::ArgMatches) -> CmdResult {
    if !Path::new(secrets_path).exists() {
        return Err(Stop::Failed("Secrets file not found. Run `vivo secrets init` first.".into()));
    }

    let key_id = require_or_prompt(matches, "key-id", "Access key ID (AWS_ACCESS_KEY_ID):")?;
    let key = require_or_prompt(matches, "key", "Secret access key (AWS_SECRET_ACCESS_KEY):")?;
    let profile = match matches.get_one::<String>("profile") {
        Some(p) => p.clone(),
        None if interactive() => {
            prompt(inquire::Text::new("Profile name:").with_default("s3").prompt())?
        }
        None => "s3".to_string(),
    };

    vivo::update_s3_in_secrets(secrets_path, &profile, &key_id, &key)?;
    println!("S3 credentials saved to profile '{profile}'.");
    Ok(())
}

pub fn cmd_secrets_edit(secrets_path: &str) {
    if !Path::new(secrets_path).exists() {
        eprintln!("Secrets file not found. Run `vivo secrets init` first.");
        return;
    }
    open_with_sops(secrets_path);
}

pub fn cmd_secrets_show(secrets_path: &str) {
    let masked = decrypt_sops_file(secrets_path)
        .and_then(|contents| vivo::backup_config::mask_secrets_yaml(&contents));
    match masked {
        Ok(masked) => print!("{masked}"),
        Err(e) => eprintln!("error: {e}"),
    }
}

/// Prompts for the restic repository password if the secrets file still has
/// the placeholder. Returns true if a real password is set afterwards.
pub fn setup_restic_password(secrets_path: &str) -> bool {
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
