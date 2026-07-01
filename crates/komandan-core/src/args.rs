use std::sync::{OnceLock, RwLock};

use clap::{Args as ClapArgs, Parser, Subcommand};

/// Your army commander
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(about, long_about = None)]
pub struct Args {
    /// Main file location
    #[arg()]
    pub main_file: Option<String>,

    /// Trailing arguments forwarded verbatim to a plugin subcommand
    /// (`komandan <plugin> <trailing...>`). Captured with `trailing_var_arg`
    /// so any flag-like token after the plugin name reaches the plugin
    /// untouched instead of being parsed by the host. Empty on the normal
    /// script path. The plugin owns its own argv parsing.
    #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 0..)]
    pub trailing: Vec<String>,

    /// Execute string 'chunk'
    #[arg(short = 'e')]
    pub chunk: Option<String>,

    #[clap(flatten)]
    pub flags: Flags,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum Commands {
    /// Project management commands
    Project(ProjectArgs),
}

#[derive(ClapArgs, Debug, PartialEq, Eq)]
pub struct ProjectArgs {
    #[command(subcommand)]
    pub command: ProjectCommands,
}

#[derive(Subcommand, Debug, PartialEq, Eq)]
pub enum ProjectCommands {
    /// Initialize a project in an existing directory
    Init(InitArgs),
    /// Create a new project in a new directory
    New(NewArgs),
}

#[derive(ClapArgs, Debug, PartialEq, Eq)]
pub struct InitArgs {
    /// Directory to initialize (defaults to current directory)
    #[arg(default_value = ".")]
    pub directory: String,
}

#[derive(ClapArgs, Debug, PartialEq, Eq)]
pub struct NewArgs {
    /// Project name
    pub name: String,

    /// Directory to create the project in (defaults to project name)
    #[arg(short, long)]
    pub dir: Option<String>,
}

#[derive(ClapArgs, Clone, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct Flags {
    /// Dry run mode
    #[arg(short, long)]
    pub dry_run: bool,

    /// Don't print report
    #[arg(short, long)]
    pub no_report: bool,

    /// Enter interactive mode after executing 'script'.
    #[arg(short, long)]
    pub interactive: bool,

    /// Print debug messages
    #[arg(short, long)]
    pub verbose: bool,

    /// The created Lua state will not have safety guarantees and will allow to load C modules
    #[arg(short, long)]
    pub unsafe_lua: bool,

    /// Print version information
    #[arg(short = 'V', long)]
    pub version: bool,
}

/// Updatable global resolved-config store.
///
/// Held in a `RwLock` so that repeated `init_global_config` calls (e.g.
/// `create_lua_with_args` invoked once per task in the same process, or unit
/// tests running `run_app` repeatedly with different args) refresh the active
/// config instead of silently keeping the first one. This replaces the older
/// `OnceLock<ResolvedConfig>` behavior that discarded later values and could
/// leave later VMs reading stale flags / `project_dir` / package-path state.
static GLOBAL_CONFIG: OnceLock<RwLock<ResolvedConfig>> = OnceLock::new();

fn config_cell() -> &'static RwLock<ResolvedConfig> {
    GLOBAL_CONFIG.get_or_init(|| {
        RwLock::new(ResolvedConfig {
            flags: Flags::default(),
            project_dir: String::new(),
        })
    })
}

/// Resolved runtime configuration, set once from parsed CLI args.
///
/// Carries the immutable flag set and the project directory used to seed Lua's
/// `package.path`. Mirrors the `Defaults::global()` pattern in `defaults.rs`.
#[derive(Clone, Debug)]
pub struct ResolvedConfig {
    /// CLI flag set.
    pub flags: Flags,
    /// Project directory (parent of the main Lua file, or CWD).
    pub project_dir: String,
}

/// Initialize (or refresh) the global resolved config.
///
/// The config lives in an updatable `RwLock`, so this is **not** a one-shot
/// any more: calling it again with new values replaces the active config and
/// subsequent `create_lua` / `create_lua_with_args` / `global_flags` callers
/// observe the latest values. This avoids the prior bug where the second call
/// silently dropped new flags/`project_dir`.
///
/// Returns an error only if the underlying lock is poisoned.
///
/// # Errors
///
/// Returns an error if the global config `RwLock` is poisoned.
///
/// # Panics
///
/// Never panics.
pub fn init_global_config(config: ResolvedConfig) -> Result<(), String> {
    {
        let mut guard = config_cell()
            .write()
            .map_err(|e| format!("global config lock poisoned: {e}"))?;
        *guard = config;
    }
    Ok(())
}

/// Returns a snapshot of the resolved global config.
///
/// Because the store is now updatable, callers receive a clone of the current
/// values rather than a static reference. `Flags::default()` / empty
/// `project_dir` are returned before `init_global_config` was ever called
/// (e.g. unit tests calling `create_lua()` directly).
///
/// # Errors
///
/// Returns an error if the global config `RwLock` is poisoned.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn global_config() -> ResolvedConfig {
    config_cell()
        .read()
        .map_or_else(|_| ResolvedConfig::default_empty(), |guard| guard.clone())
}

impl ResolvedConfig {
    fn default_empty() -> Self {
        Self {
            flags: Flags::default(),
            project_dir: String::new(),
        }
    }
}

/// Returns a snapshot of the resolved global flags.
///
/// Returns `Flags::default()` (all-false) when the global config was never
/// initialized or the lock is poisoned.
///
/// # Errors
///
/// Infallible: never returns an error.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn global_flags() -> Flags {
    config_cell()
        .read()
        .map(|guard| guard.flags.clone())
        .unwrap_or_default()
}
