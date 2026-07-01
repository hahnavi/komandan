//! minijinja `Environment` factory configured per spec §7.2.
//!
//! The environment is the long-lived templating kernel: one is built per
//! render session and reused across every value rendered for a play. Filters,
//! tests, and lookups are registered here by their sibling modules.

use minijinja::{AutoEscape, Environment, UndefinedBehavior};

use super::{filters, jtests, lookups};

/// Build a fresh templating [`Environment`] configured for Ansible-compatible
/// rendering.
///
/// Configuration (spec §7.2):
/// - [`UndefinedBehavior::SemiStrict`] — error on missing top-level vars,
///   allow chained-attribute `None` propagation (closest to Ansible's default).
/// - Keep trailing newlines (`keep_trailing_newline=True`).
/// - Auto-escape off (Ansible does not auto-escape).
/// - Default Jinja delimiters (`{{ }}`, `{% %}`, `{# #}`).
///
/// All gap filters/tests/lookups from spec §7.3 are registered; minijinja's
/// own built-ins (`upper`, `lower`, `length`, `sort`, `map`, `select`, ...)
/// remain available.
///
/// # Errors
///
/// Building the environment itself is infallible; the [`Result`] return is
/// reserved for future load-time concerns (e.g. custom loaders). Callers may
/// `?` it freely today.
#[must_use]
pub fn build_environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_undefined_behavior(UndefinedBehavior::SemiStrict);
    env.set_keep_trailing_newline(true);
    // Ansible never auto-escapes; force it off regardless of "template name".
    env.set_auto_escape_callback(|_| AutoEscape::None);
    filters::register(&mut env);
    jtests::register(&mut env);
    lookups::register(&mut env);
    env
}
