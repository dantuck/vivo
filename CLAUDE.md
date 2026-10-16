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

**Dev config:** `.cargo/config.toml` sets `VIVO_BACKUP_CONFIG=sample.kdl` so `cargo run` uses a local sample config during development.

**`--dry-run`:** Skips remote sync entirely; forwards `--dry-run` to `restic backup` and `restic forget`.

**`--start-step` / `-S`:** Skips earlier steps. Valid values: `backup`, `check`, `forget`, `sync`.

**Subcommands:**
- `vivo init` — checks prerequisites (restic, sops, age key) and bootstraps config/secrets files; on a TTY: offers to install tools and create the age key, asks what to back up, sets the restic password, adds and tests a first remote, and offers a first backup. `vivo` refuses to run with the `change-me` placeholder password, and exits non-zero when a backup or remote sync fails
- `vivo config {init,edit,show}` — manage the KDL backup config file
- `vivo secrets {init,edit,show,import-b2,import-s3}` — manage the SOPS-encrypted secrets file
- `vivo remote {add,list,test,remove}` — `add` is guided on a TTY when `--url`/`--credentials` are omitted; `test` checks every remote (or `--task`)
- `vivo manage` — full-screen TUI config manager (see below)
- `vivo mount [path]` — mount a backup repo as a FUSE filesystem

**TUI (`vivo manage`):** Four panes, Tab to cycle: Tasks → Fields → Remotes → Calls → Tasks.
- `src/tui/app.rs` — `App` state: `tasks`, `default_task`, selection indices per pane, `focused_pane`
- `src/tui/ui.rs` — ratatui rendering; Tasks list (default marked `*`), Fields (6 fixed rows), Remotes (content-sized), Calls (content-sized)
- `src/tui/events.rs` — key dispatch; `run_prompt()` suspends/resumes TUI around inquire prompts
- `src/tui/credentials.rs` — credential profile picker/creator used by remote add/edit
- `src/config_editor.rs` — pure KDL mutation functions (add/remove/edit task, remote, call; set default task); all return `Result<String, String>`
