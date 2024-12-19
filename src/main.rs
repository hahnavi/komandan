mod args;

use args::Args;
use clap::Parser;
use komandan::{print_version, repl, run_main_file, setup_lua_env};
use mlua::Lua;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    if args.version {
        print_version();
        return Ok(());
    }

    let lua = Lua::new();

    setup_lua_env(&lua)?;

    let chunk = args.chunk.clone();
    match chunk {
        Some(chunk) => {
            lua.load(&chunk).eval::<()>()?;
        }
        None => {}
    }

    match &args.main_file {
        Some(main_file) => {
            run_main_file(&lua, &main_file)?;
        }
        None => {
            if args.chunk.is_none() {
                repl(&lua);
            }
        }
    };

    if args.interactive && (!args.main_file.is_none() || !&args.chunk.is_none()) {
        repl(&lua);
    }

    Ok(())
}
