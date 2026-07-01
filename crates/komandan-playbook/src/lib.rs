//! `komandan-playbook` — runs Ansible-format playbooks via komandan's Rust core.
//!
//! Builds as a `cdylib` producing `libkomandan_playbook.{so,dylib,dll}`, loaded
//! by the komandan host and dispatched on `komandan playbook ...`. Implements
//! [`Plugin`](komandan_plugin_abi::Plugin) from the `komandan-plugin-abi`
//! interface crate.
//!
//! Phase 1 ships parse + listing (`--syntax-check` / `--list-hosts` /
//! `--list-tasks`). Templating, execution, and orchestration land in later
//! phases (see `docs/PLAYBOOK_PLAN.md`).
//!
//! # Toolchain
//!
//! Rust nightly, edition 2024 — matches the komandan workspace.

#![deny(unsafe_code)]
#![deny(
    clippy::pedantic,
    clippy::nursery,
    clippy::enum_glob_use,
    clippy::unwrap_used,
    clippy::expect_used
)]

pub mod cli;
pub mod commands;
pub mod connection_pool;
pub mod error;
pub mod executors;
pub mod host;
pub mod inventory;
pub mod leak;
pub mod parser;
pub mod role;
pub mod runner;
pub mod templating;
pub mod vars;

/// In-crate test helpers (mock `CoreApi`, host builders).
///
/// Compiled under `#[cfg(test)]` for unit tests, and under the `testing` Cargo
/// feature for integration tests in `tests/`. The feature is enabled via the
/// crate's `[dev-dependencies]` self-import, so `test_support` is **off** in
/// normal/release builds (it never ships in the `cdylib`).
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

use clap::Parser;
use komandan_plugin_abi::Plugin_TO;
use komandan_plugin_abi::prelude::*;

/// The playbook plugin.
#[derive(Debug)]
pub struct PlaybookPlugin;

impl Plugin for PlaybookPlugin {
    fn name(&self) -> RStr<'static> {
        RStr::from("playbook")
    }

    fn version(&self) -> RStr<'static> {
        RStr::from("0.1.0")
    }

    fn register(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: RStr::from("playbook"),
            version: RStr::from("0.1.0"),
            abi_version: ABI_VERSION,
            description: RStr::from("Run Ansible-format playbooks via komandan's Rust core."),
            subcommand_name: RStr::from("playbook"),
            // v1 capability schema is undefined; empty is the documented default.
            capabilities: RStr::from(""),
        }
    }

    /// Dispatch the plugin's argv tail through the playbook CLI.
    ///
    /// The host forwards the raw tail *after* the subcommand name; we prepend a
    /// synthetic bin name so [`clap::Parser::try_parse_from`] sees a normal
    /// argv. `--help` / `--version` (clap exit code 0) come back as the plugin's
    /// success output; parse failures surface as a `cli`-kinded
    /// [`PluginError`].
    fn run(&self, ctx: &PluginContext, args: RVec<RString>) -> RResult<RString, PluginError> {
        ctx.logger
            .log(LogLevel::Info, RStr::from("playbook plugin invoked"));

        let cli_argv: Vec<String> = std::iter::once("komandan-playbook".to_string())
            .chain(args.iter().map(RString::to_string))
            .collect();
        let opts = match cli::PlaybookArgs::try_parse_from(cli_argv) {
            Ok(o) => o,
            Err(e) => {
                if e.exit_code() == 0 {
                    return ROk(RString::from(e.to_string()));
                }
                return RResult::RErr(PluginError::new("cli", e.to_string()));
            }
        };

        match commands::run(&opts, &ctx.core) {
            Ok(output) => ROk(RString::from(output)),
            Err(e) => RResult::RErr(PluginError::new("runtime", e.to_string())),
        }
    }
}

/// Entry symbol the komandan host loader resolves via `libloading`.
///
/// Mirrors `komandan-hello-plugin`: the local `#[allow(unsafe_code)]` is the
/// single audited entry-symbol site (edition-2024 `#[unsafe(no_mangle)]` plus
/// the `abi_stable` phantom layout pinned by `#[sabi_trait]`).
#[must_use]
#[allow(unsafe_code)] // #[unsafe(no_mangle)] in edition 2024; see fn docs.
#[allow(improper_ctypes_definitions)] // PluginBox abi_stable internal phantom; layout pinned by #[sabi_trait].
#[unsafe(no_mangle)]
pub extern "C" fn komandan_plugin_v1() -> PluginBox {
    Plugin_TO::from_value(PlaybookPlugin, TD_Opaque)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_owns_playbook_subcommand() {
        let d = PlaybookPlugin.register();
        assert_eq!(d.name.as_str(), "playbook");
        assert_eq!(d.subcommand_name.as_str(), "playbook");
        assert_eq!(d.abi_version, ABI_VERSION);
    }

    /// A minimal mock [`PluginContext`] so `run()` can be exercised without the
    /// host. Mirrors `komandan-hello-plugin`'s in-test mocks.
    fn make_ctx() -> PluginContext {
        use komandan_plugin_abi::{CoreApi_TO, LoggerSink_TO, RArc};
        #[derive(Debug)]
        struct NoopCore;
        impl komandan_plugin_abi::CoreApi for NoopCore {
            fn create_connection(&self, _host: HostInfo) -> RResult<ConnectionHandle, CoreError> {
                ROk(ConnectionHandle::INVALID)
            }
            fn executor_run(
                &self,
                _conn: &ConnectionHandle,
                _command: RStr<'_>,
            ) -> RResult<ModuleResult, CoreError> {
                ROk(ModuleResult::ok())
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
            fn komando(
                &self,
                _task: TaskInput,
                _host: HostInfo,
            ) -> RResult<ModuleResult, CoreError> {
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
        struct NoopSink;
        impl komandan_plugin_abi::LoggerSink for NoopSink {
            fn log(&self, _level: LogLevel, _msg: RStr<'_>) {}
        }
        PluginContext {
            core: CoreApi_TO::from_ptr(RArc::new(NoopCore), TD_Opaque),
            host: HostInfo {
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
            },
            logger: LoggerSink_TO::from_value(NoopSink, TD_Opaque),
        }
    }

    #[test]
    fn run_syntax_check_against_a_temp_playbook() -> anyhow::Result<()> {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new()?;
        writeln!(
            tmp,
            "- hosts: all\n  tasks:\n    - command: echo hi\n    - name: greet\n      debug: msg=hi"
        )?;
        tmp.flush()?;
        let path = tmp
            .path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-utf-8 temp path"))?
            .to_string();
        let args = RVec::from(vec![RString::from(path), RString::from("--syntax-check")]);
        let out = PlaybookPlugin
            .run(&make_ctx(), args)
            .into_result()
            .map(|s| s.to_string())
            .unwrap_or_default();
        assert!(out.contains("syntax OK"), "got: {out}");
        assert!(out.contains("2 task(s)"), "got: {out}");
        Ok(())
    }

    #[test]
    fn run_execution_path_runs_a_task() -> anyhow::Result<()> {
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new()?;
        writeln!(
            tmp,
            "- hosts: localhost\n  tasks:\n    - debug: msg=hi\n    - ping:"
        )?;
        tmp.flush()?;
        let path = tmp
            .path()
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("non-utf-8 temp path"))?
            .to_string();
        let args = RVec::from(vec![RString::from(path)]);
        let out = match PlaybookPlugin.run(&make_ctx(), args).into_result() {
            Ok(m) => m.to_string(),
            Err(e) => anyhow::bail!("expected execution, got error: {}", e.message),
        };
        assert!(out.contains("TASK [debug]"), "got: {out}");
        assert!(out.contains("PLAY RECAP"), "got: {out}");
        assert!(out.contains("ok="), "got: {out}");
        Ok(())
    }
}
