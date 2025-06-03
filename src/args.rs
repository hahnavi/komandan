use clap::Parser;

/// Your army commander
#[derive(Parser, Debug, PartialEq)]
#[command(about, long_about = None)]
pub struct Args {
    /// Main file location
    #[arg()]
    pub main_file: Option<String>,

    /// Execute string 'chunk'
    #[arg(short = 'e')]
    pub chunk: Option<String>,

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
