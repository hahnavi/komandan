//! Gap executors for runtime operations.
//!
//! [`UnArchive`] unpacks an archive on the remote host (tar/zip/gz by
//! extension); [`WaitFor`] polls a path or port until a condition is met.

use std::time::{Duration, Instant};

use komandan_plugin_abi::prelude::*;
use serde_json::Value;

use super::{Connection, ModuleError, ModuleExecutor, ModuleRegistry, TaskContext};

/// Register the ops gap executors on `reg`.
pub fn register_all(reg: &mut ModuleRegistry) {
    reg.register(UnArchive);
    reg.register(WaitFor);
}

/// Build a successful result carrying `stdout`.
fn ok_with_stdout(stdout: &str) -> ModuleResult {
    ModuleResult {
        changed: false,
        rc: 0,
        stdout: RString::from(stdout),
        stderr: RString::new(),
        success: true,
        msg: ROption::RNone,
    }
}

// ---- unarchive ----------------------------------------------------------

/// `unarchive` — unpack an archive on the remote host.
struct UnArchive;

impl ModuleExecutor for UnArchive {
    fn name(&self) -> &'static str {
        "unarchive"
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let src = args
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("unarchive requires a 'src'"))?;
        let dest = args
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("unarchive requires a 'dest'"))?;
        let remote_src = args
            .get("remote_src")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let unpack = unpack_cmd(src, dest).ok_or_else(|| {
            ModuleError::args(format!(
                "unarchive: unrecognized archive extension for {src}"
            ))
        })?;

        let (remote_archive, cleanup) = if remote_src {
            (src.to_string(), false)
        } else {
            // Upload to a temp path, then unpack from there.
            let tmp = format!("/tmp/komandan_unarchive_{}.arc", unique_suffix());
            conn.upload(src, &tmp)?;
            (tmp, true)
        };

        let prefix = super::shell_prefix(ctx);
        let cmd = format!(
            "{prefix}mkdir -p {dest} && {unpack}",
            unpack = unpack_with_path(unpack, &remote_archive, dest)
        );
        let result = conn.run_command(&cmd)?;
        if cleanup {
            let _ = conn.run_command(&format!("{prefix}rm -f {remote_archive}"));
        }
        let changed = result.success && result.rc == 0;
        Ok(ModuleResult {
            changed,
            rc: result.rc,
            stdout: result.stdout,
            stderr: result.stderr,
            success: result.success,
            msg: ROption::RNone,
        })
    }
}

/// The unpack tool + flag for a given archive path, or `None` if unrecognized.
#[allow(clippy::case_sensitive_file_extension_comparisons)] // `src` is lowercased above.
fn unpack_cmd(src: &str, _dest: &str) -> Option<UnpackKind> {
    let lower = src.to_ascii_lowercase();
    if lower.ends_with(".tar.gz") || lower.ends_with(".tgz") {
        Some(UnpackKind::Tar('z'))
    } else if lower.ends_with(".tar.bz2") || lower.ends_with(".tbz2") {
        Some(UnpackKind::Tar('j'))
    } else if lower.ends_with(".tar.xz") || lower.ends_with(".txz") {
        Some(UnpackKind::Tar('J'))
    } else if lower.ends_with(".tar") {
        Some(UnpackKind::Tar(' '))
    } else if lower.ends_with(".zip") {
        Some(UnpackKind::Unzip)
    } else if lower.ends_with(".gz") {
        Some(UnpackKind::Gunzip)
    } else {
        None
    }
}

/// How to unpack an archive.
#[derive(Debug, Clone, Copy)]
enum UnpackKind {
    /// `tar -x<flag>f`, with the given compression flag (`z`/`j`/`J` or space for plain).
    Tar(char),
    /// `unzip -o`.
    Unzip,
    /// `gunzip` (single-file .gz).
    Gunzip,
}

/// Build the concrete shell command for an unpack, given the remote archive
/// path and destination directory.
fn unpack_with_path(kind: UnpackKind, archive: &str, dest: &str) -> String {
    match kind {
        UnpackKind::Tar(' ') => format!("tar -C {dest} -xf {archive}"),
        UnpackKind::Tar(flag) => format!("tar -C {dest} -x{flag}f {archive}"),
        UnpackKind::Unzip => format!("unzip -o -d {dest} {archive}"),
        UnpackKind::Gunzip => format!("gunzip -c {archive} > {dest}"),
    }
}

/// A short process-unique suffix for temp-file names (time + pid).
fn unique_suffix() -> String {
    let pid = std::process::id();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("{pid}{nanos}")
}

// ---- wait_for -----------------------------------------------------------

/// `wait_for` — poll a remote path or TCP port until a condition holds.
struct WaitFor;

impl ModuleExecutor for WaitFor {
    fn name(&self) -> &'static str {
        "wait_for"
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let timeout = args.get("timeout").and_then(Value::as_u64).unwrap_or(300);
        let sleep_secs = args.get("sleep").and_then(Value::as_u64).unwrap_or(1);
        let delay = args.get("delay").and_then(Value::as_u64).unwrap_or(0);

        let probe = build_probe(args)?;
        let prefix = super::shell_prefix(ctx);

        if delay > 0 {
            std::thread::sleep(Duration::from_secs(delay));
        }
        let deadline = Instant::now() + Duration::from_secs(timeout);
        loop {
            let met = probe.met(conn, &prefix)?;
            if met {
                return Ok(ok_with_stdout(&probe.describe(true)));
            }
            if Instant::now() >= deadline {
                return Ok(ModuleResult::failure(1, probe.describe(false)));
            }
            std::thread::sleep(Duration::from_secs(sleep_secs));
        }
    }
}

/// A single pollable condition.
enum Probe {
    /// Wait for a remote path to exist (`present`) or not exist (`absent`).
    Path { path: String, present: bool },
    /// Wait for a TCP port to be open (`started`) or closed (`stopped`).
    Port { host: String, port: u64, open: bool },
}

impl Probe {
    /// Evaluate the condition once. `prefix` is the task's combined shell
    /// prefix (`become` + `environment:`, may be empty).
    fn met(&self, conn: &Connection<'_>, prefix: &str) -> Result<bool, ModuleError> {
        match self {
            Self::Path { path, present } => {
                let cmd = format!("{prefix}test -e {path} && echo Y || echo N");
                let r = conn.run_command(&cmd)?;
                let exists = r.stdout.trim() == "Y";
                Ok(exists == *present)
            }
            Self::Port { host, port, open } => {
                let cmd = format!(
                    "{prefix}timeout 1 bash -c 'echo > /dev/tcp/{host}/{port}' 2>/dev/null && echo Y || echo N"
                );
                let r = conn.run_command(&cmd)?;
                let listening = r.stdout.trim() == "Y";
                Ok(listening == *open)
            }
        }
    }

    /// Human-readable status line.
    fn describe(&self, met: bool) -> String {
        let state = if met { "ok" } else { "timed out" };
        match self {
            Self::Path { path, present } => {
                format!("wait_for path={path} present={present}: {state}")
            }
            Self::Port { host, port, open } => {
                format!("wait_for port={port} host={host} open={open}: {state}")
            }
        }
    }
}

/// Build the [`Probe`] from the task args.
fn build_probe(args: &Value) -> Result<Probe, ModuleError> {
    let state = args
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("present");
    if let Some(path) = args.get("path").and_then(Value::as_str) {
        let present = !matches!(state, "absent");
        return Ok(Probe::Path {
            path: path.to_string(),
            present,
        });
    }
    if let Some(port) = args.get("port").and_then(Value::as_u64) {
        let host = args
            .get("host")
            .and_then(Value::as_str)
            .unwrap_or("127.0.0.1")
            .to_string();
        let open = !matches!(state, "stopped" | "absent" | "drained");
        return Ok(Probe::Port { host, port, open });
    }
    Err(ModuleError::args("wait_for requires a 'path' or a 'port'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executors::{Connection, CoreApiRef};
    use crate::test_support::{MockCore, localhost_host, null_core};
    use serde_json::json;

    fn conn(core: &CoreApiRef) -> Connection<'_> {
        Connection::new(core, ConnectionHandle::INVALID, localhost_host())
    }
    fn noop_ctx() -> TaskContext {
        TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        )
    }

    #[test]
    fn unpack_kind_detects_common_formats() {
        assert!(matches!(
            unpack_cmd("a.tar.gz", "/d"),
            Some(UnpackKind::Tar('z'))
        ));
        assert!(matches!(
            unpack_cmd("a.tgz", "/d"),
            Some(UnpackKind::Tar('z'))
        ));
        assert!(matches!(
            unpack_cmd("a.tar", "/d"),
            Some(UnpackKind::Tar(' '))
        ));
        assert!(matches!(unpack_cmd("a.zip", "/d"), Some(UnpackKind::Unzip)));
        assert!(matches!(unpack_cmd("a.gz", "/d"), Some(UnpackKind::Gunzip)));
        assert!(unpack_cmd("a.bin", "/d").is_none());
    }

    #[test]
    fn unpack_with_path_tar_gz() {
        let s = unpack_with_path(UnpackKind::Tar('z'), "/tmp/a.tar.gz", "/opt");
        assert!(s.contains("tar -C /opt -xzf /tmp/a.tar.gz"), "{s}");
    }

    #[test]
    fn unpack_with_path_zip() {
        let s = unpack_with_path(UnpackKind::Unzip, "/tmp/a.zip", "/opt");
        assert!(s.contains("unzip -o -d /opt /tmp/a.zip"), "{s}");
    }

    #[test]
    fn unarchive_remote_src_runs_tar() {
        let core = null_core();
        let r = UnArchive
            .run(
                &conn(&core),
                &json!({"src": "/tmp/pkg.tar.gz", "dest": "/opt", "remote_src": true}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "{}", r.stdout);
        assert!(
            r.stdout.contains("tar -C /opt -xzf /tmp/pkg.tar.gz"),
            "{}",
            r.stdout
        );
    }

    #[test]
    fn unarchive_missing_src_errors() {
        assert!(
            UnArchive
                .run(&conn(&null_core()), &json!({"dest": "/d"}), &noop_ctx())
                .is_err()
        );
    }

    #[test]
    fn wait_for_path_present_succeeds_immediately() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("Y"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = WaitFor
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/here", "timeout": 1, "sleep": 0}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.success, "{}", r.stdout);
    }

    #[test]
    fn wait_for_requires_path_or_port() {
        assert!(
            WaitFor
                .run(&conn(&null_core()), &json!({}), &noop_ctx())
                .is_err()
        );
    }

    #[test]
    fn build_probe_prefers_path() {
        let p = build_probe(&json!({"path": "/x", "port": 80})).unwrap_or_else(|e| panic!("{e:?}"));
        assert!(matches!(p, Probe::Path { .. }));
    }

    #[test]
    fn build_probe_port_uses_default_host() {
        let p = build_probe(&json!({"port": 22})).unwrap_or_else(|e| panic!("{e:?}"));
        let Probe::Port { host, port, open } = p else {
            panic!("expected Port probe");
        };
        assert_eq!(host, "127.0.0.1");
        assert_eq!(port, 22);
        assert!(open);
    }
}
