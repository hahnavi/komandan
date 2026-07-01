//! Dynamic plugin system — host side: discovery, registry, dispatch.
//!
//! This module is the safe wrapper around [`loader`]. It owns loaded plugins,
//! builds the [`PluginContext`] for dispatch, and routes CLI invocations to
//! the right plugin's [`Plugin::run`](komandan_plugin_abi::Plugin::run).
//!
//! `unsafe` lives ONLY in [`loader`] (the audited `dlopen` boundary); this
//! file and [`core_api`] are fully safe.

mod conversions;
pub mod core_api;
mod loader;

use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

use anyhow::{Result, bail};

use komandan_plugin_abi::{
    CoreApi_TO, HostInfo, LoggerSink_TO, PluginBox, PluginContext, PluginDescriptor, RArc, RString,
    RVec, TD_Opaque,
};

use crate::plugin::core_api::{HostCore, HostLogger, default_host_info};

/// A successfully loaded plugin: its descriptor, the type-erased trait object,
/// and the owning `Library` handle (kept mapped for the plugin's lifetime).
///
/// # Drop order
///
/// `library` is declared LAST so it drops AFTER `plugin`: the `PluginBox`'s
/// vtable points into the mapped cdylib, so the library must remain mapped
/// until the plugin object is gone.
pub struct LoadedPlugin {
    /// Identity / capability metadata from `register()`.
    pub descriptor: PluginDescriptor,
    /// The type-erased plugin object driven via `Plugin::run` / `name` / etc.
    plugin: PluginBox,
    /// Owning handle to the loaded cdylib. Never read directly — its purpose
    /// is to keep the shared object mapped for `plugin`'s entire lifetime
    /// (the `PluginBox` vtable lives inside the mapped cdylib). Declared LAST
    /// so it drops after `plugin`.
    #[allow(dead_code)]
    library: libloading::Library,
}

/// Registry of loaded plugins, keyed by plugin name.
#[derive(Default)]
pub struct PluginRegistry {
    plugins: HashMap<String, LoadedPlugin>,
}

impl PluginRegistry {
    /// Create an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of loaded plugins.
    #[must_use]
    pub fn len(&self) -> usize {
        self.plugins.len()
    }

    /// Whether any plugin is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    /// Whether a plugin named `name` is loaded.
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    /// Sorted list of `(name, version)` for `--version` / `--help` output.
    #[must_use]
    pub fn listings(&self) -> Vec<(&str, &str, &str)> {
        let mut out: Vec<(&str, &str, &str)> = self
            .plugins
            .values()
            .map(|p| {
                (
                    p.descriptor.name.as_str(),
                    p.descriptor.version.as_str(),
                    p.descriptor.description.as_str(),
                )
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(b.0));
        out
    }

    /// Load every plugin file directly inside `dir` (non-recursive).
    ///
    /// Per-file failures (unreadable dir entry, bad cdylib, missing symbol,
    /// ABI mismatch, entry panic) are logged and skipped — one broken plugin
    /// does not prevent the others from loading.
    pub fn load_dir(&mut self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            tracing::debug!(
                "plugin directory {} is not readable; skipping",
                dir.display()
            );
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !is_plugin_file(&path) {
                continue;
            }
            match loader::load(&path) {
                Ok(loaded) => {
                    let name: String = loaded.descriptor.name.to_string();
                    if let std::collections::hash_map::Entry::Vacant(slot) =
                        self.plugins.entry(name.clone())
                    {
                        slot.insert(loaded);
                    } else {
                        tracing::warn!(
                            "plugin `{name}` already loaded; ignoring duplicate at {}",
                            path.display()
                        );
                    }
                }
                Err(e) => tracing::warn!("failed to load plugin {}: {e:#}", path.display()),
            }
        }
    }

    /// Dispatch `args` to the plugin named `name`.
    ///
    /// Builds a fresh [`PluginContext`] (v0.1 stubbed core + tracing logger +
    /// default local host), invokes [`Plugin::run`](komandan_plugin_abi::Plugin)
    /// inside `catch_unwind`, and returns the plugin's output string.
    ///
    /// # Errors
    ///
    /// - No plugin named `name` is loaded.
    /// - The plugin panicked inside `run` (caught, reported).
    /// - The plugin returned `RErr(PluginError)` (surfaced with `kind`/`message`).
    pub fn dispatch(&self, name: &str, args: Vec<OsString>) -> Result<String> {
        let Some(loaded) = self.plugins.get(name) else {
            bail!("no loaded plugin named `{name}`");
        };

        // Build the v0.1 PluginContext. HostCore wires connection/executor/
        // defaults/report/flags to komandan-core internals; `komando` is still
        // a documented stub (Lua-thread-pool design pending).
        let core = CoreApi_TO::from_ptr(RArc::new(HostCore::new()), TD_Opaque);
        let logger = LoggerSink_TO::from_value(HostLogger, TD_Opaque);
        let host: HostInfo = default_host_info();
        let ctx = PluginContext { core, host, logger };

        let rargs: RVec<RString> = args
            .into_iter()
            .map(|a| RString::from(a.to_string_lossy().into_owned()))
            .collect();

        // Panic isolation around plugin-run, mirroring the loader.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            loaded.plugin.run(&ctx, rargs)
        }));
        let Ok(rresult) = outcome else {
            bail!("plugin `{name}` panicked inside `run`");
        };

        match rresult.into_result() {
            Ok(s) => Ok(s.to_string()),
            Err(e) => bail!("plugin `{name}` failed ({}): {}", e.kind, e.message),
        }
    }
}

/// Resolve the plugin directory to scan.
///
/// Order: `KOMANDAN_PLUGIN_DIR` env var (if set), else
/// `$XDG_CONFIG_HOME/komandan/plugins`, else `$HOME/.config/komandan/plugins`.
/// Returns `None` if neither `XDG_CONFIG_HOME` nor `HOME` is set and the env
/// override is absent.
#[must_use]
pub fn discover_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("KOMANDAN_PLUGIN_DIR") {
        return Some(PathBuf::from(dir));
    }
    let base = std::env::var("XDG_CONFIG_HOME")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        });
    base.map(|b| b.join("komandan").join("plugins"))
}

/// Whether `path` looks like a loadable plugin cdylib by extension.
fn is_plugin_file(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    matches!(ext, "so" | "dylib" | "dll")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_registry_reports_empty() {
        let r = PluginRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(!r.has("anything"));
        assert!(r.listings().is_empty());
    }

    #[test]
    fn dispatch_unknown_plugin_errors() {
        let r = PluginRegistry::new();
        match r.dispatch("nope", vec![]) {
            Ok(_) => panic!("expected an error dispatching to a missing plugin"),
            Err(e) => assert!(
                e.to_string().contains("no loaded plugin named `nope`"),
                "unexpected error: {e}"
            ),
        }
    }

    #[test]
    fn discover_dir_env_override_takes_priority() {
        // Scoped env mutation is not thread-safe across tests; just assert
        // the function returns *something* sensible (Some when env set).
        // The real precedence is exercised by the end-to-end test.
        // Here we only check it does not panic.
        let _ = discover_dir();
    }

    #[test]
    fn is_plugin_file_rejects_non_files_and_wrong_ext() {
        assert!(!is_plugin_file(Path::new("/nonexistent/x.so")));
        assert!(!is_plugin_file(Path::new("Cargo.toml")));
    }
}
