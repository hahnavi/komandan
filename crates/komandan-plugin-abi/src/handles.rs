//! Opaque handles the host hands to plugins.
//!
//! These deliberately do **not** expose raw pointers or `DynTrait`-erased
//! references to host-internal types. They are either zero-cost marker types
//! or plain integer-ID wrappers. The host maintains any real state behind
//! them; the plugin only ever hands them back to [`crate::CoreApi`] methods.

use abi_stable::StableAbi;

/// Opaque handle to an open host-side connection (an SSH session or a local
/// shell channel).
///
/// # Design — integer ID, not a type-erased pointer
///
/// The spec (`docs/PLUGIN_SYSTEM_SPEC.md` §5) sketches an
/// `executor_run(conn, command)` shape and §4.2 floats an opaque
/// `RConnection` `DynTrait`. Implementing that directly would require either:
///
/// 1. Crossing a `DynTrait<'static, RBox<()>, Connection_VTable>` (more
///    trait surface in the ABI crate, more vtable dispatch, and the host
///    must downcast on every call), **or**
/// 2. Crossing a raw `RBox<()>` and `unsafe`-downcasting it on the host
///    side.
///
/// Both are worse on the "no unsafe in this crate" and "minimal surface"
/// axes than a plain integer handle. Instead:
///
/// - The host assigns a fresh `u64` ID per `create_connection` call and
///   tracks the real connection object in an internal registry.
/// - [`crate::CoreApi::executor_run`] / `executor_upload` / `executor_write_file`
///   take `&ConnectionHandle` and look the real connection up by ID.
/// - [`crate::CoreApi::close_connection`] drops the registry entry.
///
/// This keeps the ABI crate 100% safe (no `unsafe`, no `DynTrait` erase/
/// unerase), and confines the registry lookup to the host crate where it
/// belongs.
///
/// IDs are process-local; they never serialize and never cross process
/// boundaries. An ID equal to [`Self::INVALID`] (i.e. `0`) is reserved to
/// mean "no connection" and is rejected by host methods.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, StableAbi)]
pub struct ConnectionHandle {
    /// Process-local connection ID assigned by the host. `0` is reserved as
    /// [`ConnectionHandle::INVALID`].
    pub id: u64,
}

impl ConnectionHandle {
    /// Sentinel for "no connection". Host methods reject this.
    pub const INVALID: Self = Self { id: 0 };

    /// Construct a handle from a host-assigned ID. The host is the only
    /// legitimate caller; plugins receive handles, they do not mint them.
    #[must_use]
    pub const fn from_id(id: u64) -> Self {
        Self { id }
    }

    /// Whether this handle is the invalid sentinel.
    #[must_use]
    pub const fn is_invalid(self) -> bool {
        self.id == Self::INVALID.id
    }
}

/// Placeholder handle returned by [`crate::CoreApi::worker_lua`].
///
/// # Why a placeholder
///
/// `mlua::Lua` is `!Send` and its API is full of generic lifetime parameters
/// — it cannot cross an FFI boundary, period (see
/// `docs/PLUGIN_SYSTEM_SPEC.md` §10.4). The plan for v1 is that the host's
/// `worker_lua` accessor accepts a closure (marshalled via a separate
/// `DynTrait`-wrapped `RWorkFn`), and runs that closure on the worker thread
/// where the per-worker Lua VM already lives (preserving the
/// "one Lua VM per rayon worker" invariant, AGENTS.md). That closure-marshalling
/// trait is not yet defined; until it lands, this handle is an opaque marker.
///
/// Plugins **MUST NOT** attempt to dereference, transmute, or store this
/// handle as anything other than a tag. Calling [`CoreApi::worker_lua`] and
/// receiving this marker is purely a probe that the host supports the
/// accessor.
///
/// # Layout
///
/// Carries a single private `u8` so the struct has a non-ZST C layout (ZST
/// unit structs are not FFI-safe; rustc rejects them in `extern "C"` fn
/// signatures). The byte carries no information; construction is via
/// [`LuaHandle::new`] only.
///
/// [`CoreApi::worker_lua`]: crate::CoreApi::worker_lua
#[repr(C)]
#[derive(Debug, StableAbi)]
pub struct LuaHandle {
    /// Opaque tag byte. Always `0`. Public-read via nothing; private-set so
    /// foreign code cannot pretend to construct a meaningful handle.
    _opaque: u8,
}

impl LuaHandle {
    /// Construct the v0.1 placeholder handle. The single argument exists
    /// only so this is callable as a free function-style constructor.
    #[must_use]
    pub const fn new() -> Self {
        Self { _opaque: 0 }
    }
}

impl Default for LuaHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_handle_round_trips() {
        let h = ConnectionHandle::from_id(42);
        assert_eq!(h.id, 42);
        assert!(!h.is_invalid());
        assert_eq!(ConnectionHandle::INVALID.id, 0);
        assert!(ConnectionHandle::INVALID.is_invalid());
    }

    #[test]
    fn lua_handle_is_unit() {
        let h = LuaHandle::new();
        let other = LuaHandle::default();
        // Smoke-check the type is movable; assign into a non-underscore name
        // so clippy::used_underscore_binding stays quiet.
        let moved: LuaHandle = h;
        let _: &LuaHandle = &moved;
        let _: &LuaHandle = &other;
    }
}
