//! Starting points for a new task: sensible settings for a kind of backup, so
//! nobody has to learn six config options to get a good photos backup.
//!
//! A [`Plan`] is plain data (folders in, config settings out), built by pure
//! functions and written by [`apply_plan`]; the prompts that gather the folders
//! live in `vivo manage`.

use std::path::Path;

use crate::config_editor as editor;
use crate::expand_env_vars;
use crate::sources::photos_libraries_in;

/// Refuses to back up while the Photos app is open: its library database
/// changes constantly, so a snapshot taken meanwhile can be inconsistent.
pub const PHOTOS_OPEN_GUARD: &str = "! pgrep -x Photos";

/// The thumbnails Photos rebuilds itself. Matched inside a `.photoslibrary`
/// only: excluding the bundle itself would also drop anything included from
/// inside it, and a looser pattern would hit other apps' folders.
pub const LIBRARY_DERIVATIVES: &str = "**/*.photoslibrary/resources/derivatives";

/// Matches Photos libraries, for leaving them out of a run.
pub const PHOTOS_LIBRARIES: &str = "**/*.photoslibrary";

/// Where an export of the Photos library goes unless the user says otherwise.
pub const EXPORT_DEFAULT_DIR: &str = "$HOME/Photos Export";

const PHOTOS_STEPS: &str = "steps {\n    check { every \"weekly\"; subset \"5%\" }\n    forget { every \"weekly\" }\n}";
const PHOTOS_SCHEDULE: &str = "daily 02:00";

/// How to install osxphotos, for the places that tell the user it is missing.
pub const OSXPHOTOS_INSTALL: &str = "uv tool install osxphotos (or brew tap RhetTbull/osxphotos && brew install osxphotos)";

const EXPORT_REASON: &str = "Exporting your Photos library failed";
const EXPORT_FIX: &str = "Run the export by hand to see why. It needs osxphotos (`uv tool install osxphotos`) and macOS permission to read Photos (Full Disk Access)";

/// A command that must succeed before each backup, and what to say when it doesn't.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Check {
    pub command: String,
    pub reason: Option<String>,
    pub fix: Option<String>,
    /// A soft check: when it fails, leave this pattern out of that run and carry
    /// on, instead of stopping the task.
    pub skip: Option<String>,
}

impl Check {
    /// The check that stops a backup while Photos is open.
    pub fn photos_closed() -> Self {
        Check {
            command: PHOTOS_OPEN_GUARD.to_string(),
            reason: Some("Photos is open".to_string()),
            fix: Some("Quit Photos (Cmd+Q)".to_string()),
            skip: None,
        }
    }

    /// Whether this is the Photos-open check (however it has been softened).
    pub fn is_photos_guard(&self) -> bool {
        self.command == PHOTOS_OPEN_GUARD
    }

    /// The step that exports `library` into `dest` (see [`export_command`]).
    pub fn export(library: &str, dest: &str) -> Self {
        Check {
            command: export_command(library, dest),
            reason: Some(EXPORT_REASON.to_string()),
            fix: Some(EXPORT_FIX.to_string()),
            skip: None,
        }
    }
}

/// What a template sets on a new task, besides its name and repository.
#[derive(Debug, Default, PartialEq)]
pub struct Plan {
    /// Folders to back up: the first becomes `directory`, the rest `include`.
    pub folders: Vec<String>,
    pub excludes: Vec<String>,
    /// A file of exclude patterns (`exclude-file`).
    pub exclude_file: Option<String>,
    /// Checks that must pass before each backup.
    pub before: Vec<Check>,
    /// A folder a `before` step fills in (an export). It is safe to create, which
    /// a folder being backed up is not.
    pub output_dir: Option<String>,
    /// A `steps { }` block as KDL text.
    pub steps: Option<String>,
    pub schedule: Option<String>,
}

/// The Photos library in its usual place, if there is one.
pub fn default_photos_library() -> Option<String> {
    photos_libraries_in("$HOME/Pictures").into_iter().next()
}

/// Whether `path` is `folder` or inside it (compared as paths, after expanding `$HOME`).
pub fn is_within(path: &str, folder: &str) -> bool {
    let (path, folder) = (expand_env_vars(path), expand_env_vars(folder));
    Path::new(&path).starts_with(Path::new(&folder))
}

/// True if backing up `folders` backs up a Photos library.
fn includes_library(folders: &[String]) -> bool {
    folders.iter().any(|f| !photos_libraries_in(f).is_empty())
}

/// Adds the settings that protect a Photos library to `plan`, if its folders
/// hold one: the open-Photos guard, and no thumbnails.
fn protect_library(plan: &mut Plan) {
    if !includes_library(&plan.folders) {
        return;
    }
    if !plan.excludes.iter().any(|e| e == LIBRARY_DERIVATIVES) {
        plan.excludes.push(LIBRARY_DERIVATIVES.to_string());
    }
    if !plan.guards_photos() {
        plan.before.push(Check::photos_closed());
    }
}

/// A task for `folders` and nothing else, plus protection for any Photos
/// library among them.
pub fn custom_plan(folders: Vec<String>) -> Plan {
    let mut plan = Plan { folders, ..Plan::default() };
    protect_library(&mut plan);
    plan
}

/// Photos and video: big files that rarely change. Check a sample of the data
/// weekly instead of every run, prune weekly, run nightly.
pub fn photos_plan(folders: Vec<String>) -> Plan {
    let mut plan = Plan {
        folders,
        excludes: vec!["**/.DS_Store".to_string()],
        steps: Some(PHOTOS_STEPS.to_string()),
        schedule: Some(PHOTOS_SCHEDULE.to_string()),
        ..Plan::default()
    };
    protect_library(&mut plan);
    plan
}

impl Plan {
    /// True if the plan keeps a Photos library from being backed up while Photos is open.
    pub fn guards_photos(&self) -> bool {
        self.before.iter().any(Check::is_photos_guard)
    }

    /// True if, with Photos open, only the library is left out of a run (a soft
    /// guard) rather than the whole backup being held back.
    pub fn soft_photos_guard(&self) -> bool {
        self.before.iter().any(|c| c.is_photos_guard() && c.skip.is_some())
    }
}

/// The `osxphotos` command that keeps `dest` up to date with `library`: files in
/// year/month folders, only what is new or changed each time (`--update`), and
/// XMP sidecars carrying the metadata that does not live in the files.
pub fn export_command(library: &str, dest: &str) -> String {
    format!(
        "osxphotos export \"{dest}\" --library \"{library}\" --update --directory \"{{created.year}}/{{created.mm}}\" --sidecar xmp"
    )
}

/// Photos and video, backed up both ways at once: the library as it is, and an
/// export of it, in one task and one repository. The export's files are clones of
/// the library's (on APFS) and identical content is stored once in the
/// repository, so the second copy costs little. The export runs first and never
/// waits for Photos. The library is copied only while Photos is closed: with
/// Photos open it is left out of that run and everything else still backs up.
pub fn photos_both_plan(library: &str, dest: &str, folders: Vec<String>) -> Plan {
    let mut plan = photos_plan(folders);
    if !plan.folders.iter().any(|f| f == dest) {
        plan.folders.push(dest.to_string());
    }

    // a hard guard would stop the export too; the library is what needs Photos
    // closed, so with Photos open only the library is left out of that run
    if let Some(guard) = plan.before.iter_mut().find(|c| c.is_photos_guard()) {
        guard.skip = Some(PHOTOS_LIBRARIES.to_string());
    }
    plan.before.insert(0, Check::export(library, dest));
    plan.output_dir = Some(dest.to_string());
    plan
}

/// Photos and video, backed up as an export instead of the library itself:
/// plain files with real names that any system can read. A `before` step runs
/// the export, so the backup always holds a current one. `others` are folders
/// to back up alongside it; a Photos library inside one of them is left out,
/// since the export is the copy of it.
pub fn photos_export_plan(library: &str, dest: &str, others: Vec<String>) -> Plan {
    let mut folders = vec![dest.to_string()];
    for other in others {
        if other != dest && !folders.contains(&other) {
            folders.push(other);
        }
    }

    let holds_library = folders[1..].iter().any(|f| is_within(library, f) || !photos_libraries_in(f).is_empty());
    let mut excludes = vec!["**/.DS_Store".to_string()];
    if holds_library {
        excludes.push(PHOTOS_LIBRARIES.to_string());
    }

    Plan {
        folders,
        excludes,
        before: vec![Check::export(library, dest)],
        output_dir: Some(dest.to_string()),
        steps: Some(PHOTOS_STEPS.to_string()),
        schedule: Some(PHOTOS_SCHEDULE.to_string()),
        ..Plan::default()
    }
}

/// Whether `tool` is an executable file in a directory on `PATH`.
pub fn tool_on_path(tool: &str) -> bool {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path).any(|dir| dir.join(tool).is_file())
}

/// The KDL text of the task `name` with repository `repo` and everything in
/// `plan`, indented the way a person would write it.
pub fn render_task(name: &str, repo: &str, plan: &Plan) -> String {
    let q = editor::quote;
    let mut out = format!("task {} {{\n", q(name));
    if let Some(schedule) = &plan.schedule {
        out.push_str(&format!("    schedule {}\n", q(schedule)));
    }
    for check in &plan.before {
        out.push_str(&format!("    before {}", q(&check.command)));
        if let Some(reason) = &check.reason {
            out.push_str(&format!(" reason={}", q(reason)));
        }
        if let Some(fix) = &check.fix {
            out.push_str(&format!(" fix={}", q(fix)));
        }
        if let Some(skip) = &check.skip {
            out.push_str(&format!(" skip={}", q(skip)));
        }
        out.push('\n');
    }
    out.push_str("    backup {\n");
    out.push_str(&format!("        repo {}\n", q(repo)));
    if let Some(first) = plan.folders.first() {
        out.push_str(&format!("        directory {}\n", q(first)));
    }
    for folder in plan.folders.iter().skip(1) {
        out.push_str(&format!("        include {}\n", q(folder)));
    }
    for exclude in &plan.excludes {
        out.push_str(&format!("        exclude {}\n", q(exclude)));
    }
    if let Some(file) = &plan.exclude_file {
        out.push_str(&format!("        exclude-file {}\n", q(file)));
    }
    for line in plan.steps.iter().flat_map(|s| s.lines()) {
        out.push_str(&format!("        {line}\n"));
    }
    out.push_str("    }\n}\n");
    out
}

/// Adds the task `name` with repository `repo` and everything in `plan` to the
/// config text.
pub fn apply_plan(kdl: &str, name: &str, repo: &str, plan: &Plan) -> Result<String, String> {
    editor::add_task_text(kdl, name, &render_task(name, repo, plan))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup_config::BackupConfig;

    /// A folder holding a `Photos Library.photoslibrary` and some other folders.
    fn pictures() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["Photos Library.photoslibrary/originals", "2024", "Old.PhotosLibrary"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(dir.path().join("notes.photoslibrary"), "a file, not a library").unwrap();
        dir
    }

    #[test]
    fn finds_libraries_beside_and_at_the_folder() {
        let dir = pictures();
        let base = dir.path().display().to_string();
        assert_eq!(
            photos_libraries_in(&base),
            [format!("{base}/Old.PhotosLibrary"), format!("{base}/Photos Library.photoslibrary")],
            "folders only, any case, sorted"
        );
        // the library itself counts, with or without a trailing slash
        let lib = format!("{base}/Photos Library.photoslibrary");
        assert_eq!(photos_libraries_in(&lib), std::slice::from_ref(&lib));
        assert_eq!(photos_libraries_in(&format!("{lib}/")), [lib]);
        // no libraries, or no such folder
        assert!(photos_libraries_in(&format!("{base}/2024")).is_empty());
        assert!(photos_libraries_in("/no/such/folder").is_empty());
    }

    #[test]
    fn within_means_the_folder_or_inside_it_not_a_lookalike() {
        assert!(is_within("/a/b/c", "/a/b"));
        assert!(is_within("/a/b", "/a/b"));
        assert!(!is_within("/a/bc", "/a/b"));
        assert!(!is_within("/a", "/a/b"));
        assert!(is_within("$HOME/Pictures/x", "$HOME/Pictures"));
    }

    #[test]
    fn photos_plan_sets_cadence_schedule_and_ignores_junk() {
        let plan = photos_plan(vec!["/nowhere/pics".to_string()]);
        assert_eq!(plan.folders, ["/nowhere/pics"]);
        assert_eq!(plan.excludes, ["**/.DS_Store"]);
        assert!(plan.before.is_empty(), "no library, so no Photos guard");
        assert_eq!(plan.schedule.as_deref(), Some("daily 02:00"));
        assert!(plan.steps.as_deref().unwrap().contains("subset \"5%\""));
    }

    #[test]
    fn a_library_among_the_folders_adds_the_guard_and_skips_thumbnails() {
        let dir = pictures();
        let base = dir.path().display().to_string();
        for plan in [photos_plan(vec![base.clone()]), custom_plan(vec![base.clone()])] {
            assert_eq!(plan.before, [Check::photos_closed()]);
            assert!(plan.excludes.contains(&LIBRARY_DERIVATIVES.to_string()));
        }
        // a plain custom task without a library gets nothing extra
        assert_eq!(custom_plan(vec![format!("{base}/2024")]), Plan { folders: vec![format!("{base}/2024")], ..Plan::default() });
        // and nothing is added twice
        let mut plan = photos_plan(vec![base]);
        protect_library(&mut plan);
        assert_eq!(plan.before.len(), 1);
        assert_eq!(plan.excludes.iter().filter(|e| e.as_str() == LIBRARY_DERIVATIVES).count(), 1);
    }

    #[test]
    fn apply_plan_writes_a_config_that_loads_back_as_planned() {
        let dir = pictures();
        let base = dir.path().display().to_string();
        let plan = photos_plan(vec![base.clone(), "/Volumes/Big Drive/Raw".to_string(), "$HOME/Movies".to_string()]);
        let skeleton = "default-task \"x\"\ntasks {\n    task \"x\" {\n        backup {\n            repo \"/r/x\"\n            directory \"/d\"\n        }\n    }\n}\n";

        let kdl = apply_plan(skeleton, "photos", "$HOME/.restic/repo/photos", &plan).unwrap();
        let cfg = BackupConfig::parse("t", &kdl).unwrap_or_else(|e| panic!("{e}\n{kdl}"));
        let task = cfg.tasks.iter().find(|t| t.name == "photos").unwrap();
        let backup = task.backup().unwrap();

        assert_eq!(backup.repo(), "$HOME/.restic/repo/photos");
        assert_eq!(backup.directory(), Some(base.as_str()));
        assert_eq!(backup.include(), ["/Volumes/Big Drive/Raw", "$HOME/Movies"]);
        assert_eq!(backup.exclude(), ["**/.DS_Store", LIBRARY_DERIVATIVES]);
        assert_eq!(backup.steps().summary().as_deref(), Some("check weekly (5%) \u{b7} forget weekly"));
        assert_eq!(task.before_commands(), ["! pgrep -x Photos"]);
        assert!(kdl.contains("before \"! pgrep -x Photos\" reason=\"Photos is open\" fix=\"Quit Photos (Cmd+Q)\""), "{kdl}");
        assert_eq!(task.schedule().map(|w| w.to_string()).as_deref(), Some("daily 02:00"));
        // the task that was already there is untouched
        assert_eq!(cfg.tasks.iter().find(|t| t.name == "x").unwrap().backup().unwrap().directory(), Some("/d"));
    }

    #[test]
    fn apply_plan_with_just_folders_adds_just_folders() {
        let skeleton = "default-task \"x\"\ntasks {\n    task \"x\" {\n    }\n}\n";
        let kdl = apply_plan(skeleton, "docs", "/r/docs", &custom_plan(vec!["/a".into(), "/b".into()])).unwrap();
        let cfg = BackupConfig::parse("t", &kdl).unwrap();
        let task = cfg.tasks.iter().find(|t| t.name == "docs").unwrap();
        assert_eq!(task.backup().unwrap().directory(), Some("/a"));
        assert_eq!(task.backup().unwrap().include(), ["/b"]);
        assert!(task.schedule().is_none() && task.before_commands().is_empty());

        // no folders at all still makes a valid, empty backup task
        let kdl = apply_plan(skeleton, "later", "/r/later", &custom_plan(Vec::new())).unwrap();
        assert!(BackupConfig::parse("t", &kdl).is_ok());
    }

    #[test]
    fn apply_plan_quotes_awkward_commands_and_paths() {
        let skeleton = "default-task \"x\"\ntasks {\n    task \"x\" {\n    }\n}\n";
        let plan = Plan { folders: vec!["/My \"Pics\"".into(), "/b".into()], before: vec![Check { command: "echo \"hi\" && true".into(), ..Check::default() }], ..Plan::default() };
        let kdl = apply_plan(skeleton, "q", "/r", &plan).unwrap();
        let cfg = BackupConfig::parse("t", &kdl).unwrap_or_else(|e| panic!("{e}\n{kdl}"));
        let task = cfg.tasks.iter().find(|t| t.name == "q").unwrap();
        assert_eq!(task.backup().unwrap().directory(), Some("/My \"Pics\""));
        assert_eq!(task.before_commands(), ["echo \"hi\" && true"]);
    }

    #[test]
    fn the_new_task_is_laid_out_like_one_written_by_hand() {
        let skeleton = "default-task \"x\"\ntasks {\n    task \"x\" {\n        backup {\n            repo \"/r/x\"\n        }\n    }\n}\n";
        let plan = photos_plan(vec!["/p".to_string(), "$HOME/Raw".to_string()]);
        let kdl = apply_plan(skeleton, "photos", "/r/photos", &plan).unwrap();

        let expected = concat!(
            "    task \"x\" {\n        backup {\n            repo \"/r/x\"\n        }\n    }\n",
            "    task \"photos\" {\n",
            "        schedule \"daily 02:00\"\n",
            "        backup {\n",
            "            repo \"/r/photos\"\n",
            "            directory \"/p\"\n",
            "            include \"$HOME/Raw\"\n",
            "            exclude \"**/.DS_Store\"\n",
            "            steps {\n",
            "                check { every \"weekly\"; subset \"5%\" }\n",
            "                forget { every \"weekly\" }\n",
            "            }\n",
            "        }\n",
            "    }\n",
            "}\n",
        );
        assert!(kdl.ends_with(expected), "{kdl}");
        assert!(kdl.starts_with("default-task \"x\"\ntasks {\n"), "the rest of the file is untouched");
        assert!(BackupConfig::parse("t", &kdl).is_ok());
    }

    #[test]
    fn an_empty_tasks_block_and_a_taken_name_are_handled() {
        let empty = "default-task \"x\"\ntasks {\n}\n";
        let kdl = apply_plan(empty, "docs", "/r/docs", &custom_plan(vec!["/d".into()])).unwrap();
        assert!(kdl.contains("\n    task \"docs\" {\n        backup {\n            repo \"/r/docs\"\n"), "{kdl}");
        assert!(BackupConfig::parse("t", &kdl).is_ok());

        let err = apply_plan(&kdl, "docs", "/r/other", &custom_plan(Vec::new())).unwrap_err();
        assert!(err.contains("task 'docs' already exists"), "{err}");
        assert!(apply_plan("default-task \"x\"\n", "docs", "/r", &Plan::default()).unwrap_err().contains("'tasks' block"));
    }

    #[test]
    fn exclude_file_is_written_when_set() {
        let plan = Plan { folders: vec!["/d".into()], exclude_file: Some("$HOME/.vivoexclude".into()), ..Plan::default() };
        let kdl = apply_plan("default-task \"x\"\ntasks {\n}\n", "t", "/r", &plan).unwrap();
        let cfg = BackupConfig::parse("t", &kdl).unwrap();
        assert_eq!(cfg.tasks[0].backup().unwrap().exclude_file(), Some("$HOME/.vivoexclude"));
    }

    #[test]
    fn the_export_command_is_the_one_osxphotos_documents() {
        assert_eq!(
            export_command("$HOME/Pictures/Photos Library.photoslibrary", "$HOME/Photos Export"),
            "osxphotos export \"$HOME/Photos Export\" --library \"$HOME/Pictures/Photos Library.photoslibrary\" --update --directory \"{created.year}/{created.mm}\" --sidecar xmp"
        );
    }

    #[test]
    fn an_export_plan_backs_up_the_export_and_runs_it_first() {
        let lib = "$HOME/Pictures/Photos Library.photoslibrary";
        let plan = photos_export_plan(lib, "$HOME/Photos Export", vec!["/Volumes/Raw".into(), "$HOME/Photos Export".into(), "/Volumes/Raw".into()]);

        assert_eq!(plan.folders, ["$HOME/Photos Export", "/Volumes/Raw"], "the export first; no duplicates");
        assert_eq!(plan.output_dir.as_deref(), Some("$HOME/Photos Export"));
        assert_eq!(plan.before.len(), 1);
        assert_eq!(plan.before[0].command, export_command(lib, "$HOME/Photos Export"));
        assert!(!plan.guards_photos(), "the export reads the library itself; no need to wait for Photos to close");
        assert_eq!(plan.excludes, ["**/.DS_Store"]);
        assert_eq!(plan.schedule.as_deref(), Some("daily 02:00"));
        assert!(plan.steps.is_some());
    }

    #[test]
    fn an_export_plan_leaves_the_library_out_of_the_other_folders() {
        // Pictures holds the library; the export is its copy, so the bundle itself is not backed up
        let plan = photos_export_plan("$HOME/Pictures/Photos Library.photoslibrary", "/x/export", vec!["$HOME/Pictures".into()]);
        assert_eq!(plan.excludes, ["**/.DS_Store", "**/*.photoslibrary"]);

        let dir = pictures();
        let base = dir.path().display().to_string();
        let plan = photos_export_plan("/elsewhere/Photos Library.photoslibrary", "/x/export", vec![base]);
        assert!(plan.excludes.contains(&"**/*.photoslibrary".to_string()), "a library found inside is left out too");
    }

    #[test]
    fn an_export_task_is_written_with_its_command_reason_and_fix() {
        let lib = "$HOME/Pictures/Photos Library.photoslibrary";
        let plan = photos_export_plan(lib, "$HOME/Photos Export", Vec::new());
        let kdl = apply_plan("default-task \"x\"\ntasks {\n}\n", "photos", "/r/photos", &plan).unwrap();
        let cfg = BackupConfig::parse("t", &kdl).unwrap_or_else(|e| panic!("{e}\n{kdl}"));
        let task = &cfg.tasks[0];

        // the command carries double quotes and braces: it must read back exactly as written
        assert_eq!(task.before_commands(), [export_command(lib, "$HOME/Photos Export").as_str()]);
        assert_eq!(task.backup().unwrap().directory(), Some("$HOME/Photos Export"));
        assert!(kdl.contains("reason=\"Exporting your Photos library failed\""), "{kdl}");
        assert!(kdl.contains("fix=\"Run the export by hand"), "{kdl}");
        assert!(!kdl.contains("pgrep"), "{kdl}");
    }

    #[test]
    fn tool_lookup_follows_path() {
        let _guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("osxphotos-fake"), "").unwrap();
        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{original}", dir.path().display()));
        let (present, absent) = (tool_on_path("osxphotos-fake"), tool_on_path("definitely-not-a-tool-9f3a"));
        std::env::set_var("PATH", original);
        assert!(present && !absent);
    }

    #[test]
    fn a_both_plan_backs_up_the_library_and_an_export_in_one_task() {
        let dir = pictures();
        let pics = dir.path().display().to_string();
        let lib = format!("{pics}/Photos Library.photoslibrary");
        let plan = photos_both_plan(&lib, "$HOME/Photos Export", vec![pics.clone()]);

        assert_eq!(plan.folders, [pics.as_str(), "$HOME/Photos Export"], "the folders, and the export beside them");
        assert_eq!(plan.output_dir.as_deref(), Some("$HOME/Photos Export"));
        assert_eq!(plan.before.len(), 2);
        assert_eq!(plan.before[0].command, export_command(&lib, "$HOME/Photos Export"), "the export runs first");
        assert!(plan.before[0].skip.is_none(), "and nothing it does needs Photos closed");
        assert_eq!(plan.before[1].command, PHOTOS_OPEN_GUARD);
        assert_eq!(plan.before[1].skip.as_deref(), Some("**/*.photoslibrary"), "with Photos open only the library is left out");
        assert_eq!(plan.before.iter().filter(|c| c.command == PHOTOS_OPEN_GUARD).count(), 1, "no second, hard guard");
        assert!(plan.excludes.contains(&LIBRARY_DERIVATIVES.to_string()), "thumbnails are still left out of the library copy");
        assert!(plan.guards_photos());
        assert!(plan.steps.is_some() && plan.schedule.is_some());
    }

    #[test]
    fn a_both_plan_without_a_library_in_the_folders_has_no_photos_guard() {
        let plan = photos_both_plan("$HOME/Pictures/Photos Library.photoslibrary", "/x/export", vec!["/nowhere/Movies".into()]);
        assert_eq!(plan.folders, ["/nowhere/Movies", "/x/export"]);
        assert_eq!(plan.before.len(), 1, "just the export");
        assert!(!plan.guards_photos());
        // an export folder that is already one of the folders is not listed twice
        let twice = photos_both_plan("/l/Photos Library.photoslibrary", "/x/export", vec!["/x/export".into()]);
        assert_eq!(twice.folders, ["/x/export"]);
    }

    #[test]
    fn a_both_task_is_written_with_its_soft_guard() {
        let dir = pictures();
        let pics = dir.path().display().to_string();
        let lib = format!("{pics}/Photos Library.photoslibrary");
        let plan = photos_both_plan(&lib, "$HOME/Photos Export", vec![pics]);
        let kdl = apply_plan("default-task \"x\"\ntasks {\n}\n", "photos", "/r/photos", &plan).unwrap();
        let cfg = BackupConfig::parse("t", &kdl).unwrap_or_else(|e| panic!("{e}\n{kdl}"));

        assert_eq!(cfg.tasks[0].before_commands().len(), 2);
        assert!(kdl.contains("before \"! pgrep -x Photos\" reason=\"Photos is open\" fix=\"Quit Photos (Cmd+Q)\" skip=\"**/*.photoslibrary\""), "{kdl}");
        assert_eq!(cfg.tasks[0].backup().unwrap().include(), ["$HOME/Photos Export"]);
    }
}
