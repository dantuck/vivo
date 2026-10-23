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
vivo doctor         # check tools, config, secrets, folders, and remote connectivity
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

On macOS, grant Full Disk Access (System Settings → Privacy & Security) to the app you run `vivo` from, and to the `vivo` binary itself for scheduled runs, if you back up Documents, Desktop, a Photos library or other protected folders. A Mac that is asleep at the scheduled time runs the backup when it wakes. See [A backup that captured nothing](#a-backup-that-captured-nothing) if a protected folder comes out empty.

## Backing up photos and other big media

Photos and video are large and change rarely, so the right rhythm differs from documents: back up often locally, verify and push to remotes less often, and keep long transfers out of working hours. In `vivo manage`, press `a` and choose **Photos & video** to get the setup below without writing it by hand. To write it yourself, give the bucket its own task, repository and (optionally) password:

```kdl
task "photos" {
    schedule "daily 02:00"
    before "! pgrep -x Photos" reason="Photos is open"   // see the Photos library note below
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

### Many folders

You don't need to list every folder. Pick the one that holds them all:

- **A parent folder plus `exclude`** is the best default. Point at `~/Pictures` and exclude what you don't want. New year or event folders are picked up automatically, and restic backs up identical files once.
- **A few specific folders** work as `include` lines (or the **Also back up** row in `vivo manage`). It gets tedious past about five.
- **Dozens of scattered folders**: keep their paths in a text file, one per line, and point `files-from` at it. The list lives outside the config, so a script can maintain it.

### The macOS Photos library: back up the library or an export

`~/Pictures/Photos Library.photoslibrary` is a package: a folder Photos manages, with a database that changes constantly. You can back it up in two ways, or both at once, and `vivo manage` asks which when you add a **Photos & video** task and it finds a library.

| | **The library as it is** | **An export** ([osxphotos](https://github.com/RhetTbull/osxphotos)) |
|---|---|---|
| What is backed up | the `.photoslibrary` bundle | a folder of plain files, kept current by `osxphotos export` before each backup |
| Restores into Photos on a Mac, with albums and edits | yes | no (files and metadata sidecars only) |
| Readable on any system, with real names | no: file names are UUIDs and albums live in the library's database | yes: `2026/10/IMG_1234.heic`, with an XMP sidecar per photo |
| Photos must be closed | yes, or the copy can be inconsistent | not required by vivo; osxphotos reads the library itself |
| Needs | nothing extra | osxphotos: `uv tool install osxphotos` (or `brew tap RhetTbull/osxphotos && brew install osxphotos`) |
| Extra disk space on this Mac | none | little on APFS (see below) |

Pick the library if you will only ever restore onto a Mac, the export if you want files you can open anywhere, including Linux, or **both**, which is what I would choose: the second copy costs little (below), and you get a restore that works on a Mac and one that works anywhere. Everything here needs Full Disk Access for the app running vivo, because macOS guards the library.

#### Does an export double the disk space?

Usually not, but it depends:

- **On APFS (the default macOS filesystem) osxphotos clones files while exporting**, according to its documentation. A clone shares the original's disk blocks, so the export folder takes almost no extra space until a file changes. Finder still shows it at full size; check Disk Utility or `df` for what is really used.
- **It does take real space** if the export is on a different drive, or if you add `--exiftool` or `--convert-to-jpeg`, which rewrite the files. `--export-as-hardlink` is the other way to avoid copies (same drive only, and not with `--exiftool`).
- **In the backup repository and on remotes, identical content is stored once.** An export made from clones or hardlinks is byte-identical to the originals, so it adds very little to the repository even if you back up the library too.

These are the documented behaviours; vivo does not measure them. Compare the sizes on your own library before relying on them.

#### Setting it up

In `vivo manage` (`a`, then **Photos & video**) choose **An export** and vivo writes the task for you. By hand it is a `before` step that runs the export, and the export folder as the thing to back up:

```kdl
task "photos" {
    schedule "daily 02:00"
    before "osxphotos export \"$HOME/Photos Export\" --library \"$HOME/Pictures/Photos Library.photoslibrary\" --update --directory \"{created.year}/{created.mm}\" --sidecar xmp" reason="Exporting your Photos library failed" fix="Run the export by hand to see why"
    backup {
        repo "$HOME/.restic/repo/photos"
        directory "$HOME/Photos Export"
        exclude "**/.DS_Store"
        steps {
            check  { every "weekly"; subset "5%" }
            forget { every "weekly" }
        }
    }
}
```

`--update` makes each run export only what is new or changed. If the export fails (osxphotos not installed, or macOS blocking the library), the backup does not run and vivo says why, with osxphotos' own output above it. Scheduled runs find `osxphotos` the same way they find `restic`, and on a Mac `vivo doctor` warns, with the install command, if a task uses it and it isn't installed (it doesn't check on other systems, where a restored Photos task isn't one you would run). The export step is not checked against a real osxphotos by vivo's tests, which use a stand-in program: if its options change, adjust the command.

#### Doing both at once

Choosing **Both** makes one task with one repository that backs up your folders (including the library) and the export folder. Because the export's files are the same content as the library's, the repository stores them once, so it grows very little. It writes two `before` steps:

```kdl
before "osxphotos export ... --update ..." reason="Exporting your Photos library failed" fix="..."
before "! pgrep -x Photos" reason="Photos is open" fix="Quit Photos (Cmd+Q)" skip="**/*.photoslibrary"
```

The export runs first and never waits for Photos. The second check is a **soft** one (`skip=`): the library should only be copied while Photos is closed, but Photos being open should not cost you the export or your other folders. So with Photos open, vivo leaves just the library out of that run, says so, and backs up the rest:

```
[i] Part of 'photos' is being left out this time.

    Blocked by       Photos is open
    Left out         **/*.photoslibrary (this run only)
    Still backed up  everything else in this task
    To include it    Quit Photos (Cmd+Q), then run `vivo photos` again
```

The run succeeds, and `vivo status` shows "Photos is open, so **/*.photoslibrary was left out of this backup" until the next run that includes it. If Photos is left open every night, the export keeps backing up but the library never does, so quit it now and then.

#### Backing up the library as it is

Refuse to run while Photos is open with a `before` guard (a failing `before` stops the whole task and shows up in `vivo status`):

```kdl
before "! pgrep -x Photos" reason="Photos is open" fix="Quit Photos (Cmd+Q)"
backup {
    directory "$HOME/Pictures"
    exclude "**/*.photoslibrary/resources/derivatives"   // thumbnails Photos rebuilds
    ...
}
```

When the guard holds a run back (Photos is open, perhaps at the scheduled hour), vivo explains instead of printing an error:

```
[i] Backup of 'photos' did not run.

    Blocked by   Photos is open
    Check        ! pgrep -x Photos (a `before` line in your config)
    To continue  Quit Photos (Cmd+Q), then run `vivo photos` again
```

The run still exits with status 1 and `vivo status` shows it ("Photos is open, so nothing was backed up") until a backup succeeds, and `on-failure` hooks get that same line in `$VIVO_ERROR`. A guard that stops every night means Photos is being left open overnight. The same layout explains folders vivo can't read, both before a backup starts and when it ends up backing up nothing.

If "Download Originals to this Mac" is off (iCloud Photos with optimised storage), `originals/` is incomplete: only what is on this Mac is backed up, by either method.

### Restoring photos on another system, such as Linux

The repository is an ordinary [restic](https://restic.net) repository, so any machine with restic and the repository password can restore it. Nothing here depends on macOS or on vivo. This section follows how restic and osxphotos are documented to behave; it has not been tried on Linux with a real Photos library, so do a test restore before you depend on it.

**You need three things, and none of them can be on the Mac that failed:**

1. **A copy of the repository elsewhere.** A repository that only exists in `~/.restic/repo/photos` is lost with the Mac. Add a remote (`vivo remote add`), or copy the repository somewhere safe.
2. **The repository password**, and the profile's password if the task uses `password-profile`.
3. **For `vivo recover` only, the age key** that decrypts your vivo secrets, and `sops` and `age` installed; see [Restoring on a new machine](#restoring-on-a-new-machine) for the steps.

**Restoring, with vivo or with restic alone:**

```sh
# with vivo (Linux builds are published): rebuild the config from the remote, then restore.
# With no local repository on this machine, vivo reads the off-site copy (--from picks one).
vivo recover --from "b2:my-bucket:photos"
vivo restore photos --target ~/photos-restored                                    # everything
vivo restore photos --target ~/photos-restored --path "/Users/me/Photos Export"   # one folder

# or restic alone, with the repository password in RESTIC_PASSWORD
restic -r b2:my-bucket:photos restore latest --target ~/photos-restored
restic -r b2:my-bucket:photos mount ~/mnt        # browse without restoring (needs FUSE)
```

Backups keep full paths, so files land under the original folder structure inside the target, for example `~/photos-restored/Users/me/Photos Export/2026/10/IMG_1234.heic`. Add `--path` (restic: `--include`) to restore only part of a snapshot.

**What you get back**

- **From an export:** your photos with real names in year/month folders and an XMP sidecar next to each, which photo managers such as digiKam read. This is the easy case.
- **From the library as it is:** a `Photos Library.photoslibrary` folder. Your original files are in `originals/0` to `originals/F`, named by UUID, with their EXIF data (dates, GPS) intact, and any photo manager that reads HEIC and EXIF can import that folder. Albums, captions, keywords and people are in `database/Photos.sqlite`, which is not meant for other programs. I believe edited versions are kept separately under `resources/` (the backup excludes only the `derivatives` thumbnails), but have not confirmed it against a real library.
- **osxphotos on Linux:** its project says the `export` and `query` commands work on Linux, so you may be able to run `osxphotos export <folder> --library "<restored bundle>"` there to get named files and sidecars from a restored library. Many of its features are macOS-only, and this is untested here. If you want a Linux-ready copy, make the export on the Mac while you can.

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

### A backup that captured nothing

restic treats files it can't read as a warning and still saves a snapshot, which is how a folder that macOS blocks (the Photos library is the usual one) used to produce an empty backup that looked fine. vivo now checks for this:

- **Before a backup**, it tries to read each folder (and any Photos library inside it) and says plainly what it can't, with the fix. `vivo doctor` runs the same check for every task, and `vivo task add` and `vivo manage` run it when you create a task.
- **After a backup**, it asks restic how many files the new snapshot holds. If restic couldn't read files **and** the snapshot is empty, the run fails (exit code 1, shown in `vivo status`) and names the likely cause. If some files were skipped, the run succeeds but the count is recorded as a warning that `vivo status` shows until the next run. A snapshot that is empty with no errors (an empty or fully excluded folder) is a warning too.

The usual fix on macOS is Full Disk Access for the app running vivo (Terminal, iTerm, ...), then restarting it. Check it worked by running the backup again and looking in `vivo snapshots` and `vivo mount`.

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
| **Fields** | `↑↓` select field · `Enter`/`e` edit field (including the folder lists, **Schedule** and **Password profile**) |
| **Remotes** | `a` add · `d` delete · `e` edit · `t` test connection |
| **Calls** | `a` add · `d` delete · `Ctrl+↑↓` reorder |

The **Also back up** and **Exclude** rows open a list: pick an entry to remove it, or choose **+ Add a folder** (Tab completes folder names). Each change is saved at once, and Esc closes the list. `before`, `steps` and a remote's `limit-upload` have no row of their own; they are listed read-only under **Also set**, and `o` opens the file to change them. Editing the **Schedule** row only changes the config: run `vivo schedule apply` to install it. The **Password profile** row picks which entry under `passwords:` in the secrets is used, and it must match what the repository was created with.

**Adding a task** (`a`, or `vivo task add`) starts from what you're backing up:

- **Photos & video** asks for the folder with your photos (offering `~/Pictures`), then more folders until you leave one blank. It sets up what a photos backup wants: a weekly data check and prune, a nightly schedule (run `vivo schedule apply` to install it), and a `.DS_Store` exclude. If a Photos library is in the folders, or sits in `~/Pictures` and you say yes, it also adds the guard that stops the backup while Photos is open, and skips the library's thumbnails.
- **Other folders** is the same folder questions with nothing preset.
- **Group** makes a task with no backup of its own that only runs other tasks, for example one that backs up everything.

Folder questions help you get the path right: Tab completes folder names, `~` and a dragged-in folder both work, and a folder that doesn't exist is confirmed first. vivo never creates the folder being backed up: a typo would otherwise leave you backing up an empty folder. Only the backup repository is created.

The default task (run when no task name is given) is marked with `*` in the Tasks pane.

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
| `before "..." reason="..." fix="..." skip="..."` | Shell command run before the backup (repeatable, via `sh -c`); if one fails, nothing is backed up and `on-failure` runs. `reason` says what is in the way ("Photos is open") and `fix` what to do ("Quit Photos (Cmd+Q)"); both are shown when the run is held back, and the command's own output is hidden. A plain `! pgrep -x Name` check gets "Name is running" and "Quit Name" without them.  With `skip="<pattern>"` a failure does not stop the task: that pattern is left out of that run's backup and the rest backs up (a soft check). Skipped on dry runs |
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
