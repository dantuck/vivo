//! Checking that the folders a task backs up can actually be read, before and
//! after a backup. restic treats an unreadable source as a warning (exit code
//! 3) and still saves a snapshot, so without a check a task whose only folder
//! is blocked, such as a Photos library on macOS without Full Disk Access,
//! "succeeds" with an empty backup.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;

use crate::expand_env_vars;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// The folder isn't there (a typo, or a drive that isn't connected).
    Missing(String),
    /// The folder is there but the operating system won't let vivo read it.
    Denied(String),
}

impl Problem {
    pub fn path(&self) -> &str {
        match self {
            Problem::Missing(p) | Problem::Denied(p) => p,
        }
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, Problem::Denied(_))
    }

    /// One line for a person to read.
    pub fn describe(&self) -> String {
        match self {
            Problem::Missing(p) => format!("'{p}' does not exist, so there is nothing to back up there"),
            // macOS guards a Photos library on its own: closing Photos does not lift it
            Problem::Denied(p) if p.to_lowercase().ends_with(".photoslibrary") => {
                format!("vivo is not allowed to read '{p}' (macOS blocks a Photos library even when Photos is closed)")
            }
            Problem::Denied(p) => format!("vivo is not allowed to read '{p}'"),
        }
    }
}

/// What to do about a [`Problem::Denied`], for this operating system.
pub fn permission_hint() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS is blocking access. Add the app you run vivo from (Terminal, iTerm, ...) in System Settings \
         \u{2192} Privacy & Security \u{2192} Full Disk Access, then restart it. A scheduled backup needs \
         the same for the `vivo` binary itself."
    } else {
        "Check the folder's permissions for the user running vivo."
    }
}

/// Photos libraries (`*.photoslibrary` bundles) that are `folder` itself or sit
/// directly inside it, written as `folder` is (so `$HOME` stays `$HOME`).
pub fn photos_libraries_in(folder: &str) -> Vec<String> {
    let is_library = |name: &str| name.to_lowercase().ends_with(".photoslibrary");
    let trimmed = folder.trim_end_matches('/');
    if trimmed.rsplit('/').next().is_some_and(is_library) {
        return vec![trimmed.to_string()];
    }
    let Ok(entries) = fs::read_dir(expand_env_vars(trimmed)) else { return Vec::new() };
    let mut found: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            // the name is the cheap test: only a match is worth a stat
            let name = e.file_name().into_string().ok()?;
            (is_library(&name) && e.path().is_dir()).then(|| format!("{trimmed}/{name}"))
        })
        .collect();
    found.sort();
    found
}

/// Whether `path` can be listed, and why not if it can't.
fn listable(path: &Path) -> Result<(), ErrorKind> {
    fs::read_dir(path).map(|_| ()).map_err(|e| e.kind())
}

/// Problems with `folder` (written as in a config, `$HOME` allowed): it
/// doesn't exist, can't be read, or holds a Photos library that can't be read.
/// The last matters because macOS guards a library on its own, so its folder
/// can look readable while everything useful inside it is blocked.
pub fn check_folder(folder: &str) -> Vec<Problem> {
    let expanded = expand_env_vars(folder);
    let path = Path::new(&expanded);

    match fs::metadata(path) {
        Err(e) if e.kind() == ErrorKind::NotFound => return vec![Problem::Missing(folder.to_string())],
        Err(e) if e.kind() == ErrorKind::PermissionDenied => return vec![Problem::Denied(folder.to_string())],
        Err(_) => return Vec::new(),
        Ok(meta) if !meta.is_dir() => return Vec::new(),
        Ok(_) => {}
    }

    if listable(path) == Err(ErrorKind::PermissionDenied) {
        return vec![Problem::Denied(folder.to_string())];
    }
    photos_libraries_in(folder)
        .into_iter()
        .filter(|library| listable(Path::new(&expand_env_vars(library))) == Err(ErrorKind::PermissionDenied))
        .map(Problem::Denied)
        .collect()
}

/// Problems with any of `folders`, in order.
pub fn check_folders<S: AsRef<str>>(folders: &[S]) -> Vec<Problem> {
    folders.iter().flat_map(|f| check_folder(f.as_ref())).collect()
}

/// Lines to show for `problems`: each one, then what to do if any was a
/// permission problem. Empty if there are none.
pub fn report(problems: &[Problem]) -> Vec<String> {
    let mut lines: Vec<String> = problems.iter().map(Problem::describe).collect();
    if problems.iter().any(Problem::is_denied) {
        lines.push(permission_hint().to_string());
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn block(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    }

    #[cfg(unix)]
    fn unblock(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_readable_folder_has_no_problems_and_a_file_is_fine() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().display().to_string();
        assert!(check_folder(&folder).is_empty());
        let file = dir.path().join("a.txt");
        fs::write(&file, "x").unwrap();
        assert!(check_folder(&file.display().to_string()).is_empty());
        assert!(check_folder("$HOME").is_empty());
    }

    #[test]
    fn a_missing_folder_is_reported_as_missing() {
        let problems = check_folder("/no/such/folder/for/vivo");
        assert_eq!(problems, [Problem::Missing("/no/such/folder/for/vivo".to_string())]);
        assert!(!problems[0].is_denied());
        assert!(problems[0].describe().contains("does not exist"));
    }

    #[cfg(unix)]
    #[test]
    fn a_blocked_folder_is_reported_as_denied() {
        // root ignores permissions, so there is nothing to detect when running as root
        if unsafe_is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        fs::create_dir(&locked).unwrap();
        block(&locked);
        let shown = locked.display().to_string();
        let problems = check_folder(&shown);
        unblock(&locked);
        assert_eq!(problems, [Problem::Denied(shown)]);
        assert!(problems[0].is_denied());
    }

    #[cfg(unix)]
    #[test]
    fn a_blocked_photos_library_inside_a_readable_folder_is_found() {
        if unsafe_is_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join("Photos Library.photoslibrary");
        fs::create_dir(&library).unwrap();
        fs::create_dir(dir.path().join("2024")).unwrap();
        block(&library);
        let folder = dir.path().display().to_string();
        let problems = check_folder(&folder);
        unblock(&library);
        // the folder itself is readable; the library inside it is what is blocked
        assert_eq!(problems, [Problem::Denied(format!("{folder}/Photos Library.photoslibrary"))]);
    }

    #[cfg(unix)]
    fn unsafe_is_root() -> bool {
        // `id -u` avoids a libc dependency just for this check
        std::process::Command::new("id").arg("-u").output().map(|o| o.stdout == b"0\n").unwrap_or(false)
    }

    #[test]
    fn report_adds_the_permission_hint_only_when_needed() {
        assert!(report(&[]).is_empty());
        let missing = report(&[Problem::Missing("/x".into())]);
        assert_eq!(missing.len(), 1, "a missing folder is not a permission problem");
        let denied = report(&[Problem::Denied("/y".into()), Problem::Missing("/x".into())]);
        assert_eq!(denied.len(), 3);
        assert_eq!(denied.last().unwrap(), permission_hint());
    }

    #[test]
    fn check_folders_covers_every_folder() {
        assert_eq!(check_folders(&["/no/such/a", "/no/such/b"]).len(), 2);
    }

    #[test]
    fn a_blocked_photos_library_says_that_closing_photos_does_not_help() {
        let library = Problem::Denied("$HOME/Pictures/Photos Library.photoslibrary".into()).describe();
        assert!(library.contains("even when Photos is closed"), "{library}");
        let plain = Problem::Denied("/data".into()).describe();
        assert_eq!(plain, "vivo is not allowed to read '/data'");
    }
}
