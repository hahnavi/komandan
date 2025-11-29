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

#[derive(ClapArgs, Debug, PartialEq, Eq)]
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
