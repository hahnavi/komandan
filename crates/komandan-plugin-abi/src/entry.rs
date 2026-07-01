//! Entry-symbol contract between plugins and the host loader.
//!
//! This crate does **not** define the `komandan_plugin_v1` function — every
//! plugin defines its own, and a second definition here would collide with
//! every dependent's linker. What this module pins is the *type* of that
//! symbol's return value ([`PluginBox`]) and of the function pointer the
//! host loader resolves ([`PluginEntryFn`]).

use abi_stable::std_types::RBox;

use crate::traits::Plugin_TO;

/// Numeric ABI version this crate currently targets.
///
/// Bump on **any** breaking change to:
///
/// - the [`crate::Plugin`] or [`crate::CoreApi`] trait method lists or
///   signatures (renames, reorders, signature changes — *not* appending
///   defaulted methods, which `abi_stable` tolerates),
/// - any mirror-type field set ([`crate::HostInfo`], [`crate::TaskInput`],
///   [`crate::ModuleResult`], [`crate::PluginDescriptor`]),
/// - [`crate::RValue`]'s variant set,
/// - the entry-symbol name or return type.
///
/// The spec (`docs/PLUGIN_SYSTEM_SPEC.md` §4.3) suffixes the entry symbol
/// with `_vN`; the host refuses plugins whose reported `abi_version`
/// differs.
pub const ABI_VERSION: u32 = 1;

/// The byte-string name of the entry symbol plugins export.
///
/// The host loader matches this exactly via `libloading::Library::get` (the
/// trailing NUL is included so the byte string is ready for libloading
/// without further allocation).
pub const ENTRY_SYMBOL: &[u8] = b"komandan_plugin_v1\0";

/// The type-erased plugin object returned by a plugin's entry symbol.
///
/// This is the [`abi_stable`] recommended shape for a type-erased plugin
/// object crossing an FFI boundary:
///
/// - `Plugin_TO<'static, RBox<()>>` is the `#[sabi_trait]`-generated trait
///   object for [`crate::Plugin`]. The `_TO` suffix is the conventional
///   `abi_stable` name for "trait object" type.
/// - The `'static` lifetime parameter says the host's view of the plugin
///   borrows nothing shorter-lived.
/// - `RBox<()>` is the erased pointer: `RBox` owns its referent (so the
///   caller owns the plugin after the entry symbol returns it), and `()` is
///   the type-erased referent (the real plugin struct's identity is
///   recovered only inside the plugin that constructed it).
///
/// `RBox` (rather than `RArc`) is the right pick here because ownership of
/// the plugin is single-owner: the host loader takes the boxed plugin from
/// the entry symbol and never shares it.
///
/// Plugins construct this via:
///
/// ```rust,ignore
/// #[no_mangle]
/// pub extern "C" fn komandan_plugin_v1() -> komandan_plugin_abi::PluginBox {
///     use komandan_plugin_abi::{Plugin_TO, sabi_trait::TD_Opaque};
///     Plugin_TO::from_value(MyPlugin, TD_Opaque)
/// }
/// ```
pub type PluginBox = Plugin_TO<'static, RBox<()>>;

/// Function-pointer type for the [`ENTRY_SYMBOL`] entry point.
///
/// Plugins MUST export a function with this exact signature and name
/// (`#[no_mangle] pub extern "C" fn komandan_plugin_v1`). The host loader
/// resolves the symbol and calls it exactly once per plugin load.
///
/// The signature is `extern "C"` (not `extern "Rust"`) so the calling
/// convention is pinned independent of the Rust toolchain. `RBox`'s layout
/// is pinned by `abi_stable`, so the *return value* is also toolchain-safe.
///
/// # The `improper_ctypes_definitions` allow
///
/// rustc warns that `PluginBox` contains `NonOwningPhantom<...>`, an
/// abi_stable-internal zero-sized type without `#[repr(C)]`. The lint is
/// conservative: the *actual* layout of `PluginBox` is pinned by
/// `abi_stable`'s `#[sabi_trait]`-generated vtable (it is a `DynTrait`
/// wrapper around `RBox<()>`, both of which are `#[repr(C)]`). The phantom
/// is a type-system tag with zero size and no runtime representation. This
/// allow suppresses the false positive for the type alias only; it does
/// not relax any real FFI-safety guarantee.
#[allow(improper_ctypes_definitions)] // abi_stable internal phantom; see doc above.
pub type PluginEntryFn = extern "C" fn() -> PluginBox;
