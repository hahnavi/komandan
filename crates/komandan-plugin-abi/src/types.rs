//! FFI-safe mirror types of komandan-core's runtime structs.
//!
//! These types are **redefined** here (not depended upon from `komandan-core`)
//! so that the ABI crate is fully standalone. The host crate provides
//! `From<komandan_core::X> for X-mirror` impls at the boundary; those impls
//! are the single controlled place where layout drift would surface.
//!
//! All types use `#[repr(C)]` + [`abi_stable::StableAbi`] so their layout is
//! pinned and load-time checked by `abi_stable`'s layout-checker when a plugin
//! is `dlopen`'d.

use abi_stable::StableAbi;
use abi_stable::std_types::{RHashMap, ROption, RStr, RString};

/// Mirror of the subset of `komandan::models::Host` a plugin needs at runtime.
///
/// Field names match the Lua-facing keys (see `src/models.rs::Host::into_lua`)
/// so the host's boundary conversion is mechanical.
///
/// # Secrets — deliberate exposure boundary
///
/// `password` and `private_key_pass` are represented here as plain `RStr`
/// (not wrapped in any secrecy type). This is intentional and matches the
/// existing Lua trust boundary documented in komandan's `AGENTS.md`:
/// *"Host exposes secrets when building the Lua table for module consumption —
/// that is the controlled boundary, not a leak"*. A loaded plugin is trusted
/// exactly like a built-in module: it runs as native code in the host process
/// with full access to anything it is handed. Do not add a new secrecy
/// wrapper at this layer; do review any new `HostInfo`-adjacent API that
/// widens what crosses the boundary (see `docs/PLUGIN_SYSTEM_SPEC.md` §7).
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct HostInfo {
    /// Host label (the `name` key on the Lua host table).
    pub name: ROption<RStr<'static>>,
    /// Network address / hostname / IP. May be empty for `local` connections.
    pub address: RStr<'static>,
    /// TCP port for SSH connections. Unused for `local`.
    pub port: ROption<u16>,
    /// Remote login user (`nil` ⇒ current user).
    pub user: ROption<RStr<'static>>,
    /// Path to the SSH private key file (`nil` ⇒ agent/default).
    pub ssh_key_path: ROption<RStr<'static>>,
    /// SSH key passphrase — **secret, deliberately exposed** (see type docs).
    pub private_key_pass: ROption<RStr<'static>>,
    /// Login password — **secret, deliberately exposed** (see type docs).
    pub password: ROption<RStr<'static>>,
    /// Privilege-elevation method (`"sudo"` / `"su"` / `"doas"` / ...).
    pub become_method: ROption<RStr<'static>>,
    /// User to become via `become_method`. Mirrors `models::Host::as_user`.
    pub become_user: ROption<RStr<'static>>,
    /// Whether elevation is requested for this host's commands. Mirrors
    /// `models::Host::elevate`. Named `elevate` (not `become`, which is a
    /// reserved keyword in edition 2024) to match the source-of-truth field.
    pub elevate: ROption<bool>,
    /// `"local"` or `"ssh"`. Matches `models::ConnectionType::as_str`.
    pub connection_type: RStr<'static>,
}

/// Mirror of the subset of `komandan::models::Task` a plugin needs.
///
/// Only the descriptive fields cross the boundary; the Lua-side `Module`
/// internals (functions dump, JSON-serialized args) collapse into `args`
/// (a flat `RHashMap` of named values). Plugins that need the raw module
/// name (e.g. for dispatch) read it from [`TaskInput::module_name`].
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct TaskInput {
    /// Name of the built-in module this task invokes (e.g. `"apt"`, `"cmd"`).
    pub module_name: RStr<'static>,
    /// Flat module arguments. Keys mirror the per-module Lua API. Values are
    /// tagged via [`RValue`].
    pub args: RHashMap<RStr<'static>, RValue>,
    /// Optional task label (the Lua `name` key).
    pub name: ROption<RStr<'static>>,
    /// Optional human description.
    pub description: ROption<RStr<'static>>,
}

/// Mirror of `komandan::models::KomandoResult`.
///
/// The `exit_code` field is renamed to `rc` for plugin-facing clarity (the
/// name komandan uses in its user-facing output). `changed` is hoisted to a
/// top-level field (komandan's struct buries it as a flag; here it is first
/// class because plugin authors query it directly).
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct ModuleResult {
    /// Whether the module reported it changed remote state.
    pub changed: bool,
    /// Process exit code (`0` ⇒ success).
    pub rc: i32,
    /// Captured standard output. Owned (the host heap-allocates it from the
    /// child process's pipe); lossy UTF-8.
    pub stdout: RString,
    /// Captured standard error. Owned, lossy UTF-8.
    pub stderr: RString,
    /// Whether the module considers this run a success (`rc == 0` unless
    /// overridden by `ignore_exit_code`).
    pub success: bool,
    /// Optional summary message.
    pub msg: ROption<RStr<'static>>,
}

impl ModuleResult {
    /// Construct a successful result with no output.
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // RString::new() is not yet const.
    pub fn ok() -> Self {
        Self {
            changed: false,
            rc: 0,
            stdout: RString::new(),
            stderr: RString::new(),
            success: true,
            msg: ROption::RNone,
        }
    }

    /// Construct a failed result with the given exit code and stderr.
    #[must_use]
    pub fn failure(rc: i32, stderr: impl Into<RString>) -> Self {
        Self {
            changed: false,
            rc,
            stdout: RString::new(),
            stderr: stderr.into(),
            success: false,
            msg: ROption::RNone,
        }
    }
}

/// Tagged value used for module argument passing across the FFI boundary.
///
/// # Stability choice
///
/// This enum uses `#[repr(u8)]` + `#[non_exhaustive]` + [`StableAbi`] rather
/// than `abi_stable`'s `NonExhaustive<>` wrapper (`#[sabi(kind(WithNonExhaustive))]`).
/// The trade-off:
///
/// | Approach | Pro | Con |
/// |---|---|---|
/// | `repr(u8) + non_exhaustive` (chosen) | Direct pattern matching; no `.as_enum()` wrapper; trivial `RHashMap<RStr, RValue>` | Adding a variant is an ABI break (bumps `komandan_plugin_v1` → `_v2`) |
/// | `WithNonExhaustive` | Variants can be added in a compatible minor version | Every `RValue` site becomes `RValue_NE`; matching requires `.as_enum().unwrap()` boilerplate; recursive variants need boxed storage |
///
/// For v1 the variant set is closed (the seven listed below cover every
/// argument shape the built-in modules accept). Adding a variant is rare and
/// warrants the ABI bump the spec (`docs/PLUGIN_SYSTEM_SPEC.md` §4.3)
/// already mandates for breaking changes. The ergonomics win is large.
///
/// `#[non_exhaustive`] prevents plugins from exhaustive matching today, so
/// adding a variant later only forces plugins to add a wildcard arm, not
/// recompile against a matching layout change.
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, StableAbi)]
pub enum RValue {
    /// `nil` / absent.
    Null = 0,
    /// Boolean.
    Bool(bool) = 1,
    /// 64-bit signed integer (Lua `number` without fractional part).
    Int(i64) = 2,
    /// 64-bit IEEE-754 float (Lua `number` with fractional part).
    Float(f64) = 3,
    /// UTF-8 string. Borrows `'static` data when constructed from literals.
    Str(RStr<'static>) = 4,
    /// Raw bytes.
    Bytes(abi_stable::std_types::RVec<u8>) = 5,
    /// Ordered-ish list of values.
    #[allow(clippy::use_self)]
    // enum variant data type position; Self not yet stable here in all toolchains.
    List(abi_stable::std_types::RVec<RValue>) = 6,
    /// Key/value map. Keys are `'static` (typical for arg maps).
    #[allow(clippy::use_self)] // see above.
    Map(RHashMap<RStr<'static>, RValue>) = 7,
}

impl RValue {
    /// Construct a string variant from any `&'static str` literal.
    #[must_use]
    pub fn str_literal(s: &'static str) -> Self {
        Self::Str(RStr::from(s))
    }
}

/// Logging level for the host's tracing sink (mirrors `tracing::Level`).
///
/// `#[non_exhaustive]` so adding levels (e.g. `Trace` laterality) does not
/// break ABI; the discriminant layout is pinned by `#[repr(u8)]`.
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
pub enum LogLevel {
    /// Most verbose.
    Trace = 0,
    /// Debug-only diagnostic.
    Debug = 1,
    /// Default informational level.
    Info = 2,
    /// Non-fatal but notable.
    Warn = 3,
    /// Recoverable error.
    Error = 4,
}

/// Status for the host's unified report (mirrors `report::RecordStatus`).
#[repr(u8)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, StableAbi)]
pub enum ReportStatus {
    /// Task completed without changing remote state.
    Ok = 0,
    /// Task completed and changed remote state.
    Changed = 1,
    /// Task failed.
    Failed = 2,
    /// Task was skipped (e.g. by a `when` condition).
    Skipped = 3,
}

/// Mirror of the global CLI flags the host parses up-front and exposes to
/// plugins (see `PLUGIN_SYSTEM_SPEC.md` §5.3).
///
/// `--version` is deliberately omitted: it short-circuits the host before any
/// plugin loads, so no plugin ever observes it.
#[repr(C)]
#[derive(Debug, Clone, Copy, Default, StableAbi)]
pub struct GlobalFlags {
    /// `-v` / `--verbose` ⇒ host turns up tracing verbosity.
    pub verbose: bool,
    /// `--dry-run` ⇒ modules print what they would do but make no changes.
    pub dry_run: bool,
    /// `--no-report` ⇒ host skips writing the unified execution report.
    pub no_report: bool,
    /// `--unsafe-lua` ⇒ host permits unrestricted Lua FFI in task scripts.
    pub unsafe_lua: bool,
}

/// Capability / identity descriptor returned by [`crate::Plugin::register`].
///
/// The spec (`docs/PLUGIN_SYSTEM_SPEC.md` §4.1) lists identity fields as
/// individual `Plugin` trait methods. This crate collapses them into a single
/// descriptor returned by `register()` so the host can read everything in
/// one cheap call before deciding whether to dispatch. `capabilities` is a
/// comma/space-tagged placeholder string for v1; the spec is silent on a
/// concrete capability schema (open question §10.5). Plugins may leave it
/// empty.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct PluginDescriptor {
    /// Stable plugin name (unique across loaded plugins; first wins on
    /// collision).
    pub name: RStr<'static>,
    /// Semantic version of the plugin itself, as a literal (`"0.1.0"`).
    pub version: RStr<'static>,
    /// ABI version the plugin was compiled against. The host refuses to load
    /// a plugin whose `abi_version` differs from
    /// [`crate::ABI_VERSION`].
    pub abi_version: u32,
    /// One-line description for `komandan --help`.
    pub description: RStr<'static>,
    /// Subcommand name this plugin owns (`"playbook"`, ...). Empty string ⇒
    /// no CLI surface; the plugin is loaded for its side effects only.
    pub subcommand_name: RStr<'static>,
    /// Free-form capability tags (v1 placeholder; spec is silent). Examples
    /// the host *might* interpret in future versions: `"provides-lua-module"`,
    /// `"provides-executor"`. Separators undefined; do not parse
    /// programmatically yet.
    pub capabilities: RStr<'static>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use abi_stable::std_types::{RHashMap, RNone, ROption, RSome, RStr, RVec};

    fn sample_host() -> HostInfo {
        HostInfo {
            name: ROption::RSome(RStr::from("web-01")),
            address: RStr::from("10.0.0.5"),
            port: ROption::RSome(2222),
            user: ROption::RSome(RStr::from("deploy")),
            ssh_key_path: ROption::RSome(RStr::from("/home/deploy/.ssh/id_ed25519")),
            private_key_pass: ROption::RSome(RStr::from("hunter2")),
            password: ROption::RSome(RStr::from("correct horse battery staple")),
            become_method: ROption::RSome(RStr::from("sudo")),
            become_user: ROption::RSome(RStr::from("root")),
            elevate: ROption::RSome(true),
            connection_type: RStr::from("ssh"),
        }
    }

    #[test]
    fn host_info_round_trips() {
        let h = sample_host();
        assert_eq!(h.name.unwrap().as_str(), "web-01");
        assert_eq!(h.address.as_str(), "10.0.0.5");
        assert_eq!(h.port.unwrap(), 2222);
        assert_eq!(h.user.unwrap().as_str(), "deploy");
        assert_eq!(
            h.ssh_key_path.unwrap().as_str(),
            "/home/deploy/.ssh/id_ed25519"
        );
        assert_eq!(h.become_method.unwrap().as_str(), "sudo");
        assert_eq!(h.become_user.unwrap().as_str(), "root");
        assert_eq!(h.elevate, RSome(true));
        assert_eq!(h.connection_type.as_str(), "ssh");
    }

    #[test]
    fn host_info_secret_fields_round_trip_identical_bytes() {
        // The password and private_key_pass fields must round-trip identical
        // bytes — a single changed bit would silently corrupt SSH auth.
        let h = sample_host();
        let pass: &[u8] = h.password.unwrap().as_str().as_bytes();
        let key_pass: &[u8] = h.private_key_pass.unwrap().as_str().as_bytes();
        assert_eq!(pass, b"correct horse battery staple");
        assert_eq!(key_pass, b"hunter2");
        // Negative: ensure nothing mangled the bytes.
        assert_ne!(pass, b"correct horse battery stapl");
        assert_ne!(key_pass, b"hunter3");
    }

    #[test]
    fn task_input_round_trips() {
        let mut args = RHashMap::new();
        args.insert(RStr::from("command"), RValue::str_literal("echo hi"));
        args.insert(RStr::from("timeout"), RValue::Int(30));
        let t = TaskInput {
            module_name: RStr::from("cmd"),
            args,
            name: ROption::RSome(RStr::from("greet")),
            description: RNone,
        };
        assert_eq!(t.module_name.as_str(), "cmd");
        assert_eq!(t.name.unwrap().as_str(), "greet");
        assert!(matches!(t.description, ROption::RNone));
        match t.args.get(&RStr::from("command")) {
            Some(RValue::Str(s)) => assert_eq!(s.as_str(), "echo hi"),
            other => panic!("expected Str, got {other:?}"),
        }
        match t.args.get(&RStr::from("timeout")) {
            Some(RValue::Int(n)) => assert_eq!(*n, 30),
            other => panic!("expected Int, got {other:?}"),
        }
    }

    #[test]
    fn module_result_ok_and_failure() {
        let ok = ModuleResult::ok();
        assert!(ok.success);
        assert_eq!(ok.rc, 0);
        assert!(!ok.changed);

        let fail = ModuleResult::failure(127, "command not found");
        assert!(!fail.success);
        assert_eq!(fail.rc, 127);
        assert_eq!(fail.stderr.as_str(), "command not found");
    }

    #[test]
    fn rvalue_map_with_nested_list_round_trips() {
        // Build { "items": [Null, Bool(true), Int(7), Str("x") ], "flag": Float(1.5) }
        let inner = RValue::List(RVec::from(vec![
            RValue::Null,
            RValue::Bool(true),
            RValue::Int(7),
            RValue::str_literal("x"),
        ]));
        let mut outer = RHashMap::new();
        outer.insert(RStr::from("items"), inner);
        outer.insert(RStr::from("flag"), RValue::Float(1.5));

        let map = RValue::Map(outer);

        // Walk and verify.
        match &map {
            RValue::Map(m) => {
                assert_eq!(m.len(), 2);
                let items = m
                    .get(&RStr::from("items"))
                    .and_then(|v| match v {
                        RValue::List(l) => Some(l),
                        _ => None,
                    })
                    .map(RVec::as_slice);
                let items = items.unwrap_or(&[]);
                assert_eq!(items.len(), 4);
                assert!(matches!(items[0], RValue::Null));
                assert!(matches!(items[1], RValue::Bool(true)));
                assert!(matches!(items[2], RValue::Int(7)));
                match &items[3] {
                    RValue::Str(s) => assert_eq!(s.as_str(), "x"),
                    other => panic!("expected Str, got {other:?}"),
                }
                match m.get(&RStr::from("flag")) {
                    Some(RValue::Float(f)) => assert!((*f - 1.5_f64).abs() < f64::EPSILON),
                    other => panic!("expected Float, got {other:?}"),
                }
            }
            other => panic!("expected Map, got {other:?}"),
        }
    }

    #[test]
    fn rvalue_bytes_variant() {
        let v = RValue::Bytes(RVec::from(vec![0_u8, 1, 2, 3]));
        match &v {
            RValue::Bytes(b) => assert_eq!(b.as_slice(), &[0, 1, 2, 3]),
            other => panic!("expected Bytes, got {other:?}"),
        }
    }

    #[test]
    fn global_flags_default_all_false() {
        let f = GlobalFlags::default();
        assert!(!f.verbose);
        assert!(!f.dry_run);
        assert!(!f.no_report);
        assert!(!f.unsafe_lua);
    }
}
