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

const MAIN_LUA_TEMPLATE: &str = r#"local hosts = require("hosts")

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
        ProjectCommands::Init(init_args) => init_project(init_args, None),
        ProjectCommands::New(new_args) => new_project(new_args),
    }
}

/// Initialize a project in a directory, creating it if it does not exist.
///
/// # Errors
///
/// Returns an error if:
/// - The directory cannot be created.
/// - The directory is not empty.
/// - Files cannot be created.
fn init_project(args: &InitArgs, project_name: Option<String>) -> Result<()> {
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
    let project_name = project_name.unwrap_or_else(|| {
        let canonical_dir = dir.canonicalize().ok();
        canonical_dir
            .as_ref()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("myproject")
            .to_string()
    });

    // Create komandan.toml
    let komandan_toml = KOMANDAN_TOML_TEMPLATE.replace("{{project_name}}", &project_name);
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

/// Create a new project, in a new or existing empty directory.
///
/// The project name from the `name` argument is used. If `--dir` is provided, the project
/// will be created in that directory. If `--dir` is not provided, a directory with the
/// same name as the project will be created in the current location.
///
/// # Errors
///
/// Returns an error if:
/// - The target directory already exists and is not empty.
/// - Project initialization fails.
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
    } else {
        // Create the directory
        fs::create_dir_all(dir)
            .with_context(|| format!("Failed to create directory: {}", dir.display()))?;
    }

    // Initialize the project
    let init_args = InitArgs {
        directory: dir_name.clone(),
    };
    init_project(&init_args, Some(args.name.clone()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_init_project_success() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let dir_path = temp_dir
            .path()
            .to_str()
            .context("invalid utf-8 path")?
            .to_string();

        let args = InitArgs {
            directory: dir_path,
        };

        init_project(&args, None)?;

        // Verify files were created
        assert!(temp_dir.path().join("komandan.toml").exists());
        assert!(temp_dir.path().join("hosts.lua").exists());
        assert!(temp_dir.path().join("main.lua").exists());

        Ok(())
    }

    #[test]
    fn test_init_project_non_empty_directory() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let dir_path = temp_dir
            .path()
            .to_str()
            .context("invalid utf-8 path")?
            .to_string();

        // Create a file to make directory non-empty
        fs::write(temp_dir.path().join("existing.txt"), "content")?;

        let args = InitArgs {
            directory: dir_path,
        };

        let result = init_project(&args, None);
        if let Err(e) = result {
            assert!(e.to_string().contains("Directory is not empty"));
        } else {
            panic!("Expected an error, but got Ok");
        }

        Ok(())
    }

    #[test]
    fn test_init_project_creates_all_files() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let dir_path = temp_dir
            .path()
            .to_str()
            .context("invalid utf-8 path")?
            .to_string();

        let args = InitArgs {
            directory: dir_path,
        };

        init_project(&args, None)?;

        // Verify komandan.toml exists and has content
        let toml_content = fs::read_to_string(temp_dir.path().join("komandan.toml"))?;
        assert!(toml_content.contains("[project]"));
        assert!(toml_content.contains("version = \"0.1.0\""));

        // Verify hosts.lua exists and has content
        let hosts_content = fs::read_to_string(temp_dir.path().join("hosts.lua"))?;
        assert!(hosts_content.contains("return {"));
        assert!(hosts_content.contains("server1"));

        // Verify main.lua exists and has content
        let main_content = fs::read_to_string(temp_dir.path().join("main.lua"))?;
        assert!(main_content.contains("local hosts = require(\"hosts\")"));
        assert!(main_content.contains("komandan.komando"));

        Ok(())
    }

    #[test]
    fn test_init_project_template_substitution() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let dir_path = temp_dir
            .path()
            .to_str()
            .context("invalid utf-8 path")?
            .to_string();

        let args = InitArgs {
            directory: dir_path,
        };

        init_project(&args, None)?;

        // Verify project name was substituted in komandan.toml
        let toml_content = fs::read_to_string(temp_dir.path().join("komandan.toml"))?;
        let dir_name = temp_dir
            .path()
            .file_name()
            .and_then(|n| n.to_str())
            .context("valid directory name")?;
        assert!(toml_content.contains(&format!("name = \"{dir_name}\"")));

        Ok(())
    }

    #[test]
    fn test_new_project_success() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let project_name = "test_project".to_string();
        let project_path = temp_dir.path().join(&project_name);

        let args = NewArgs {
            name: project_name.clone(),
            dir: Some(
                project_path
                    .to_str()
                    .context("invalid utf-8 path")?
                    .to_string(),
            ),
        };

        new_project(&args)?;

        // Verify directory was created
        assert!(project_path.exists());

        // Verify files were created
        assert!(project_path.join("komandan.toml").exists());
        assert!(project_path.join("hosts.lua").exists());
        assert!(project_path.join("main.lua").exists());

        // Verify project name in toml
        let toml_content = fs::read_to_string(project_path.join("komandan.toml"))?;
        assert!(toml_content.contains(&format!("name = \"{project_name}\"")));

        Ok(())
    }

    #[test]
    fn test_new_project_with_custom_dir() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let project_name = "myproject".to_string();
        let custom_dir = "custom_directory".to_string();
        let project_path = temp_dir.path().join(&custom_dir);

        let args = NewArgs {
            name: project_name,
            dir: Some(
                project_path
                    .to_str()
                    .context("invalid utf-8 path")?
                    .to_string(),
            ),
        };

        new_project(&args)?;

        // Verify custom directory was created
        assert!(project_path.exists());
        assert!(project_path.join("komandan.toml").exists());

        // Verify project name in toml
        let toml_content = fs::read_to_string(project_path.join("komandan.toml"))?;
        assert!(toml_content.contains("name = \"myproject\""));

        Ok(())
    }

    #[test]
    fn test_new_project_existing_non_empty_dir() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let project_name = "test_project".to_string();

        // Create a non-empty directory
        let existing_dir = temp_dir.path().join(&project_name);
        fs::create_dir(&existing_dir)?;
        fs::write(existing_dir.join("existing.txt"), "content")?;

        let args = NewArgs {
            name: project_name,
            dir: Some(
                existing_dir
                    .to_str()
                    .context("invalid utf-8 path")?
                    .to_string(),
            ),
        };

        let result = new_project(&args);
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .contains("Directory already exists and is not empty")
            );
        } else {
            panic!("Expected an error, but got Ok");
        }

        Ok(())
    }

    #[test]
    fn test_handle_project_command_init() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let dir_path = temp_dir
            .path()
            .to_str()
            .context("invalid utf-8 path")?
            .to_string();

        let args = ProjectArgs {
            command: ProjectCommands::Init(InitArgs {
                directory: dir_path,
            }),
        };

        handle_project_command(&args)?;

        // Verify files were created
        assert!(temp_dir.path().join("komandan.toml").exists());
        assert!(temp_dir.path().join("hosts.lua").exists());
        assert!(temp_dir.path().join("main.lua").exists());

        Ok(())
    }

    #[test]
    fn test_handle_project_command_new() -> Result<()> {
        let temp_dir = TempDir::new()?;
        let project_name = "test_project".to_string();
        let project_path = temp_dir.path().join(&project_name);

        let args = ProjectArgs {
            command: ProjectCommands::New(NewArgs {
                name: project_name,
                dir: Some(
                    project_path
                        .to_str()
                        .context("invalid utf-8 path")?
                        .to_string(),
                ),
            }),
        };

        handle_project_command(&args)?;

        // Verify directory and files were created
        assert!(project_path.exists());
        assert!(project_path.join("komandan.toml").exists());
        assert!(project_path.join("hosts.lua").exists());
        assert!(project_path.join("main.lua").exists());

        Ok(())
    }
}
