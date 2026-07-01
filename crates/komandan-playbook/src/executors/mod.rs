//! Native module execution: the [`ModuleExecutor`] trait, [`ModuleRegistry`],
//! the [`Connection`] host-abstraction, and the per-task [`TaskContext`].
//!
//! Spec: `docs/PLAYBOOK_SPEC.md` §6.
//!
//! Reuse executors ([`reuse`]) dispatch komandan's existing built-in modules
//! through [`Connection::komando`]; raw executors reach the host via
//! [`Connection::run_command`] / [`Connection::upload`] /
//! [`Connection::write_file`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use indexmap::IndexMap;
use komandan_plugin_abi::prelude::*;
use komandan_plugin_abi::{ConnectionHandle, CoreApiRef};
use thiserror::Error;

pub mod control;
pub mod gap;
pub mod gap_files;
pub mod gap_misc;
pub mod gap_ops;
pub mod reuse;

/// Error from module lookup or execution.
#[derive(Debug, Error)]
pub enum ModuleError {
    /// The module name resolved to nothing registered in this build.
    #[error(
        "module '{0}' is not implemented in this komandan build — see docs/ansible-compat.md for the supported-module matrix"
    )]
    NotImplemented(String),
    /// A host-side failure surfaced through `CoreApi`.
    #[error("host error: {0}")]
    Host(String),
    /// Bad / missing module arguments.
    #[error("invalid arguments: {0}")]
    Args(String),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}

impl ModuleError {
    /// Construct an [`ModuleError::Args`].
    #[must_use]
    pub fn args(msg: impl Into<String>) -> Self {
        Self::Args(msg.into())
    }
}

/// Become / privilege-elevation settings (merged from play + task).
#[derive(Debug, Clone, Default)]
pub struct BecomeSettings {
    /// Whether elevation was requested.
    pub enabled: bool,
    /// `sudo` / `su` / `doas` / ...
    pub method: Option<String>,
    /// User to become.
    pub user: Option<String>,
}

/// Runtime inventory additions (`add_host` / `group_by`).
///
/// Shared across plays via [`RuntimeInventory`] so that hosts added in one play
/// are visible in the next.
#[derive(Debug, Default)]
pub struct RuntimeAdditions {
    /// `hostname → host vars` (JSON object of `ansible_*` and custom vars).
    pub hosts: IndexMap<String, serde_json::Value>,
    /// `group name → [hostnames]`.
    pub groups: HashMap<String, Vec<String>>,
}

impl RuntimeAdditions {
    /// Add a host with its vars and register it in the named groups.
    pub fn add_host(&mut self, name: &str, groups: &[String], vars: serde_json::Value) {
        self.hosts.entry(name.to_string()).or_insert(vars);
        for g in groups {
            self.groups
                .entry(g.clone())
                .or_default()
                .push(name.to_string());
        }
    }

    /// Add a host to a runtime group.
    pub fn add_to_group(&mut self, group: &str, host: &str) {
        self.groups
            .entry(group.to_string())
            .or_default()
            .push(host.to_string());
    }
}

/// Shared, thread-safe runtime inventory additions.
pub type RuntimeInventory = std::sync::Arc<std::sync::Mutex<RuntimeAdditions>>;

/// Control-flow signal a control executor (`meta`) can raise; the runner
/// inspects it after each task.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FlowControl {
    /// Keep going.
    #[default]
    Continue,
    /// Stop processing the current host.
    EndHost,
    /// Stop processing every host in the current play.
    EndPlay,
    /// Flush notified handlers now (then reset to [`FlowControl::Continue`]).
    FlushHandlers,
}

/// Per-task execution context: settings, a read-only variable snapshot, a
/// shared mutable sink for `set_fact` write-back, and a control-flow signal.
#[derive(Debug)]
pub struct TaskContext {
    /// `--check` / dry-run.
    pub check_mode: bool,
    /// `--diff` mode: show before/after content for file changes.
    pub diff_mode: bool,
    /// `no_log:` on the task.
    pub no_log: bool,
    /// Effective become settings (merged from play + task); drive the
    /// `become_prefix` portion of [`shell_prefix`].
    pub become_settings: BecomeSettings,
    /// Per-task `environment:` map.
    pub environment: HashMap<String, String>,
    /// Runtime inventory additions (`add_host` / `group_by`).
    pub runtime: RuntimeInventory,
    /// Flattened variable snapshot for the host+play+task (read-only).
    pub vars: serde_json::Value,
    /// Shared sink: `set_fact` writes land here; the runner drains it into the
    /// per-host var store after the executor returns.
    facts: Arc<Mutex<IndexMap<String, serde_json::Value>>>,
    /// Shared control-flow signal (`meta` writes here; the runner reads).
    flow: Arc<Mutex<FlowControl>>,
}

impl TaskContext {
    /// Build a context from a variable snapshot and shared sinks.
    #[must_use]
    pub fn new(
        vars: serde_json::Value,
        facts: Arc<Mutex<IndexMap<String, serde_json::Value>>>,
        flow: Arc<Mutex<FlowControl>>,
        runtime: RuntimeInventory,
    ) -> Self {
        Self {
            check_mode: false,
            diff_mode: false,
            no_log: false,
            become_settings: BecomeSettings::default(),
            environment: HashMap::new(),
            runtime,
            vars,
            facts,
            flow,
        }
    }

    /// Record a `set_fact:` key (called by the `set_fact` executor).
    pub fn set_fact(&self, key: &str, value: serde_json::Value) {
        if let Ok(mut f) = self.facts.lock() {
            f.insert(key.to_string(), value);
        }
    }

    /// Raise a control-flow directive (called by the `meta` executor).
    pub fn set_flow(&self, flow: FlowControl) {
        if let Ok(mut cur) = self.flow.lock() {
            // A stronger directive wins over a weaker one.
            if *cur != FlowControl::EndPlay && flow != FlowControl::Continue {
                *cur = flow;
            }
        }
    }

    /// Build an empty (default) fact sink.
    #[must_use]
    pub fn empty_facts() -> Arc<Mutex<IndexMap<String, serde_json::Value>>> {
        Arc::new(Mutex::new(IndexMap::new()))
    }

    /// Build a default (Continue) flow signal.
    #[must_use]
    pub fn default_flow() -> Arc<Mutex<FlowControl>> {
        Arc::new(Mutex::new(FlowControl::Continue))
    }

    /// Build an empty (default) runtime inventory handle.
    #[must_use]
    pub fn empty_runtime() -> RuntimeInventory {
        std::sync::Arc::new(std::sync::Mutex::new(RuntimeAdditions::default()))
    }
}

/// An open host connection: a borrowed `CoreApi` handle + a pooled connection id
/// + the host's [`HostInfo`]. Executors reach the host exclusively through this.
///
/// Borrows the `CoreApiRef` (the handle is `RArc`-backed but not `Clone` under
/// the v1 `abi_stable` interface — see `komandan-plugin-abi::traits`), so a
/// [`Connection`] is tied to the borrow's lifetime.
#[derive(Debug)]
pub struct Connection<'core> {
    core: &'core CoreApiRef,
    handle: ConnectionHandle,
    host: HostInfo,
}

impl<'core> Connection<'core> {
    /// Build a connection view from its parts.
    #[must_use]
    pub const fn new(core: &'core CoreApiRef, handle: ConnectionHandle, host: HostInfo) -> Self {
        Self { core, handle, host }
    }

    /// The pooled connection id.
    #[must_use]
    pub const fn handle(&self) -> &ConnectionHandle {
        &self.handle
    }

    /// The host this connection targets.
    #[must_use]
    pub const fn host(&self) -> &HostInfo {
        &self.host
    }

    /// The host `CoreApi` handle.
    #[must_use]
    pub const fn core(&self) -> &'core CoreApiRef {
        self.core
    }

    /// Run a raw shell command on the host. A non-zero exit is **not** an
    /// error here — inspect [`ModuleResult::success`] / [`ModuleResult::rc`].
    ///
    /// # Errors
    ///
    /// [`ModuleError::Host`] on I/O / connection failure.
    pub fn run_command(&self, command: &str) -> Result<ModuleResult, ModuleError> {
        self.core
            .executor_run(&self.handle, RStr::from(command))
            .into_result()
            .map_err(|e| ModuleError::Host(e.message.to_string()))
    }

    /// Upload a local file to a remote path.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Host`] on I/O failure.
    pub fn upload(&self, local: &str, remote: &str) -> Result<(), ModuleError> {
        self.core
            .executor_upload(&self.handle, RStr::from(local), RStr::from(remote))
            .into_result()
            .map_err(|e| ModuleError::Host(e.message.to_string()))
    }

    /// Atomically write `bytes` to `path` on the host.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Host`] on write failure.
    pub fn write_file(&self, path: &str, bytes: &[u8]) -> Result<(), ModuleError> {
        self.core
            .executor_write_file(&self.handle, RStr::from(path), RVec::from(bytes.to_vec()))
            .into_result()
            .map_err(|e| ModuleError::Host(e.message.to_string()))
    }

    /// Reuse path: dispatch a komandan built-in module (`cmd`/`apt`/`file`/...)
    /// against the host via the host's `komando()`.
    ///
    /// # Errors
    ///
    /// [`ModuleError::Host`] on connection / module failure.
    pub fn komando(&self, task: TaskInput) -> Result<ModuleResult, ModuleError> {
        self.core
            .komando(task, self.host.clone())
            .into_result()
            .map_err(|e| ModuleError::Host(e.message.to_string()))
    }
}

/// A native module executor: receives rendered args + an open connection +
/// per-task context, returns a [`ModuleResult`].
///
/// Spec §6.1. The signature diverges from the spec only in taking
/// `&Connection` (immutable) rather than `&mut Connection`: no v0.1 executor
/// needs exclusive access (the pooled connection is stateless across calls),
/// and the spec's `&mut` anticipated connection-state caching that v0.1 does
/// not do.
pub trait ModuleExecutor: Send + Sync {
    /// Canonical name this executor answers to (post-aliasing).
    fn name(&self) -> &'static str;
    /// Aliases that also resolve to this executor.
    fn aliases(&self) -> &'static [&'static str] {
        &[]
    }
    /// Whether this module honors `--check`.
    fn supports_check_mode(&self) -> bool {
        false
    }
    /// Whether this module needs Python on the target (spec §6.4).
    fn requires_remote_python(&self) -> bool {
        false
    }
    /// Execute the module.
    ///
    /// # Errors
    ///
    /// [`ModuleError`] on lookup / arg / host failure.
    fn run(
        &self,
        conn: &Connection<'_>,
        args: &serde_json::Value,
        ctx: &TaskContext,
    ) -> Result<ModuleResult, ModuleError>;
}

/// The native module registry: canonical name (and aliases) → executor.
#[derive(Default)]
pub struct ModuleRegistry {
    map: HashMap<String, Arc<dyn ModuleExecutor>>,
}

impl ModuleRegistry {
    /// Register an executor under its canonical name and all aliases.
    pub fn register<E: ModuleExecutor + 'static>(&mut self, exec: E) {
        let arc: Arc<dyn ModuleExecutor> = Arc::new(exec);
        for alias in arc.aliases() {
            self.map.insert(alias.to_string(), Arc::clone(&arc));
        }
        self.map.insert(arc.name().to_string(), arc);
    }

    /// Look up an executor by (possibly collection-qualified) module name.
    #[must_use]
    pub fn lookup(&self, module: &str) -> Option<Arc<dyn ModuleExecutor>> {
        self.map.get(&canonicalize(module)).cloned()
    }

    /// Whether `module` (or an alias) is registered.
    #[must_use]
    pub fn contains(&self, module: &str) -> bool {
        self.map.contains_key(&canonicalize(module))
    }

    /// Number of registered canonical names + aliases.
    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    /// Whether no executor is registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

/// Canonicalize a (possibly fully-qualified) module name to its last dotted
/// segment: `ansible.builtin.command` → `command`, `community.general.git` →
/// `git`, `apt` → `apt`.
#[must_use]
pub fn canonicalize(module: &str) -> String {
    match module.rsplit_once('.') {
        Some((_, last)) => last.to_string(),
        None => module.to_string(),
    }
}

/// Build a registry with every v0.1 executor registered.
#[must_use]
pub fn register_all() -> ModuleRegistry {
    let mut r = ModuleRegistry::default();
    reuse::register_all(&mut r);
    control::register_all(&mut r);
    gap::register_all(&mut r);
    gap_files::register_all(&mut r);
    gap_ops::register_all(&mut r);
    gap_misc::register_all(&mut r);
    r
}

/// Build a shell environment-variable prefix from the task context's
/// `environment` map (Ansible `environment:` directive).
///
/// Returns `""` when no env vars are set. Each entry becomes `KEY='value'`
/// (single-quoted, with embedded single quotes escaped per POSIX shell rules).
///
/// Example: `{"FOO": "bar", "BAZ": "qux"}` → `"FOO='bar' BAZ='qux' "`
#[must_use]
pub fn env_prefix(ctx: &TaskContext) -> String {
    if ctx.environment.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for (k, v) in &ctx.environment {
        out.push_str(k);
        out.push_str("='");
        for ch in v.chars() {
            if ch == '\'' {
                out.push_str("'\\''");
            } else {
                out.push(ch);
            }
        }
        out.push_str("' ");
    }
    out
}

/// Build a privilege-elevation prefix from the task context's `become_settings`.
///
/// Returns `""` when elevation is not requested. When enabled with a user,
/// returns `"sudo -u <user> "`; when enabled without a user, returns `"sudo "`.
///
/// (Ansible's default `become_user` is `root`, which `sudo` already defaults
/// to, so bare `sudo` is correct when no user is specified.)
#[must_use]
pub fn become_prefix(ctx: &TaskContext) -> String {
    if !ctx.become_settings.enabled {
        return String::new();
    }
    match &ctx.become_settings.user {
        Some(user) if !user.is_empty() => format!("sudo -u {user} "),
        _ => String::from("sudo "),
    }
}

/// Combined shell prefix: privilege elevation + environment variables.
///
/// This is the standard prefix for all `run_command` calls. Use this instead
/// of calling `become_prefix` and `env_prefix` separately.
#[must_use]
pub fn shell_prefix(ctx: &TaskContext) -> String {
    let mut out = become_prefix(ctx);
    out.push_str(&env_prefix(ctx));
    out
}

/// Compute a unified-diff-style string between two file contents.
///
/// Returns an empty string when `before == after`. Otherwise produces a
/// line-by-line diff with `---`/`+++` headers and `+`/`-`/` ` prefixes.
#[must_use]
pub fn compute_file_diff(path: &str, before: &str, after: &str) -> String {
    use similar::{ChangeTag, TextDiff};
    use std::fmt::Write;
    if before == after {
        return String::new();
    }
    let diff = TextDiff::from_lines(before, after);
    let mut out = String::new();
    let _ = writeln!(out, "--- {path}:before");
    let _ = writeln!(out, "+++ {path}:after");
    for change in diff.iter_all_changes() {
        let prefix = match change.tag() {
            ChangeTag::Delete => '-',
            ChangeTag::Insert => '+',
            ChangeTag::Equal => ' ',
        };
        let _ = write!(out, "{prefix}{}", change.value());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_ctx_with_become(enabled: bool, user: Option<&str>) -> TaskContext {
        let mut ctx = TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        );
        ctx.become_settings = BecomeSettings {
            enabled,
            method: None,
            user: user.map(String::from),
        };
        ctx
    }

    #[test]
    fn canonicalize_strips_collection_prefix() {
        assert_eq!(canonicalize("apt"), "apt");
        assert_eq!(canonicalize("ansible.builtin.command"), "command");
        assert_eq!(canonicalize("ansible.legacy.shell"), "shell");
        assert_eq!(canonicalize("community.general.archive"), "archive");
    }

    #[test]
    fn registry_registers_name_and_aliases() {
        struct Dummy;
        impl ModuleExecutor for Dummy {
            fn name(&self) -> &'static str {
                "dummy"
            }
            fn aliases(&self) -> &'static [&'static str] {
                &["alias_a", "alias_b"]
            }
            fn run(
                &self,
                _conn: &Connection<'_>,
                _args: &serde_json::Value,
                _ctx: &TaskContext,
            ) -> Result<ModuleResult, ModuleError> {
                Ok(ModuleResult::ok())
            }
        }
        let mut r = ModuleRegistry::default();
        r.register(Dummy);
        assert!(r.contains("dummy"));
        assert!(r.contains("alias_a"));
        assert!(r.contains("alias_b"));
        assert!(r.lookup("ansible.builtin.dummy").is_some());
        assert!(r.lookup("nope").is_none());
        assert_eq!(r.len(), 3);
    }

    #[test]
    fn env_prefix_empty_when_no_env() {
        let ctx = TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        );
        assert_eq!(env_prefix(&ctx), "");
    }

    #[test]
    fn env_prefix_formats_key_value_pairs() {
        let mut ctx = TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        );
        ctx.environment.insert("FOO".to_string(), "bar".to_string());
        ctx.environment.insert("BAZ".to_string(), "qux".to_string());
        let prefix = env_prefix(&ctx);
        assert!(prefix.contains("FOO='bar'"), "prefix: {prefix}");
        assert!(prefix.contains("BAZ='qux'"), "prefix: {prefix}");
        assert!(
            prefix.ends_with(' '),
            "prefix should end with space: {prefix}"
        );
    }

    #[test]
    fn env_prefix_escapes_embedded_single_quotes() {
        let mut ctx = TaskContext::new(
            serde_json::Value::Null,
            TaskContext::empty_facts(),
            TaskContext::default_flow(),
            TaskContext::empty_runtime(),
        );
        ctx.environment
            .insert("MSG".to_string(), "it's me".to_string());
        let prefix = env_prefix(&ctx);
        assert!(
            prefix.contains("MSG='it'\\''s me'"),
            "prefix should escape embedded single quote: {prefix}"
        );
    }

    #[test]
    fn become_prefix_empty_when_disabled() {
        let ctx = make_ctx_with_become(false, Some("root"));
        assert_eq!(become_prefix(&ctx), "");
    }

    #[test]
    fn become_prefix_sudo_with_user() {
        let ctx = make_ctx_with_become(true, Some("root"));
        assert_eq!(become_prefix(&ctx), "sudo -u root ");
    }

    #[test]
    fn become_prefix_bare_sudo_without_user() {
        let ctx = make_ctx_with_become(true, None);
        assert_eq!(become_prefix(&ctx), "sudo ");
    }

    #[test]
    fn shell_prefix_combines_become_and_env() {
        let mut ctx = make_ctx_with_become(true, Some("root"));
        ctx.environment.insert("FOO".to_string(), "bar".to_string());
        let prefix = shell_prefix(&ctx);
        assert!(prefix.starts_with("sudo -u root "), "{prefix}");
        assert!(prefix.contains("FOO='bar'"), "{prefix}");
    }

    #[test]
    fn compute_file_diff_shows_changes() {
        let before = "line1\nline2\nline3\n";
        let after = "line1\nline2-modified\nline3\n";
        let diff = compute_file_diff("/tmp/test", before, after);
        assert!(diff.contains("--- /tmp/test:before"), "{diff}");
        assert!(diff.contains("+++ /tmp/test:after"), "{diff}");
        assert!(diff.contains("-line2"), "{diff}");
        assert!(diff.contains("+line2-modified"), "{diff}");
    }

    #[test]
    fn compute_file_diff_empty_when_identical() {
        let content = "same\ncontent\n";
        let diff = compute_file_diff("/tmp/test", content, content);
        assert!(diff.is_empty());
    }

    #[test]
    fn compute_file_diff_shows_insertion() {
        let before = "a\nb\n";
        let after = "a\nb\nc\n";
        let diff = compute_file_diff("/tmp/test", before, after);
        assert!(diff.contains("+c"), "{diff}");
    }
}
