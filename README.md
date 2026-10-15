# Vivo [![Latest Version]][crates.io]

[Latest Version]: https://img.shields.io/crates/v/vivo.svg
[crates.io]: https://crates.io/crates/vivo

<a href="https://www.buymeacoffee.com/dantuck" target="_blank"><img src="https://cdn.buymeacoffee.com/buttons/default-orange.png" alt="Buy Me A Coffee" style="height: 51px !important;width: 217px !important;" ></a>

Vivo is a CLI wrapper around [restic](https://restic.net) that orchestrates backups with sync to multiple remote backends. It uses [SOPS](https://github.com/getsops/sops) with [age](https://age-encryption.org) to keep credentials encrypted at rest.

## Prerequisites

- [restic](https://restic.net) — backup engine
- [sops](https://github.com/getsops/sops) — secrets encryption
- [age](https://age-encryption.org) — encryption key management
- [b2](https://github.com/Backblaze/B2_Command_Line_Tool) — required only for Backblaze B2 remotes (`pip install b2`)

## Install

### One-line install (Linux and macOS)

```sh
curl -sSf https://raw.githubusercontent.com/dantuck/vivo/main/install.sh | sh
```

Downloads the latest binary from GitHub Releases, verifies the SHA256 checksum, and installs to `/usr/local/bin/vivo` (or `~/.local/bin` if not writable). Set `VIVO_INSTALL_DIR` to override the destination.

### Self-update

```sh
vivo update          # update to latest release
vivo update --dry-run  # preview without applying
```

vivo also checks for updates in the background once per 24 hours and prints a notice after a backup run if a newer version is available.

### From Cargo

```sh
cargo install vivo
```

### Build from Source

```sh
git clone https://codeberg.org/tuck/vivo
cd vivo
cargo build --release
cp target/release/vivo /usr/local/bin/
```

## Quick Start

```sh
vivo init
```

`vivo init` is a guided setup. On a terminal it walks through:

1. **Prerequisites** — checks for `restic`, `sops` and an `age` key. With Homebrew it offers to install anything missing, and it offers to create the age key for you. (Keep a copy of that key; without it your secrets can't be decrypted.)
2. **What to back up** — the directory and where the local restic repository lives. It warns before backing up your whole home directory and if the directory doesn't exist.
3. **Restic password** — prompted and saved to the encrypted secrets file. vivo refuses to back up while it is still the `change-me` placeholder, so a repo can never be created with it.
4. **Off-site remote** — pick RustFS, S3, or Backblaze B2, enter a validated URL, and choose or create a credentials profile (keys are stored encrypted). The remote is tested right away; for RustFS vivo offers to create a missing bucket, and for `s3:` it offers `restic init`.
5. **First backup** — a dry-run preview, then an offer to run the real backup.

Not on a terminal (CI, piped input)? `init` writes a static starter config instead and prompts for nothing. Each step is also available on its own:

```sh
vivo remote add     # guided: backend, URL, credentials, then a connection test
vivo remote test    # re-test every configured remote (or --task NAME)
vivo doctor         # check tools, config, secrets, and remote connectivity
vivo --dry-run      # preview the backup without changing anything
vivo                # run the backup
```

vivo exits non-zero if the backup fails or any remote fails to sync, so schedulers can alert on it.

## Scheduling

vivo runs once and exits; schedule it to back up automatically. Use the full path to `vivo` (`which vivo`) and `vivo --dry-run` first to confirm it works non-interactively. Scheduled jobs don't get your shell's `PATH`, so make sure `restic`, `sops`, and any sync tool (`mc`, `b2`) are findable — the examples below set it.

### macOS (launchd)

Save as `~/Library/LaunchAgents/dev.vivo.backup.plist` (change the path, and add `/opt/homebrew/bin` if you installed tools with Homebrew), then run `launchctl load ~/Library/LaunchAgents/dev.vivo.backup.plist`:

```xml
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>dev.vivo.backup</string>
  <key>ProgramArguments</key><array><string>/usr/local/bin/vivo</string></array>
  <key>EnvironmentVariables</key>
  <dict><key>PATH</key><string>/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin</string></dict>
  <key>StartCalendarInterval</key><dict><key>Hour</key><integer>2</integer><key>Minute</key><integer>0</integer></dict>
  <key>StandardOutPath</key><string>/tmp/vivo.log</string>
  <key>StandardErrorPath</key><string>/tmp/vivo.log</string>
</dict>
</plist>
```

### Linux (systemd user timer)

`~/.config/systemd/user/vivo.service`:

```ini
[Unit]
Description=vivo backup

[Service]
Type=oneshot
ExecStart=%h/.local/bin/vivo
```

`~/.config/systemd/user/vivo.timer`:

```ini
[Unit]
Description=Daily vivo backup

[Timer]
OnCalendar=daily
Persistent=true

[Install]
WantedBy=timers.target
```

Enable with `systemctl --user enable --now vivo.timer`. A failed run shows in `systemctl --user status vivo.service`.

### cron

```cron
0 2 * * * PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin /usr/local/bin/vivo >> $HOME/vivo.log 2>&1
```

## Usage

```
Usage: vivo [OPTIONS] [task_name] [COMMAND]

Commands:
  init     Set up vivo for first use
  config   Manage backup configuration
  secrets  Manage encrypted secrets

Arguments:
  [task_name]  Optional task name to run (default: value of default-task in config)

Options:
  -c, --config <FILE>            Use a custom config file
  -d, --debug...                 Enable debug logging
      --dry-run                  Dry run: skip writes and remote sync
  -S, --start-step <STEP>        Skip earlier steps; start at: backup, check, forget, sync
  -h, --help                     Print help
  -V, --version                  Print version
```

### Config subcommands

```
vivo config init    # Create ~/.config/vivo/backup.kdl from template
vivo config edit    # Open config in $EDITOR (creates from template if absent)
vivo config show    # Print config to stdout
```

### Secrets subcommands

```
vivo secrets init   # Create and encrypt ~/.config/vivo/secrets.yaml
vivo secrets edit   # Edit secrets with sops
vivo secrets show   # Print secrets with values masked
```

### Interactive TUI

```sh
vivo manage
```

Opens a full-screen config manager with four navigable panes (Tab to cycle):

| Pane | Keys |
|---|---|
| **Tasks** | `a` add · `d` delete · `e` edit all fields · `s` set as default · `o` open in `$EDITOR` |
| **Fields** | `↑↓` select field · `Enter`/`e` edit field |
| **Remotes** | `a` add · `d` delete · `e` edit · `t` test connection |
| **Calls** | `a` add · `d` delete · `Ctrl+↑↓` reorder |

The default task (run when no task name is given) is marked with `*` in the Tasks pane. Leaving the repo path blank when adding a task creates a calls-only orchestrator task with no backup block.

### Step control

Use `--start-step` / `-S` to resume a backup from a specific phase without re-running earlier ones:

```sh
vivo -S sync        # skip backup/check/forget, only sync to remotes
vivo -S forget      # skip backup/check, run forget then sync
```

Steps in order: `backup` → `check` → `forget` → `sync`

## Configuration

Config is stored in [KDL](https://kdl.dev) format. Default path: `~/.config/vivo/backup.kdl`. Override with `VIVO_BACKUP_CONFIG` or `-c`.

```kdl
default-task "backup"

tasks {
    task "backup" {
        description "Main backup task"
        backup {
            repo "$HOME/.local/share/restic/main"
            directory "$HOME"
            exclude-file "$HOME/.config/vivo/excludes"

            retention {
                daily 7
                weekly 5
                monthly 12
                yearly 2
            }

            remote "s3:https://s3.amazonaws.com/my-bucket" {
                credentials "aws"
            }
            remote "b2:my-bucket:restic/main" {
                credentials "b2"
            }
        }

        command "notify-send 'Backup complete'"
        calls "another-task"
    }

    task "another-task" {
        command "echo done"
    }
}
```

### Backup options

| Field | Description |
|---|---|
| `repo` | Path to local restic repository |
| `directory` | Directory to back up |
| `files-from` | File containing list of paths to back up (alternative to `directory`) |
| `exclude-file` | File containing exclude patterns |
| `dry-run true` | Per-task dry run override |
| `remote` | One or more remote destinations (see below) |
| `retention` | Snapshot retention policy |

### Retention defaults

| Field | Default |
|---|---|
| `daily` | 7 |
| `weekly` | 5 |
| `monthly` | 12 |
| `yearly` | 2 |

### Task options

| Field | Description |
|---|---|
| `command "..."` | Shell command to run after backup (via `sh -c`) |
| `calls "name"` | Run another named task; circular references are detected and skipped |

## Secrets

Secrets are stored in a SOPS/age-encrypted YAML file. Default path: `~/.config/vivo/secrets.yaml`. Override with `VIVO_BACKUP_SECRETS`.

```yaml
restic_password: "your-restic-repo-password"
credentials:
  aws:
    AWS_ACCESS_KEY_ID: "..."
    AWS_SECRET_ACCESS_KEY: "..."
    RESTIC_REPOSITORY: "s3:https://s3.amazonaws.com/my-bucket"
  b2:
    B2_ACCOUNT_ID: "..."
    B2_ACCOUNT_KEY: "..."
```

Each key under `credentials` is a named profile. The `credentials` field on a `remote` block references one of these profiles by name. All key/value pairs in the profile are injected as environment variables when the remote sync command runs.

## Remote Backends

### RustFS (and other S3-compatible servers via file sync)

Mirrors the local restic repository into a bucket using `mc` (preferred), `aws`, or `rclone`. URL format: `rustfs:<scheme>://<host>/<bucket>[/<path>]`

```kdl
remote "rustfs:https://rustfs.example.com/my-bucket" {
    credentials "rustfs"
}
```

The credentials profile needs `AWS_ACCESS_KEY_ID` and `AWS_SECRET_ACCESS_KEY`. Create one with `vivo remote add` (guided) or non-interactively:

```sh
vivo secrets import-s3 --profile rustfs --key-id <ACCESS_KEY> --key <SECRET_KEY>
vivo remote add --task backup --url rustfs:https://rustfs.example.com/my-bucket --credentials rustfs
```

The bucket is created on first sync (or earlier, by `vivo remote add` / `vivo remote test`), and no `restic init` is needed on the remote — the sync copies the local repository as-is. Test it any time with `vivo remote test`.

### S3-compatible (AWS S3, MinIO, etc.)

Uses `restic copy`. URL format: `s3:<endpoint>/<bucket>`

```kdl
remote "s3:https://s3.amazonaws.com/my-bucket" {
    credentials "aws"
}
remote "s3:http://minio.local:9000/backup" {
    credentials "minio"
}
```

The remote restic repository must be initialized before first sync. `vivo remote add` and `vivo remote test` offer to do this for you, or run it yourself:

```sh
restic init --repo s3:https://s3.amazonaws.com/my-bucket
```

### Backblaze B2

Uses the `b2 sync` CLI. URL format: `b2:<bucket>:<path>`

```kdl
remote "b2:my-bucket:restic/main" {
    credentials "b2"
}
```

## Releasing

Releases are automated via [Release Please](https://github.com/googleapis/release-please). On every merge to `main`, Release Please scans commits since the last release and opens (or updates) a release PR that bumps `Cargo.toml` and updates `CHANGELOG.md`. Merging that PR creates a version tag, which triggers the build workflow to produce binaries for Linux (x86_64, aarch64) and macOS (aarch64) and publish them as a GitHub Release.

Commit messages must follow [Conventional Commits](https://www.conventionalcommits.org/):

| Prefix | Version bump |
|---|---|
| `fix:` | patch (0.1.0 → 0.1.1) |
| `feat:` | minor (0.1.0 → 0.2.0) |
| `feat!:` or `BREAKING CHANGE:` | major (0.1.0 → 1.0.0) |

No manual tagging or version editing is required.

## License

Licensed under either of

- Apache License, Version 2.0
- MIT License

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be dual-licensed as above, without any additional terms or conditions.
