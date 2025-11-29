use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;

use crate::args::{InitArgs, NewArgs, ProjectArgs, ProjectCommands};

const KOMANDAN_TOML_TEMPLATE: &str = r#"[project]
name = "{{project_name}}"
version = "0.1.0"
main = "main.lua"
"#;

const HOSTS_LUA_TEMPLATE: &str = r#"return {
	{
		name = "server1",
		address = "10.0.0.1",
		user = "user1",
		tags = { "webserver" },
	}
}
"#;

const MAIN_LUA_TEMPLATE: &str = r#"local host = require("hosts")

local task = {
	name = "Hello world!",
	komandan.modules.cmd({
		cmd = "echo 123",
	}),
}

komandan.komando(hosts[1], task)
"#;

/// Handles the project command
///
/// # Errors
///
/// Returns an error if project initialization or creation fails
pub fn handle_project_command(args: &ProjectArgs) -> Result<()> {
    match &args.command {
        ProjectCommands::Init(init_args) => init_project(init_args),
        ProjectCommands::New(new_args) => new_project(new_args),
    }
}

/// Initialize a project in an existing directory
///
/// # Errors
///
/// Returns an error if:
/// - Directory is not empty
/// - Files cannot be created
fn init_project(args: &InitArgs) -> Result<()> {
    let dir = Path::new(&args.directory);

    // Create directory if it doesn't exist
    if !dir.exists() {
        fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
    }

    // Check if directory is empty
    let entries = fs::read_dir(dir)
        .with_context(|| format!("Failed to read directory: {}", dir.display()))?;

    if entries.count() > 0 {
        bail!("Directory is not empty: {}", dir.display());
    }

    // Get project name from directory
    let project_name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("myproject");

    // Create komandan.toml
    let komandan_toml = KOMANDAN_TOML_TEMPLATE.replace("{{project_name}}", project_name);
    let komandan_toml_path = dir.join("komandan.toml");
    fs::write(&komandan_toml_path, komandan_toml)
        .with_context(|| format!("Failed to write {}", komandan_toml_path.display()))?;

    // Create hosts.lua
    let hosts_lua_path = dir.join("hosts.lua");
    fs::write(&hosts_lua_path, HOSTS_LUA_TEMPLATE)
        .with_context(|| format!("Failed to write {}", hosts_lua_path.display()))?;

    // Create main.lua
    let main_lua_path = dir.join("main.lua");
    fs::write(&main_lua_path, MAIN_LUA_TEMPLATE)
        .with_context(|| format!("Failed to write {}", main_lua_path.display()))?;

    println!("Initialized komandan project in {}", dir.display());
    Ok(())
}

/// Create a new project in a new directory
///
/// # Errors
///
/// Returns an error if:
/// - Directory already exists and is not empty
/// - Project initialization fails
fn new_project(args: &NewArgs) -> Result<()> {
    let dir_name = args.dir.as_ref().unwrap_or(&args.name);
    let dir = Path::new(dir_name);

    // Check if directory exists and is not empty
    if dir.exists() {
        let entries = fs::read_dir(dir)
            .with_context(|| format!("Failed to read directory: {}", dir.display()))?;

        if entries.count() > 0 {
            bail!(
                "Directory already exists and is not empty: {}",
                dir.display()
            );
        }
    }

    // Create the directory
    fs::create_dir_all(dir)
        .with_context(|| format!("Failed to create directory: {}", dir.display()))?;

    // Initialize the project
    let init_args = InitArgs {
        directory: dir_name.clone(),
    };
    init_project(&init_args)?;

    Ok(())
}
