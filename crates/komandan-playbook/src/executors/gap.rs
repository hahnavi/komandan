//! §6.3 "gap" executors: Ansible modules without a komandan built-in.
//!
//! Hostname, Timezone, Git, Pip, Stat, `KnownHosts` — system-config modules that
//! shell out via [`Connection::run_command`] (or locally for [`KnownHosts`]).
//! See `docs/PLAYBOOK_PLAN.md` for the gap-module roadmap.

use std::path::Path;

use komandan_plugin_abi::prelude::*;
use serde_json::Value;

use super::{Connection, ModuleError, ModuleExecutor, ModuleRegistry, TaskContext};

/// Register every gap executor on `reg`.
pub fn register_all(reg: &mut ModuleRegistry) {
    reg.register(Hostname);
    reg.register(Timezone);
    reg.register(Git);
    reg.register(Pip);
    reg.register(Stat);
    reg.register(KnownHosts);
}

/// Return `r` with a replaced `changed` flag (stdout/stderr/rc/success kept).
const fn with_changed(mut r: ModuleResult, changed: bool) -> ModuleResult {
    r.changed = changed;
    r
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

// ---- hostname -----------------------------------------------------------

/// `hostname` — set the system hostname via `hostnamectl`.
struct Hostname;

impl ModuleExecutor for Hostname {
    fn name(&self) -> &'static str {
        "hostname"
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let name = args
            .as_str()
            .or_else(|| args.get("name").and_then(Value::as_str))
            .ok_or_else(|| ModuleError::args("hostname requires a 'name'"))?;
        let prefix = super::shell_prefix(ctx);
        let result = conn.run_command(&format!("{prefix}hostnamectl set-hostname {name}"))?;
        let changed = result.success && result.rc == 0;
        Ok(with_changed(result, changed))
    }
}

// ---- timezone -----------------------------------------------------------

/// `timezone` — set the system timezone via `timedatectl`.
struct Timezone;

impl ModuleExecutor for Timezone {
    fn name(&self) -> &'static str {
        "timezone"
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let name = args
            .as_str()
            .or_else(|| args.get("name").and_then(Value::as_str))
            .ok_or_else(|| ModuleError::args("timezone requires a 'name'"))?;
        let prefix = super::shell_prefix(ctx);
        let result = conn.run_command(&format!("{prefix}timedatectl set-timezone {name}"))?;
        let changed = result.success && result.rc == 0;
        Ok(with_changed(result, changed))
    }
}

// ---- git ----------------------------------------------------------------

/// `git` — clone (or update) a repository on the remote host.
struct Git;

impl ModuleExecutor for Git {
    fn name(&self) -> &'static str {
        "git"
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let repo = args
            .get("repo")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("git requires a 'repo'"))?;
        let dest = args
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("git requires a 'dest'"))?;
        let depth = args.get("depth").and_then(Value::as_u64);
        let force = args
            .get("force")
            .and_then(Value::as_bool)
            .unwrap_or_default();
        let version = args.get("version").and_then(Value::as_str);

        let prefix = super::shell_prefix(ctx);
        let clone_cmd = build_clone(repo, dest, version, depth, force);
        let out = conn.run_command(&format!("{prefix}{clone_cmd}"))?;
        if out.success {
            return Ok(with_changed(out, true));
        }
        // Clone failed (typically: dest already exists) — fall back to an update.
        let update_cmd = build_update(dest, version);
        let mut out2 = conn.run_command(&format!("{prefix}{update_cmd}"))?;
        out2.changed = out2.success;
        Ok(out2)
    }
}

/// Build the `git clone` invocation (with optional depth / force / checkout).
fn build_clone(
    repo: &str,
    dest: &str,
    version: Option<&str>,
    depth: Option<u64>,
    force: bool,
) -> String {
    use std::fmt::Write as _;
    let mut cmd = String::from("git clone");
    if let Some(n) = depth {
        let _ = write!(cmd, " --depth {n}");
    }
    if force {
        cmd.push_str(" --force");
    }
    let _ = write!(cmd, " {repo} {dest}");
    if let Some(v) = version {
        let _ = write!(cmd, " && git -C {dest} checkout {v}");
    }
    cmd
}

/// Build the update command used when a fresh clone is not possible.
fn build_update(dest: &str, version: Option<&str>) -> String {
    version.map_or_else(
        || format!("git -C {dest} pull --ff-only"),
        |v| format!("git -C {dest} fetch --all && git -C {dest} checkout {v}"),
    )
}

// ---- pip ----------------------------------------------------------------

/// `pip` — manage a Python package (requires Python on the target).
struct Pip;

impl ModuleExecutor for Pip {
    fn name(&self) -> &'static str {
        "pip"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["ansible.builtin.pip"]
    }
    fn requires_remote_python(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let names = pip_names(args.get("name"))
            .ok_or_else(|| ModuleError::args("pip requires a 'name'"))?;
        let state = args
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("present");
        let action = match state {
            "absent" => "uninstall -y",
            "latest" => "install --upgrade",
            _ => "install",
        };
        let pip_bin = pip_executable(args);
        let prefix = super::shell_prefix(ctx);
        let result = conn.run_command(&format!("{prefix}{pip_bin} {action} {names}"))?;
        let changed = result.success && result.rc == 0;
        Ok(with_changed(result, changed))
    }
}

/// Coerce a pip `name:` value (string or array) into a space-joined string.
fn pip_names(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => {
            let parts: Vec<&str> = a.iter().filter_map(Value::as_str).collect();
            if parts.is_empty() {
                None
            } else {
                Some(parts.join(" "))
            }
        }
        _ => None,
    }
}

/// Resolve the pip executable: explicit `executable:` wins, then `virtualenv:`,
/// then the default `python3 -m pip`.
fn pip_executable(args: &Value) -> String {
    args.get("executable")
        .and_then(Value::as_str)
        .map(String::from)
        .or_else(|| {
            args.get("virtualenv")
                .and_then(Value::as_str)
                .map(|v| format!("{v}/bin/pip"))
        })
        .unwrap_or_else(|| String::from("python3 -m pip"))
}

// ---- stat ---------------------------------------------------------------

/// `stat` — gather metadata about a remote path (read-only).
struct Stat;

impl ModuleExecutor for Stat {
    fn name(&self) -> &'static str {
        "stat"
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let path = args
            .get("path")
            .or_else(|| args.get("dest"))
            .or_else(|| args.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("stat requires a 'path'"))?;
        // v0.1: the path is single-quoted verbatim — embedded single-quotes
        // are NOT escaped (assumes clean paths).
        let prefix = super::shell_prefix(ctx);
        let cmd = format!("{prefix}stat -c '%n|%F|%s|%U|%G|%a|%Y' '{path}' 2>/dev/null || true");
        let result = conn.run_command(&cmd)?;
        let stat = parse_stat(result.stdout.as_str(), path);
        let json_str = serde_json::to_string(&stat)
            .map_err(|e| ModuleError::Other(format!("stat json encode: {e}")))?;
        Ok(ModuleResult {
            changed: false,
            rc: result.rc,
            stdout: RString::from(json_str),
            stderr: result.stderr,
            success: true,
            msg: ROption::RNone,
        })
    }
}

/// Parse the `|`-delimited `stat -c` line into the JSON result object.
fn parse_stat(stdout: &str, path: &str) -> Value {
    let line = stdout.trim();
    if line.is_empty() {
        return serde_json::json!({"exists": false, "stat": {}});
    }
    let parts: Vec<&str> = line.split('|').collect();
    let field = |i: usize| parts.get(i).copied().unwrap_or_default();
    let ftype = field(1);
    serde_json::json!({
        "exists": true,
        "stat": {
            "path": path,
            "isdir": ftype == "directory",
            "isreg": ftype == "regular file",
            "size": field(2).parse::<i64>().unwrap_or_default(),
            "owner": field(3),
            "group": field(4),
            "mode": field(5),
            "mtime": field(6).parse::<i64>().unwrap_or_default(),
        }
    })
}

// ---- known_hosts --------------------------------------------------------

/// `known_hosts` — manage a LOCAL `~/.ssh/known_hosts` entry (controller-side).
struct KnownHosts;

impl ModuleExecutor for KnownHosts {
    fn name(&self) -> &'static str {
        "known_hosts"
    }
    fn run(
        &self,
        _conn: &Connection<'_>,
        args: &Value,
        _ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let host = args
            .as_str()
            .or_else(|| args.get("name").and_then(Value::as_str))
            .ok_or_else(|| ModuleError::args("known_hosts requires a 'name'"))?;
        let state = args
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("present");
        let path = known_hosts_path()?;
        if state == "absent" {
            remove_host(host, &path)
        } else {
            add_host(host, &path)
        }
    }
}

/// Resolve `~/.ssh/known_hosts` via the `HOME` env var (`~` is not expanded by std).
fn known_hosts_path() -> Result<std::path::PathBuf, ModuleError> {
    let home = std::env::var("HOME").map_err(|_| {
        ModuleError::Other("HOME not set; cannot locate ~/.ssh/known_hosts".to_string())
    })?;
    Ok(std::path::PathBuf::from(home)
        .join(".ssh")
        .join("known_hosts"))
}

/// `ssh-keyscan` the host and append its key (v0.1: append only).
fn add_host(host: &str, known_hosts: &Path) -> Result<ModuleResult, ModuleError> {
    let scan = std::process::Command::new("ssh-keyscan")
        .args(["-H", host])
        .output()
        .map_err(|e| ModuleError::Other(format!("ssh-keyscan failed: {e}")))?;
    if !scan.status.success() {
        let stderr = String::from_utf8_lossy(&scan.stderr).to_string();
        return Ok(ModuleResult::failure(
            scan.status.code().unwrap_or(1),
            stderr,
        ));
    }
    let appended = if scan.stdout.is_empty() {
        false
    } else {
        append_bytes(known_hosts, &scan.stdout)?;
        true
    };
    Ok(with_changed(
        ok_with_stdout(&format!("keyscan for {host}")),
        appended,
    ))
}

/// Remove all entries for `host` from the `known_hosts` file via `ssh-keygen -R`.
fn remove_host(host: &str, known_hosts: &Path) -> Result<ModuleResult, ModuleError> {
    let out = std::process::Command::new("ssh-keygen")
        .args(["-R", host, "-f"])
        .arg(known_hosts)
        .output()
        .map_err(|e| ModuleError::Other(format!("ssh-keygen failed: {e}")))?;
    let rc = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    Ok(ModuleResult {
        changed: out.status.success(),
        rc,
        stdout: RString::new(),
        stderr: RString::from(stderr),
        success: out.status.success(),
        msg: ROption::RSome(RStr::from("removed")),
    })
}

/// Append `bytes` to `path`, creating the file if it does not exist.
fn append_bytes(path: &Path, bytes: &[u8]) -> Result<(), ModuleError> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| ModuleError::Other(format!("open {}: {e}", path.display())))?;
    f.write_all(bytes)
        .map_err(|e| ModuleError::Other(format!("write {}: {e}", path.display())))?;
    Ok(())
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
    fn hostname_runs_hostnamectl() {
        let core = null_core();
        let r = Hostname
            .run(&conn(&core), &json!({"name": "web-01"}), &noop_ctx())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed);
        assert!(r.success);
        assert_eq!(r.stdout.as_str(), "hostnamectl set-hostname web-01");
    }

    #[test]
    fn hostname_missing_name_errors() {
        let core = null_core();
        assert!(Hostname.run(&conn(&core), &json!({}), &noop_ctx()).is_err());
    }

    #[test]
    fn timezone_runs_timedatectl() {
        let core = null_core();
        let r = Timezone
            .run(
                &conn(&core),
                &json!({"name": "Europe/Amsterdam"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed);
        assert_eq!(
            r.stdout.as_str(),
            "timedatectl set-timezone Europe/Amsterdam"
        );
    }

    #[test]
    fn git_fresh_clone_reports_changed() {
        let core = null_core();
        let r = Git
            .run(
                &conn(&core),
                &json!({"repo": "https://example.invalid/a.git", "dest": "/tmp/repo"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed);
        assert!(r.stdout.as_str().contains("git clone"));
    }

    #[test]
    fn git_falls_back_to_update_when_clone_fails() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 1,
            success: false,
            stdout: RString::from("fatal: destination path already exists"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Git
            .run(
                &conn(&core_ref),
                &json!({"repo": "u", "dest": "d"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        // Second run_command hits the default mock → success → changed.
        assert!(r.changed);
    }

    #[test]
    fn git_missing_repo_errors() {
        let core = null_core();
        assert!(
            Git.run(&conn(&core), &json!({"dest": "d"}), &noop_ctx())
                .is_err()
        );
    }

    #[test]
    fn pip_present_runs_install() {
        let core = null_core();
        let r = Pip
            .run(&conn(&core), &json!({"name": "requests"}), &noop_ctx())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed);
        assert!(Pip.requires_remote_python());
        assert!(r.stdout.as_str().contains("install requests"));
    }

    #[test]
    fn pip_virtualenv_overrides_default_executable() {
        let core = null_core();
        let r = Pip
            .run(
                &conn(&core),
                &json!({"name": "pkg", "virtualenv": "/opt/venv"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.stdout.as_str().contains("/opt/venv/bin/pip"));
    }

    #[test]
    fn pip_absent_joins_array() {
        let core = null_core();
        let r = Pip
            .run(
                &conn(&core),
                &json!({"name": ["a", "b"], "state": "absent"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.stdout.as_str().contains("uninstall -y a b"));
    }

    #[test]
    fn pip_missing_name_errors() {
        let core = null_core();
        assert!(Pip.run(&conn(&core), &json!({}), &noop_ctx()).is_err());
    }

    #[test]
    fn stat_emits_json_for_existing_path() {
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_run(ModuleResult {
            changed: false,
            rc: 0,
            success: true,
            stdout: RString::from("/etc/hosts|regular file|123|root|root|644|1700000000"),
            stderr: RString::new(),
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let r = Stat
            .run(
                &conn(&core_ref),
                &json!({"path": "/etc/hosts"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(!r.changed);
        assert!(r.success);
        let parsed: Value =
            serde_json::from_str(r.stdout.as_str()).unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(parsed["exists"], true);
        assert_eq!(parsed["stat"]["isreg"], true);
        assert_eq!(parsed["stat"]["owner"], "root");
        assert_eq!(parsed["stat"]["size"], 123);
    }

    #[test]
    fn parse_stat_missing_path_reports_no_exist() {
        let v = parse_stat("", "/no/such");
        assert_eq!(v["exists"], false);
    }

    #[test]
    fn parse_stat_directory() {
        let v = parse_stat("/var|directory|4096|root|root|755|1", "/var");
        assert_eq!(v["stat"]["isdir"], true);
        assert_eq!(v["stat"]["isreg"], false);
    }

    #[test]
    fn known_hosts_missing_name_errors() {
        // Arg validation only — does not touch the filesystem.
        assert!(
            KnownHosts
                .run(&conn(&null_core()), &json!({}), &noop_ctx())
                .is_err()
        );
    }
}
