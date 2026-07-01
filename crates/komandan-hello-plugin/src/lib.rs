//! `komandan-hello-plugin` — reference plugin for komandan's plugin ABI.
//!
//! Builds as a `cdylib` producing `libkomandan_hello_plugin.so` (Linux), which
//! the komandan host `dlopen`s and dispatches to whenever the user runs
//! `komandan hello ...`. Implements the [`Plugin`](komandan_plugin_abi::Plugin)
//! trait from the `komandan-plugin-abi` interface crate; see that crate's docs
//! for the full plugin ↔ host contract and the entry-symbol spec.
//!
//! The plugin is deliberately trivial: it greets and echoes its argv tail. Its
//! job is to prove the end-to-end plugin loop (load → `register()` →
//! `run()` → logger sink → return) works, serving as the canonical example
//! third-party plugin authors copy.
//!
//! # Toolchain
//!
//! Rust **nightly**, edition 2024 — matches the komandan workspace and the
//! `komandan-plugin-abi` interface crate. Pin via `rust-toolchain.toml`.

#![deny(unsafe_code)]
#![deny(
    clippy::pedantic,
    clippy::nursery,
    clippy::enum_glob_use,
    clippy::unwrap_used,
    clippy::expect_used
)]

use komandan_plugin_abi::prelude::*;
// `Plugin_TO` is exported at the ABI crate root (NOT in the prelude — the
// prelude lists the `Plugin` trait but not its `#[sabi_trait]`-generated trait
// object type). Import it explicitly.
use komandan_plugin_abi::Plugin_TO;

/// Reference plugin implementing the `hello` subcommand.
///
/// Stateless unit struct: all behaviour lives in the
/// [`Plugin`](komandan_plugin_abi::Plugin) impl. [`core::fmt::Debug`] is
/// derived so the `#[sabi_trait]` `Plugin: Debug` supertrait is satisfied
/// without per-instance state.
#[derive(Debug)]
pub struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn name(&self) -> RStr<'static> {
        RStr::from("hello")
    }

    fn version(&self) -> RStr<'static> {
        RStr::from("0.1.0")
    }

    fn register(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: RStr::from("hello"),
            version: RStr::from("0.1.0"),
            abi_version: ABI_VERSION,
            description: RStr::from("Reference komandan plugin: prints a greeting."),
            subcommand_name: RStr::from("hello"),
            // v1 capability schema is undefined (PLUGIN_SYSTEM_SPEC.md §10.5);
            // empty is the documented default for plugins with no special tags.
            capabilities: RStr::from(""),
        }
    }

    /// Build a friendly greeting that echoes the argv tail.
    ///
    /// Calls the bundled [`LoggerSink`](komandan_plugin_abi::LoggerSink) once
    /// at [`LogLevel::Info`] to prove the log path works end-to-end, then
    /// joins `args` with single spaces. Empty argv ⇒ the literal `(no args)`.
    ///
    /// # Errors
    ///
    /// Returns a [`PluginError`] with `kind = "runtime"` only if greeting
    /// construction fails — which for this trivial plugin is effectively
    /// never; the path exists to honour the trait's fallible signature.
    fn run(&self, ctx: &PluginContext, args: RVec<RString>) -> RResult<RString, PluginError> {
        ctx.logger
            .log(LogLevel::Info, RStr::from("hello plugin invoked"));

        let parts: Vec<&str> = args.iter().map(RString::as_str).collect();
        let joined = parts.join(" ");
        let args_part = if joined.is_empty() {
            "(no args)"
        } else {
            joined.as_str()
        };
        ROk(RString::from(format!(
            "Hello from komandan-hello-plugin v0.1.0! (args: {args_part})"
        )))
    }
}

/// Entry symbol the komandan host loader resolves via `libloading`.
///
/// Name (`komandan_plugin_v1`) and signature (`extern "C" fn() -> PluginBox`)
/// are pinned by [`ENTRY_SYMBOL`] and
/// [`PluginEntryFn`](komandan_plugin_abi::PluginEntryFn) in the ABI crate. The
/// host calls this exactly once per plugin load and takes ownership of the
/// returned [`PluginBox`].
///
/// # Panics
///
/// Never panics in practice: [`Plugin_TO::from_value`] is infallible for any
/// `Debug + Send + Sync` value, which [`HelloPlugin`] is.
///
/// # `unsafe` attributes
///
/// `#[unsafe(no_mangle)]` is the edition-2024 form of `#[no_mangle]` (the
/// attribute became unsafe in 2024). The crate-level `deny(unsafe_code)` lint
/// flags even the wrapped form, so the attribute is locally allowed here —
/// this is the single, audited entry-symbol site, mirroring how the ABI crate
/// allows `improper_ctypes_definitions` on `PluginEntryFn`. The return type's
/// real layout is pinned by the `#[sabi_trait]`-generated vtable.
#[must_use]
#[allow(unsafe_code)] // #[unsafe(no_mangle)] in edition 2024; see fn docs.
#[allow(improper_ctypes_definitions)] // PluginBox abi_stable internal phantom; layout pinned by #[sabi_trait].
#[unsafe(no_mangle)]
pub extern "C" fn komandan_plugin_v1() -> PluginBox {
    Plugin_TO::from_value(HelloPlugin, TD_Opaque)
}

#[cfg(test)]
mod tests {
    use super::HelloPlugin;
    use komandan_plugin_abi::prelude::*;
    // Trait-object constructors and the shared/owned pointer types live at the
    // ABI crate root, not in the prelude.
    use komandan_plugin_abi::{CoreApi_TO, LoggerSink_TO, RArc};

    // ---- In-test mock impls mirroring komandan-plugin-abi's context.rs ----
    // ---- test (`EchoCore` / `PrintSink`). All 14 CoreApi methods return --
    // ---- trivial values; only what `run()` touches (`log`) matters.     --

    #[derive(Debug)]
    struct EchoCore;

    impl komandan_plugin_abi::CoreApi for EchoCore {
        fn create_connection(&self, host: HostInfo) -> RResult<ConnectionHandle, CoreError> {
            ROk(ConnectionHandle::from_id(u64::from(host.port.unwrap_or(0))))
        }
        fn executor_run(
            &self,
            _conn: &ConnectionHandle,
            command: RStr<'_>,
        ) -> RResult<ModuleResult, CoreError> {
            ROk(ModuleResult {
                changed: false,
                rc: 0,
                stdout: command.to_string().into(),
                stderr: RString::new(),
                success: true,
                msg: ROption::RNone,
            })
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
        fn komando(&self, _task: TaskInput, _host: HostInfo) -> RResult<ModuleResult, CoreError> {
            ROk(ModuleResult::ok())
        }
        fn defaults_get(&self, _key: RStr<'_>) -> ROption<RValue> {
            ROption::RNone
        }
        fn defaults_set(&self, _key: RStr<'_>, _value: RValue) -> RResult<(), CoreError> {
            ROk(())
        }
        fn report_record(&self, _task: RStr<'_>, _host: RStr<'_>, _status: ReportStatus) {}
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

    #[derive(Debug)]
    struct PrintSink;
    impl komandan_plugin_abi::LoggerSink for PrintSink {
        fn log(&self, _level: LogLevel, _msg: RStr<'_>) {}
    }

    /// Build a minimal [`PluginContext`] wired to the in-test mocks, mirroring
    /// `komandan-plugin-abi`'s `plugin_context_round_trips_through_dyn`.
    fn make_ctx() -> PluginContext {
        let core: CoreApiRef = CoreApi_TO::from_ptr(RArc::new(EchoCore), TD_Opaque);
        let logger: LoggerSinkBox = LoggerSink_TO::from_value(PrintSink, TD_Opaque);
        PluginContext {
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
        }
    }

    #[test]
    fn name_is_hello() {
        let p = HelloPlugin;
        assert_eq!(p.name().as_str(), "hello");
    }

    #[test]
    fn version_is_0_1_0() {
        let p = HelloPlugin;
        assert_eq!(p.version().as_str(), "0.1.0");
    }

    #[test]
    fn register_descriptor_matches_abi_version_and_subcommand() {
        let p = HelloPlugin;
        let d = p.register();
        assert_eq!(d.name.as_str(), "hello");
        assert_eq!(d.version.as_str(), "0.1.0");
        assert_eq!(d.abi_version, ABI_VERSION);
        assert_eq!(d.subcommand_name.as_str(), "hello");
        assert!(!d.description.as_str().is_empty());
    }

    #[test]
    fn run_with_args_echoes_them() {
        let p = HelloPlugin;
        let ctx = make_ctx();
        let args = RVec::from(vec![RString::from("foo"), RString::from("bar")]);
        let out = p
            .run(&ctx, args)
            .into_result()
            .map(|s| s.to_string())
            .unwrap_or_default();
        assert!(out.contains("Hello"), "greeting missing: {out}");
        assert!(out.contains("foo"), "first arg missing: {out}");
        assert!(out.contains("bar"), "second arg missing: {out}");
        assert!(out.contains("(args: foo bar)"), "joined args wrong: {out}");
    }

    #[test]
    fn run_with_no_args_says_so() {
        let p = HelloPlugin;
        let ctx = make_ctx();
        let out = p
            .run(&ctx, RVec::new())
            .into_result()
            .map(|s| s.to_string())
            .unwrap_or_default();
        assert!(
            out.contains("(no args)"),
            "empty-args marker missing: {out}"
        );
        assert!(out.contains("Hello"), "greeting missing: {out}");
    }
}
