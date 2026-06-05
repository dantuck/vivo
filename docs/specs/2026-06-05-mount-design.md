# vivo mount — Design Spec

**Date:** 2026-06-05

## Summary

Add a `vivo mount` subcommand that mounts any configured restic repository (local or remote) as a FUSE filesystem, with an interactive picker, doctor-style pre-flight checks, and tooling guidance.

---

## CLI Interface

```
vivo mount [path]
```

- **No args:** vivo creates a temp mount point at `/tmp/vivo-mount-<task>-<timestamp>`, prints the path, and removes it on exit.
- **With `[path]`:** vivo uses the specified directory (creates it if needed). Does not remove it on exit.
- **Interactive picker:** always shown — user selects from all configured tasks and their repos, e.g.:
  - `backup (local)`
  - `backup → s3:https://s3.amazonaws.com/my-bucket`
  - `backup → b2:my-bucket:restic/main`
- After mounting, vivo prints the mount path and blocks with: `Mounted at <path>. Press Ctrl+C to unmount.`
- On Ctrl+C or exit: unmounts (`fusermount -u` on Linux, `umount` on macOS) and removes the temp dir if auto-created.

---

## Pre-mount Checks

Runs before any mount attempt; results printed in `vivo doctor` style (`[ok]` / `[warn]` / `[fail]`):

| Check | Required | Detail |
|---|---|---|
| restic present | Yes | Reuses `doctor::check_tool_present("restic", ...)` |
| FUSE available | Yes | Linux: `fusermount` or `fusermount3` on PATH; hint `sudo apt install fuse` / `sudo dnf install fuse`. macOS: `mount_macfuse` on PATH or `/Library/Filesystems/macfuse.kext` exists; hint `brew install --cask macfuse` |
| Repo accessible | Warn | `restic snapshots --repo <url> --no-lock` with 15s timeout and credentials injected as env vars. Always uses restic (including for B2 repos) since the mount is restic-driven — unlike `doctor::check_remote_connectivity` which uses `b2 ls` for B2 remotes. |
| Mount point valid | Warn | If a path was supplied: exists and is empty directory |

Required failures (restic, FUSE) cause exit before mount. Warnings are shown but do not block.

---

## Module Structure

### New: `src/mount.rs`

```rust
pub fn run(config_path: &str, secrets_path: &str, mount_path: Option<&str>)
```

Internal flow:

1. **Load config + secrets** — `BackupConfig::load_config` (same as backup flow); needed to resolve credentials for remote repos.
2. **Interactive picker** — `inquire::Select` listing all tasks × repos (local + each configured remote). Returns `(repo_url: String, credentials: Option<HashMap<String, String>>)`.
3. **Pre-mount checks** — runs 4 checks, prints results, exits on required failures.
4. **Mount point resolution** — `None` → create `/tmp/vivo-mount-<task>-<timestamp>`, set `owned = true`; `Some(path)` → create if missing, `owned = false`.
5. **Invoke restic** — `restic mount --repo <url> <mountpoint>` with credentials + `RESTIC_PASSWORD` injected as env vars; stdio inherited.
6. **Cleanup** — `ctrlc` crate handler: unmount + remove dir if `owned`.

### Modified: `src/doctor.rs`

Add one new public function:

```rust
pub fn check_fuse() -> CheckResult
```

Platform-branched: Linux checks `fusermount`/`fusermount3`; macOS checks `mount_macfuse` or kext path. Used by `mount.rs`; also available to `vivo doctor` in future.

### Modified: `src/config.rs` (`build_cli`)

Add:
```rust
Command::new("mount")
    .about("Mount a backup repository as a filesystem")
    .arg(Arg::new("path").help("Mount point (default: auto temp dir)").required(false))
```

### Modified: `src/bin/vivo.rs`

Add dispatch:
```rust
Some(("mount", sub)) => {
    let mount_path = sub.get_one::<String>("path").map(String::as_str);
    if let Err(e) = vivo::mount::run(&config_path, &secrets_path, mount_path) {
        eprintln!("error: {e}");
        process::exit(1);
    }
    return;
}
```

### Modified: `src/lib.rs`

Export `pub mod mount;`.

---

## Error Handling

- Config/secrets load failure: print error and exit (same pattern as backup flow).
- No tasks configured: print `No backup tasks configured. Run \`vivo task add\` first.` and exit.
- `restic mount` fails to start: surface error message.
- Cleanup failure (unmount): print warning but do not panic.

---

## Dependencies

- `ctrlc` crate — for Ctrl+C handler to trigger cleanup. Not yet in `Cargo.toml`; must be added.

---

## Out of Scope

- Mounting inside the TUI (`vivo manage`) — blocked by the TUI event loop; not in this spec.
- `vivo doctor` showing FUSE status by default — can be added later; `check_fuse()` will be available.
- Windows support — restic FUSE mount is not supported on Windows.
