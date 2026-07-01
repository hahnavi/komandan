//! Reuse executors: route Ansible module names to komandan's built-in modules.
//!
//! Each translates Ansible field names to komandan's per-module arg schemas (see
//! the schema map in `crates/komandan-core/src/modules/`) and dispatches via
//! [`Connection::komando`]. Spec §6.3 "Reuse of existing komandan modules".

use komandan_plugin_abi::prelude::*;
use komandan_plugin_abi::{RHashMap, RStr};
use serde_json::Value;

use super::{Connection, ModuleError, ModuleExecutor, ModuleRegistry, TaskContext};

/// Register every reuse executor on `reg`.
pub fn register_all(reg: &mut ModuleRegistry) {
    reg.register(Command);
    reg.register(Apt);
    reg.register(Dnf);
    reg.register(FileMod);
    reg.register(Copy);
    reg.register(Fetch);
    reg.register(GetUrl);
    reg.register(LineInFile);
    reg.register(Template);
    reg.register(User);
    reg.register(Group);
    reg.register(Systemd);
    reg.register(Script);
    reg.register(PostgresqlUser);
}

/// Promote a dynamic string slice to an `RStr<'static>` by leaking it.
///
/// komandan is a short-lived CLI process (one playbook run per invocation), so
/// the leak is process-bounded and benign — there is no long-running daemon.
/// This is the sound, `unsafe`-free way to satisfy the `RStr<'static>` keys the
/// v1 ABI requires for `TaskInput.args` (the alternative — a lifetime
/// transmute — is denied by the workspace `unsafe_code` lint).
fn leaked_rstr(s: &str) -> RStr<'static> {
    crate::leak::rstr(s)
}

/// Convert a rendered JSON value into the ABI [`RValue`] (strings leaked to
/// `'static` via [`leaked_rstr`]).
fn to_rvalue(v: &Value) -> RValue {
    match v {
        Value::Null => RValue::Null,
        Value::Bool(b) => RValue::Bool(*b),
        Value::Number(n) => n
            .as_i64()
            .map(RValue::Int)
            .or_else(|| n.as_f64().map(RValue::Float))
            .unwrap_or(RValue::Null),
        Value::String(s) => RValue::Str(leaked_rstr(s)),
        Value::Array(a) => RValue::List(a.iter().map(to_rvalue).collect()),
        Value::Object(o) => {
            let mut m = RHashMap::new();
            for (k, val) in o {
                m.insert(leaked_rstr(k), to_rvalue(val));
            }
            RValue::Map(m)
        }
    }
}

/// Dispatch a komandan built-in module against the host via the pooled
/// connection. `module` is always a `&'static str` literal (no leak needed).
fn dispatch(
    conn: &Connection<'_>,
    module: &'static str,
    args: RHashMap<RStr<'static>, RValue>,
) -> Result<ModuleResult, ModuleError> {
    conn.komando(TaskInput {
        module_name: RStr::from(module),
        args,
        name: ROption::RNone,
        description: ROption::RNone,
    })
}

/// Build a check-mode stub result for a module that would mutate.
///
/// Reports `changed: true` (it *would* change) with `success: true`, without
/// touching the host. The message string leak is process-bounded (see
/// [`leaked_rstr`]).
fn check_mode_result(module: &str) -> ModuleResult {
    ModuleResult {
        changed: true,
        msg: ROption::RSome(leaked_rstr(&format!("{module}: check mode, would execute"))),
        ..ModuleResult::ok()
    }
}

/// Read a remote file via `cat`. Returns an empty string on any error (missing
/// file, permission denied, etc.) — callers use this for best-effort before/
/// after diff capture, not for authoritative reads.
fn cat_remote(conn: &Connection<'_>, path: &str) -> String {
    conn.run_command(&format!("cat '{path}'"))
        .map(|r| r.stdout.to_string())
        .unwrap_or_default()
}

/// Dispatch a komandan module, optionally capturing before/after file content
/// for `--diff` display.
///
/// When `ctx.diff_mode` is true and `file_path` is `Some`, reads the file
/// before and after the dispatch call and prepends a unified diff to the
/// result's stdout (only when the dispatch reported a change).
fn dispatch_with_diff(
    conn: &Connection<'_>,
    ctx: &TaskContext,
    module: &'static str,
    args: RHashMap<RStr<'static>, RValue>,
    file_path: Option<&str>,
) -> Result<ModuleResult, ModuleError> {
    let before = if ctx.diff_mode {
        file_path.map(|p| cat_remote(conn, p))
    } else {
        None
    };

    let mut result = dispatch(conn, module, args)?;

    if ctx.diff_mode
        && result.changed
        && let Some(path) = file_path
    {
        let after = cat_remote(conn, path);
        let diff = super::compute_file_diff(path, before.as_deref().unwrap_or(""), &after);
        if !diff.is_empty() {
            let combined = format!("{}\n{}", diff, result.stdout.as_str());
            result.stdout = RString::from(combined);
        }
    }

    Ok(result)
}

/// Builder for a komandan arg map with translated keys.
struct ArgMap {
    inner: RHashMap<RStr<'static>, RValue>,
}

impl ArgMap {
    fn new() -> Self {
        Self {
            inner: RHashMap::new(),
        }
    }
    /// Insert `src_key` from the rendered args under komandan's `dst_key`.
    fn carry(mut self, args: &Value, src_key: &str, dst_key: &str) -> Self {
        if let Some(v) = args.get(src_key)
            && !v.is_null()
        {
            self.inner.insert(leaked_rstr(dst_key), to_rvalue(v));
        }
        self
    }
    /// Insert a literal key/value.
    fn set(mut self, dst_key: &str, v: RValue) -> Self {
        self.inner.insert(leaked_rstr(dst_key), v);
        self
    }
}

// ---- command / shell / raw → cmd ----------------------------------------

struct Command;

impl ModuleExecutor for Command {
    fn name(&self) -> &'static str {
        "command"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["shell", "raw"]
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let cmd = if let Some(s) = args.as_str() {
            s.to_string()
        } else {
            args.get("cmd")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| {
                    args.get("argv").and_then(Value::as_array).map(|a| {
                        a.iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                })
                .ok_or_else(|| ModuleError::args("command/shell requires a 'cmd' string"))?
        };
        let prefix = super::shell_prefix(ctx);
        let full_cmd = if prefix.is_empty() {
            cmd
        } else {
            format!("{prefix}{cmd}")
        };
        let map = ArgMap::new().set("cmd", RValue::Str(leaked_rstr(&full_cmd)));
        dispatch(conn, "cmd", map.inner)
    }
}

// ---- apt → apt ----------------------------------------------------------

struct Apt;

impl ModuleExecutor for Apt {
    fn name(&self) -> &'static str {
        "apt"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("apt"));
        }
        let action = pkg_action_for_apt(args);
        let mut map = ArgMap::new();
        if let Some(name) = args.get("name").or_else(|| args.get("package")) {
            map.inner.insert(leaked_rstr("package"), to_rvalue(name));
        }
        map = map.carry(args, "update_cache", "update_cache");
        map = map.set("action", RValue::Str(leaked_rstr(&action)));
        dispatch(conn, "apt", map.inner)
    }
}

/// Ansible `state` → komandan apt `action` (present→install, absent→remove,
/// latest→upgrade; default present).
fn pkg_action_for_apt(args: &Value) -> String {
    match args.get("state").and_then(Value::as_str) {
        Some("absent") => "remove",
        Some("latest" | "upgrade") => "upgrade",
        Some("purge") => "purge",
        _ => "install",
    }
    .to_string()
}

// ---- dnf / yum → dnf ----------------------------------------------------

struct Dnf;

impl ModuleExecutor for Dnf {
    fn name(&self) -> &'static str {
        "dnf"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["yum"]
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("dnf"));
        }
        let action = pkg_action_for_dnf(args);
        let mut map = ArgMap::new();
        if let Some(name) = args.get("name").or_else(|| args.get("package")) {
            map.inner.insert(leaked_rstr("package"), to_rvalue(name));
        }
        map = map.carry(args, "update_cache", "update_cache");
        map = map.set("action", RValue::Str(leaked_rstr(&action)));
        dispatch(conn, "dnf", map.inner)
    }
}

/// Ansible `state` → komandan dnf `action` (present→install, absent→remove,
/// latest→upgrade; default present).
fn pkg_action_for_dnf(args: &Value) -> String {
    match args.get("state").and_then(Value::as_str) {
        Some("absent" | "removed") => "remove",
        Some("latest" | "upgrade" | "updated") => "upgrade",
        _ => "install",
    }
    .to_string()
}

// ---- file → file --------------------------------------------------------

struct FileMod;

impl ModuleExecutor for FileMod {
    fn name(&self) -> &'static str {
        "file"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("file"));
        }
        let path = args
            .get("path")
            .or_else(|| args.get("dest"))
            .or_else(|| args.get("name"))
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("file requires a 'path'"))?;
        let map = ArgMap::new()
            .set("path", RValue::Str(leaked_rstr(path)))
            .carry(args, "state", "state")
            .carry(args, "src", "src")
            .carry(args, "mode", "mode")
            .carry(args, "owner", "owner")
            .carry(args, "group", "group");
        dispatch(conn, "file", map.inner)
    }
}

// ---- copy → upload (or write_file for content:) -------------------------

struct Copy;

impl ModuleExecutor for Copy {
    fn name(&self) -> &'static str {
        "copy"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        let dest = args
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("copy requires a 'dest'"))?;
        if let Some(content) = args.get("content").and_then(Value::as_str) {
            // `content:` writes bytes directly (no local source file to upload).
            let before = if ctx.diff_mode {
                cat_remote(conn, dest)
            } else {
                String::new()
            };
            if !ctx.check_mode {
                conn.write_file(dest, content.as_bytes())?;
            }
            let mut stdout = RString::new();
            if ctx.diff_mode {
                let diff = super::compute_file_diff(dest, &before, content);
                if !diff.is_empty() {
                    stdout = RString::from(diff);
                }
            }
            return Ok(ModuleResult {
                changed: true,
                rc: 0,
                stdout,
                stderr: RString::new(),
                success: true,
                msg: ROption::RSome(RStr::from(if ctx.check_mode {
                    "would write content"
                } else {
                    "wrote content"
                })),
            });
        }
        let src = args
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("copy requires 'src' or 'content'"))?;
        if ctx.check_mode {
            return Ok(check_mode_result("copy"));
        }
        let map = ArgMap::new()
            .set("src", RValue::Str(leaked_rstr(src)))
            .set("dst", RValue::Str(leaked_rstr(dest)));
        dispatch_with_diff(conn, ctx, "upload", map.inner, Some(dest))
    }
}

// ---- fetch → download ---------------------------------------------------

struct Fetch;

impl ModuleExecutor for Fetch {
    fn name(&self) -> &'static str {
        "fetch"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("fetch"));
        }
        let src = args
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("fetch requires a 'src'"))?;
        let dest = args
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("fetch requires a 'dest'"))?;
        let map = ArgMap::new()
            .set("src", RValue::Str(leaked_rstr(src)))
            .set("dst", RValue::Str(leaked_rstr(dest)));
        dispatch(conn, "download", map.inner)
    }
}

// ---- get_url → get_url --------------------------------------------------

struct GetUrl;

impl ModuleExecutor for GetUrl {
    fn name(&self) -> &'static str {
        "get_url"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("get_url"));
        }
        let url = args
            .get("url")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("get_url requires a 'url'"))?;
        let dest = args
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("get_url requires a 'dest'"))?;
        let map = ArgMap::new()
            .set("url", RValue::Str(leaked_rstr(url)))
            .set("dst", RValue::Str(leaked_rstr(dest)))
            .carry(args, "force", "force");
        dispatch(conn, "get_url", map.inner)
    }
}

// ---- lineinfile → lineinfile -------------------------------------------

struct LineInFile;

impl ModuleExecutor for LineInFile {
    fn name(&self) -> &'static str {
        "lineinfile"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("lineinfile"));
        }
        let path = args
            .get("path")
            .or_else(|| args.get("dest"))
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("lineinfile requires a 'path'"))?;
        let map = ArgMap::new()
            .set("path", RValue::Str(leaked_rstr(path)))
            // Ansible `regexp` ≈ komandan `pattern`.
            .carry(args, "regexp", "pattern")
            .carry(args, "pattern", "pattern")
            .carry(args, "line", "line")
            .carry(args, "state", "state")
            .carry(args, "create", "create")
            .carry(args, "backup", "backup")
            .carry(args, "insertafter", "insert_after")
            .carry(args, "insertbefore", "insert_before");
        dispatch_with_diff(conn, ctx, "lineinfile", map.inner, Some(path))
    }
}

// ---- template → template -----------------------------------------------

struct Template;

impl ModuleExecutor for Template {
    fn name(&self) -> &'static str {
        "template"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("template"));
        }
        let src = args
            .get("src")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("template requires a 'src'"))?;
        let dest = args
            .get("dest")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("template requires a 'dest'"))?;
        let map = ArgMap::new()
            .set("src", RValue::Str(leaked_rstr(src)))
            .set("dst", RValue::Str(leaked_rstr(dest)))
            .carry(args, "vars", "vars");
        dispatch_with_diff(conn, ctx, "template", map.inner, Some(dest))
    }
}

// ---- user → user --------------------------------------------------------

struct User;

impl ModuleExecutor for User {
    fn name(&self) -> &'static str {
        "user"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("user"));
        }
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("user requires a 'name'"))?;
        let map = ArgMap::new()
            .set("name", RValue::Str(leaked_rstr(name)))
            .carry(args, "state", "state")
            .carry(args, "uid", "uid")
            .carry(args, "group", "group")
            .carry(args, "groups", "groups")
            .carry(args, "home", "home")
            .carry(args, "shell", "shell")
            .carry(args, "password", "password")
            .carry(args, "system", "system")
            .carry(args, "create_home", "create_home")
            .carry(args, "remove", "remove")
            .carry(args, "force", "force");
        dispatch(conn, "user", map.inner)
    }
}

// ---- group → group ------------------------------------------------------

struct Group;

impl ModuleExecutor for Group {
    fn name(&self) -> &'static str {
        "group"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("group"));
        }
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("group requires a 'name'"))?;
        let map = ArgMap::new()
            .set("name", RValue::Str(leaked_rstr(name)))
            .carry(args, "state", "state")
            .carry(args, "gid", "gid")
            .carry(args, "system", "system")
            .carry(args, "force", "force")
            .carry(args, "non_unique", "non_unique")
            // Ansible `local` ≈ komandan `local_group`.
            .carry(args, "local", "local_group");
        dispatch(conn, "group", map.inner)
    }
}

// ---- systemd / service → systemd_service --------------------------------

struct Systemd;

impl ModuleExecutor for Systemd {
    fn name(&self) -> &'static str {
        "systemd"
    }
    fn aliases(&self) -> &'static [&'static str] {
        &["service", "ansible.builtin.systemd"]
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("systemd"));
        }
        let unit = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("systemd requires a 'name'"))?;
        let actions = systemd_actions(args);
        if actions.is_empty() {
            return Err(ModuleError::args("systemd requires a 'state' or 'enabled'"));
        }
        let mut combined = ModuleResult::ok();
        let mut any_changed = false;
        for action in actions {
            let map = ArgMap::new()
                .set("name", RValue::Str(leaked_rstr(unit)))
                .set("action", RValue::Str(leaked_rstr(&action)))
                .carry(args, "force", "force")
                .carry(args, "daemon_reload", "daemon_reload");
            let r = dispatch(conn, "systemd_service", map.inner)?;
            any_changed |= r.changed;
            combined = r;
        }
        combined.changed = any_changed;
        Ok(combined)
    }
}

/// Translate Ansible's independent `state` + `enabled` into a sequence of
/// komandan `systemd_service` actions (applied in order).
fn systemd_actions(args: &Value) -> Vec<String> {
    let mut out = Vec::new();
    match args.get("state").and_then(Value::as_str) {
        Some("started") => out.push("start".to_string()),
        Some("stopped") => out.push("stop".to_string()),
        Some("restarted") => out.push("restart".to_string()),
        Some("reloaded") => out.push("reload".to_string()),
        _ => {}
    }
    match args.get("enabled") {
        Some(Value::Bool(true)) => out.push("enable".to_string()),
        Some(Value::Bool(false)) => out.push("disable".to_string()),
        _ => {}
    }
    out
}

// ---- script → script ----------------------------------------------------

struct Script;

impl ModuleExecutor for Script {
    fn name(&self) -> &'static str {
        "script"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("script"));
        }
        // Ansible `script:` is free-form: "<local_path> [args...]". komandan's
        // script module uploads+runs a local file. v0.1 takes the first token
        // as the script path (extra args are ignored — documented limitation).
        let free = args
            .as_str()
            .or_else(|| args.get("cmd").and_then(Value::as_str))
            .ok_or_else(|| ModuleError::args("script requires a free-form command"))?;
        let path = free
            .split_whitespace()
            .next()
            .ok_or_else(|| ModuleError::args("script command is empty"))?
            .to_string();
        let mut map = ArgMap::new().set("from_file", RValue::Str(leaked_rstr(&path)));
        if let Some(interp) = args.get("executable").and_then(Value::as_str) {
            map = map.set("interpreter", RValue::Str(leaked_rstr(interp)));
        }
        dispatch(conn, "script", map.inner)
    }
}

// ---- postgresql_user → postgresql_user ---------------------------------

struct PostgresqlUser;

impl ModuleExecutor for PostgresqlUser {
    fn name(&self) -> &'static str {
        "postgresql_user"
    }
    fn supports_check_mode(&self) -> bool {
        true
    }
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError> {
        if ctx.check_mode {
            return Ok(check_mode_result("postgresql_user"));
        }
        let name = args
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| ModuleError::args("postgresql_user requires a 'name'"))?;
        let action = match args.get("state").and_then(Value::as_str) {
            Some("absent") => "drop",
            _ => "create",
        };
        let map = ArgMap::new()
            .set("name", RValue::Str(leaked_rstr(name)))
            .set("action", RValue::Str(leaked_rstr(action)))
            .carry(args, "role_attr_flags", "role_attr_flags")
            .carry(args, "password", "password");
        dispatch(conn, "postgresql_user", map.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{MockCore, localhost_host};
    use serde_json::json;

    fn conn(core: &komandan_plugin_abi::CoreApiRef) -> Connection<'_> {
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

    /// Helper to build a registry for alias-resolution tests.
    fn full_registry() -> ModuleRegistry {
        let mut r = ModuleRegistry::default();
        super::register_all(&mut r);
        r
    }

    #[test]
    fn command_routes_to_cmd_module() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        Command
            .run(&conn(&core_ref), &json!("echo hi"), &noop_ctx())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(handle.komando_calls().contains(&"cmd".to_string()));
    }

    #[test]
    fn command_with_environment_prefixes_cmd() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        let mut ctx = noop_ctx();
        ctx.environment
            .insert("PATH".to_string(), "/custom/bin".to_string());
        Command
            .run(&conn(&core_ref), &json!("echo hi"), &ctx)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let cmds = handle.komando_cmds();
        assert!(
            cmds.iter().any(|c| c.contains("PATH='/custom/bin'")),
            "dispatched cmds: {cmds:?}"
        );
        assert!(
            cmds.iter().any(|c| c.contains("echo hi")),
            "dispatched cmds: {cmds:?}"
        );
    }

    #[test]
    fn command_without_environment_passes_cmd_unchanged() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        Command
            .run(&conn(&core_ref), &json!("echo hi"), &noop_ctx())
            .unwrap_or_else(|e| panic!("{e:?}"));
        let cmds = handle.komando_cmds();
        assert!(
            cmds.iter().any(|c| c == "echo hi"),
            "dispatched cmds: {cmds:?}"
        );
    }

    #[test]
    fn apt_translates_state_present_to_install() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        Apt.run(
            &conn(&core_ref),
            &json!({"name": "nginx", "state": "present"}),
            &noop_ctx(),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(handle.komando_calls().contains(&"apt".to_string()));
    }

    #[test]
    fn yum_alias_routes_to_dnf() {
        let reg = full_registry();
        assert!(reg.lookup("yum").is_some());
    }

    #[test]
    fn copy_content_uses_write_file() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        let r = Copy
            .run(
                &conn(&core_ref),
                &json!({"content": "hi", "dest": "/tmp/x"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed);
        // content path must NOT invoke komando.
        assert!(handle.komando_calls().is_empty());
    }

    #[test]
    fn systemd_translates_state_and_enabled() {
        assert_eq!(
            systemd_actions(&json!({"state": "started", "enabled": true})),
            vec!["start".to_string(), "enable".to_string()]
        );
    }

    /// A `TaskContext` with `diff_mode` enabled (everything else default).
    fn diff_ctx() -> TaskContext {
        let mut ctx = TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        );
        ctx.diff_mode = true;
        ctx
    }

    /// A `TaskContext` with `check_mode` enabled (everything else default).
    fn check_ctx() -> TaskContext {
        let mut ctx = TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        );
        ctx.check_mode = true;
        ctx
    }

    #[test]
    fn cat_remote_returns_stdout_on_success() {
        // The mock `executor_run` echoes the command back as stdout, so
        // `cat_remote` surfaces that — verifying it does not panic and
        // forwards the captured stdout.
        let core = MockCore::default();
        let core_ref = core.into_ref();
        let s = cat_remote(&conn(&core_ref), "/tmp/diff-test.txt");
        assert!(s.contains("cat '/tmp/diff-test.txt'"), "{s}");
    }

    #[test]
    fn dispatch_with_diff_skips_diff_when_not_changed() {
        // Stage a `changed: false` komando result with a marker stdout; with
        // diff_mode enabled the helper must NOT prepend a diff.
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_komando(ModuleResult {
            changed: false,
            rc: 0,
            stdout: RString::from("marker-stdout"),
            stderr: RString::new(),
            success: true,
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let map = ArgMap::new()
            .set("path", RValue::Str(leaked_rstr("/tmp/x")))
            .inner;
        let r = dispatch_with_diff(
            &conn(&core_ref),
            &diff_ctx(),
            "lineinfile",
            map,
            Some("/tmp/x"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(r.stdout.as_str(), "marker-stdout");
        assert!(
            !r.stdout.as_str().contains("--- "),
            "no diff expected: {}",
            r.stdout
        );
        // The komando dispatch itself was made.
        assert!(handle.komando_calls().contains(&"lineinfile".to_string()));
    }

    #[test]
    fn dispatch_with_diff_runs_safely_in_diff_mode() {
        // diff_mode on, changed result, but before/after cat both echo the
        // same command from the mock → diff empty → stdout untouched. The
        // helper must complete without panicking.
        let core = MockCore::default();
        let handle = core.handle();
        handle.expect_komando(ModuleResult {
            changed: true,
            rc: 0,
            stdout: RString::from("changed-stdout"),
            stderr: RString::new(),
            success: true,
            msg: ROption::RNone,
        });
        let core_ref = core.into_ref();
        let map = ArgMap::new()
            .set("path", RValue::Str(leaked_rstr("/tmp/y")))
            .inner;
        let r = dispatch_with_diff(
            &conn(&core_ref),
            &diff_ctx(),
            "lineinfile",
            map,
            Some("/tmp/y"),
        )
        .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(r.stdout.as_str(), "changed-stdout");
        assert!(handle.komando_calls().contains(&"lineinfile".to_string()));
    }

    #[test]
    fn copy_content_emits_diff_in_diff_mode() {
        // `copy: content=` path: the mock cat echoes the command (non-empty
        // "before"), and the new content differs → a diff header must appear
        // in stdout.
        let core = MockCore::default();
        let core_ref = core.into_ref();
        let r = Copy
            .run(
                &conn(&core_ref),
                &json!({"content": "new-line\n", "dest": "/tmp/z"}),
                &diff_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "copy content should report changed");
        assert!(
            r.stdout.as_str().contains("--- /tmp/z:before"),
            "expected diff header in stdout: {}",
            r.stdout
        );
        assert!(
            r.stdout.as_str().contains("+new-line"),
            "expected added line in diff: {}",
            r.stdout
        );
    }

    #[test]
    fn copy_content_skips_diff_when_not_diff_mode() {
        // No diff_mode → stdout stays empty on the content path.
        let core = MockCore::default();
        let core_ref = core.into_ref();
        let r = Copy
            .run(
                &conn(&core_ref),
                &json!({"content": "hi\n", "dest": "/tmp/w"}),
                &noop_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed);
        assert!(
            r.stdout.as_str().is_empty(),
            "no diff expected: {}",
            r.stdout
        );
    }

    #[test]
    fn apt_check_mode_skips_komando() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        let r = Apt
            .run(
                &conn(&core_ref),
                &json!({"name": "nginx", "state": "present"}),
                &check_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "apt check mode should report changed");
        assert!(r.success, "apt check mode should report success");
        assert!(
            handle.komando_calls().is_empty(),
            "apt check mode must not dispatch komando: {:?}",
            handle.komando_calls()
        );
    }

    #[test]
    fn user_check_mode_skips_komando() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        let r = User
            .run(
                &conn(&core_ref),
                &json!({"name": "bob", "state": "present"}),
                &check_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "user check mode should report changed");
        assert!(r.success, "user check mode should report success");
        assert!(
            handle.komando_calls().is_empty(),
            "user check mode must not dispatch komando: {:?}",
            handle.komando_calls()
        );
    }

    #[test]
    fn lineinfile_check_mode_skips_komando() {
        let core = MockCore::default();
        let handle = core.handle();
        let core_ref = core.into_ref();
        let r = LineInFile
            .run(
                &conn(&core_ref),
                &json!({"path": "/tmp/x", "line": "hi"}),
                &check_ctx(),
            )
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert!(r.changed, "lineinfile check mode should report changed");
        assert!(r.success, "lineinfile check mode should report success");
        assert!(
            handle.komando_calls().is_empty(),
            "lineinfile check mode must not dispatch komando: {:?}",
            handle.komando_calls()
        );
    }
}
