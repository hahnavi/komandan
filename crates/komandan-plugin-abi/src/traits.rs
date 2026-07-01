//! `#[sabi_trait]` interface traits crossing the plugin ↔ host boundary.
//!
//! Two primary traits:
//!
//! - [`Plugin`] — implemented by plugins; the host drives it.
//! - [`CoreApi`] — implemented by the host; plugins call back into it.
//!
//! Plus a tiny [`LoggerSink`] used by [`PluginContext`](crate::PluginContext).
//!
//! All three use `#[sabi_trait]` + `#[sabi(use_dyntrait)]`. `use_dyntrait`
//! switches the generated trait object's backing from `RObject` to
//! [`abi_stable::DynTrait`], which permits `Send + Sync + Debug` supertraits
//! and is the abi_stable-recommended mode for plugin systems that want
//! type-erased handles crossing `.so` boundaries.
//!
//! # `unsafe` and `non_local_definitions` allowances
//!
//! `#[sabi_trait]`'s macro expansion emits `unsafe impl StableAbi` (and
//! related `unsafe` blocks) plus `impl InterfaceType` blocks inside
//! generated `const _: () = { ... }` scopes. These are `abi_stable`'s
//! load-time layout assertions — the same trust boundary komandan's
//! `AGENTS.md` confines to one audited module (the loader). Here they live
//! inside macro expansion, not hand-written code, and the layout assertions
//! are the *whole point* of depending on `abi_stable`.
//!
//! The two narrow module-level allows below cover **only** this macro
//! expansion. The crate remains `deny(unsafe_code)` (see `lib.rs`); any
//! hand-written `unsafe` or hand-written non-local `impl` in this module
//! still fails the build.

// abi_stable macro conflict — see module docs above. Covers ONLY
// `#[sabi_trait]`-generated code in this module.
#![allow(unsafe_code)]
// abi_stable 0.11.3 generates `impl InterfaceType` inside `const _:` blocks;
// harmless under macro expansion. Tracked upstream; remove on abi_stable bump.
#![allow(non_local_definitions)]
// abi_stable's `#[sabi_trait]` macro generates `_`-prefixed bindings inside
// its vtable witnesses (e.g. `_self`, `_this`). They are intentional macro
// hygiene, not laziness; silencing the lint at the trait-declaration level
// is impossible (the names live in macro-emitted items), so allow at module
// level.
#![allow(clippy::used_underscore_binding)]
// abi_stable's `#[sabi_trait]` vtable construction casts byte-arrays of
// function-pointer-storage (1-aligned) to actual function pointers
// (8-aligned). The cast is sound because the storage is `#[repr(C)]`-padded
// to the function-pointer size; clippy cannot see through the macro to
// verify that. Macro-only; allow at module level.
#![allow(clippy::cast_ptr_alignment)]
// abi_stable's `#[sabi_trait]`-generated trait object structs have an `obj`
// field of type `_ErasedPtr` (the erased pointer type). clippy cannot prove
// from the macro expansion that the field is `Send`/`Sync` even though the
// concrete pointer types the macro is instantiated with (`RBox<()>`,
// `RArc<()>`) are. Macro-only false positive; allow at module level.
#![allow(clippy::non_send_fields_in_send_ty)]

use abi_stable::sabi_trait;
use abi_stable::std_types::{RArc, RBox, RResult, RStr, RString, RVec};

use crate::context::PluginContext;
use crate::error::{CoreError, PluginError};
use crate::handles::{ConnectionHandle, LuaHandle};
use crate::types::{
    GlobalFlags, HostInfo, LogLevel, ModuleResult, PluginDescriptor, RValue, ReportStatus,
    TaskInput,
};

/// Plugin-facing entry point implemented by every komandan plugin.
///
/// A plugin is a `cdylib` that exports [`crate::ENTRY_SYMBOL`] (`komandan_plugin_v1`)
/// returning a [`crate::PluginBox`]. The host loader resolves that symbol,
/// calls it once per load, obtains a [`Plugin_TO`] trait object, and drives
/// the plugin through this trait.
///
/// # Method set
///
/// Beyond the three methods the task names (`name`, `version`, `register`),
/// this trait also defines [`Plugin::run`]: the host invokes it when a CLI
/// invocation routes to this plugin's subcommand. Without `run` the plugin
/// could declare itself but never execute — the spec's §4.1 `Plugin::run`
/// signature is preserved verbatim.
///
/// # Extensibility
///
/// `#[sabi(last_prefix_field)]` marks `name` as the last method of the v1
/// vtable. Adding methods is ABI-compatible so long as they are appended at
/// the end (the `abi_stable` contract). Renaming, reordering, or changing
/// signatures of any method above a later `last_prefix_field` is an ABI
/// break that bumps [`crate::ABI_VERSION`].
///
/// # Errors
///
/// [`Plugin::run`] returns [`PluginError`] on failure; the host surfaces it
/// to the user and exits non-zero.
#[allow(clippy::module_name_repetitions)] // `Plugin_TO` is the `abi_stable` convention; mirrors the trait name.
#[sabi_trait]
#[sabi(use_dyntrait)]
pub trait Plugin: Send + Sync + std::fmt::Debug {
    /// Stable plugin name, e.g. `"playbook"`. Must be unique across loaded
    /// plugins; first one wins on collision (spec §5.1).
    #[sabi(last_prefix_field)]
    fn name(&self) -> RStr<'static>;

    /// Semantic version of the plugin itself (not the ABI version).
    fn version(&self) -> RStr<'static>;

    /// One-shot metadata probe. The host calls this immediately after
    /// [`crate::ENTRY_SYMBOL`] resolution to learn the plugin's
    /// subcommand name, description, and claimed ABI version.
    fn register(&self) -> PluginDescriptor;

    /// Execute the plugin's subcommand.
    ///
    /// # Arguments
    ///
    /// - `ctx`: host-provided [`PluginContext`] carrying the [`CoreApi`]
    ///   handle, current [`HostInfo`], and a [`LoggerSink`]. Plugins do all
    ///   host-side work through `ctx`.
    /// - `args`: raw argv tail *after* the subcommand name (the plugin owns
    ///   its own arg parsing, typically via `clap::Parser::parse_from`).
    ///
    /// # Errors
    ///
    /// Returns [`PluginError`] on any failure. The host prints `kind` /
    /// `message` verbatim and exits non-zero.
    fn run(&self, ctx: &PluginContext, args: RVec<RString>) -> RResult<RString, PluginError>;
}

/// Host surface exposed *to* plugins.
///
/// The host implements this; plugins obtain a shared handle via
/// [`PluginContext::core`](crate::PluginContext::core) and call back into the host through these methods.
///
/// # Method set — provenance
///
/// The list mirrors the capability table in
/// `docs/PLUGIN_SYSTEM_SPEC.md` §3 ("`komandan-core` public surface (stable)")
/// and §4.2, kept ABI-stable here:
///
/// - `create_connection` / `close_connection` — host's `connection::create_connection`.
/// - `executor_run` / `executor_upload` / `executor_write_file` — host's `CommandExecutor`.
/// - `komando` — host's Rust-native `komando_rust` (spec §3 refactor).
/// - `defaults_get` / `defaults_set` — host's `Defaults::global`.
/// - `report_record` — host's `report::insert_record`.
/// - `host_info` / `now_playing_task` — task-scoped accessors (added per task §5).
/// - `worker_lua` — placeholder for the worker-thread Lua accessor (spec §10.4).
/// - `log` — tracing sink (task §5 "logging sink").
/// - `global_flags` — host's parsed top-level flags (spec §5.3).
///
/// # What's deliberately absent
///
/// `parallel_map(items, work_fn)` from spec §4.2 is **not** in v1: marshalling
/// an arbitrary plugin `Fn` closure back through the host's rayon executor
/// requires a separate `DynTrait`-wrapped `RWorkFn` whose precise shape the
/// spec leaves open (§10). It is the first thing v1.1 adds; see the
/// `RWorkFn` placeholder in the v1.1 design doc (TBD).
///
/// # Connection-handle safety
///
/// Executor methods take `&ConnectionHandle` — a plain `u64` ID, not a
/// type-erased pointer. The host resolves the ID to its real connection
/// internally. See [`ConnectionHandle`] for the rationale.
#[allow(clippy::module_name_repetitions)] // `CoreApi_TO` mirrors the trait name; `abi_stable` convention.
#[sabi_trait]
#[sabi(use_dyntrait)]
pub trait CoreApi: Send + Sync + std::fmt::Debug {
    /// Open a connection to `host`. Returns an opaque handle the plugin
    /// passes back to executor methods.
    ///
    /// # Errors
    ///
    /// [`CoreError`] with `kind = "connection"` on auth failure, network
    /// error, or unknown host.
    #[sabi(last_prefix_field)]
    fn create_connection(&self, host: HostInfo) -> RResult<ConnectionHandle, CoreError>;

    /// Run a shell command on `conn`. Blocking.
    ///
    /// # Errors
    ///
    /// [`CoreError`] with `kind = "executor"` on I/O failure. A non-zero
    /// process exit is *not* an error here — it is surfaced via
    /// <code>[ModuleResult::success] == false</code> and [`ModuleResult::rc`].
    fn executor_run(
        &self,
        conn: &ConnectionHandle,
        command: RStr<'_>,
    ) -> RResult<ModuleResult, CoreError>;

    /// Upload a local file to a remote path on `conn`.
    ///
    /// # Errors
    ///
    /// [`CoreError`] with `kind = "executor"` or `kind = "io"`.
    fn executor_upload(
        &self,
        conn: &ConnectionHandle,
        local: RStr<'_>,
        remote: RStr<'_>,
    ) -> RResult<(), CoreError>;

    /// Atomically write `bytes` to `path` on `conn` (used by `copy` /
    /// `unarchive` executors).
    ///
    /// # Errors
    ///
    /// [`CoreError`] with `kind = "io"` on write failure.
    fn executor_write_file(
        &self,
        conn: &ConnectionHandle,
        path: RStr<'_>,
        bytes: abi_stable::std_types::RVec<u8>,
    ) -> RResult<(), CoreError>;

    /// Drop a connection. Idempotent; calling with [`ConnectionHandle::INVALID`]
    /// or an unknown ID is a no-op (logged at `Warn`).
    fn close_connection(&self, conn: ConnectionHandle);

    /// Rust-native entry into the host's `komando()` — run `task` against
    /// `host` without going through Lua.
    ///
    /// # Errors
    ///
    /// [`CoreError`] with `kind = "module"` if `task.module_name` is unknown,
    /// or `kind = "executor"` if the underlying connection fails.
    fn komando(&self, task: TaskInput, host: HostInfo) -> RResult<ModuleResult, CoreError>;

    /// Read a host default by key (mirror of `Defaults::global`).
    ///
    /// Returns [`ROption::RNone`](abi_stable::std_types::ROption::RNone) if the key is unset; never errors.
    fn defaults_get(&self, key: RStr<'_>) -> abi_stable::std_types::ROption<RValue>;

    /// Write a host default. The host may reject keys it considers reserved.
    ///
    /// # Errors
    ///
    /// [`CoreError`] with `kind = "defaults"` if the key is reserved.
    fn defaults_set(&self, key: RStr<'_>, value: RValue) -> RResult<(), CoreError>;

    /// Record a `(task, host, status)` row in the host's unified report.
    /// Fire-and-forget; never errors.
    fn report_record(&self, task: RStr<'_>, host: RStr<'_>, status: ReportStatus);

    /// The [`HostInfo`] for the host this plugin is currently dispatching
    /// against. Cheap (returns a clone of a cached struct).
    fn host_info(&self) -> HostInfo;

    /// The [`TaskInput`] currently being executed, if any. The returned
    /// struct may carry an empty `module_name` when no task is in flight.
    fn now_playing_task(&self) -> TaskInput;

    /// Probe whether the host exposes the worker-Lua accessor. See
    /// [`LuaHandle`] for why this is a v0.1 placeholder and **must not**
    /// be dereferenced.
    fn worker_lua(&self) -> LuaHandle;

    /// Emit a log line into the host's `tracing` subscriber.
    fn log(&self, level: LogLevel, msg: RStr<'_>);

    /// Snapshot of the host's top-level CLI flags. See [`GlobalFlags`].
    fn global_flags(&self) -> GlobalFlags;
}

/// Minimal log emitter bundled into [`PluginContext`] so plugins can write
/// to the host's `tracing` subscriber without holding a full [`CoreApi`]
/// handle (e.g. during early init before `register()` returns).
///
/// `#[sabi(last_prefix_field)]` on `log` allows adding more sink methods
/// (e.g. `metric`, `span`) in v1.1 without breaking ABI.
#[allow(clippy::module_name_repetitions)] // `LoggerSink_TO` mirrors the trait name; `abi_stable` convention.
#[sabi_trait]
#[sabi(use_dyntrait)]
pub trait LoggerSink: Send + Sync + std::fmt::Debug {
    /// Emit `msg` at the given [`LogLevel`].
    #[sabi(last_prefix_field)]
    fn log(&self, level: LogLevel, msg: RStr<'_>);
}

/// Type-erased, shared-ownership handle to the host's [`CoreApi`] impl.
///
/// `RArc` lets both host and plugin hold a reference-counted view of the
/// same core without either side owning it exclusively. The plugin receives
/// this through [`PluginContext::core`](crate::PluginContext::core).
///
/// # Construction
///
/// The host constructs it via:
///
/// ```rust,ignore
/// let core: CoreApiRef = CoreApi_TO::from_ptr(RArc::new(my_core), TD_Opaque);
/// ```
///
/// Plugins never construct this; they receive it from the host.
pub type CoreApiRef = CoreApi_TO<'static, RArc<()>>;

/// Type-erased, owned handle to a [`LoggerSink`] impl.
///
/// Bundled into [`PluginContext`](crate::PluginContext). The host constructs
/// it; plugins call through it.
pub type LoggerSinkBox = LoggerSink_TO<'static, RBox<()>>;
