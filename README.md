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

`vivo init` checks that prerequisites are installed, creates a starter config at `~/.config/vivo/backup.kdl`, and creates an encrypted secrets file at `~/.config/vivo/secrets.yaml`.

After init:

```sh
vivo config edit    # set your repo path, directories, and remotes
vivo secrets edit   # set your restic password and remote credentials
vivo --dry-run      # test the backup without making changes
vivo                # run the backup
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
vivo secrets show   # Decrypt and print secrets to stdout
```

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

### S3-compatible (AWS S3, rustfs, MinIO, etc.)

Uses `restic copy`. URL format: `s3:<endpoint>/<bucket>`

```kdl
remote "s3:https://s3.amazonaws.com/my-bucket" {
    credentials "aws"
}
remote "s3:http://rustfs.local:9000/backup" {
    credentials "rustfs"
}
```

The remote restic repository must be initialized before first sync:

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

## License

Licensed under either of

- Apache License, Version 2.0
- MIT License

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this project shall be dual-licensed as above, without any additional terms or conditions.
