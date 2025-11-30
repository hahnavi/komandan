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
    run_app(&args)
}

fn run_app(args: &Args) -> anyhow::Result<()> {
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

                        match defaults.hosts.write() {
                            Ok(mut hosts_lock) => {
                                *hosts_lock = hosts_vec;
                            }
                            Err(e) => {
                                eprintln!("Warning: Failed to set hosts defaults: {e}");
                            }
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
    use komandan::args::{Flags, InitArgs, ProjectArgs, ProjectCommands};
    use std::io::Write;
    use tempfile::TempDir;

    // Helper to create empty Args with default flags
    fn default_args() -> Args {
        Args {
            main_file: None,
            chunk: None,
            flags: Flags {
                dry_run: false,
                no_report: false,
                interactive: false,
                verbose: false,
                unsafe_lua: false,
                version: false,
            },
            command: None,
        }
    }

    #[test]
    fn test_module_imports() {
        // This test verifies that all necessary modules are accessible
        let _ = Args::parse_from(["komandan", "--version"]);
    }

    #[test]
    fn test_args_struct_exists() {
        let args = Args::parse_from(["komandan"]);
        assert!(args.main_file.is_none());
    }

    #[test]
    fn test_run_app_version() {
        let mut args = default_args();
        args.flags.version = true;
        assert!(run_app(&args).is_ok());
    }

    #[test]
    fn test_run_app_chunk() {
        let mut args = default_args();
        args.chunk = Some("print('hello from test')".to_string());
        assert!(run_app(&args).is_ok());
    }

    #[test]
    fn test_run_app_dry_run() {
        let mut args = default_args();
        args.chunk = Some("print('hello')".to_string());
        args.flags.dry_run = true;
        assert!(run_app(&args).is_ok());
    }

    #[test]
    fn test_run_app_subcommand() -> anyhow::Result<()> {
        let mut args = default_args();
        let temp_dir = TempDir::new()?;
        let dir_str = temp_dir
            .path()
            .to_str()
            .context("temp dir path should be valid UTF-8")?
            .to_string();

        args.command = Some(Commands::Project(ProjectArgs {
            command: ProjectCommands::Init(InitArgs { directory: dir_str }),
        }));

        assert!(run_app(&args).is_ok());
        Ok(())
    }

    #[test]
    fn test_run_app_file() -> anyhow::Result<()> {
        let mut args = default_args();
        let mut temp_file = tempfile::NamedTempFile::new()?;
        writeln!(temp_file, "print('file test')")?;

        args.main_file = Some(
            temp_file
                .path()
                .to_str()
                .context("temp file path should be valid UTF-8")?
                .to_string(),
        );

        assert!(run_app(&args).is_ok());
        Ok(())
    }
    #[test]
    fn test_run_app_directory_missing_config() -> anyhow::Result<()> {
        let mut args = default_args();
        let temp_dir = TempDir::new()?;
        args.main_file = Some(
            temp_dir
                .path()
                .to_str()
                .context("temp dir path should be valid UTF-8")?
                .to_string(),
        );

        let result = run_app(&args);
        assert!(result.is_err());
        assert!(
            result
                .err()
                .context("expected error for missing config")?
                .to_string()
                .contains("does not contain komandan.json")
        );
        Ok(())
    }
    #[test]
    fn test_run_app_directory_valid() -> anyhow::Result<()> {
        let mut args = default_args();
        let temp_dir = TempDir::new()?;
        let path = temp_dir.path();

        // Create komandan.json
        let config = r#"{
        "name": "test",
        "version": "0.1.0",
        "main": "main.lua",
        "defaults": {
            "hosts": "hosts.lua"
        }
    }"#;
        fs::write(path.join("komandan.json"), config)?;

        // Create main.lua
        fs::write(path.join("main.lua"), "print('main running')")?;

        // Create hosts.lua
        let hosts_content = r#"
        return {
            { address = "localhost", connection = "local"
            }
        }
    "#;
        fs::write(path.join("hosts.lua"), hosts_content)?;

        args.main_file = Some(
            path.to_str()
                .context("temp dir path should be valid UTF-8")?
                .to_string(),
        );

        assert!(run_app(&args).is_ok());
        Ok(())
    }
}
