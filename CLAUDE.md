# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
cargo build              # debug build
cargo build --release    # release build
cargo run                # run with dev config (sets VIVO_BACKUP_CONFIG=sample.kdl via .cargo/config.toml)
cargo test               # run all tests
cargo clippy             # lint
cargo test <test_name>   # run a single test
```

## Architecture

Vivo is a CLI wrapper around `restic` that orchestrates backups with sync to multiple remote backends. It requires `restic`, `sops`, and `age` to be installed; `b2` CLI is needed for Backblaze B2 remotes.

**Execution flow:**
1. `src/bin/vivo/main.rs` — entry point; dispatches subcommands or runs the backup flow. Each subcommand lives in a sibling module (`init`, `config`, `secrets`, `task`, `remote`, `onboard`); `util.rs` holds shared helpers and `Stop`/`CmdResult` (subcommands return `Err(Stop::Cancelled)` for Esc/Ctrl-C, which exits 0 quietly)
2. `src/config.rs` — `VivoConfig` (CLI args via clap) + `Secrets` (YAML); `build_cli()` defines the full clap command tree; path resolution via env vars
3. `src/backup_config/mod.rs` — loads KDL config; decrypts secrets via `sops -d`; sets `RESTIC_PASSWORD` env var; returns `(BackupConfig, Secrets)` tuple
4. `src/backup_config/task.rs` — `Task` execution with `calls` (named task references) and `commands` (shell commands); circular-reference detection
5. `src/backup_config/backup.rs` — runs `restic backup → restic check → restic forget → sync remotes` in sequence; each step gated by `config.start_step`
6. `src/remote/mod.rs` — `RemoteBackend` trait + `from_url()` factory dispatching on URL prefix (`b2:`, `s3:`, `rustfs:`)
7. `src/remote/b2.rs` — B2Backend: uses `b2 sync --delete --replace-newer --compare-versions size`
8. `src/remote/s3.rs` — S3Backend: uses `restic copy --repo <local> --to <url>` (AWS S3, MinIO, etc.). `src/remote/rustfs.rs` — RustfsBackend (`rustfs:https://host/bucket[/path]`): mirrors the local repo with `mc`/`aws`/`rclone`; the bucket is created on sync and no remote `restic init` is needed
9. `src/remote_setup.rs` — guided setup shared by `vivo init`, `vivo remote add|test`, and the TUI: `prompt_new_remote`, `guided_add`, `test_remote` (RustFS → bucket check/create; `s3:` → `restic snapshots` + offer `restic init`; B2 → `doctor` connectivity check)
10. `src/step.rs` — `Step` enum (Backup < Check < Forget < Sync) with `PartialOrd` for `--start-step` gating

**Config format:** KDL (decoded from the `kdl` crate by `src/backup_config/kdl_decode.rs`; reads KDL v2 and falls back to v1). Backup config defaults to `~/.config/vivo/backup.kdl` (override via `VIVO_BACKUP_CONFIG` env var or `-c` flag). Secrets default to `~/.config/vivo/secrets.yaml` (override via `VIVO_BACKUP_SECRETS`). Secrets must be SOPS/age-encrypted YAML.

**Secrets format:**
```yaml
restic_password: "your-restic-repo-password"
passwords:                      # optional: extra repository passwords, see `password-profile`
  photos: "a-different-repo-password"
credentials:
  aws:
    AWS_ACCESS_KEY_ID: "..."
    AWS_SECRET_ACCESS_KEY: "..."
  b2:
    B2_ACCOUNT_ID: "..."
    B2_ACCOUNT_KEY: "..."
```

**KDL config format:**
```kdl
default-task "backup"
tasks {
    task "backup" {
        backup {
            repo "$HOME/.local/share/restic/main"
            directory "$HOME"
            remote "s3:https://s3.amazonaws.com/my-bucket" {
                credentials "aws"
            }
        }
        command "notify-send 'backup complete'"
        calls "other-task"
    }
}
```

**Per-task repository password (`password-profile "<name>"` in `backup`):** uses `passwords.<name>` from the secrets instead of the global `restic_password`, so a leak of one bucket's password exposes only that bucket. `Secrets::password_for`/`for_profile` resolve it; `BackupConfig::check_password_profiles` (run by `load_config` and `repos::load`) rejects a missing, empty or `change-me` profile. `Task::run` now takes `&Secrets`; `Backup::run` sets `RESTIC_PASSWORD` for its duration through `PasswordEnv` (restored on drop, so later tasks/hooks don't see it), `repos::targets_for` puts the profile's password in each local and remote `RepoTarget`, and remote tests/doctor/TUI use the task's password (`remote_setup::password_profile_of`). A profile only selects a password: it must match what the repo was initialized with, so set it before the first backup (changing it on an existing repo locks vivo out until it matches again). Remote mirrors share the local repo's password.

**Sources and excludes:** inside `backup { }`, `directory`, `files-from` and any number of `include "<path>"` nodes are combined into one `restic backup` (all paths are env-expanded). `exclude "<pattern>"` (repeatable) becomes `--exclude`, alongside the optional `exclude-file`. A backup with none of `directory`/`files-from`/`include` fails with a clear error instead of calling restic with no paths.

**Step cadence (`steps { }` in `backup`):** `check`, `forget` and `sync` may each have `every "<hourly|daily|weekly|monthly|Nh|Nd|Nw>"`; `check` may also have `subset "<5%|1/10|2G>"` (→ `restic check --read-data-subset`). Parsed in `src/backup_config/steps.rs`. A step with no `every` runs every time; `backup` always runs. Last-success times live in `state.json` under each task's `steps` (written from `RunOutcome::steps_done`; `sync` only counts when every remote succeeded). A step is due once `every` minus up to 1h of slack has elapsed, and `sync` is also due when a remote has no recorded sync. Skipped steps print "Skipping …". `sync { window "HH:MM-HH:MM" }` (overnight spans work) limits *when* a sync runs: it only starts inside the window (`Backup::sync_deadline`; outside it the step is skipped and stays due), and `remote::run_streaming` stops the transfer tool with SIGTERM (SIGKILL after 30s) when the window closes, via a thread-local deadline set by `remote::set_deadline`. That yields `SyncResult { interrupted: true, error: "the sync window closed; …" }`: it is not counted by `RunOutcome::failed_syncs`, doesn't fail the run or record the `sync` step, and `state.json` marks the remote `interrupted` without touching `last_ok`, so `vivo status` shows "paused … resumes next run" and still judges the last complete sync by age. Every sync backend resumes incrementally, so stopping mid-transfer is the same as a dropped connection. `--force` ignores the window. `--force`, or an explicit `--start-step`, ignores the cadence (`VivoConfig::force`). Dry runs consult the cadence but record nothing. `vivo status` widens the remote "older than expected" threshold by the task's sync interval.

**Upload speed cap (`limit-upload "<rate>"` in a `remote`):** `20MiB`, `5M`, `500k` or `2MB/s` per second (every unit is binary; `remote::parse_rate_kib` turns it into KiB/s). `s3:` remotes pass `restic --limit-upload` to `restic copy`; `rustfs:` remotes pass `mc mirror --limit-upload` or `rclone --bwlimit` (the `aws` fallback has no throttle and warns); `b2:` remotes reject the setting at config load because the `b2` CLI has no bandwidth option. It replaces `mc-limit-upload` (setting both is an error; the old node still works alone).

**Dev config:** `.cargo/config.toml` sets `VIVO_BACKUP_CONFIG=sample.kdl` so `cargo run` uses a local sample config during development.

**Pre-backup guard (`before "<cmd>"` in a task, repeatable):** run via `sh -c` before the task's backup, in order (`Task::run_before`). The first non-zero exit stops the whole task — no backup, `command`s or `calls` — records the error in `state.json` (so `vivo status` shows it), fires `on-failure` with `VIVO_ERROR`, and the task returns false. Dry runs only print what they would run, since these may change things. Used for e.g. `before "! pgrep -x Photos"` so a live Photos library isn't snapshotted.

**Task hooks:** `on-success` / `on-failure` commands run after a task (not on dry runs) with `VIVO_TASK`, `VIVO_STATUS`, `VIVO_ERROR`. A failed backup step stops the task's `command`/`calls`; restic exit code 3 (some files unreadable) is a warning, not a failure (`RESTIC_PARTIAL_BACKUP` in `backup.rs`). `Backup::run` returns a `RunOutcome` that `state::record_run` persists.

**`--dry-run`:** Skips remote sync entirely; forwards `--dry-run` to `restic backup` and `restic forget`.

**`--start-step` / `-S`:** Skips earlier steps. Valid values: `backup`, `check`, `forget`, `sync`.

**Subcommands:**
- `vivo init` — checks prerequisites (restic, sops, age key) and bootstraps config/secrets files; on a TTY: offers to install tools and create the age key, asks what to back up, sets the restic password, adds and tests a first remote, and offers a first backup. `vivo` refuses to run with the `change-me` placeholder password, and exits non-zero when a backup or remote sync fails
- `vivo config {init,edit,show}` — manage the KDL backup config file
- `vivo secrets {init,edit,show,import-b2,import-s3}` — manage the SOPS-encrypted secrets file
- `vivo remote {add,list,test,remove}` — `add` is guided on a TTY when `--url`/`--credentials` are omitted; `test` checks every remote (or `--task`)
- `vivo manage` — full-screen TUI config manager (see below)
- `vivo schedule {add,apply,remove,status}` — one launchd agent / systemd user timer / crontab block per scheduled task, plus the original unnamed unit that runs the default task (`src/schedule.rs` renders and installs; `src/bin/vivo/schedule.rs` is the CLI; `init` offers the unnamed one). A task is scheduled by a `schedule "daily HH:MM"` or `schedule "weekly <day> HH:MM"` node (a bare `HH:MM` means daily) beside its `backup` block; `When` in `src/schedule.rs` parses it and `Task::schedule()` exposes it. The config is the source of truth: `apply` installs every declared schedule and removes per-task units no longer declared; `add --task T --at HH:MM [--weekly DAY]` writes the node (`config_editor::set_task_schedule`) then installs; `remove --task T` removes the unit and the node, `remove` alone removes every unit but keeps the nodes; `status` lists units and flags drift from the config. Unit names come from `slug(task)` (`dev.vivo.backup.<slug>`, `vivo-<slug>.timer`, a `[slug]`-marked cron block; `apply` rejects two tasks with the same slug). Each unit runs `vivo <task>`, so `steps` cadence applies. Scheduled jobs get an explicit `PATH` and the resolved config/secrets/age-key/state paths pinned as environment variables; `install` writes a record (`schedule.json` for the unnamed unit, `schedule-<slug>.json` otherwise) to the state dir, which `status` reads instead of parsing the rendered unit. Installing a per-task unit removes an older unnamed unit that ran the same task
- `vivo status` — newest snapshot per local repo (via `restic snapshots --json`) plus last-sync results recorded in `state.json` (`src/state.rs`, in `$VIVO_STATE_DIR` / `~/.local/state/vivo`); exits non-zero on stale/failed/never-run
- `vivo snapshots [task] [--remote]` and `vivo restore [task]` — built on `src/repos.rs` (`RepoTarget`: per-task local + remote repos with restic env; `rustfs:` → `s3:`, B2 key names mapped to `B2_ACCOUNT_*`). Restore defaults to `~/vivo-restore/<task>`, local repo first, falling back to a remote when the local repo is gone
- `vivo recover --from <url>` — rebuilds `backup.kdl` + `secrets.yaml` from the recovery copy (`src/recovery.rs`). Each real sync uploads `secrets.yaml`, an age-encrypted `backup.kdl.age` and a README to a *sibling* of the remote's repo path (never inside it: b2/mc/aws/rclone sync all delete extras). Each `RemoteBackend` decides its own location via `recovery_store()` (an `ObjectStore` in `src/remote/objects.rs`); the same trait also owns `restic_url()`, `restic_env()` (B2 key renames) and `credential_vars()`, which `repos.rs`, `mount.rs` and `recover` use instead of per-scheme branches. `recovery::Source` hashes the plaintext and runs `age` only when some remote needs an upload; an upload is skipped when the hash matches the remote's `RecoveryState` (`Current`/`Stale`/`Missing` in `state.json`); the README file is only sent on the first upload; `remote { recovery false }` opts out; failures warn and show in `vivo status`, never fail the backup
- `vivo mount [path] [--task T [--from local|URL]] [--host H] [--snapshot-path P]` — mount a backup repo read-only as a FUSE filesystem (`src/mount.rs`). With `--task` it mounts that task's local repo (or `--from <remote URL>`) with no prompts; without it, a menu of every repo. Always passes `restic mount --no-lock` (no lock file written into a remote copy). `--host` / `--snapshot-path` are restic's snapshot filters (`--host` / `--path`), not a subdirectory: restic cannot mount one snapshot, so browse `snapshots/latest/` or `ids/<id>/` inside the mount. `repos::find_source` is the shared non-interactive `local`/remote-URL picker used by mount, verify and restore
- `vivo verify [task] [--from local|URL | --remotes] [--sample 5%|--quick|--full]` — `restic check` with `--read-data-subset` (default 5%; `--quick` reads no data, `--full` is `--read-data`) on the local repo, one remote, or every remote, then prints the newest snapshot's age (`src/bin/vivo/verify.rs`). Remote checks use `--no-lock` and warn that they download data; exits non-zero if any repo fails

**TUI (`vivo manage`):** Four panes, Tab to cycle: Tasks → Fields → Remotes → Calls → Tasks.
- `src/tui/app.rs` — `App` state: `tasks`, `default_task`, selection indices per pane, `focused_pane` The Schedule and Password profile rows (`FIELD_SCHEDULE`, `FIELD_PASSWORD_PROFILE`, after `FIELD_NAMES`) are edited by their own prompts in `events.rs` (`set_task_schedule` / `set_backup_password_profile`), not through `EditTaskSpec`, so `edit_task_prompt` iterates only `FIELD_NAMES`; the TUI writes config only and never installs schedules
- `src/tui/ui.rs` — ratatui rendering; Tasks list (default marked `*`), Fields (8 fixed rows: the six `EditTaskSpec` fields, then Schedule and Password profile), an "Also set" read-only block (`TaskEntry::detail_lines`: include/exclude/before/steps, shown only when a task has any), Remotes (with their `limit-upload`), Remotes (content-sized), Calls (content-sized). All colors are the style constants at the top (`MUTED`, `FAINT`, `ACCENT`, `BORDER_IDLE`, `SELECTED`, `SELECTED_IDLE`): don't use `Color::DarkGray` for text (nearly invisible on several dark themes), and give unfocused panes a visible selection via `selection(focused)`
- `src/tui/events.rs` — key dispatch; `run_prompt()` suspends/resumes TUI around inquire prompts
- `src/tui/credentials.rs` — credential profile picker/creator used by remote add/edit
- `src/config_editor.rs` — pure KDL mutation functions (add/remove/edit task, remote, call; set default task); all return `Result<String, String>`
