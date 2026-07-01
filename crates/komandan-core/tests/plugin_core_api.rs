//! Integration test for the wired `CoreApi` host impl (`plugin::core_api::HostCore`).
//!
//! Drives the real connection/executor path against the local transport: a
//! localhost `HostInfo` → `create_connection` (Lua-bridged) → `executor_run`
//! runs an actual `echo` on this machine → asserts the captured stdout. Also
//! round-trips a scalar default, drops the connection, and asserts a
//! follow-up `executor_run` errors on the now-unknown handle.

use komandan::plugin::core_api::HostCore;
use komandan_plugin_abi::{
    ConnectionHandle, CoreApi_TO, HostInfo, LogLevel, RArc, ROption, RResult, RStr, RValue,
    ReportStatus, TD_Opaque, TaskInput,
};

/// Build a `HostCore` wrapped in the `#[sabi_trait]` dyn object the way the
/// plugin dispatcher does.
fn core() -> CoreApi_TO<'static, RArc<()>> {
    CoreApi_TO::from_ptr(RArc::new(HostCore::new()), TD_Opaque)
}

/// Minimal localhost `HostInfo` (local transport ⇒ no SSH creds needed).
fn local_host() -> HostInfo {
    HostInfo {
        name: ROption::RNone,
        address: RStr::from("localhost"),
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

/// Drain an `RResult` into a panic with context on failure.
fn unwrap_r<T, E: std::fmt::Debug>(r: RResult<T, E>, ctx: &str) -> T {
    match r.into_result() {
        Ok(v) => v,
        Err(e) => panic!("{ctx}: {e:?}"),
    }
}

#[test]
fn create_local_connection_runs_real_command() {
    let core = core();
    let handle = unwrap_r(core.create_connection(local_host()), "create_connection");
    assert!(
        !handle.is_invalid(),
        "expected a non-zero connection handle"
    );

    let marker = "hello_core_api";
    let cmd = format!("echo {marker}");
    let res = unwrap_r(
        core.executor_run(&handle, RStr::from(cmd.as_str())),
        "executor_run",
    );
    assert!(res.success, "echo should exit 0");
    assert_eq!(res.rc, 0);
    assert_eq!(
        res.stdout.as_str().trim(),
        marker,
        "captured stdout mismatch"
    );

    core.close_connection(handle);
}

#[test]
fn executor_run_on_unknown_handle_errors() {
    let core = core();
    let bogus = ConnectionHandle::from_id(999_999);
    match core.executor_run(&bogus, RStr::from("true")).into_result() {
        Ok(_) => panic!("expected an error for an unknown connection id"),
        Err(e) => assert!(
            e.message.as_str().contains("unknown connection id"),
            "unexpected error: {e:?}"
        ),
    }
}

#[test]
fn executor_run_after_close_errors() {
    let core = core();
    let handle = unwrap_r(core.create_connection(local_host()), "create_connection");
    core.close_connection(handle);
    match core
        .executor_run(&handle, RStr::from("echo nope"))
        .into_result()
    {
        Ok(_) => panic!("expected error running on a closed connection"),
        Err(e) => assert!(
            e.message.as_str().contains("unknown connection id"),
            "unexpected error: {e:?}"
        ),
    }
}

#[test]
fn defaults_scalar_round_trip() {
    let core = core();

    // `elevate` default is false; flip it true via the plugin ABI and read back.
    let before = core.defaults_get(RStr::from("elevate"));
    assert!(matches!(before, ROption::RSome(RValue::Bool(false))));

    unwrap_r(
        core.defaults_set(RStr::from("elevate"), RValue::Bool(true)),
        "defaults_set(elevate=true)",
    );

    let after = core.defaults_get(RStr::from("elevate"));
    assert!(matches!(after, ROption::RSome(RValue::Bool(true))));
}

#[test]
fn defaults_unknown_key_is_unset() {
    let core = core();
    assert!(matches!(
        core.defaults_get(RStr::from("no-such-key")),
        ROption::RNone
    ));
}

#[test]
fn defaults_reserved_key_rejected() {
    let core = core();
    match core
        .defaults_set(RStr::from("password"), RValue::Null)
        .into_result()
    {
        Ok(()) => panic!("setting a reserved key must error"),
        Err(e) => assert!(
            e.kind.as_str() == "defaults" && e.message.as_str().contains("reserved"),
            "unexpected error: {e:?}"
        ),
    }
}

#[test]
fn komando_runs_cmd_module_via_plugin_abi() {
    // End-to-end: the plugin ABI `komando` dispatches the built-in `cmd` module
    // against a local host and returns captured stdout. Exercises the full
    // bridge (thread-local Lua pool -> komandan.modules.cmd(params) ->
    // komandan.komando(task, host) -> KomandoResult -> ModuleResult).
    let core = core();
    let mut args = komandan_plugin_abi::RHashMap::new();
    args.insert(
        RStr::from("cmd"),
        RValue::str_literal("echo plugin_komando_ok"),
    );
    let res = unwrap_r(
        core.komando(
            TaskInput {
                module_name: RStr::from("cmd"),
                args,
                name: ROption::RSome(RStr::from("greet")),
                description: ROption::RNone,
            },
            local_host(),
        ),
        "komando(cmd)",
    );
    assert!(res.success, "echo must exit 0; got rc={}", res.rc);
    assert_eq!(res.rc, 0);
    assert!(
        res.stdout.as_str().contains("plugin_komando_ok"),
        "stdout must contain marker; got: {}",
        res.stdout
    );
}

#[test]
fn komando_unknown_module_errors_gracefully() {
    // A bogus module name must surface as a `komando`-kinded CoreError (Lua
    // raises a type error trying to treat nil as a Function), not a panic.
    let core = core();
    let res = core.komando(
        TaskInput {
            module_name: RStr::from("no-such-module"),
            args: komandan_plugin_abi::RHashMap::new(),
            name: ROption::RNone,
            description: ROption::RNone,
        },
        local_host(),
    );
    match res.into_result() {
        Ok(m) => panic!("expected error for unknown module, got: {m:?}"),
        Err(e) => assert!(
            e.kind.as_str() == "komando",
            "expected kind=`komando`, got: {e:?}"
        ),
    }
}

#[test]
fn report_record_and_log_are_fire_and_forget() {
    let core = core();
    // These must not panic; their observable effect (report vector / tracing
    // sink) is not asserted here.
    core.report_record(
        RStr::from("demo-task"),
        RStr::from("localhost"),
        ReportStatus::Changed,
    );
    core.report_record(
        RStr::from("demo-task"),
        RStr::from("localhost"),
        ReportStatus::Skipped, // dropped (no Skipped variant in core)
    );
    core.log(LogLevel::Info, RStr::from("integration test log line"));
}

#[test]
fn host_info_reflects_last_connection() {
    let core = core();
    // Before any connection, host_info returns the default (local, empty address).
    let initial = core.host_info();
    assert_eq!(initial.address.as_str(), "");

    // create_connection updates the ambient host; host_info should mirror it.
    let _ = unwrap_r(core.create_connection(local_host()), "create_connection");
    let after = core.host_info();
    assert_eq!(after.address.as_str(), "localhost");
    assert_eq!(after.connection_type.as_str(), "local");
}
