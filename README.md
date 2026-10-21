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
git clone https://github.com/dantuck/vivo
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
vivo schedule add   # run it every day
vivo status         # is it working?
vivo restore        # get files back
```

vivo exits non-zero if the backup fails or any remote fails to sync, so schedulers can alert on it.

## Scheduling

Each task can have its own schedule, written in the config next to its `backup` block:

```kdl
task "filecabinet" {
    schedule "daily 02:00"
    backup { ... }
}
task "photos" {
    schedule "weekly sun 03:30"
    backup { ... }
}
```

```sh
vivo schedule apply            # make the installed schedules match the config
vivo schedule add --task photos --at 03:30 --weekly sun   # writes the schedule node, then installs it
vivo schedule status           # what is installed, and does it match the config?
vivo schedule remove --task photos   # remove one (and its config node); no --task removes them all
vivo schedule add              # the original: the default task, daily at 02:00 (asks on a terminal)
```

`vivo schedule` sets up a launchd agent on macOS, a systemd user timer on Linux, or a crontab entry where systemd isn't running, one per scheduled task. Each runs `vivo <task>`, so that task's [`steps`](#backing-up-photos-and-other-big-media) cadence applies. It handles the details that make hand-written jobs fail: the full path to `vivo`, a `PATH` that includes `restic`, `sops` and your sync tool, your custom config/key locations, and a log file (`~/.local/state/vivo/vivo.log`). `vivo init` offers a daily schedule as its last step. Use `--print` with `add` or `apply` to see exactly what would be installed.

A schedule is `daily HH:MM` or `weekly <day> HH:MM` (`sun`, `mon`, ...). If you rename a task, run `vivo schedule apply`: it removes the old unit and installs the new one.

On macOS, grant the `vivo` binary Full Disk Access (System Settings → Privacy & Security) if you back up Documents, Desktop or other protected folders; anything vivo can't read is skipped with a warning and the rest of the backup still completes. A Mac that is asleep at the scheduled time runs the backup when it wakes.

## Backing up photos and other big media

Photos and video are large and change rarely, so the right rhythm differs from documents: back up often locally, verify and push to remotes less often, and keep long transfers out of working hours. Give such a bucket its own task, repository and (optionally) password:

```kdl
task "photos" {
    schedule "daily 02:00"
    before "! pgrep -x Photos"          // see the Photos library note below
    backup {
        repo "$HOME/.restic/repo/photos"
        password-profile "photos"
        include "$HOME/Pictures/Archive"
        include "$HOME/Movies/Raw"
        exclude "**/*.tmp"
        exclude "**/Thumbnails"
        steps {
            check  { every "weekly"; subset "5%" }
            forget { every "weekly" }
            sync   { every "daily"; window "01:00-06:00" }
        }
        remote "rustfs:https://host/photos" {
            credentials "rustfs"
            limit-upload "20MiB"
        }
        remote "b2:my-photos:photos" { credentials "b2" }
    }
}
```

- **Folders.** `include` and `exclude` pick exactly what goes in, alongside `directory`, `files-from` and `exclude-file`.
- **When each step runs.** `backup` always runs. `check`, `forget` and `sync` can each have `every` (`hourly`, `daily`, `weekly`, `monthly`, or `12h`, `3d`, `2w`); a step that isn't due is skipped and says so. `check` can also have `subset` (`5%`, `1/10` or `2G`), which reads back that much of the data (`restic check --read-data-subset`) instead of only checking structure. A new remote syncs immediately, whatever `every` says. `vivo status` allows for a slow sync cadence. Pass `--force` to run everything once.
- **Long uploads.** `sync { window "01:00-06:00" }` starts a sync only inside that window (a window like `22:00-06:00` crosses midnight) and stops it when the window closes. The next run picks up where it left off, so a first upload of a big library finishes over several nights. A paused sync is not a failure: `vivo status` shows it as paused. `limit-upload` (above) caps the speed.
- **Its own key.** `password-profile "photos"` uses `passwords.photos` from `vivo secrets edit`, so a leak of this one password exposes only this bucket. Set it before the first backup: the profile selects a password, it does not change an existing repository's.

Everything is encrypted before it leaves your machine, with or without a profile. The trade-offs worth knowing: a remote only ever holds opaque data, so nothing there can preview your photos (browse them with `vivo mount` or restore them); losing a password means losing that bucket, so store each one with your age key; and restoring a single photo from a remote downloads whole pack files, which costs time and, on some hosts, egress. Verify and browse the local repository routinely and save the remotes for disasters.

### The macOS Photos library

`~/Pictures/Photos Library.photoslibrary` is a package, and its `database/` changes constantly, so a snapshot taken while Photos is open can be inconsistent. Either:

- **Back up the library with Photos closed.** Include `originals/` and `database/`, and refuse to run while Photos is open with a `before` guard (a failing `before` stops the whole task and shows up in `vivo status`):

  ```kdl
  before "! pgrep -x Photos"
  backup {
      include "$HOME/Pictures/Photos Library.photoslibrary/originals"
      include "$HOME/Pictures/Photos Library.photoslibrary/database"
      exclude "**/resources/derivatives"   // thumbnails Photos rebuilds
      ...
  }
  ```

- **Or back up exports.** A folder export (for example `osxphotos export`) is plain files, can be backed up at any time, and is the easiest to browse after a restore, at the cost of albums and edits unless you export them too.

Originals alone are always recoverable as ordinary files, but without `database/` you lose albums, edits and metadata. If "Download Originals to this Mac" is off, `originals/` is incomplete: only what is on this Mac is backed up.

## Verifying and browsing backups

```sh
vivo verify photos                  # read back a 5% sample of the local repository
vivo verify photos --remotes        # the same, on every off-site copy
vivo verify photos --from b2:my-photos:photos --sample 1/10
vivo verify --quick                 # structure only, reads no data;  --full reads everything

vivo mount ~/mnt/photos --task photos                       # the local repository, read-only
vivo mount ~/mnt/photos --task photos --from rustfs:https://host/photos
```

`vivo verify` exits non-zero if any repository fails, so it works from a schedule or hook. Checking a remote downloads data, so keep `--sample` small. `vivo mount` needs FUSE (macFUSE or FUSE-T on macOS). Inside the mount, browse `snapshots/latest/` or `ids/<snapshot id>/`; `--host` and `--snapshot-path` only filter which snapshots appear.

## Is it working? — `vivo status`

```sh
vivo status                # latest snapshot, last sync per remote, schedule
vivo status --max-age 24   # flag anything older than 24 hours (default 48)
```

`status` reads the newest snapshot from each local repository and the result of the last sync to each remote, and exits non-zero when something is stale, failed, or has never run, so you can also call it from a monitor.

To be told when a scheduled run fails, add hooks to a task. They run with `VIVO_TASK`, `VIVO_STATUS` (`success`/`failure`) and `VIVO_ERROR` set, and are skipped on dry runs:

```kdl
task "backup" {
    calls "docs"
    on-failure "osascript -e 'display notification \"Backup failed: $VIVO_ERROR\" with title \"vivo\"'"
    on-success "curl -fsS https://hc-ping.com/your-uuid"
}
```

## Restoring

```sh
vivo snapshots [TASK]            # list what's in the local repository (--remote for off-site copies)
vivo restore                     # guided: pick a task, snapshot and destination
vivo restore docs --path /Users/me/Documents/taxes   # just part of a backup
vivo restore docs --snapshot 1a2b3c4d --target ~/old # a specific snapshot, somewhere specific
vivo restore docs --from rustfs:https://host/bucket  # straight from an off-site copy
```

Restores go to `~/vivo-restore/<task>` unless you choose otherwise, never over your live files, and keep each file's original folder structure beneath that directory. If the local repository is gone (a lost or new machine), `restore` reads from your off-site remote automatically.

### Restoring on a new machine

Every time vivo syncs to a remote it also keeps a small **recovery copy** next to the repository: your `secrets.yaml` (already encrypted by sops/age) and your `backup.kdl` (encrypted with your age key). It is only re-uploaded when one of them changes. To rebuild a lost machine you need just two things kept somewhere else — your **age key** and **access to the remote** (you can issue a new access key from the provider's console):

1. Install vivo, `restic`, `sops` and `age`.
2. Put your age key back at `~/.config/sops/age/keys.txt`.
3. `vivo recover --from <remote URL>` — asks for the remote's access key, downloads and decrypts the recovery copy, and writes `~/.config/vivo/backup.kdl` and `secrets.yaml` (the restic password and every credentials profile come back with them).
4. `vivo restore` — with no local repository yet, it reads from the off-site copy.

Where the recovery copy lives (a sibling of the repository, because vivo's sync tools delete anything extra inside it):

| Remote | Recovery copy |
|---|---|
| `b2:bucket:folder` | `folder.vivo-recovery/` in the same bucket (a repo at the bucket root has no safe place; vivo says so) |
| `rustfs:https://host/bucket/sub` | `sub.vivo-recovery/` in the same bucket |
| `rustfs:https://host/bucket` | a separate `bucket-vivo-recovery` bucket (created for you) |
| `s3:https://host/bucket[/path]` | `path.vivo-recovery/` (or `vivo-recovery/` at the bucket root) |

A failed or impossible recovery upload only warns — it never fails the backup — and `vivo status` shows it until fixed. To opt a remote out (for example a shared bucket):

```kdl
remote "s3:https://s3.amazonaws.com/shared" {
    credentials "aws"
    recovery false
}
```

Keep your age key and restic password in a password manager or somewhere else off this computer: they are the two things vivo cannot recover for you.

## Usage

```
Usage: vivo [OPTIONS] [task_name] [COMMAND]

Commands:
  init       Set up vivo for first use
  status     Show when each task last backed up and synced, and flag problems
  schedule   Run backups automatically, one schedule per task
  snapshots  List the snapshots in a task's backup repository
  restore    Restore files from a backup
  verify     Check that a backup repository is intact by reading data back
  recover    Rebuild config and secrets on a new machine from a remote
  config     Manage backup configuration
  secrets    Manage encrypted secrets
  task       Manage backup tasks
  remote     Manage backup remotes
  doctor     Check system health
  mount      Mount a backup repository read-only as a FUSE filesystem
  manage     Open the interactive config manager (TUI)
  update     Update vivo to the latest release

Arguments:
  [task_name]  Optional task name to run (default: value of default-task in config)

Options:
  -c, --config <FILE>            Use a custom config file
  -d, --debug...                 Enable debug logging
      --dry-run                  Dry run: skip writes and remote sync
  -S, --start-step <STEP>        Skip earlier steps; start at: backup, check, forget, sync
      --force                    Run every step, ignoring each backup's `steps` cadence and sync window
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
| **Fields** | `↑↓` select field · `Enter`/`e` edit field (including **Schedule** and **Password profile**) |
| **Remotes** | `a` add · `d` delete · `e` edit · `t` test connection |
| **Calls** | `a` add · `d` delete · `Ctrl+↑↓` reorder |

Settings without a row of their own (`include`, `exclude`, `before`, `steps`, and a remote's `limit-upload`) are listed read-only under **Also set**; press `o` to change them in your editor. Editing the **Schedule** row only changes the config: run `vivo schedule apply` to install it. The **Password profile** row picks which entry under `passwords:` in the secrets is used, and it must match what the repository was created with.

The default task (run when no task name is given) is marked with `*` in the Tasks pane. Leaving the repo path blank when adding a task creates a calls-only orchestrator task with no backup block.

### Step control

Use `--start-step` / `-S` to resume a backup from a specific phase without re-running earlier ones:

```sh
vivo -S sync        # skip backup/check/forget, only sync to remotes
vivo -S forget      # skip backup/check, run forget then sync
```

Steps in order: `backup` → `check` → `forget` → `sync`

A backup can also run its slower steps less often (see [`steps`](#backing-up-photos-and-other-big-media)). `--force`, or naming a step with `-S`, ignores that cadence for one run.

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
| `include "path"` | Another path to back up (repeatable; combined with `directory` and `files-from`) |
| `exclude "pattern"` | Pattern to leave out, passed to `restic --exclude` (repeatable) |
| `exclude-file` | File containing exclude patterns |
| `steps` | How often `check`, `forget` and `sync` run, and when `sync` may run (see below) |
| `password-profile "name"` | Use `passwords.<name>` from the secrets instead of the global restic password |
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
| `schedule "..."` | Run this task automatically: `daily HH:MM` or `weekly <day> HH:MM` (see [Scheduling](#scheduling)) |
| `before "..."` | Shell command run before the backup (repeatable, via `sh -c`); if one fails, nothing is backed up and `on-failure` runs. Skipped on dry runs |
| `command "..."` | Shell command to run after backup (via `sh -c`) |
| `calls "name"` | Run another named task; circular references are detected and skipped |
| `on-success "..."` | Shell command run when the task finishes without errors (skipped on dry runs) |
| `on-failure "..."` | Shell command run when the task or one of its remotes fails (skipped on dry runs) |

## Secrets

Secrets are stored in a SOPS/age-encrypted YAML file. Default path: `~/.config/vivo/secrets.yaml`. Override with `VIVO_BACKUP_SECRETS`.

```yaml
restic_password: "your-restic-repo-password"
passwords:                       # optional: more repository passwords, one per `password-profile`
  photos: "a-different-repo-password"
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

### Limiting upload speed

`limit-upload` caps how fast a remote is written, e.g. so a first photo upload doesn't saturate your connection:

```kdl
remote "rustfs:https://host/photos" {
    credentials "rustfs"
    limit-upload "20MiB"    // per second; 500k, 5M, 2MB/s also work (units are binary)
}
```

It works for `s3:` remotes (`restic --limit-upload`) and `rustfs:` remotes (`mc` or `rclone`; the `aws` fallback can't throttle and says so). `b2:` remotes reject it, because the `b2` CLI has no bandwidth option. Use a sync window instead. It replaces `mc-limit-upload`, which still works on its own.

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
