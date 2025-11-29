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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_imports() {
        // This test verifies that all necessary modules are accessible
        // and can be imported without errors
        let _ = Args::parse_from(["komandan", "--version"]);
    }

    #[test]
    fn test_args_struct_exists() {
        // Verify Args struct can be constructed
        let args = Args::parse_from(["komandan"]);
        assert!(args.main_file.is_none());
        assert!(args.chunk.is_none());
        assert!(args.command.is_none());
    }

    #[test]
    fn test_flags_default_values() {
        // Verify default flag values
        let args = Args::parse_from(["komandan"]);
        assert!(!args.flags.dry_run);
        assert!(!args.flags.no_report);
        assert!(!args.flags.interactive);
        assert!(!args.flags.verbose);
        assert!(!args.flags.unsafe_lua);
        assert!(!args.flags.version);
    }
}
