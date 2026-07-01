//! [`PluginContext`] — the bundle a plugin receives when the host dispatches
//! a subcommand invocation to it.

use abi_stable::StableAbi;

use crate::traits::{CoreApiRef, LoggerSinkBox};
use crate::types::HostInfo;

/// What the host hands to [`crate::Plugin::run`] on every dispatch.
///
/// The bundle carries everything a plugin needs to do real work without
/// forcing it to make three separate `CoreApi` calls up front (the same
/// data is also reachable via [`crate::CoreApi`] methods — `host_info()` in
/// particular — but caching it here is the common-case fast path).
///
/// # Ownership
///
/// - `core` is a shared (`RArc`) handle: the host retains one reference,
///   the plugin's `RArc` keeps the underlying `CoreApi` alive for the
///   duration of `run()`.
/// - `host` is a flat clone; cheap (all `RStr` fields).
/// - `logger` is an owned (`RBox`) sink: the host hands it over and the
///   plugin drops it at end-of-`run`.
///
/// Plugins receive this by reference inside [`crate::Plugin::run`]. The
/// bundle is **not** `Clone` in v1: `CoreApiRef` and `LoggerSinkBox` are
/// single-ownership trait objects. Plugins that need to share the context
/// across closures should wrap it in their own `Arc<PluginContext>` (the
/// fields are all cheap to keep alive behind a shared ref).
///
/// # Layout
///
/// `#[repr(C)]` + [`StableAbi`] so adding fields at the end is ABI-compatible
/// for downstream (prefix-type semantics are not needed here because the
/// host is always the constructor and the plugin never embeds this struct
/// into its own public layout).
#[repr(C)]
#[derive(StableAbi)]
pub struct PluginContext {
    /// Shared handle to the host's [`crate::CoreApi`] impl. All host-side
    /// work (connection, executor, komando, defaults, report) flows through
    /// this.
    pub core: CoreApiRef,
    /// The host this dispatch is targeting. Cheap clone of a cached value.
    pub host: HostInfo,
    /// Owned handle to the host's log emitter. Prefer this for tracing over
    /// `core.log()` during early init (e.g. inside `register()`).
    pub logger: LoggerSinkBox,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traits::{CoreApi_TO, LoggerSink_TO};
    use abi_stable::sabi_trait::TD_Opaque;
    use abi_stable::std_types::{RArc, RHashMap, RNone, ROption, RStr};

    // ---- In-test mock impls proving the traits can be type-erased and ----
    // ---- called through the generated `_TO` dyn objects.              ----

    #[derive(Debug)]
    struct EchoCore;

    impl crate::CoreApi for EchoCore {
        fn create_connection(
            &self,
            host: HostInfo,
        ) -> abi_stable::std_types::RResult<crate::ConnectionHandle, crate::CoreError> {
            abi_stable::std_types::ROk(crate::ConnectionHandle::from_id(u64::from(
                host.port.unwrap_or(0),
            )))
        }
        fn executor_run(
            &self,
            _conn: &crate::ConnectionHandle,
            command: RStr<'_>,
        ) -> abi_stable::std_types::RResult<crate::ModuleResult, crate::CoreError> {
            abi_stable::std_types::ROk(crate::ModuleResult {
                changed: false,
                rc: 0,
                stdout: command.to_string().into(),
                stderr: abi_stable::std_types::RString::new(),
                success: true,
                msg: ROption::RNone,
            })
        }
        fn executor_upload(
            &self,
            _conn: &crate::ConnectionHandle,
            _local: RStr<'_>,
            _remote: RStr<'_>,
        ) -> abi_stable::std_types::RResult<(), crate::CoreError> {
            abi_stable::std_types::ROk(())
        }
        fn executor_write_file(
            &self,
            _conn: &crate::ConnectionHandle,
            _path: RStr<'_>,
            _bytes: abi_stable::std_types::RVec<u8>,
        ) -> abi_stable::std_types::RResult<(), crate::CoreError> {
            abi_stable::std_types::ROk(())
        }
        fn close_connection(&self, _conn: crate::ConnectionHandle) {}
        fn komando(
            &self,
            _task: crate::TaskInput,
            _host: HostInfo,
        ) -> abi_stable::std_types::RResult<crate::ModuleResult, crate::CoreError> {
            abi_stable::std_types::ROk(crate::ModuleResult::ok())
        }
        fn defaults_get(&self, _key: RStr<'_>) -> abi_stable::std_types::ROption<crate::RValue> {
            ROption::RNone
        }
        fn defaults_set(
            &self,
            _key: RStr<'_>,
            _value: crate::RValue,
        ) -> abi_stable::std_types::RResult<(), crate::CoreError> {
            abi_stable::std_types::ROk(())
        }
        fn report_record(&self, _task: RStr<'_>, _host: RStr<'_>, _status: crate::ReportStatus) {}
        fn host_info(&self) -> HostInfo {
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
        fn now_playing_task(&self) -> crate::TaskInput {
            crate::TaskInput {
                module_name: RStr::from(""),
                args: RHashMap::new(),
                name: ROption::RNone,
                description: ROption::RNone,
            }
        }
        fn worker_lua(&self) -> crate::LuaHandle {
            crate::LuaHandle::new()
        }
        fn log(&self, _level: crate::LogLevel, _msg: RStr<'_>) {}
        fn global_flags(&self) -> crate::GlobalFlags {
            crate::GlobalFlags::default()
        }
    }

    #[derive(Debug)]
    struct PrintSink;
    impl crate::LoggerSink for PrintSink {
        fn log(&self, _level: crate::LogLevel, _msg: RStr<'_>) {}
    }

    #[test]
    fn plugin_context_round_trips_through_dyn() {
        let core: CoreApiRef = CoreApi_TO::from_ptr(RArc::new(EchoCore), TD_Opaque);
        let logger: LoggerSinkBox = LoggerSink_TO::from_value(PrintSink, TD_Opaque);

        let ctx = PluginContext {
            core,
            host: HostInfo {
                name: ROption::RSome(RStr::from("t")),
                address: RStr::from("127.0.0.1"),
                port: ROption::RSome(2222),
                user: ROption::RNone,
                ssh_key_path: ROption::RNone,
                private_key_pass: ROption::RNone,
                password: ROption::RNone,
                become_method: ROption::RNone,
                become_user: ROption::RNone,
                elevate: ROption::RNone,
                connection_type: RStr::from("ssh"),
            },
            logger,
        };

        // Drive through the type-erased CoreApi.
        let conn = ctx
            .core
            .create_connection(ctx.host.clone())
            .into_result()
            .unwrap_or(crate::ConnectionHandle::INVALID);
        assert_eq!(conn.id, 2222);

        let ran = ctx
            .core
            .executor_run(&conn, RStr::from("echo hi"))
            .into_result();
        assert!(ran.is_ok());
        let mr = ran.unwrap_or_else(|_| crate::ModuleResult::ok());
        assert_eq!(mr.stdout.as_str(), "echo hi");

        // Logger goes through its own dyn.
        ctx.logger
            .log(crate::LogLevel::Info, RStr::from("hi from test"));
        let _ = RNone::<u8>;
    }
}
