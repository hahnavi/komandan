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

#[cfg(test)]
mod tests {
    use std::env;
    use std::io::Write;

    use super::*;

    #[tokio::test]
    async fn test_main_version() -> anyhow::Result<()> {
        let mut args = Args::parse_from(["komandan", "--version"]);
        args.version = true;

        let mut buf = Vec::new();
        let result = {
            let writer = std::io::BufWriter::new(&mut buf);
            let mut writer = std::io::LineWriter::new(writer);

            let res = main_with_args(args, &mut writer).await;
            writer.flush()?;
            res
        };

        assert!(result.is_ok());

        let output = String::from_utf8(buf)?;
        assert!(output.contains(env!("CARGO_PKG_VERSION")));
        Ok(())
    }

    async fn main_with_args(args: Args, writer: &mut impl std::io::Write) -> anyhow::Result<()> {
        if args.version {
            print_version_with_writer(writer)?;
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

    fn print_version_with_writer(writer: &mut impl std::io::Write) -> anyhow::Result<()> {
        writeln!(writer, "{}", env::var("CARGO_PKG_VERSION").unwrap())?;
        Ok(())
    }
}
