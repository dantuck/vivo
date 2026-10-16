//! Single-object transfer to and from a remote, used for recovery copies.
//! Each backend decides where its recovery copy lives (`RemoteBackend::recovery_store`)
//! and which client commands move the files.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use super::rustfs::{detect_tool, is_bucket_already_exists_error, RustfsBackend, SyncTool};

pub trait ObjectStore {
    /// Where this store is, for messages: `b2://bucket/prefix`.
    fn describe(&self) -> String;
    /// Creates whatever must exist before the first `put` (a no-op unless the
    /// store is a bucket of its own).
    fn ensure_container(&self, creds: &HashMap<String, String>) -> Result<(), String>;
    fn put(&self, local: &Path, name: &str, creds: &HashMap<String, String>) -> Result<(), String>;
    fn get(&self, name: &str, dest: &Path, creds: &HashMap<String, String>) -> Result<(), String>;
}

fn key(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") }
}

/// One external command: what to run and with which environment.
#[derive(Debug)]
pub(crate) struct Invocation {
    pub program: &'static str,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

impl Invocation {
    fn run(self) -> Result<(), String> {
        let out = Command::new(self.program)
            .args(&self.args)
            .envs(&self.env)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("could not run {}: {e}", self.program))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("{} failed: {}", self.program, String::from_utf8_lossy(&out.stderr).trim()))
        }
    }
}

fn p(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A folder in a Backblaze B2 bucket (via the `b2` CLI).
pub(crate) struct B2Store {
    pub bucket: String,
    pub prefix: String,
}

impl B2Store {
    fn put_cmd(&self, local: &Path, name: &str, creds: &HashMap<String, String>) -> Invocation {
        let args = ["file", "upload", "--no-progress", &self.bucket, &p(local), &key(&self.prefix, name)];
        Invocation { program: "b2", args: args.map(String::from).to_vec(), env: creds.clone() }
    }

    fn get_cmd(&self, name: &str, dest: &Path, creds: &HashMap<String, String>) -> Invocation {
        let src = format!("b2://{}/{}", self.bucket, key(&self.prefix, name));
        Invocation { program: "b2", args: vec!["file".into(), "download".into(), src, p(dest)], env: creds.clone() }
    }
}

impl ObjectStore for B2Store {
    fn describe(&self) -> String {
        format!("b2://{}/{}", self.bucket, self.prefix)
    }

    fn ensure_container(&self, _: &HashMap<String, String>) -> Result<(), String> {
        Ok(())
    }

    fn put(&self, local: &Path, name: &str, creds: &HashMap<String, String>) -> Result<(), String> {
        self.put_cmd(local, name, creds).run()
    }

    fn get(&self, name: &str, dest: &Path, creds: &HashMap<String, String>) -> Result<(), String> {
        self.get_cmd(name, dest, creds).run()
    }
}

/// A prefix in an S3-compatible bucket, moved with whichever of `mc`, `aws`
/// or `rclone` is installed.
pub(crate) struct S3Store {
    pub endpoint: String,
    pub bucket: String,
    pub prefix: String,
    /// The bucket is vivo's own and may not exist yet.
    pub create_bucket: bool,
    tool: OnceLock<SyncTool>,
}

enum Op<'a> {
    MakeBucket,
    Put { local: &'a Path, name: &'a str },
    Get { name: &'a str, dest: &'a Path },
}

impl S3Store {
    pub fn new(endpoint: &str, bucket: &str, prefix: &str, create_bucket: bool) -> S3Store {
        S3Store {
            endpoint: endpoint.into(),
            bucket: bucket.into(),
            prefix: prefix.into(),
            create_bucket,
            tool: OnceLock::new(),
        }
    }

    /// The installed client; detected on first use.
    fn tool(&self) -> Result<SyncTool, String> {
        if let Some(t) = self.tool.get() {
            return Ok(*t);
        }
        let (tool, _) = detect_tool()?;
        Ok(*self.tool.get_or_init(|| tool))
    }

    fn invocation(&self, tool: SyncTool, op: Op, creds: &HashMap<String, String>) -> Invocation {
        let (endpoint, bucket) = (&self.endpoint, &self.bucket);
        let remote_key = |name: &str| key(&self.prefix, name);
        match tool {
            SyncTool::Mc => {
                let env = RustfsBackend::for_endpoint(endpoint, bucket).mc_env(creds);
                let args = match op {
                    Op::MakeBucket => vec!["mb".into(), "--ignore-existing".into(), format!("vivo-sync/{bucket}")],
                    Op::Put { local, name } => vec!["cp".into(), p(local), format!("vivo-sync/{bucket}/{}", remote_key(name))],
                    Op::Get { name, dest } => vec!["cp".into(), format!("vivo-sync/{bucket}/{}", remote_key(name)), p(dest)],
                };
                Invocation { program: "mc", args, env }
            }
            SyncTool::Aws => {
                let mut args: Vec<String> = match op {
                    Op::MakeBucket => vec!["s3".into(), "mb".into(), format!("s3://{bucket}")],
                    Op::Put { local, name } => vec!["s3".into(), "cp".into(), p(local), format!("s3://{bucket}/{}", remote_key(name))],
                    Op::Get { name, dest } => vec!["s3".into(), "cp".into(), format!("s3://{bucket}/{}", remote_key(name)), p(dest)],
                };
                args.extend(["--endpoint-url".to_string(), endpoint.clone()]);
                Invocation { program: "aws", args, env: creds.clone() }
            }
            SyncTool::Rclone => {
                let mut args: Vec<String> = match op {
                    Op::MakeBucket => vec!["mkdir".into(), format!(":s3:{bucket}")],
                    Op::Put { local, name } => vec!["copyto".into(), p(local), format!(":s3:{bucket}/{}", remote_key(name))],
                    Op::Get { name, dest } => vec!["copyto".into(), format!(":s3:{bucket}/{}", remote_key(name)), p(dest)],
                };
                args.extend(["--s3-provider", "Other", "--s3-endpoint", endpoint].map(String::from));
                Invocation { program: "rclone", args, env: RustfsBackend::rclone_env(creds) }
            }
        }
    }
}

impl ObjectStore for S3Store {
    fn describe(&self) -> String {
        format!("s3://{}/{} ({})", self.bucket, self.prefix, self.endpoint)
    }

    fn ensure_container(&self, creds: &HashMap<String, String>) -> Result<(), String> {
        if !self.create_bucket {
            return Ok(());
        }
        // `aws s3 mb` exits non-zero if the bucket is already ours; that is fine.
        match self.invocation(self.tool()?, Op::MakeBucket, creds).run() {
            Err(e) if !is_bucket_already_exists_error(&e) => Err(e),
            _ => Ok(()),
        }
    }

    fn put(&self, local: &Path, name: &str, creds: &HashMap<String, String>) -> Result<(), String> {
        self.invocation(self.tool()?, Op::Put { local, name }, creds).run()
    }

    fn get(&self, name: &str, dest: &Path, creds: &HashMap<String, String>) -> Result<(), String> {
        self.invocation(self.tool()?, Op::Get { name, dest }, creds).run()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn creds() -> HashMap<String, String> {
        HashMap::from([
            ("AWS_ACCESS_KEY_ID".to_string(), "k/y".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "s".to_string()),
        ])
    }

    #[test]
    fn mc_commands_target_the_alias_and_percent_encode_keys() {
        let store = S3Store::new("https://h", "bkt", "p.vivo-recovery", false);
        let put = store.invocation(SyncTool::Mc, Op::Put { local: Path::new("/t/x"), name: "secrets.yaml" }, &creds());
        assert_eq!(put.program, "mc");
        assert_eq!(put.args, ["cp", "/t/x", "vivo-sync/bkt/p.vivo-recovery/secrets.yaml"]);
        assert_eq!(put.env["MC_HOST_vivo-sync"], "https://k%2Fy:s@h");
        let mb = S3Store::new("https://h", "b-vivo-recovery", "", true).invocation(SyncTool::Mc, Op::MakeBucket, &creds());
        assert_eq!(mb.args, ["mb", "--ignore-existing", "vivo-sync/b-vivo-recovery"]);
    }

    #[test]
    fn aws_and_rclone_commands() {
        let store = S3Store::new("https://h", "bkt", "", false);
        let get = store.invocation(SyncTool::Aws, Op::Get { name: "backup.kdl.age", dest: Path::new("/d/f") }, &creds());
        assert_eq!(get.program, "aws");
        assert_eq!(get.args, ["s3", "cp", "s3://bkt/backup.kdl.age", "/d/f", "--endpoint-url", "https://h"]);
        let put = store.invocation(SyncTool::Rclone, Op::Put { local: Path::new("/t/x"), name: "README.txt" }, &creds());
        assert_eq!(put.program, "rclone");
        assert_eq!(put.args[..3], ["copyto", "/t/x", ":s3:bkt/README.txt"]);
        assert!(put.args.contains(&"--s3-endpoint".to_string()));
        assert_eq!(put.env["RCLONE_S3_ACCESS_KEY_ID"], "k/y");
    }

    #[test]
    fn b2_commands() {
        let store = B2Store { bucket: "bkt".into(), prefix: "fc.vivo-recovery".into() };
        let put = store.put_cmd(Path::new("/t/x"), "secrets.yaml", &HashMap::new());
        assert_eq!(put.program, "b2");
        assert_eq!(put.args, ["file", "upload", "--no-progress", "bkt", "/t/x", "fc.vivo-recovery/secrets.yaml"]);
        let get = store.get_cmd("secrets.yaml", Path::new("/d/s"), &HashMap::new());
        assert_eq!(get.args, ["file", "download", "b2://bkt/fc.vivo-recovery/secrets.yaml", "/d/s"]);
    }
}
