//! `vivo recover`: rebuild config and secrets on a new machine from the
//! recovery copy kept next to a remote.

use std::collections::HashMap;
use std::path::Path;

use clap::ArgMatches;
use vivo::recovery;

use crate::util::{ensure_parent_dirs, interactive, prompt, CmdResult};

/// The remote's two credential variables, from `--key-id`/`--key` or a prompt.
fn credentials_for(vars: [(&str, &str); 2], args: &ArgMatches) -> Result<HashMap<String, String>, crate::util::Stop> {
    let mut creds = HashMap::new();
    for (i, (var, label)) in vars.into_iter().enumerate() {
        let flag = ["key-id", "key"][i];
        let value = match args.get_one::<String>(flag) {
            Some(v) => v.clone(),
            None if !interactive() => return Err(format!("--{flag} is required in non-interactive mode").into()),
            None if i == 1 => prompt(inquire::Password::new(label).without_confirmation().prompt())?,
            None => prompt(inquire::Text::new(label).prompt())?,
        };
        creds.insert(var.to_string(), value);
    }
    Ok(creds)
}

pub fn cmd_recover(config_path: &str, secrets_path: &str, args: &ArgMatches) -> CmdResult {
    let url = match args.get_one::<String>("from") {
        Some(u) => u.clone(),
        None if interactive() => prompt(
            inquire::Text::new("Remote to recover from:")
                .with_help_message("the URL from your old config, e.g. rustfs:https://host/bucket or b2:bucket:folder")
                .prompt(),
        )?,
        None => return Err("--from <remote URL> is required".to_string().into()),
    };
    let backend = vivo::remote::from_url(&url)?;
    backend.recovery_store()?;

    let key_file = vivo::age_keys_path();
    if !Path::new(&key_file).exists() {
        return Err(format!(
            "your age key is not at {key_file}. Restore the copy you saved when you first ran `vivo init` to that path, then run this again — the recovery files can't be opened without it"
        )
        .into());
    }
    let force = args.get_flag("force");
    for path in [config_path, secrets_path] {
        if Path::new(path).exists() && !force {
            return Err(format!("{path} already exists; pass --force to replace it").into());
        }
    }

    let creds = credentials_for(backend.credential_vars(), args)?;
    let tmp = tempfile::tempdir().map_err(|e| e.to_string())?;
    println!("Downloading the recovery copy next to {url} ...");
    recovery::fetch(&url, &creds, tmp.path())?;

    let config_file = tmp.path().join("backup.kdl");
    recovery::age_decrypt(&tmp.path().join(recovery::CONFIG_NAME), &config_file, &key_file)?;
    let config_text = std::fs::read_to_string(&config_file).map_err(|e| e.to_string())?;
    let config = vivo::BackupConfig::parse(config_path, &config_text).map_err(|e| format!("recovered config is invalid: {e}"))?;

    let secrets_src = tmp.path().join(recovery::SECRETS_NAME);
    let yaml = vivo::decrypt_sops_file(&secrets_src.to_string_lossy())
        .map_err(|e| format!("your age key can't decrypt the recovered secrets (is it the same key you used before?): {}", e.trim()))?;
    vivo::parse_secrets(&yaml)?;

    ensure_parent_dirs(config_path)?;
    ensure_parent_dirs(secrets_path)?;
    std::fs::write(config_path, &config_text).map_err(|e| format!("could not write {config_path}: {e}"))?;
    std::fs::copy(&secrets_src, secrets_path).map_err(|e| format!("could not write {secrets_path}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(secrets_path, std::fs::Permissions::from_mode(0o600));
    }

    println!("\nRecovered:\n  {config_path}\n  {secrets_path}");
    let names: Vec<&str> = config.tasks.iter().map(|t| t.name.as_str()).collect();
    println!("  tasks: {}", names.join(", "));
    println!("\nNext:");
    println!("  vivo restore     get your files back (reads from the off-site copy while there is no local repository)");
    println!("  vivo status      check everything is healthy");
    Ok(())
}
