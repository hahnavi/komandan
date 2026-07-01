//! The single audited `unsafe` boundary in komandan-core.
//!
//! This module is the ONLY place hand-written `unsafe` lives in the crate.
//! It loads a plugin cdylib via [`libloading`], resolves the
//! [`komandan_plugin_v1`](komandan_plugin_abi::ENTRY_SYMBOL) entry symbol,
//! invokes it once inside [`catch_unwind`] (panic isolation), and verifies the
//! plugin's reported [`ABI_VERSION`] matches the host. Everything that escapes
//! this module is a fully-owned, safe handle ([`LoadedPlugin`]).
//!
//! # Why `unsafe` is unavoidable here
//!
//! `dlopen`/`dlsym` are inherently unsafe operations: resolving a symbol by
//! name and calling through the resulting pointer requires trusting that the
//! symbol has the expected type. The trust is bounded by:
//!
//! 1. The entry-symbol type is pinned by `komandan-plugin-abi` as
//!    `extern "C" fn() -> PluginBox` (a plain C-ABI function pointer).
//! 2. Every plugin MUST export exactly that signature; a mismatched plugin is
//!    UB, same as any FFI contract. The `abi_stable` layout-checker then
//!    validates the plugin's types on first call.
//! 3. The loaded `Library` is stored in [`LoadedPlugin`] and dropped AFTER the
//!    [`PluginBox`] (struct field drop order: `plugin` before `library`), so
//!    the cdylib stays mapped for the plugin's entire usable lifetime.
//!
//! See `docs/PLUGIN_SYSTEM_SPEC.md` §5 for the full security model.
//!
//! # Lint carve-out
//!
//! `#![allow(unsafe_code)]` is scoped to THIS FILE ONLY. komandan-core remains
//! `#![deny(unsafe_code)]` everywhere else (inherited via
//! `[workspace.lints]`). AGENTS.md documents this carve-out.

// THIS FILE IS THE AUDITED UNSAFE BOUNDARY. Do not add `unsafe` elsewhere.
#![allow(unsafe_code)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use anyhow::{Context, Result, bail};

use komandan_plugin_abi::{ABI_VERSION, ENTRY_SYMBOL, PluginBox, PluginEntryFn};

use crate::plugin::LoadedPlugin;

/// Load a single plugin cdylib and verify it.
///
/// Performs, in order:
///
/// 1. `dlopen` the file at `path`.
/// 2. Resolve [`ENTRY_SYMBOL`] as a [`PluginEntryFn`].
/// 3. Call the entry symbol inside [`catch_unwind`] (a panicking plugin entry
///    is reported as an error, not an abort).
/// 4. Probe `register()` and refuse the plugin if its reported `abi_version`
///    differs from the host's [`ABI_VERSION`].
///
/// # Errors
///
/// - `Library::new` failure (missing file, wrong arch, link errors).
/// - Missing entry symbol.
/// - Entry symbol panicked.
/// - ABI version mismatch (the plugin was compiled against a different
///   `komandan-plugin-abi` than the host).
///
/// # Panics
///
/// Never. Plugin panics are caught and converted to errors.
pub(super) fn load(path: &Path) -> Result<LoadedPlugin> {
    // Safety: `Library::new` is unsafe in libloading 0.8 because loading a
    // shared library may execute its constructor code. A komandan plugin
    // cdylib's only constructor side effect is Rust runtime init (no plugin
    // author code runs until the entry symbol is called), and the operator
    // controls the path we load from. We hold `library` for the plugin's
    // entire lifetime (stored in `LoadedPlugin`).
    let library = unsafe {
        libloading::Library::new(path)
            .with_context(|| format!("failed to load plugin library {}", path.display()))?
    };

    // Safety: `library` owns the loaded shared object and stays alive for the
    // plugin's entire usable lifetime (stored in `LoadedPlugin`, dropped after
    // the `PluginBox` by struct field-declaration order). `get` resolves a
    // symbol by its null-terminated byte name — `ENTRY_SYMBOL` already includes
    // the trailing NUL. The resolved type is `PluginEntryFn`
    // (`extern "C" fn() -> PluginBox`), pinned by the ABI crate; every plugin
    // MUST export exactly this signature.
    let entry_fn: PluginEntryFn = unsafe {
        let symbol: libloading::Symbol<'_, PluginEntryFn> =
            library.get(ENTRY_SYMBOL).with_context(|| {
                format!(
                    "plugin {} has no `komandan_plugin_v1` symbol",
                    path.display()
                )
            })?;
        // Copy the function pointer out, releasing the borrow on `library`.
        *symbol
    };

    // Panic isolation: a plugin's entry symbol must never abort the host.
    // `AssertUnwindSafe` wraps the closure (which captures only the plain
    // function pointer `entry_fn`); the returned `PluginBox` is owned.
    let plugin_box: PluginBox = match catch_unwind(AssertUnwindSafe(|| entry_fn())) {
        Ok(p) => p,
        Err(_) => bail!("plugin {} panicked inside its entry symbol", path.display()),
    };

    let descriptor = plugin_box.register();
    if descriptor.abi_version != ABI_VERSION {
        bail!(
            "plugin {} was built against plugin ABI v{} but the host expects v{}; \
             refusing to load (rebuild the plugin against the current komandan-plugin-abi)",
            path.display(),
            descriptor.abi_version,
            ABI_VERSION
        );
    }

    let name = descriptor.name.to_string();
    tracing::info!(
        "loaded plugin `{name}` v{} (ABI v{}) from {}",
        descriptor.version,
        descriptor.abi_version,
        path.display()
    );

    // Field order matters: `library` is declared LAST in `LoadedPlugin` so it
    // drops AFTER `plugin` (whose vtable lives inside the mapped cdylib).
    Ok(LoadedPlugin {
        descriptor,
        plugin: plugin_box,
        library,
    })
}
