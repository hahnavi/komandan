mod args;

use args::Args;
use clap::Parser;
use komandan::{print_version, repl, run_main_file, setup_lua_env};
use mlua::Lua;

fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.version {
        print_version();
        return Ok(());
    }

    let lua = Lua::new();

    setup_lua_env(&lua)?;

    if let Some(chunk) = args.chunk.clone() {
        lua.load(&chunk).eval::<()>()?;
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
