use anyhow::Context;
use clap::Parser;
use komandan::{
    args::{Args, Commands},
    create_lua,
    defaults::Defaults,
    models::KomandanConfig,
    print_version, project, repl, run_main_file,
};
use mlua::LuaSerdeExt;
use std::fs;
use std::path::Path;

fn main() -> anyhow::Result<()> {
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
        let path = Path::new(main_file);
        if path.is_dir() {
            let config_path = path.join("komandan.json");
            if config_path.exists() {
                let config_content = fs::read_to_string(&config_path)?;
                let config: KomandanConfig = serde_json::from_str(&config_content)?;

                // Load defaults
                if let Some(hosts_file) = config.defaults.hosts {
                    let hosts_path = path.join(hosts_file);
                    if hosts_path.exists() {
                        let hosts_content = fs::read_to_string(&hosts_path)?;
                        let chunk = lua.load(&hosts_content);
                        let hosts_table: mlua::Table = chunk.eval()?;

                        let defaults = Defaults::global();
                        let mut hosts_vec = Vec::new();
                        for pair in hosts_table.pairs::<mlua::Value, mlua::Value>() {
                            let (_, value) = pair?;
                            let json_value: serde_json::Value =
                                LuaSerdeExt::from_value(&lua, value)?;
                            hosts_vec.push(json_value);
                        }

                        if let Ok(mut hosts_lock) = defaults.hosts.write() {
                            *hosts_lock = hosts_vec;
                        }
                    }
                }

                let main_script_path = path.join(config.main);
                run_main_file(
                    &lua,
                    &main_script_path
                        .to_str()
                        .context("Invalid path")?
                        .to_string(),
                )?;
            } else {
                // Directory but no komandan.json, maybe just try running main.lua?
                // Or error out? User said "scan the file komandan.json if any".
                // If not found, maybe fall back to treating it as a file (which will fail) or just error.
                // I'll assume if it's a dir and no json, it's an error or just try `main.lua`.
                // But `run_main_file` expects a file.
                anyhow::bail!("Directory {main_file} does not contain komandan.json");
            }
        } else {
            run_main_file(&lua, main_file)?;
        }
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
    use clap::Parser;

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
