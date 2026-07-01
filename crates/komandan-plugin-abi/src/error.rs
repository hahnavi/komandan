//! Error types crossing the plugin ↔ host boundary.
//!
//! Both errors are deliberately minimal `(kind, message)` pairs, matching the
//! komandan project rule (AGENTS.md "Error handling") against bespoke
//! multi-field error enums. The Phase-2 cleanup that removed
//! `suggestion` / `troubleshooting` / `recovery_suggestion` from komandan's
//! own error types applies here too: user-facing troubleshooting text belongs
//! at call sites (in the host or plugin), not on the ABI types.

use abi_stable::StableAbi;
use abi_stable::std_types::{RStr, RString};

/// Error returned *by a plugin* to the host (e.g. from [`crate::Plugin::run`]).
///
/// `kind` is a short stable tag the host can switch on (`"usage"`,
/// `"runtime"`, `"io"`, `"module-not-found"`, ...). The set of kinds is open;
/// the host treats unknown kinds as generic failures. `message` is a
/// free-form, human-readable description the host prints verbatim.
///
/// # Layout
///
/// `#[repr(C)]` + [`StableAbi`] so the layout is pinned across Rust versions.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct PluginError {
    /// Short stable tag (e.g. `"usage"`, `"runtime"`). Open set; host
    /// tolerates unknown values.
    pub kind: RStr<'static>,
    /// Free-form human-readable detail. Printed verbatim by the host.
    pub message: RString,
}

impl PluginError {
    /// Construct a [`PluginError`] from a kind literal and an owned message.
    ///
    /// Convenience for plugin authors; performs no allocation beyond the
    /// argument's own.
    #[must_use]
    pub fn new(kind: &'static str, message: impl Into<RString>) -> Self {
        Self {
            kind: RStr::from(kind),
            message: message.into(),
        }
    }
}

/// Error returned *by the host* to a plugin (from [`crate::CoreApi`] methods).
///
/// Same shape and rationale as [`PluginError`]: a `(kind, message)` pair, no
/// bespoke troubleshooting fields. The host maps `kind` to user-facing text
/// at its call sites, preserving the Phase-2 boundary principle.
#[repr(C)]
#[derive(Debug, Clone, StableAbi)]
pub struct CoreError {
    /// Short stable tag identifying the failure class.
    pub kind: RStr<'static>,
    /// Free-form human-readable detail.
    pub message: RString,
}

impl CoreError {
    /// Construct a [`CoreError`] from a kind literal and an owned message.
    #[must_use]
    pub fn new(kind: &'static str, message: impl Into<RString>) -> Self {
        Self {
            kind: RStr::from(kind),
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_error_round_trips() {
        let e = PluginError::new("usage", "missing --foo");
        assert_eq!(e.kind.as_str(), "usage");
        assert_eq!(e.message.as_str(), "missing --foo");
    }

    #[test]
    fn core_error_round_trips() {
        let e = CoreError::new("io", "disk on fire");
        assert_eq!(e.kind.as_str(), "io");
        assert_eq!(e.message.as_str(), "disk on fire");
    }
}
