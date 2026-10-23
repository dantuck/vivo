//! Asking the user for the folders a task backs up: tab-completion, `~` and
//! pasted/dragged paths understood, a warning for a folder that doesn't exist,
//! and any number of them. Shared by `vivo manage` and `vivo task add`.
//!
//! The folder being backed up is never created here: a typo would otherwise
//! produce an empty folder and a backup of nothing.

use std::path::Path;

use inquire::autocompletion::Replacement;
use inquire::{Autocomplete, Confirm, CustomUserError, InquireError, Text};

use crate::expand_env_vars;

const MAX_SUGGESTIONS: usize = 100;

fn home() -> String {
    std::env::var("HOME").unwrap_or_default()
}

/// Cleans up a typed or pasted folder path and writes it the way configs do:
/// `~` and the home directory become `$HOME`, so the config works on another
/// machine. Quotes and backslash-escaped spaces (what dragging a folder into a
/// terminal pastes) are removed, and trailing slashes dropped.
pub fn normalize_path(input: &str, home: &str) -> String {
    let mut path = input.trim().to_string();
    let quoted = |q: char| path.len() >= 2 && path.starts_with(q) && path.ends_with(q);
    if quoted('\'') || quoted('"') {
        path = path[1..path.len() - 1].to_string();
    }
    path = path.replace("\\ ", " ");
    while path.len() > 1 && path.ends_with('/') {
        path.pop();
    }

    if path == "~" {
        return "$HOME".to_string();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return format!("$HOME/{rest}");
    }
    if !home.is_empty() {
        if path == home {
            return "$HOME".to_string();
        }
        if let Some(rest) = path.strip_prefix(&format!("{home}/")) {
            return format!("$HOME/{rest}");
        }
    }
    path
}

/// Whether `path` (as written in a config, `$HOME` and `~` allowed) is a folder.
pub fn folder_exists(path: &str) -> bool {
    Path::new(&expand_env_vars(path)).is_dir()
}

/// Folders to offer while the user has typed `input`: the subfolders of what
/// comes before the last `/`, starting with what comes after it. They keep the
/// way the user wrote the start (`~/Pic` gives `~/Pictures/`). Hidden folders
/// are only offered once the user types a leading dot.
pub fn folder_suggestions(input: &str) -> Vec<String> {
    let Some(slash) = input.rfind('/') else {
        // Nothing to look inside yet: point at the home directory.
        return if input.is_empty() || "~/".starts_with(input) { vec!["~/".to_string()] } else { Vec::new() };
    };
    let (typed_dir, prefix) = (&input[..=slash], &input[slash + 1..]);
    let Ok(entries) = std::fs::read_dir(expand_env_vars(typed_dir)) else { return Vec::new() };

    let prefix_lower = prefix.to_lowercase();
    // Match on the name first: only the few that match are worth a stat, which
    // matters in a folder of thousands, on every keystroke.
    let mut names: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e)))
        .filter(|(name, _)| prefix.starts_with('.') || !name.starts_with('.'))
        .filter(|(name, _)| name.to_lowercase().starts_with(&prefix_lower))
        .filter(|(_, entry)| entry.path().is_dir())
        .map(|(name, _)| name)
        .collect();
    names.sort_by_key(|n| n.to_lowercase());
    names.truncate(MAX_SUGGESTIONS);
    names.into_iter().map(|name| format!("{typed_dir}{name}/")).collect()
}

/// The longest start every one of `items` shares, ignoring case (the suggestions
/// are matched that way), written as the first one has it.
fn common_prefix(items: &[String]) -> String {
    let Some(first) = items.first() else { return String::new() };
    let mut chars = first.chars().count();
    for item in &items[1..] {
        let shared = first.chars().zip(item.chars()).take_while(|(a, b)| a.to_lowercase().eq(b.to_lowercase())).count();
        chars = chars.min(shared);
    }
    first.chars().take(chars).collect()
}

/// Tab-completes folder names.
#[derive(Clone, Default)]
pub struct FolderCompleter;

impl Autocomplete for FolderCompleter {
    fn get_suggestions(&mut self, input: &str) -> Result<Vec<String>, CustomUserError> {
        Ok(folder_suggestions(input))
    }

    fn get_completion(&mut self, input: &str, highlighted: Option<String>) -> Result<Replacement, CustomUserError> {
        if highlighted.is_some() {
            return Ok(highlighted);
        }
        let suggestions = folder_suggestions(input);
        let prefix = common_prefix(&suggestions);
        Ok((prefix.len() > input.len()).then_some(prefix))
    }
}

/// Asks for one folder. A blank answer is `None`. A folder that doesn't exist
/// is confirmed first (it may be on a drive that isn't mounted right now) and
/// otherwise asked for again. With a `default`, Enter accepts it.
pub fn ask_folder(message: &str, help: &str, default: Option<&str>) -> Result<Option<String>, InquireError> {
    loop {
        let mut prompt = Text::new(message).with_help_message(help).with_autocomplete(FolderCompleter);
        if let Some(default) = default {
            prompt = prompt.with_default(default);
        }
        let path = normalize_path(&prompt.prompt()?, &home());
        if path.is_empty() {
            return Ok(None);
        }
        if folder_exists(&path) {
            return Ok(Some(path));
        }
        let keep = Confirm::new(&format!("'{}' does not exist. Use it anyway?", expand_env_vars(&path)))
            .with_default(false)
            .with_help_message("Say no to type it again; it may be a drive that isn't connected right now")
            .prompt()?;
        if keep {
            return Ok(Some(path));
        }
    }
}

/// Asks where to put something vivo or a tool will create, such as an export:
/// Tab completes, the path is cleaned up, and it need not exist yet. Enter
/// accepts `default`.
pub fn ask_new_folder(message: &str, help: &str, default: &str) -> Result<String, InquireError> {
    let answer = Text::new(message).with_help_message(help).with_autocomplete(FolderCompleter).with_default(default).prompt()?;
    let path = normalize_path(&answer, &home());
    Ok(if path.is_empty() { default.to_string() } else { path })
}

/// Asks for a task's folders: one, then more until a blank answer. `first_default`
/// is offered for the first. The result has no duplicates, in the order given.
pub fn ask_folders(first_message: &str, first_default: Option<&str>) -> Result<Vec<String>, InquireError> {
    let mut folders: Vec<String> = Vec::new();
    let push = |folders: &mut Vec<String>, path: String| {
        if !folders.contains(&path) {
            folders.push(path);
        }
    };

    let help = "Tab completes. Leave blank to add folders later with `include` in the config";
    if let Some(first) = ask_folder(first_message, help, first_default)? {
        push(&mut folders, first);
        while let Some(next) = ask_folder(
            "Another folder? (blank when done)",
            "Tip: for many folders, back up their parent (e.g. Pictures) and exclude what you don't want",
            None,
        )? {
            push(&mut folders, next);
        }
    }
    Ok(folders)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_writes_paths_the_way_configs_do() {
        let home = "/Users/me";
        for (input, want) in [
            ("~", "$HOME"),
            ("~/Pictures", "$HOME/Pictures"),
            ("~/Pictures/", "$HOME/Pictures"),
            ("/Users/me", "$HOME"),
            ("/Users/me/Pictures", "$HOME/Pictures"),
            ("  /Users/me/Movies//  ", "$HOME/Movies"),
            ("$HOME/Pictures", "$HOME/Pictures"),
            ("/Volumes/Photos", "/Volumes/Photos"),
            ("/", "/"),
            ("", ""),
            // a sibling that merely shares the home prefix is not under it
            ("/Users/meow/x", "/Users/meow/x"),
        ] {
            assert_eq!(normalize_path(input, home), want, "{input:?}");
        }
    }

    #[test]
    fn normalize_understands_dragged_and_quoted_paths() {
        let home = "/Users/me";
        assert_eq!(normalize_path("'/Users/me/My Photos'", home), "$HOME/My Photos");
        assert_eq!(normalize_path("\"/Volumes/Big Drive/pics\"", home), "/Volumes/Big Drive/pics");
        assert_eq!(normalize_path("/Users/me/My\\ Photos/2024", home), "$HOME/My Photos/2024");
        // with no known home, nothing is rewritten except `~`
        assert_eq!(normalize_path("/Users/me/x", ""), "/Users/me/x");
        assert_eq!(normalize_path("~/x", ""), "$HOME/x");
    }

    fn tree() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["Pictures", "Photos Library.photoslibrary", "photo-backup", "Movies", ".hidden", "Pictures/2024"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(dir.path().join("Pictures.txt"), "a file, not a folder").unwrap();
        dir
    }

    #[test]
    fn suggestions_are_matching_folders_only_in_the_users_own_words() {
        let dir = tree();
        let base = dir.path().display().to_string();

        // case-insensitive prefix, folders only (no Pictures.txt), trailing slash added
        assert_eq!(
            folder_suggestions(&format!("{base}/pi")),
            [format!("{base}/Pictures/")]
        );
        assert_eq!(
            folder_suggestions(&format!("{base}/p")),
            [
                format!("{base}/photo-backup/"),
                format!("{base}/Photos Library.photoslibrary/"),
                format!("{base}/Pictures/"),
            ]
        );
        // hidden folders appear only when asked for
        assert!(!folder_suggestions(&format!("{base}/")).iter().any(|s| s.contains(".hidden")));
        assert_eq!(folder_suggestions(&format!("{base}/.h")), [format!("{base}/.hidden/")]);
        // looking inside a chosen folder
        assert_eq!(folder_suggestions(&format!("{base}/Pictures/")), [format!("{base}/Pictures/2024/")]);
        // nothing matches, or the folder doesn't exist
        assert!(folder_suggestions(&format!("{base}/zzz")).is_empty());
        assert!(folder_suggestions("/no/such/place/x").is_empty());
    }

    #[test]
    fn suggestions_keep_tilde_and_home_as_typed() {
        let home = std::env::var("HOME").unwrap();
        let first_dir = std::fs::read_dir(&home)
            .unwrap()
            .filter_map(|e| e.ok())
            .find(|e| e.path().is_dir() && !e.file_name().to_string_lossy().starts_with('.'));
        let Some(first_dir) = first_dir else { return };
        let name = first_dir.file_name().to_string_lossy().into_owned();
        let typed: String = name.chars().take(1).collect();

        for start in ["~/", "$HOME/"] {
            let got = folder_suggestions(&format!("{start}{typed}"));
            assert!(got.contains(&format!("{start}{name}/")), "{start}: {got:?}");
        }
    }

    #[test]
    fn with_nothing_to_look_inside_it_points_at_home() {
        assert_eq!(folder_suggestions(""), ["~/"]);
        assert_eq!(folder_suggestions("~"), ["~/"]);
        assert!(folder_suggestions("Pictures").is_empty());
    }

    #[test]
    fn tab_completes_the_common_prefix_or_the_highlighted_choice() {
        let dir = tree();
        let base = dir.path().display().to_string();
        let mut completer = FolderCompleter;

        // "Ph" matches photo-backup and Photos Library.photoslibrary: complete to what they share
        assert_eq!(
            completer.get_completion(&format!("{base}/ph"), None).unwrap(),
            Some(format!("{base}/photo"))
        );
        // one match: complete it fully
        assert_eq!(
            completer.get_completion(&format!("{base}/mo"), None).unwrap(),
            Some(format!("{base}/Movies/"))
        );
        // already as far as it can go
        assert_eq!(completer.get_completion(&format!("{base}/photo"), None).unwrap(), None);
        // a highlighted suggestion wins
        assert_eq!(
            completer.get_completion("x", Some("chosen/".to_string())).unwrap(),
            Some("chosen/".to_string())
        );
    }

    #[test]
    fn existence_check_understands_home_and_never_creates() {
        let home = std::env::var("HOME").unwrap();
        assert!(folder_exists("$HOME") && folder_exists("~") && folder_exists(&home));
        let missing = "$HOME/vivo-no-such-folder-9f3a1c";
        assert!(!folder_exists(missing));
        assert!(!Path::new(&expand_env_vars(missing)).exists(), "checking must not create it");
    }
}
