# `komandan-plugin-abi`

**ABI-stable contracts between the komandan host binary and dynamically-loaded komandan plugins** (`.so` / `.dylib` / `.dll`).

This is the **interface crate** for komandan's in-process dynamic plugin system
(spec: `docs/PLUGIN_SYSTEM_SPEC.md` in the komandan repository). It declares
the types, the `Plugin` and `CoreApi` traits, and the entry-symbol contract.
It contains **no plugin-loading logic** and **no `unsafe` code** — the loader
lives in the host binary crate, confined to one audited module.

* Toolchain: **Rust nightly**, edition 2024.
* Dependency: **`abi_stable = "=0.11.3"`** (latest stable on crates.io at time
  of writing; minor pinned exactly because `abi_stable`'s layout-checker
  macros bake constants into generated code).
* Lint posture matches komandan: `cargo clippy --all-targets -- -D warnings`
  denies `unsafe_code`, `clippy::pedantic`, `clippy::nursery`,
  `clippy::unwrap_used`, `clippy::expect_used`, `clippy::enum_glob_use`.
* Entry symbol: **`komandan_plugin_v1`** (C-ABI, `#[no_mangle]`). Returns
  `komandan_plugin_abi::PluginBox`. The `_v1` suffix is the ABI version;
  bump on any breaking change to the surface in this crate.

## What's inside

| Item | Role |
|---|---|
| `Plugin` (`#[sabi_trait]`) | Trait every plugin implements: `name`, `version`, `register`, `run`. |
| `CoreApi` (`#[sabi_trait]`) | Trait the host implements; plugins call back into it (`executor_run`, `executor_upload`, `executor_write_file`, `komando`, `defaults_get`, `defaults_set`, `report_record`, `host_info`, `now_playing_task`, `worker_lua`, `log`, `global_flags`). |
| `LoggerSink` (`#[sabi_trait]`) | Tiny log emitter bundled into `PluginContext`. |
| `HostInfo`, `TaskInput`, `ModuleResult`, `PluginDescriptor`, `GlobalFlags` | `#[repr(C)] #[derive(StableAbi)]` mirror types of komandan-core's runtime structs. |
| `RValue` | `#[repr(u8)] #[non_exhaustive] #[derive(StableAbi)]` tagged value for module-argument passing. |
| `ConnectionHandle` | Opaque `u64` ID for a host-side connection (no `unsafe` downcast, no `DynTrait` erase/unerase). |
| `LuaHandle` | v0.1 unit placeholder for `worker_lua()` (`mlua::Lua` is `!Send` and cannot cross FFI). |
| `PluginContext` | Bundle handed to `Plugin::run`: `CoreApiRef` (`RArc`), `HostInfo`, `LoggerSinkBox`. |
| `PluginBox`, `PluginEntryFn`, `ENTRY_SYMBOL`, `ABI_VERSION` | Entry-symbol contract the host loader resolves. |
| `PluginError`, `CoreError` | Minimal `(kind, message)` error pairs (Phase-2 principle preserved across the boundary). |

## Secrets boundary

`HostInfo` exposes `password` and `private_key_pass` as plain `RStr` fields.
This is deliberate and matches komandan's existing Lua trust boundary
(AGENTS.md: *"Host exposes secrets when building the Lua table for module
consumption — that is the controlled boundary, not a leak"*). A loaded plugin
is trusted exactly like a built-in module: it runs as native code in the host
process. No new secrecy wrapper is introduced at this layer.

## Example plugin shape

```rust,ignore
// Cargo.toml:  [lib] crate-type = ["cdylib"]
//              [dependencies] komandan-plugin-abi = "0.1"
//
use komandan_plugin_abi::prelude::*;

#[derive(Debug)]
struct HelloPlugin;

impl Plugin for HelloPlugin {
    fn name(&self) -> RStr<'static>           { RStr::from("hello") }
    fn version(&self) -> RStr<'static>        { RStr::from("0.1.0") }
    fn register(&self) -> PluginDescriptor {
        PluginDescriptor {
            name: RStr::from("hello"),
            version: RStr::from("0.1.0"),
            abi_version: ABI_VERSION,
            description: RStr::from("Greet the user; proves the plugin ↔ host loop."),
            subcommand_name: RStr::from("hello"),
            capabilities: RStr::from(""), // v1 placeholder
        }
    }
    fn run(&self, ctx: &PluginContext, _args: RVec<RString>) -> RResult<RString, PluginError> {
        // One CoreApi round-trip: read global flags, log, and greet.
        let flags = ctx.core.global_flags();
        ctx.logger.log(LogLevel::Info, RStr::from("hello plugin dispatching"));
        let who = ctx.host.user.map(|u| u.to_string()).unwrap_or_else(|| "world".into());
        if flags.dry_run {
            ROk(RString::from(format!("[dry-run] hello, {who}")))
        } else {
            ROk(RString::from(format!("hello, {who}")))
        }
    }
}

// The single exported symbol the host loader looks up via libloading.
#[no_mangle]
pub extern "C" fn komandan_plugin_v1() -> PluginBox {
    Plugin_TO::from_value(HelloPlugin, TD_Opaque)
}
```

## Host consumption

The host crate is the **only** place `unsafe` lives (spec §5). It will:

1. `libloading::Library::new(path)` for each candidate plugin file.
2. `library.get::<PluginEntryFn>(ENTRY_SYMBOL)` — note the trailing NUL in
   `ENTRY_SYMBOL` is included so the byte string is ready for `libloading`.
3. Call the entry fn, obtain a `PluginBox`.
4. `plugin.register()` ⇒ check `PluginDescriptor::abi_version == ABI_VERSION`;
   skip with a warning otherwise.
5. `plugin.name()` collision check (first wins; §5.1).
6. Hold the `PluginBox` in a `PluginRegistry`. On CLI dispatch, construct a
   `PluginContext` and call `plugin.run(&ctx, argv_tail)` wrapped in
   `std::panic::catch_unwind` (§5.4).

The loader is **not** in this crate.
