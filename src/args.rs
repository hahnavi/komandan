use clap::Parser;

/// Your army commander
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Main file location
    #[arg(default_value = "main.lua")]
    pub main_file: String,

    /// Print debug messages
    #[arg(short, long)]
    pub verbose: bool,
}
