//! Host-side implementations of the plugin-facing traits.
//!
//! [`HostCore`] implements [`CoreApi`](komandan_plugin_abi::CoreApi) backed by
//! komandan-core internals:
//!
//! - `create_connection` / `executor_run` / `executor_upload` /
//!   `executor_write_file` / `close_connection` — a process-local
//!   [`ConnectionRegistry`] keyed by [`ConnectionHandle`] IDs, driving the
//!   existing [`Connection`] / [`CommandExecutor`] surface.
//! - `defaults_get` / `defaults_set` — a key router over [`Defaults::global`]
//!   (v0.1: scalar keys only; strings/complex/secrets are documented gaps).
//! - `report_record` — bridges to [`report::insert_record`]
//!   (`ReportStatus::Skipped` is dropped — komandan-core has no Skipped).
//! - `global_flags` / `host_info` / `log` / `worker_lua` / `now_playing_task`
//!   — read live state or return documented placeholders.
//! - `komando` — dispatches a task through the public Lua entrypoint
//!   `komandan.komando`. Module dispatch is Lua-only (`task[1]:run()` runs
//!   inside a Lua chunk) and `mlua::Lua` is `!Send`, so each plugin host
//!   thread gets its own lazily-seeded VM via the `PLUGIN_LUA` thread-local
//!   (mirrors `komando.rs`'s rayon-worker pool, decoupled so the load-bearing
//!   parallel path stays untouched). See [`komando`] impl + [`with_plugin_lua`].
//!
//! [`LoggerSink`](komandan_plugin_abi::LoggerSink) is fully wired via
//! [`HostLogger`] into komandan's `tracing` subscriber.
//!
//! # Concurrency
//!
//! `CoreApi` requires `Send + Sync`. [`HostCore`] holds its mutable state
//! (connection registry, ambient host) behind [`Mutex`]; [`Connection`] /
//! [`HostInfo`] are themselves `Send + Sync`, so the whole struct is. There is
//! no shared `mlua::Lua` *field*: `create_connection` builds a fresh VM on the
//! calling thread and drops it before returning, while `komando` reuses a
//! per-thread VM from the `PLUGIN_LUA` thread-local (seeded once via
//! [`create_lua`], never moved across threads — `Lua` is `!Send`). The
//! thread-local is a static, not a struct field, so `HostCore`'s `Send + Sync`
//! derive is unaffected.

use std::cell::OnceCell;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use komandan_plugin_abi::{
    ConnectionHandle, CoreApi, CoreError, GlobalFlags, HostInfo, LogLevel, LoggerSink, LuaHandle,
    ModuleResult, ROption, RResult, RStr, RValue, ReportStatus, TaskInput,
};
use mlua::{IntoLua, Lua, LuaSerdeExt, Value};

use crate::connection::{Connection, create_connection};
use crate::create_lua;
use crate::defaults::Defaults;
use crate::executor::CommandExecutor;
use crate::plugin::conversions;

/// Real host core backing the plugin `CoreApi`.
///
/// Holds the connection registry + ambient host behind a `Mutex` (the
/// `CoreApi` trait is `Send + Sync`). `Default` is hand-impl'd (not derived)
/// so the connection ID counter starts at 1, not 0.
#[derive(Debug)]
pub struct HostCore {
    connections: Mutex<ConnectionRegistry>,
    ambient_host: Mutex<HostInfo>,
}

/// Process-local connection store, keyed by `ConnectionHandle::id`.
#[derive(Debug)]
struct ConnectionRegistry {
    /// Next ID to hand out. Skips `0` (reserved as `ConnectionHandle::INVALID`).
    next_id: u64,
    map: HashMap<u64, Connection>,
}

impl HostCore {
    /// Construct a host core with an empty connection registry (`next_id`
    /// starts at 1; `0` is reserved for `ConnectionHandle::INVALID`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            connections: Mutex::new(ConnectionRegistry {
                next_id: 1,
                map: HashMap::new(),
            }),
            ambient_host: Mutex::new(default_host_info()),
        }
    }
}

impl Default for HostCore {
    fn default() -> Self {
        Self::new()
    }
}

/// Host log sink: forwards plugin log lines into the `tracing` subscriber.
#[derive(Debug)]
pub struct HostLogger;

/// A minimal local `HostInfo` used to seed the v0.1 [`PluginContext`] when no
/// real task/host is in flight.
///
/// [`PluginContext`]: komandan_plugin_abi::PluginContext
#[must_use]
pub fn default_host_info() -> HostInfo {
    HostInfo {
        name: ROption::RNone,
        address: RStr::from(""),
        port: ROption::RNone,
        user: ROption::RNone,
        ssh_key_path: ROption::RNone,
        private_key_pass: ROption::RNone,
        password: ROption::RNone,
        become_method: ROption::RNone,
        become_user: ROption::RNone,
        elevate: ROption::RNone,
        connection_type: RStr::from("local"),
    }
}

/// Map an ABI [`LogLevel`] onto the host `tracing` subscriber.
fn log_via_tracing(level: LogLevel, msg: &str) {
    match level {
        LogLevel::Trace => tracing::trace!("{msg}"),
        LogLevel::Debug => tracing::debug!("{msg}"),
        LogLevel::Info => tracing::info!("{msg}"),
        LogLevel::Warn => tracing::warn!("{msg}"),
        LogLevel::Error => tracing::error!("{msg}"),
        // LogLevel is #[non_exhaustive]; unknown levels fall back to info.
        _ => tracing::info!("{msg}"),
    }
}

/// Poisoned-lock message helper (`PoisonError` does not impl `std::error::Error`).
fn lock_err<E: std::fmt::Display>(e: E, what: &str) -> anyhow::Error {
    anyhow::anyhow!("{what} lock poisoned: {e}")
}

// One lazily-created Lua VM per plugin host thread.
//
// `mlua::Lua` is `!Send`, so each thread that drives a plugin gets its own VM
// (seeded once via `create_lua` and reused across calls); the VM never crosses
// threads. This mirrors `komando.rs`'s rayon-worker `WORKER_LUA` pool, kept
// separate so the load-bearing parallel-executor path stays untouched (see
// `AGENTS.md` perf invariants).
thread_local! {
    static PLUGIN_LUA: OnceCell<Lua> = const { OnceCell::new() };
}
/// Run `f` against this thread's plugin Lua VM (created lazily on first use).
///
/// The VM is reused across `komando` calls on the same thread, amortizing the
/// `create_lua()` cost (14-module registration + komandan table). Errors from
/// VM init or the body are surfaced as a `komando`-kinded [`CoreError`].
fn with_plugin_lua<R>(f: impl FnOnce(&Lua) -> anyhow::Result<R>) -> RResult<R, CoreError> {
    let outcome = PLUGIN_LUA.try_with(|cell| {
        let lua = cell.get_or_try_init(create_lua)?;
        f(lua)
    });
    match outcome {
        Ok(Ok(r)) => RResult::ROk(r),
        Ok(Err(e)) => RResult::RErr(CoreError::new("komando", e.to_string())),
        Err(_) => RResult::RErr(CoreError::new(
            "komando",
            "plugin thread-local Lua access failed",
        )),
    }
}

impl CoreApi for HostCore {
    fn create_connection(&self, host: HostInfo) -> RResult<ConnectionHandle, CoreError> {
        // Track ambient host for the host_info() accessor.
        if let Ok(mut guard) = self.ambient_host.lock() {
            *guard = host.clone();
        }

        // Lua-bridge (cold path): HostInfo -> Host -> Lua Value -> the existing
        // `connection::create_connection`. The Lua VM is created fresh on this
        // thread, used only to parse host config into a Connection, then
        // dropped — the returned Connection owns its SSHSession/LocalSession
        // and does not borrow the Lua. Each significant-Drop temporary (`lua`,
        // the registry guard) is scoped to the minimum block that needs it.
        let bridge = (|| -> anyhow::Result<ConnectionHandle> {
            let conn = {
                let lua = create_lua()?;
                let host_model = conversions::host_info_to_host(&host);
                let host_value = host_model.into_lua(&lua)?;
                create_connection(&lua, &host_value)?
            };
            let id = {
                let mut reg = self
                    .connections
                    .lock()
                    .map_err(|e| lock_err(e, "connection registry"))?;
                let id = reg.next_id;
                reg.next_id = reg
                    .next_id
                    .checked_add(1)
                    .ok_or_else(|| anyhow::anyhow!("connection id space exhausted"))?;
                reg.map.insert(id, conn);
                id
            };
            Ok(ConnectionHandle::from_id(id))
        })();
        match bridge {
            Ok(handle) => RResult::ROk(handle),
            Err(e) => RResult::RErr(CoreError::new("connection", e.to_string())),
        }
    }

    fn executor_run(
        &self,
        conn: &ConnectionHandle,
        command: RStr<'_>,
    ) -> RResult<ModuleResult, CoreError> {
        // The registry guard must be held for the whole body: `entry` borrows
        // `reg.map`, and `cmd` is `&mut self`. Releasing the guard early would
        // require cloning the `Connection` (an SSH session), which is riskier
        // than holding the process-local lock.
        #[allow(clippy::significant_drop_tightening)]
        let res = (|| -> anyhow::Result<ModuleResult> {
            let mut reg = self
                .connections
                .lock()
                .map_err(|e| lock_err(e, "connection registry"))?;
            let entry = reg
                .map
                .get_mut(&conn.id)
                .ok_or_else(|| anyhow::anyhow!("unknown connection id {}", conn.id))?;
            let (stdout, stderr, rc) = entry.cmd(command.as_str())?;
            let changed = match &*entry {
                Connection::SSH(s) => s.get_changed(),
                Connection::Local(l) => l.get_changed(),
            };
            Ok(ModuleResult {
                changed,
                rc,
                stdout: stdout.into(),
                stderr: stderr.into(),
                success: rc == 0,
                msg: ROption::RNone,
            })
        })();
        match res {
            Ok(m) => RResult::ROk(m),
            Err(e) => RResult::RErr(CoreError::new("executor", e.to_string())),
        }
    }

    fn executor_upload(
        &self,
        conn: &ConnectionHandle,
        local: RStr<'_>,
        remote: RStr<'_>,
    ) -> RResult<(), CoreError> {
        // Guard held for the upload call (entry borrows the map); see
        // `executor_run` rationale.
        #[allow(clippy::significant_drop_tightening)]
        let res = (|| -> anyhow::Result<()> {
            let reg = self
                .connections
                .lock()
                .map_err(|e| lock_err(e, "connection registry"))?;
            let entry = reg
                .map
                .get(&conn.id)
                .ok_or_else(|| anyhow::anyhow!("unknown connection id {}", conn.id))?;
            let local_path = Path::new(local.as_str());
            let remote_path = Path::new(remote.as_str());
            match entry {
                Connection::SSH(s) => s.upload(local_path, remote_path)?,
                Connection::Local(l) => l.upload(local_path, remote_path)?,
            }
            Ok(())
        })();
        match res {
            Ok(()) => RResult::ROk(()),
            Err(e) => RResult::RErr(CoreError::new("executor", e.to_string())),
        }
    }

    fn executor_write_file(
        &self,
        conn: &ConnectionHandle,
        path: RStr<'_>,
        bytes: komandan_plugin_abi::RVec<u8>,
    ) -> RResult<(), CoreError> {
        // Guard held for the write call (entry borrows the map); see
        // `executor_run` rationale.
        #[allow(clippy::significant_drop_tightening)]
        let res = (|| -> anyhow::Result<()> {
            let reg = self
                .connections
                .lock()
                .map_err(|e| lock_err(e, "connection registry"))?;
            let entry = reg
                .map
                .get(&conn.id)
                .ok_or_else(|| anyhow::anyhow!("unknown connection id {}", conn.id))?;
            let remote_path = Path::new(path.as_str());
            match entry {
                Connection::SSH(s) => s.write_remote_file(remote_path, bytes.as_slice())?,
                Connection::Local(l) => l.write_remote_file(remote_path, bytes.as_slice())?,
            }
            Ok(())
        })();
        match res {
            Ok(()) => RResult::ROk(()),
            Err(e) => RResult::RErr(CoreError::new("io", e.to_string())),
        }
    }

    fn close_connection(&self, conn: ConnectionHandle) {
        if let Ok(mut reg) = self.connections.lock() {
            if reg.map.remove(&conn.id).is_none() && !conn.is_invalid() {
                tracing::warn!("close_connection: unknown connection id {}", conn.id);
            }
        } else {
            tracing::warn!("close_connection: connection registry lock poisoned");
        }
    }

    fn komando(&self, task: TaskInput, host: HostInfo) -> RResult<ModuleResult, CoreError> {
        // Dispatch a plugin-supplied task through the public Lua entrypoint
        // `komandan.komando(task, host)`. Module dispatch is Lua-only
        // (`task[1]:run()` runs inside komando's `execute_task` chunk), so we
        // must drive it on a Lua VM. The VM comes from this thread's
        // `PLUGIN_LUA` pool (lazily seeded once) rather than a fresh
        // `create_lua()` per call.
        with_plugin_lua(|lua| -> anyhow::Result<ModuleResult> {
            // 1. Build the module-params table from the plugin's flat arg map.
            let params = lua.create_table()?;
            for entry in &task.args {
                params.set(entry.0.as_str(), conversions::rvalue_to_lua(lua, entry.1)?)?;
            }
            // 2. Instantiate the built-in module: komandan.modules.<name>(params).
            let komandan: mlua::Table = lua.globals().get("komandan")?;
            let modules: mlua::Table = komandan.get("modules")?;
            let module_fn: mlua::Function = modules.get(task.module_name.as_str())?;
            let module: mlua::Table = module_fn.call(params)?;
            // 3. Assemble the task table: positional [1] = module; optional `name`.
            let task_tbl = lua.create_table()?;
            task_tbl.set(1, module)?;
            if let ROption::RSome(name) = &task.name {
                task_tbl.set("name", name.as_str())?;
            }
            // 4. Host table via the existing HostInfo -> Host -> IntoLua bridge.
            let host_value = conversions::host_info_to_host(&host).into_lua(lua)?;
            // 5. Dispatch via the public Lua entrypoint komandan.komando(task, host).
            let komando_fn: mlua::Function = komandan.get("komando")?;
            let result: mlua::Table = komando_fn.call((Value::Table(task_tbl), host_value))?;
            // 6. Parse + convert to the plugin mirror type.
            let parsed = lua.from_value::<crate::models::KomandoResult>(Value::Table(result))?;
            Ok(conversions::komando_result_to_module_result(&parsed))
        })
    }

    fn defaults_get(&self, key: RStr<'_>) -> ROption<RValue> {
        // v0.1: scalar (bool/int) defaults only. String defaults are not
        // exposed because `RValue::Str` holds `RStr<'static>` (owned-string
        // support needs an ABI addition). Secrets/complex types are
        // deliberately out of scope.
        let d = Defaults::global();
        let val = match key.as_str() {
            "port" => d.port.read().ok().map(|g| RValue::Int(i64::from(*g))),
            "ignore_exit_code" => d.ignore_exit_code.read().ok().map(|g| RValue::Bool(*g)),
            "elevate" => d.elevate.read().ok().map(|g| RValue::Bool(*g)),
            "host_key_check" => d.key_check.read().ok().map(|g| RValue::Bool(*g)),
            "ssh_auto_discover_keys" => d
                .ssh_auto_discover_keys
                .read()
                .ok()
                .map(|g| RValue::Bool(*g)),
            _ => None,
        };
        val.map_or(ROption::RNone, ROption::RSome)
    }

    fn defaults_set(&self, key: RStr<'_>, value: RValue) -> RResult<(), CoreError> {
        let d = Defaults::global();
        let res = (|| -> anyhow::Result<()> {
            match key.as_str() {
                "port" => {
                    let RValue::Int(n) = value else {
                        anyhow::bail!("defaults_set(`port`): expected Int");
                    };
                    let n =
                        u16::try_from(n).map_err(|e| anyhow::anyhow!("port out of range: {e}"))?;
                    *d.port.write().map_err(|e| lock_err(e, "defaults"))? = n;
                }
                "ignore_exit_code" => {
                    let RValue::Bool(b) = value else {
                        anyhow::bail!("defaults_set(`ignore_exit_code`): expected Bool");
                    };
                    *d.ignore_exit_code
                        .write()
                        .map_err(|e| lock_err(e, "defaults"))? = b;
                }
                "elevate" => {
                    let RValue::Bool(b) = value else {
                        anyhow::bail!("defaults_set(`elevate`): expected Bool");
                    };
                    *d.elevate.write().map_err(|e| lock_err(e, "defaults"))? = b;
                }
                "host_key_check" => {
                    let RValue::Bool(b) = value else {
                        anyhow::bail!("defaults_set(`host_key_check`): expected Bool");
                    };
                    *d.key_check.write().map_err(|e| lock_err(e, "defaults"))? = b;
                }
                "ssh_auto_discover_keys" => {
                    let RValue::Bool(b) = value else {
                        anyhow::bail!("defaults_set(`ssh_auto_discover_keys`): expected Bool");
                    };
                    *d.ssh_auto_discover_keys
                        .write()
                        .map_err(|e| lock_err(e, "defaults"))? = b;
                }
                // Secrets & complex defaults are reserved (must not be writable
                // through the generic key/value ABI surface).
                "password" | "private_key_pass" | "env" | "hosts" | "user" | "as_user"
                | "elevation_method" | "known_hosts_file" => {
                    anyhow::bail!("defaults_set: `{key}` is reserved in v0.1");
                }
                _ => anyhow::bail!("defaults_set: unknown key `{key}`"),
            }
            Ok(())
        })();
        match res {
            Ok(()) => RResult::ROk(()),
            Err(e) => RResult::RErr(CoreError::new("defaults", e.to_string())),
        }
    }

    fn report_record(&self, task: RStr<'_>, host: RStr<'_>, status: ReportStatus) {
        // ReportStatus::Skipped has no komandan-core equivalent — drop it.
        if let Some(task_status) = conversions::report_status_to_task_status(status) {
            crate::report::insert_record(task.to_string(), host.to_string(), task_status);
        }
    }

    fn host_info(&self) -> HostInfo {
        self.ambient_host
            .lock()
            .map_or_else(|_| default_host_info(), |guard| guard.clone())
    }

    fn now_playing_task(&self) -> TaskInput {
        // v0.1 has no per-call task context; return an empty descriptor.
        TaskInput {
            module_name: RStr::from(""),
            args: komandan_plugin_abi::RHashMap::new(),
            name: ROption::RNone,
            description: ROption::RNone,
        }
    }

    fn worker_lua(&self) -> LuaHandle {
        // v0.1 placeholder (Lua is !Send; the worker-Lua accessor returns a
        // probe-only marker until the closure-marshalling design lands).
        LuaHandle::new()
    }

    fn log(&self, level: LogLevel, msg: RStr<'_>) {
        log_via_tracing(level, msg.as_str());
    }

    fn global_flags(&self) -> GlobalFlags {
        conversions::flags_to_global_flags(&crate::args::global_flags())
    }
}

impl LoggerSink for HostLogger {
    fn log(&self, level: LogLevel, msg: RStr<'_>) {
        log_via_tracing(level, msg.as_str());
    }
}
