use anyhow::Context;
use clap::Parser;
use komandan::{
    args::{Args, Commands},
    create_lua_with_args,
    defaults::Defaults,
    models::KomandanConfig,
    print_version, project, repl, run_main_file_with_args,
};
use mlua::{Lua, LuaSerdeExt};
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

    if let Some(command) = &args.command {
        return match command {
            Commands::Project(project_args) => project::handle_project_command(project_args),
        };
    }

    let lua = create_lua_with_args(args)?;

    if let Some(chunk_src) = args.chunk.clone() {
        lua.load(&chunk_src).eval::<()>()?;
    }

    if args.flags.dry_run {
        println!("[[[ Running in dry-run mode ]]]");
    }

    match &args.main_file {
        Some(main_file) => {
            let path = Path::new(main_file);
            if path.is_dir() {
                run_project_dir(path, args, &lua)?;
            } else {
                run_main_file_with_args(&lua, args, main_file)?;
            }
        }
        None if args.chunk.is_none() => repl(&lua)?,
        _ => {}
    }

    if args.flags.interactive && (args.main_file.is_some() || args.chunk.is_some()) {
        repl(&lua)?;
    }

    Ok(())
}

/// Loads host defaults from the project's configured hosts file into the global
/// `Defaults`, if a hosts file is configured and present. Emits warnings (no
/// hard error) when the file is missing or the lock is poisoned.
///
/// # Arguments
///
/// * `path` - Project directory containing the hosts file
/// * `config` - Parsed `komandan.json` config
/// * `lua` - Lua context used to evaluate the hosts file
///
/// # Errors
///
/// Returns an error only if reading/evaluating the hosts file fails.
fn load_hosts_defaults(path: &Path, config: &KomandanConfig, lua: &Lua) -> anyhow::Result<()> {
    let Some(hosts_file) = config.defaults.hosts.as_deref() else {
        return Ok(());
    };

    let hosts_path = path.join(hosts_file);
    if !hosts_path.exists() {
        eprintln!(
            "Warning: Hosts file '{}' not found; hosts defaults were not loaded",
            hosts_path.display()
        );
        eprintln!(
            "  This may cause issues if your automation relies on global hosts configuration."
        );
        eprintln!(
            "  Remediation: Create the hosts file at '{}' or remove the 'hosts' field from komandan.json defaults.",
            hosts_path.display()
        );
        return Ok(());
    }

    let hosts_content = fs::read_to_string(&hosts_path)?;
    let hosts_table: mlua::Table = lua.load(&hosts_content).eval()?;

    let mut hosts_vec = Vec::new();
    for pair in hosts_table.pairs::<mlua::Value, mlua::Value>() {
        let (_, value) = pair?;
        let json_value: serde_json::Value = LuaSerdeExt::from_value(lua, value)?;
        hosts_vec.push(json_value);
    }

    match Defaults::global().hosts.write() {
        Ok(mut hosts_lock) => *hosts_lock = hosts_vec,
        Err(e) => {
            eprintln!("Warning: Failed to set hosts defaults from '{hosts_file}': {e}");
            eprintln!(
                "  This may cause connection issues if hosts are referenced without explicit configuration."
            );
            eprintln!(
                "  Troubleshooting: Check that the hosts file syntax is valid and that defaults are accessible."
            );
        }
    }
    Ok(())
}

/// Runs a Komandan project directory: reads its `komandan.json`, loads host
/// defaults, then executes the configured main script.
///
/// # Arguments
///
/// * `path` - Project directory containing `komandan.json`
/// * `args` - Parsed CLI args
/// * `lua` - Lua context
///
/// # Errors
///
/// Returns an error if `komandan.json` is missing, unreadable, invalid, or if
/// main-script execution fails.
fn run_project_dir(path: &Path, args: &Args, lua: &Lua) -> anyhow::Result<()> {
    let config_path = path.join("komandan.json");
    anyhow::ensure!(
        config_path.exists(),
        "Directory {} does not contain komandan.json",
        path.display()
    );

    let config_content = fs::read_to_string(&config_path)?;
    let config: KomandanConfig = serde_json::from_str(&config_content)?;

    load_hosts_defaults(path, &config, lua)?;

    let main_script = path
        .join(config.main)
        .to_str()
        .context("project main script path must be valid UTF-8")?
        .to_string();
    run_main_file_with_args(lua, args, &main_script)?;
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
