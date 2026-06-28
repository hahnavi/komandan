//! Optional REPL config loader.
//!
//! Reads `~/.config/komandan/repl.conf` (a simple `key = value` file) and turns
//! it into a [`rustyline::Config`] applied to the REPL editor. Parsing is
//! dependency-free (std + [`tracing`] only). A missing or malformed file is
//! never fatal: [`load_config`] silently falls back to
//! [`Config::default`](rustyline::Config::default).
//!
//! Supported keys (case-insensitive):
//!
//! | key                  | type / values                                  |
//! |----------------------|------------------------------------------------|
//! | `max_history_size`   | usize                                          |
//! | `tab_stop`           | usize (clamped to `0..=255`)                   |
//! | `history_ignore_dups`| bool                                           |
//! | `history_ignore_space`| bool                                          |
//! | `bracketed_paste`    | bool                                           |
//! | `completion_type`    | `circular` \| `list` \| `fuzzy` (alias `fancy`)*|
//! | `edit_mode`          | `emacs` \| `vi`                                |
//! | `bell_style`         | `none` \| `audible` \| `visible`               |
//! | `color_mode`         | `enabled` \| `forced` \| `disabled`            |
//!
//! `fuzzy`/`fancy` only take effect when Komandan is built with rustyline's
//! `with-fuzzy` feature (off by default); otherwise the value is warned and
//! skipped.

use std::{env, fs, path::PathBuf};

use rustyline::Config;
use rustyline::config::{BellStyle, Builder, Configurer};
use rustyline::{ColorMode, CompletionType, EditMode};

/// Booleans accepted (case-insensitive): `true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`.
const TRUE_VALUES: &[&str] = &["true", "1", "yes", "on"];
const FALSE_VALUES: &[&str] = &["false", "0", "no", "off"];

/// Resolves the REPL config file path.
///
/// Prefers `$XDG_CONFIG_HOME/komandan/repl.conf`, falling back to
/// `$HOME/.config/komandan/repl.conf`. Empty/unset vars are treated as absent.
/// Returns `None` when neither variable is usable.
fn config_path() -> Option<PathBuf> {
    if let Ok(xdg) = env::var("XDG_CONFIG_HOME") {
        let xdg = xdg.trim();
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("komandan").join("repl.conf"));
        }
    }
    if let Ok(home) = env::var("HOME") {
        let home = home.trim();
        if !home.is_empty() {
            return Some(
                PathBuf::from(home)
                    .join(".config")
                    .join("komandan")
                    .join("repl.conf"),
            );
        }
    }
    None
}

/// Loads the REPL config, applying `repl.conf` if it exists and parses.
///
/// Any I/O or parse problem falls back silently to
/// [`Config::default`](rustyline::Config::default); the REPL must always start.
#[must_use]
pub fn load_config() -> Config {
    let Some(path) = config_path() else {
        tracing::debug!("no config home env var set; using rustyline defaults");
        return Config::default();
    };
    let contents = match fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) => {
            tracing::debug!(
                path = %path.display(),
                %error,
                "could not read repl.conf; using rustyline defaults"
            );
            return Config::default();
        }
    };
    parse_config(&contents)
}

/// Parses a config file's full text into a [`Config`].
///
/// Pure (no I/O) so it is directly unit-testable. Blank lines and lines whose
/// first non-space character is `#` or `;` are ignored.
fn parse_config(text: &str) -> Config {
    let mut builder = Builder::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            tracing::warn!(
                line = raw,
                "malformed repl.conf line (missing '='); skipped"
            );
            continue;
        };
        apply_line(&mut builder, key.trim(), value.trim());
    }
    builder.build()
}

/// Applies a single trimmed `key = value` pair to `builder`.
///
/// Unknown keys and malformed values are warned and skipped; the builder is left
/// unchanged for that pair. The `key` is matched case-insensitively.
fn apply_line(builder: &mut Builder, key: &str, value: &str) {
    let key = key.to_ascii_lowercase();
    match key.as_str() {
        "max_history_size" => {
            if let Some(n) = parse_usize(value) {
                let _ = builder.set_max_history_size(n);
            } else {
                tracing::warn!(key = %key, value, "invalid usize value; skipped");
            }
        }
        "tab_stop" => {
            if let Some(n) = parse_usize(value) {
                if let Ok(b) = u8::try_from(n) {
                    builder.set_tab_stop(b);
                } else {
                    tracing::warn!(
                        key = %key,
                        value = %n,
                        "tab_stop out of range 0..=255; skipped"
                    );
                }
            } else {
                tracing::warn!(key = %key, value, "invalid usize value; skipped");
            }
        }
        "history_ignore_dups" => {
            if let Some(yes) = parse_bool(value) {
                let _ = builder.set_history_ignore_dups(yes);
            } else {
                tracing::warn!(key = %key, value, "invalid bool value; skipped");
            }
        }
        "history_ignore_space" => {
            if let Some(yes) = parse_bool(value) {
                builder.set_history_ignore_space(yes);
            } else {
                tracing::warn!(key = %key, value, "invalid bool value; skipped");
            }
        }
        "bracketed_paste" => {
            if let Some(enabled) = parse_bool(value) {
                builder.enable_bracketed_paste(enabled);
            } else {
                tracing::warn!(key = %key, value, "invalid bool value; skipped");
            }
        }
        "completion_type" => {
            if let Some(ct) = parse_completion_type(value) {
                builder.set_completion_type(ct);
            } else {
                tracing::warn!(key = %key, value, "invalid completion_type; skipped");
            }
        }
        "edit_mode" => {
            if let Some(em) = parse_edit_mode(value) {
                builder.set_edit_mode(em);
            } else {
                tracing::warn!(key = %key, value, "invalid edit_mode; skipped");
            }
        }
        "bell_style" => {
            if let Some(bs) = parse_bell_style(value) {
                builder.set_bell_style(bs);
            } else {
                tracing::warn!(key = %key, value, "invalid bell_style; skipped");
            }
        }
        "color_mode" => {
            if let Some(cm) = parse_color_mode(value) {
                builder.set_color_mode(cm);
            } else {
                tracing::warn!(key = %key, value, "invalid color_mode; skipped");
            }
        }
        other => tracing::warn!(key = other, value, "unknown repl.conf key; skipped"),
    }
}

/// Parses a recognized bool value (case-insensitive).
fn parse_bool(value: &str) -> Option<bool> {
    let lower = value.to_ascii_lowercase();
    if TRUE_VALUES.contains(&lower.as_str()) {
        Some(true)
    } else if FALSE_VALUES.contains(&lower.as_str()) {
        Some(false)
    } else {
        None
    }
}

/// Parses a usize via `str::parse`.
fn parse_usize(value: &str) -> Option<usize> {
    value.parse::<usize>().ok()
}

/// Parses a [`CompletionType`] value (case-insensitive).
///
/// `fuzzy`/`fancy` are only usable when rustyline is built with its
/// `with-fuzzy` feature (Komandan does not enable it), so they are rejected
/// here unconditionally — the `CompletionType::Fuzzy` variant does not exist
/// without that feature.
fn parse_completion_type(value: &str) -> Option<CompletionType> {
    match value.to_ascii_lowercase().as_str() {
        "circular" => Some(CompletionType::Circular),
        "list" => Some(CompletionType::List),
        _ => None,
    }
}

/// Parses an [`EditMode`] value (case-insensitive).
fn parse_edit_mode(value: &str) -> Option<EditMode> {
    match value.to_ascii_lowercase().as_str() {
        "emacs" => Some(EditMode::Emacs),
        "vi" => Some(EditMode::Vi),
        _ => None,
    }
}

/// Parses a [`BellStyle`] value (case-insensitive).
fn parse_bell_style(value: &str) -> Option<BellStyle> {
    match value.to_ascii_lowercase().as_str() {
        "none" => Some(BellStyle::None),
        "audible" => Some(BellStyle::Audible),
        "visible" => Some(BellStyle::Visible),
        _ => None,
    }
}

/// Parses a [`ColorMode`] value (case-insensitive).
fn parse_color_mode(value: &str) -> Option<ColorMode> {
    match value.to_ascii_lowercase().as_str() {
        "enabled" => Some(ColorMode::Enabled),
        "forced" => Some(ColorMode::Forced),
        "disabled" => Some(ColorMode::Disabled),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustyline::config::HistoryDuplicates;

    /// Builds a [`Config`] from a single `key = value` line for focused tests.
    fn config_from_line(key: &str, value: &str) -> Config {
        let mut builder = Builder::new();
        apply_line(&mut builder, key, value);
        builder.build()
    }

    #[test]
    fn bool_key_recognized() {
        // history_ignore_space default is false; setting true must flip it.
        let cfg = config_from_line("history_ignore_space", "yes");
        assert!(cfg.history_ignore_space());
        // history_ignore_dups=true -> IgnoreConsecutive; false -> AlwaysAdd.
        let cfg = config_from_line("history_ignore_dups", "off");
        assert_eq!(cfg.history_duplicates(), HistoryDuplicates::AlwaysAdd);
        // bracketed_paste default true; toggling off.
        let cfg = config_from_line("bracketed_paste", "0");
        assert!(!cfg.enable_bracketed_paste());
    }

    #[test]
    fn usize_key_recognized() {
        let cfg = config_from_line("max_history_size", "1234");
        assert_eq!(cfg.max_history_size(), 1234);
        let cfg = config_from_line("tab_stop", "4");
        assert_eq!(cfg.tab_stop(), 4);
    }

    #[test]
    fn tab_stop_clamped_and_oversized_rejected() {
        // value beyond u8::MAX must be skipped (stays default 8), not truncated.
        let cfg = config_from_line("tab_stop", "256");
        assert_eq!(cfg.tab_stop(), 8);
    }

    #[test]
    fn enum_completion_type_recognized() {
        assert_eq!(
            config_from_line("completion_type", "list").completion_type(),
            CompletionType::List
        );
        assert_eq!(
            config_from_line("COMPLETION_TYPE", "Circular").completion_type(),
            CompletionType::Circular
        );
    }

    #[test]
    fn enum_edit_mode_recognized() {
        assert_eq!(
            config_from_line("edit_mode", "vi").edit_mode(),
            EditMode::Vi
        );
        assert_eq!(
            config_from_line("edit_mode", "EMACS").edit_mode(),
            EditMode::Emacs
        );
    }

    #[test]
    fn enum_bell_style_recognized() {
        assert_eq!(
            config_from_line("bell_style", "none").bell_style(),
            BellStyle::None
        );
        assert_eq!(
            config_from_line("bell_style", "audible").bell_style(),
            BellStyle::Audible
        );
        assert_eq!(
            config_from_line("bell_style", "visible").bell_style(),
            BellStyle::Visible
        );
    }

    #[test]
    fn enum_color_mode_recognized() {
        assert_eq!(
            config_from_line("color_mode", "forced").color_mode(),
            ColorMode::Forced
        );
        assert_eq!(
            config_from_line("color_mode", "disabled").color_mode(),
            ColorMode::Disabled
        );
        assert_eq!(
            config_from_line("color_mode", "enabled").color_mode(),
            ColorMode::Enabled
        );
    }

    #[test]
    fn unknown_key_ignored() {
        let cfg = config_from_line("does_not_exist", "1");
        // Defaults untouched.
        assert_eq!(cfg.max_history_size(), 100);
        assert_eq!(cfg.edit_mode(), EditMode::Emacs);
    }

    #[test]
    fn malformed_bool_ignored() {
        let cfg = config_from_line("history_ignore_space", "maybe");
        assert!(!cfg.history_ignore_space()); // default false
    }

    #[test]
    fn malformed_usize_ignored() {
        let cfg = config_from_line("max_history_size", "abc");
        assert_eq!(cfg.max_history_size(), 100); // default
    }

    #[test]
    fn malformed_enum_ignored() {
        let cfg = config_from_line("edit_mode", "wordstar");
        assert_eq!(cfg.edit_mode(), EditMode::Emacs); // default
    }

    #[test]
    fn parse_config_full_file() {
        let text = "\
# comment line
; semicolon comment too
max_history_size = 50
  history_ignore_space = true
edit_mode = vi

color_mode = disabled
garbage line without equals
completion_type = list
";
        let cfg = parse_config(text);
        assert_eq!(cfg.max_history_size(), 50);
        assert!(cfg.history_ignore_space());
        assert_eq!(cfg.edit_mode(), EditMode::Vi);
        assert_eq!(cfg.color_mode(), ColorMode::Disabled);
        assert_eq!(cfg.completion_type(), CompletionType::List);
    }

    #[test]
    fn bool_parser_variants() {
        for v in ["true", "TRUE", "1", "yes", "YES", "on"] {
            assert_eq!(parse_bool(v), Some(true), "{v}");
        }
        for v in ["false", "FALSE", "0", "no", "NO", "off"] {
            assert_eq!(parse_bool(v), Some(false), "{v}");
        }
        assert_eq!(parse_bool("maybe"), None);
    }
}
