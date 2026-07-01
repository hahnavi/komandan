//! Test-only helpers shared across in-crate unit tests: a configurable mock
//! [`komandan_plugin_abi::CoreApi`] plus connection/host builders.
//!
//! Not compiled into release builds (`#[cfg(test)]`-gated at the declaration
//! site in `lib.rs`).

use std::sync::{Arc, Mutex};

use komandan_plugin_abi::prelude::*;
use komandan_plugin_abi::{ConnectionHandle, CoreApi_TO, CoreApiRef, RArc, RHashMap, TD_Opaque};

/// Observable state shared between a [`MockCore`] and its [`MockHandle`], so a
/// test can inspect recorded calls after the core has been moved into the
/// `RArc`-backed `CoreApiRef`.
#[derive(Debug, Default)]
struct MockState {
    run_result: Mutex<Option<ModuleResult>>,
    komando_result: Mutex<Option<ModuleResult>>,
    komando_calls: Mutex<Vec<String>>,
    /// Recorded `cmd` args from `komando` calls dispatched to the `cmd` module
    /// (so reuse-executor tests can assert on the dispatched command string,
    /// including any `environment:` prefix).
    komando_cmds: Mutex<Vec<String>>,
}

/// A mock `CoreApi`. By default every connection/executor call succeeds with
/// [`ModuleResult::ok()`]; canned responses can be staged through the
/// [`MockHandle`] returned by [`MockCore::handle`].
#[derive(Debug, Default)]
pub struct MockCore {
    state: Arc<MockState>,
}

/// An observer handle for a [`MockCore`]; clone out before wrapping the core
/// into a `CoreApiRef`.
#[derive(Debug, Clone)]
pub struct MockHandle {
    state: Arc<MockState>,
}

impl MockCore {
    /// Clone out an observer sharing the same recorded state.
    #[must_use]
    pub fn handle(&self) -> MockHandle {
        MockHandle {
            state: Arc::clone(&self.state),
        }
    }

    /// Build a `CoreApiRef` trait object from this mock (consumes it).
    #[must_use]
    pub fn into_ref(self) -> CoreApiRef {
        CoreApi_TO::from_ptr(RArc::new(self), TD_Opaque)
    }
}

impl MockHandle {
    /// Stage a canned `komando` result.
    #[allow(dead_code)] // test helper; not every test exercises it.
    pub fn expect_komando(&self, r: ModuleResult) {
        if let Ok(mut g) = self.state.komando_result.lock() {
            *g = Some(r);
        }
    }

    /// Stage a canned `executor_run` result.
    #[allow(dead_code)] // test helper; not every test exercises it.
    pub fn expect_run(&self, r: ModuleResult) {
        if let Ok(mut g) = self.state.run_result.lock() {
            *g = Some(r);
        }
    }

    /// Recorded `komando` task module names, in call order.
    #[must_use]
    pub fn komando_calls(&self) -> Vec<String> {
        self.state
            .komando_calls
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }

    /// Recorded `cmd` arg strings from `komando` calls (in call order), so a
    /// reuse-executor test can assert on the exact dispatched command —
    /// including any `environment:` prefix prepended by the executor.
    #[must_use]
    pub fn komando_cmds(&self) -> Vec<String> {
        self.state
            .komando_cmds
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default()
    }
}

impl komandan_plugin_abi::CoreApi for MockCore {
    fn create_connection(&self, _host: HostInfo) -> RResult<ConnectionHandle, CoreError> {
        // A fixed non-zero id so it is not the INVALID sentinel.
        ROk(ConnectionHandle::from_id(1))
    }
    fn executor_run(
        &self,
        _conn: &ConnectionHandle,
        command: RStr<'_>,
    ) -> RResult<ModuleResult, CoreError> {
        let canned = self.state.run_result.lock().map_or(None, |mut g| g.take());
        ROk(canned.unwrap_or_else(|| ModuleResult {
            changed: true,
            rc: 0,
            stdout: RString::from(command.to_string()),
            stderr: RString::new(),
            success: true,
            msg: ROption::RNone,
        }))
    }
    fn executor_upload(
        &self,
        _conn: &ConnectionHandle,
        _local: RStr<'_>,
        _remote: RStr<'_>,
    ) -> RResult<(), CoreError> {
        ROk(())
    }
    fn executor_write_file(
        &self,
        _conn: &ConnectionHandle,
        _path: RStr<'_>,
        _bytes: RVec<u8>,
    ) -> RResult<(), CoreError> {
        ROk(())
    }
    fn close_connection(&self, _conn: ConnectionHandle) {}
    fn komando(&self, task: TaskInput, _host: HostInfo) -> RResult<ModuleResult, CoreError> {
        if let Ok(mut g) = self.state.komando_calls.lock() {
            g.push(task.module_name.to_string());
        }
        // Capture the dispatched command string for `cmd`-module calls so
        // reuse-executor tests can assert on the full command (e.g. with an
        // `environment:` prefix). Other modules' args are not recorded.
        if task.module_name.as_str() == "cmd"
            && let Ok(mut g) = self.state.komando_cmds.lock()
        {
            for entry in &task.args {
                if entry.0.as_str() == "cmd"
                    && let RValue::Str(s) = entry.1
                {
                    g.push(s.to_string());
                }
            }
        }
        let canned = self
            .state
            .komando_result
            .lock()
            .map_or(None, |mut g| g.take());
        ROk(canned.unwrap_or_else(ModuleResult::ok))
    }
    fn defaults_get(&self, _key: RStr<'_>) -> ROption<RValue> {
        ROption::RNone
    }
    fn defaults_set(&self, _key: RStr<'_>, _value: RValue) -> RResult<(), CoreError> {
        ROk(())
    }
    fn report_record(&self, _task: RStr<'_>, _host: RStr<'_>, _status: ReportStatus) {}
    fn host_info(&self) -> HostInfo {
        localhost_host()
    }
    fn now_playing_task(&self) -> TaskInput {
        TaskInput {
            module_name: RStr::from(""),
            args: RHashMap::new(),
            name: ROption::RNone,
            description: ROption::RNone,
        }
    }
    fn worker_lua(&self) -> LuaHandle {
        LuaHandle::new()
    }
    fn log(&self, _level: LogLevel, _msg: RStr<'_>) {}
    fn global_flags(&self) -> GlobalFlags {
        GlobalFlags::default()
    }
}

/// A no-op `CoreApiRef` (every call returns a default/success).
#[must_use]
pub fn null_core() -> CoreApiRef {
    MockCore::default().into_ref()
}

/// A `local`-connection `HostInfo` for `localhost`.
#[must_use]
pub fn localhost_host() -> HostInfo {
    HostInfo {
        name: ROption::RSome(RStr::from("localhost")),
        address: RStr::from("127.0.0.1"),
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
