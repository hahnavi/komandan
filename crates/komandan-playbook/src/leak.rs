//! Process-bounded string promotion for the `RStr<'static>` ABI requirement.
//!
//! komandan is a short-lived CLI (one playbook run per process invocation), so
//! leaking dynamic strings into `'static` is benign: nothing accumulates beyond
//! a single run and the process exits at its end. This is the sound,
//! `unsafe`-free alternative to the lifetime transmute the `'static` bound
//! would otherwise demand (the workspace denies `unsafe_code` outside
//! `plugin/loader.rs`).

use komandan_plugin_abi::RStr;

/// Promote `s` to `&'static str` (process-bounded leak).
#[must_use]
pub fn static_str(s: &str) -> &'static str {
    Box::leak(s.to_string().into_boxed_str())
}

/// Promote `s` to an `RStr<'static>`.
#[must_use]
pub fn rstr(s: &str) -> RStr<'static> {
    RStr::from(static_str(s))
}
