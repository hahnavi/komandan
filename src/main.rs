use anyhow::Result;
use clap::Parser;
use komandan::{
    args::{Args, Commands},
    create_lua, print_version, project, repl, run_main_file,
};

fn main() -> Result<()> {
    let args = Args::parse();

    if args.flags.version {
        print_version();
        return Ok(());
    }

    // Handle subcommands first
    if let Some(command) = &args.command {
        match command {
            Commands::Project(project_args) => {
                return project::handle_project_command(project_args);
            }
        }
    }

    let lua = create_lua()?;

    if let Some(chunk) = args.chunk.clone() {
        lua.load(&chunk).eval::<()>()?;
    }

    if args.flags.dry_run {
        println!("[[[ Running in dry-run mode ]]]");
    }

    if let Some(main_file) = &args.main_file {
        run_main_file(&lua, main_file)?;
    } else if args.chunk.is_none() {
        repl(&lua)?;
    }

    if args.flags.interactive && (args.main_file.is_some() || args.chunk.is_some()) {
        repl(&lua)?;
    }

    Ok(())
}
