mod args;

use anyhow::Result;
use args::Args;
use clap::Parser;
use komandan::{create_lua, print_version, repl, run_main_file};

fn main() -> Result<()> {
    let args = Args::parse();

    if args.version {
        print_version();
        return Ok(());
    }

    let lua = create_lua()?;

    if let Some(chunk) = args.chunk.clone() {
        lua.load(&chunk).eval::<()>()?;
    }

    if args.dry_run {
        println!("[[[ Running in dry-run mode ]]]");
    }

    if let Some(main_file) = &args.main_file {
        run_main_file(&lua, main_file)?;
    } else if args.chunk.is_none() {
        repl(&lua);
    }

    if args.interactive && (args.main_file.is_some() || args.chunk.is_some()) {
        repl(&lua);
    }

    Ok(())
}
