use std::sync::OnceLock;

use clap::{Args as ClapArgs, Parser, Subcommand};

/// Your army commander
#[derive(Parser, Debug, PartialEq, Eq)]
#[command(about, long_about = None)]
pub struct Args {
    /// Main file location
    #[arg()]
    pub main_file: Option<String>,

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

static GLOBAL_CONFIG: OnceLock<ResolvedConfig> = OnceLock::new();

static DEFAULT_FLAGS: Flags = Flags {
    dry_run: false,
    no_report: false,
    interactive: false,
    verbose: false,
    unsafe_lua: false,
    version: false,
};

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

/// Initialize the global resolved config. Called once from `create_lua_with_args`.
///
/// Subsequent calls are ignored (config is immutable after first init).
///
/// # Errors
///
/// Infallible: never returns an error.
///
/// # Panics
///
/// Never panics. Re-initialization is silently ignored.
pub fn init_global_config(config: ResolvedConfig) {
    let _ = GLOBAL_CONFIG.set(config);
}

/// Returns the resolved global config, if it has been initialized.
///
/// `None` indicates `init_global_config` was never called (e.g. unit tests
/// calling `create_lua()` directly); callers should fall back to defaults.
///
/// # Errors
///
/// Infallible: never returns an error.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn global_config() -> Option<&'static ResolvedConfig> {
    GLOBAL_CONFIG.get()
}

/// Returns the resolved global flags.
///
/// Returns `Flags::default()` (all-false) when the global config was never
/// initialized (unit-test path).
///
/// # Errors
///
/// Infallible: never returns an error.
///
/// # Panics
///
/// Never panics.
#[must_use]
pub fn global_flags() -> &'static Flags {
    GLOBAL_CONFIG
        .get()
        .map_or(&DEFAULT_FLAGS, |config| &config.flags)
}
