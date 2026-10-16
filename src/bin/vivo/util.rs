//! Helpers shared by the subcommands.

use std::io::IsTerminal;
use std::path::Path;
use std::{env, fs, process};

use inquire::InquireError;

/// Why a subcommand stopped early.
pub enum Stop {
    /// The user pressed Esc / Ctrl-C at a prompt: exit quietly with success.
    Cancelled,
    Failed(String),
}

impl From<String> for Stop {
    fn from(message: String) -> Self {
        Stop::Failed(message)
    }
}

pub type CmdResult = Result<(), Stop>;

/// Ends the process according to how a subcommand finished.
pub fn finish(result: CmdResult) {
    match result {
        Ok(()) => {}
        Err(Stop::Cancelled) => process::exit(0),
        Err(Stop::Failed(message)) => {
            eprintln!("error: {message}");
            process::exit(1);
        }
    }
}

/// Maps an inquire result into a `CmdResult`-compatible one: Esc / Ctrl-C is
/// `Stop::Cancelled`, any other prompt failure is `Stop::Failed`.
pub fn prompt<T>(answer: Result<T, InquireError>) -> Result<T, Stop> {
    answer.map_err(|e| match e {
        InquireError::OperationCanceled | InquireError::OperationInterrupted => Stop::Cancelled,
        e => Stop::Failed(e.to_string()),
    })
}

pub fn interactive() -> bool {
    std::io::stdin().is_terminal()
}

/// The value of `--flag`, or (on a terminal) a prompt for it.
pub fn require_or_prompt(
    matches: &clap::ArgMatches,
    flag: &str,
    prompt_text: &str,
) -> Result<String, Stop> {
    if let Some(v) = matches.get_one::<String>(flag) {
        return Ok(v.clone());
    }
    if !interactive() {
        return Err(Stop::Failed(format!("--{flag} is required in non-interactive mode")));
    }
    prompt(inquire::Text::new(prompt_text).prompt())
}

/// Like [`require_or_prompt`], but optional: a blank answer (or no terminal) is `None`.
pub fn get_or_prompt_opt(
    matches: &clap::ArgMatches,
    flag: &str,
    prompt_text: &str,
) -> Result<Option<String>, Stop> {
    if let Some(v) = matches.get_one::<String>(flag) {
        return Ok(Some(v.clone()));
    }
    if !interactive() {
        return Ok(None);
    }
    let answer = prompt(
        inquire::Text::new(prompt_text)
            .with_help_message("Leave blank to skip")
            .prompt(),
    )?;
    Ok((!answer.is_empty()).then_some(answer))
}

pub fn open_in_editor(path: &str) {
    let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
    if let Err(e) = process::Command::new(&editor).arg(path).status() {
        eprintln!("error: could not open editor '{editor}': {e}");
    }
}

/// Opens `path` in `sops`. Returns true if `sops` ran and exited successfully.
pub fn open_with_sops(path: &str) -> bool {
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

pub fn ensure_parent_dirs(path: &str) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent).map_err(|e| format!("could not create directory: {e}"))?;
    }
    Ok(())
}

/// Creates parent dirs and writes `contents` to `path` if it does not already exist.
/// Returns Ok(true) on creation, Ok(false) if already exists, Err on I/O failure.
pub fn create_with_template(path: &str, contents: &str) -> Result<bool, String> {
    if Path::new(path).exists() {
        return Ok(false);
    }
    ensure_parent_dirs(path)?;
    fs::write(path, contents).map_err(|e| format!("could not write file: {e}"))?;
    Ok(true)
}

/// Exits cleanly when the user presses Esc/Ctrl-C in a setup prompt.
pub fn cancel_setup() -> ! {
    println!("\nSetup cancelled before the backup config was written. Re-run `vivo init` to continue.");
    process::exit(0)
}
