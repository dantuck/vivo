//! Off-site recovery copies: next to each remote's repository, vivo keeps the
//! two small files needed to set up a replacement machine — the (already
//! sops-encrypted) `secrets.yaml` and an age-encrypted `backup.kdl`.
//!
//! The copy can't live inside the synced repository path: every sync tool
//! vivo uses deletes files there that the local repository doesn't have. So
//! it goes in a sibling location (see [`store_for`]). With these files and the
//! age key, `vivo recover` rebuilds the local setup; everything else (the
//! restic password, every credentials profile) is inside `secrets.yaml`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

use crate::backup_config::backup::Remote;
use crate::remote::ObjectStore;

pub const SECRETS_NAME: &str = "secrets.yaml";
pub const CONFIG_NAME: &str = "backup.kdl.age";
pub const README_NAME: &str = "README.txt";

const README: &str = "This folder is a vivo recovery copy (https://crates.io/crates/vivo).\n\
\n\
  secrets.yaml    sops-encrypted: your restic password and storage credentials\n\
  backup.kdl.age  age-encrypted: your vivo backup configuration\n\
\n\
Both can only be opened with your age key (~/.config/sops/age/keys.txt).\n\
To rebuild a machine: install vivo, restic, sops and age, put the age key back, then\n\
run `vivo recover --from <this remote's URL>`.\n\
By hand: `sops -d secrets.yaml` and `age -d -i keys.txt backup.kdl.age`.\n";

/// The files to upload, staged in a temporary directory.
pub struct Bundle {
    dir: tempfile::TempDir,
}

impl Bundle {
    fn prepare(config_path: &str, secrets_path: &str) -> Result<Bundle, String> {
        let recipient = crate::age_public_key()
            .ok_or_else(|| format!("no age public key found in {}", crate::age_keys_path()))?;
        let dir = tempfile::tempdir().map_err(|e| format!("could not create a temp dir: {e}"))?;
        age_encrypt(Path::new(config_path), &dir.path().join(CONFIG_NAME), &recipient)?;
        fs::copy(secrets_path, dir.path().join(SECRETS_NAME))
            .map_err(|e| format!("could not read {secrets_path}: {e}"))?;
        fs::write(dir.path().join(README_NAME), README).map_err(|e| e.to_string())?;
        Ok(Bundle { dir })
    }

    fn files(&self) -> [(&'static str, PathBuf); 3] {
        [SECRETS_NAME, CONFIG_NAME, README_NAME].map(|n| (n, self.dir.path().join(n)))
    }
}

/// What this run would upload, prepared only as far as needed. Hashing the
/// inputs is cheap and tells every remote whether its copy is current; the
/// `age` encryption (a subprocess) happens only once some remote needs it.
pub struct Source {
    config_path: String,
    secrets_path: String,
    hash: Option<Result<String, String>>,
    bundle: Option<Result<Bundle, String>>,
}

impl Source {
    pub fn new(config_path: &str, secrets_path: &str) -> Source {
        Source { config_path: config_path.into(), secrets_path: secrets_path.into(), hash: None, bundle: None }
    }

    /// Hash of the plaintext inputs; the ciphertext differs on every run, so
    /// it can't be used to tell whether anything changed.
    fn hash(&mut self) -> Result<String, String> {
        let (config, secrets) = (&self.config_path, &self.secrets_path);
        self.hash
            .get_or_insert_with(|| {
                let read = |p: &String| fs::read(p).map_err(|e| format!("could not read {p}: {e}"));
                Ok(format!(
                    "{:x}",
                    Sha256::new().chain_update(read(config)?).chain_update([0u8]).chain_update(read(secrets)?).finalize()
                ))
            })
            .clone()
    }

    fn bundle(&mut self) -> Result<&Bundle, String> {
        let (config, secrets) = (&self.config_path, &self.secrets_path);
        self.bundle
            .get_or_insert_with(|| Bundle::prepare(config, secrets))
            .as_ref()
            .map_err(Clone::clone)
    }
}

fn run_checked(mut cmd: Command, what: &str) -> Result<(), String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run {what}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed: {}", String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn age_encrypt(plain: &Path, out: &Path, recipient: &str) -> Result<(), String> {
    let mut cmd = Command::new("age");
    cmd.args(["-r", recipient, "-o"]).arg(out).arg(plain);
    run_checked(cmd, "age (encrypt)")
}

/// Decrypts an age file with the identity in `key_file`.
pub fn age_decrypt(encrypted: &Path, out: &Path, key_file: &str) -> Result<(), String> {
    let mut cmd = Command::new("age");
    cmd.args(["-d", "-i", key_file, "-o"]).arg(out).arg(encrypted);
    run_checked(cmd, "age (decrypt)")
}

/// Uploads the bundle's files. The README is static, so it is only sent the
/// first time; `first` is also when a store of its own may need creating.
fn upload(store: &dyn ObjectStore, bundle: &Bundle, creds: &HashMap<String, String>, first: bool) -> Result<(), String> {
    if first {
        store.ensure_container(creds)?;
    }
    for (name, local) in bundle.files() {
        if first || name != README_NAME {
            store.put(&local, name, creds)?;
        }
    }
    Ok(())
}

/// Downloads the recovery copy of `url` into `dest_dir`.
pub fn fetch(url: &str, creds: &HashMap<String, String>, dest_dir: &Path) -> Result<(), String> {
    let store = crate::remote::from_url(url)?.recovery_store()?;
    for name in [SECRETS_NAME, CONFIG_NAME] {
        store
            .get(name, &dest_dir.join(name), creds)
            .map_err(|e| format!("{e}\n(is there a recovery copy next to {url}?)"))?;
    }
    Ok(())
}

/// What happened to one remote's recovery copy during a sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Uploaded; carries the hash that is now on the remote.
    Uploaded(String),
    /// The remote already has the current files.
    Unchanged,
    /// Opted out with `recovery false`.
    Disabled,
    /// Impossible for this remote (the reason); worth telling the user about.
    Skipped(String),
    Failed(String),
}

/// Brings `remote`'s recovery copy up to date. `previous` is the hash recorded
/// by the last successful upload; nothing is sent if it matches.
pub fn sync_to_remote(remote: &Remote, creds: &HashMap<String, String>, source: &mut Source, previous: Option<&str>) -> Outcome {
    if !remote.recovery {
        return Outcome::Disabled;
    }
    let store = match crate::remote::from_url(&remote.url).and_then(|b| b.recovery_store()) {
        Ok(s) => s,
        Err(e) => return Outcome::Skipped(e),
    };
    let hash = match source.hash() {
        Ok(h) => h,
        Err(e) => return Outcome::Failed(e),
    };
    if previous == Some(hash.as_str()) {
        return Outcome::Unchanged;
    }
    match source.bundle().and_then(|bundle| upload(store.as_ref(), bundle, creds, previous.is_none())) {
        Ok(()) => Outcome::Uploaded(hash),
        Err(e) => Outcome::Failed(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn remote(url: &str, recovery: bool) -> Remote {
        Remote { url: url.into(), credentials: "c".into(), mc_max_workers: None, mc_limit_upload: None, limit_upload: None, recovery }
    }

    /// A source whose hash is `hash` and whose bundle is an empty staging dir.
    fn dummy_source(hash: &str) -> Source {
        Source {
            config_path: String::new(),
            secrets_path: String::new(),
            hash: Some(Ok(hash.to_string())),
            bundle: Some(Ok(Bundle { dir: tempfile::tempdir().unwrap() })),
        }
    }

    #[test]
    fn sync_skips_when_opted_out_unchanged_or_impossible() {
        let mut b = dummy_source("h1");
        assert_eq!(sync_to_remote(&remote("b2:bkt:p", false), &HashMap::new(), &mut b, None), Outcome::Disabled);
        assert_eq!(sync_to_remote(&remote("b2:bkt:p", true), &HashMap::new(), &mut b, Some("h1")), Outcome::Unchanged);
        assert!(matches!(sync_to_remote(&remote("b2:bkt", true), &HashMap::new(), &mut b, None), Outcome::Skipped(_)));
    }

    #[cfg(unix)]
    #[test]
    fn upload_runs_b2_once_per_file_and_reports_failures() {
        use std::os::unix::fs::PermissionsExt;
        let _guard = crate::PATH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let bin = tempfile::tempdir().unwrap();
        let log = bin.path().join("calls.log");
        let fake = bin.path().join("b2");
        fs::write(&fake, format!("#!/bin/sh\necho \"$@\" >> '{}'\nexit ${{FAKE_B2_EXIT:-0}}\n", log.display())).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();

        let mut b = dummy_source("h1");
        for n in [SECRETS_NAME, CONFIG_NAME, README_NAME] {
            fs::write(b.bundle().unwrap().dir.path().join(n), "x").unwrap();
        }
        let original = std::env::var("PATH").unwrap_or_default();
        std::env::set_var("PATH", format!("{}:{original}", bin.path().display()));

        let ok = sync_to_remote(&remote("b2:bkt:fc", true), &HashMap::new(), &mut b, None);
        let calls = fs::read_to_string(&log).unwrap();
        let failing = sync_to_remote(
            &remote("b2:bkt:fc", true),
            &HashMap::from([("FAKE_B2_EXIT".to_string(), "1".to_string())]),
            &mut b,
            None,
        );
        std::env::set_var("PATH", original);

        assert_eq!(ok, Outcome::Uploaded("h1".into()));
        assert_eq!(calls.lines().count(), 3);
        assert!(calls.contains("file upload --no-progress bkt"));
        assert!(calls.contains("fc.vivo-recovery/backup.kdl.age"));
        assert!(matches!(failing, Outcome::Failed(_)));
    }
}
