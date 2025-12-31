//! # `ModulesV2` File Operations Module
//!
//! The `file` module provides file and directory management functionality for `ModulesV2`.
//! It supports both local and remote execution with automatic connection management.
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution - create file
//! local result = k.mod.file({
//!     path = "/tmp/test.txt",
//!     content = "Hello World",
//!     mode = "0644"
//! })
//!
//! -- Remote execution - create directory
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.file({
//!     path = "/opt/myapp",
//!     state = "directory",
//!     mode = "0755",
//!     owner = "myapp",
//!     group = "myapp"
//! }, host)
//!
//! -- Remove file
//! local result = k.mod.file({
//!     path = "/tmp/unwanted.txt",
//!     state = "absent"
//! })
//! ```
//!
//! ## Parameters
//!
//! - `path` (string, required): Path to the file or directory
//! - `state` (string, optional): File state - "file", "directory", "absent", or "link" (default: "file")
//! - `content` (string, optional): File content (only for state="file")
//! - `src` (string, optional): Source path for symlinks (required for state="link")
//! - `mode` (string, optional): File permissions (e.g., "0644")
//! - `owner` (string, optional): File owner
//! - `group` (string, optional): File group
//! - `backup` (boolean, optional): Create backup before modifying (default: false)
//!
//! ## Return Value
//!
//! Returns a table with:
//! - `stdout`: Operation output
//! - `stderr`: Error output
//! - `exit_code`: Exit code (0 for success)
//! - `changed`: Boolean indicating if the file system was modified

use super::execution::{ExecutionEngine, ModuleResult};
use super::factory::create_modulev2_function;
use crate::connection::Connection;
use mlua::{Lua, Table};

/// Create the `file_v2` function for `ModulesV2`
///
/// This function creates a ModulesV2-compatible file operations module that supports
/// both local and remote execution patterns.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - The configured `file_v2` function
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - File operations fail
pub fn file_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
    create_modulev2_function(lua, "file", |lua, params, host| {
        ExecutionEngine::execute_module(lua, "file", &params, host, |connection, params| {
            // Extract and validate parameters
            let file_params = extract_file_parameters(params)?;

            // Validate parameters
            validate_file_parameters(&file_params)?;

            // Execute file operations
            execute_file_operations(connection, &file_params)
        })
    })
}

/// File operation parameters
#[derive(Debug, Clone)]
struct FileParameters {
    path: String,
    state: String,
    content: Option<String>,
    src: Option<String>,
    mode: Option<String>,
    owner: Option<String>,
    group: Option<String>,
    backup: bool,
}

/// Extract file parameters from Lua table
fn extract_file_parameters(params: &Table) -> mlua::Result<FileParameters> {
    let path = params.get::<String>("path").map_err(|_| {
        mlua::Error::RuntimeError("file module requires 'path' parameter".to_string())
    })?;

    let state = params
        .get::<Option<String>>("state")?
        .unwrap_or_else(|| "file".to_string());

    let content = params.get::<Option<String>>("content")?;
    let src = params.get::<Option<String>>("src")?;
    let mode = params.get::<Option<String>>("mode")?;
    let owner = params.get::<Option<String>>("owner")?;
    let group = params.get::<Option<String>>("group")?;
    let backup = params.get::<Option<bool>>("backup")?.unwrap_or(false);

    Ok(FileParameters {
        path,
        state,
        content,
        src,
        mode,
        owner,
        group,
        backup,
    })
}

/// Validate file parameters
fn validate_file_parameters(params: &FileParameters) -> mlua::Result<()> {
    // Validate path
    if params.path.trim().is_empty() {
        return Err(mlua::Error::RuntimeError(
            "path parameter cannot be empty".to_string(),
        ));
    }

    // Validate state
    match params.state.as_str() {
        "file" | "directory" | "absent" | "link" => {}
        _ => {
            return Err(mlua::Error::RuntimeError(format!(
                "Invalid state: {}. Valid states are: file, directory, absent, link",
                params.state
            )));
        }
    }

    // Validate state-specific requirements
    if params.state == "link" && params.src.is_none() {
        return Err(mlua::Error::RuntimeError(
            "'src' parameter is required when state is 'link'".to_string(),
        ));
    }

    // Validate mode format if provided
    if let Some(mode) = &params.mode {
        validate_mode(mode)?;
    }

    // Validate path format (basic security check)
    validate_path(&params.path)?;

    Ok(())
}

/// Validate file mode format
fn validate_mode(mode: &str) -> mlua::Result<()> {
    if mode.len() != 4 || !mode.starts_with('0') {
        return Err(mlua::Error::RuntimeError(
            "mode must be a 4-digit octal number (e.g., '0644')".to_string(),
        ));
    }

    for c in mode.chars().skip(1) {
        if !('0'..='7').contains(&c) {
            return Err(mlua::Error::RuntimeError(
                "mode must contain only octal digits (0-7)".to_string(),
            ));
        }
    }

    Ok(())
}

/// Basic path validation for security
fn validate_path(path: &str) -> mlua::Result<()> {
    // Check for null bytes
    if path.contains('\0') {
        return Err(mlua::Error::RuntimeError(
            "path cannot contain null bytes".to_string(),
        ));
    }

    // Check for extremely long paths
    if path.len() > 4096 {
        return Err(mlua::Error::RuntimeError(
            "path is too long (maximum 4096 characters)".to_string(),
        ));
    }

    Ok(())
}

/// Execute file operations based on parameters
fn execute_file_operations(
    connection: &mut Connection,
    params: &FileParameters,
) -> mlua::Result<ModuleResult> {
    let mut stdout_parts = Vec::new();
    let mut stderr_parts = Vec::new();
    let mut changed = false;

    // Check if file/directory exists
    let exists = check_file_exists(connection, &params.path)?;

    // Handle different states
    match params.state.as_str() {
        "absent" => {
            if exists {
                let result = remove_file_or_directory(connection, &params.path)?;
                stdout_parts.push(result.stdout);
                if !result.stderr.is_empty() {
                    stderr_parts.push(result.stderr);
                }
                if result.exit_code == 0 {
                    changed = true;
                }
                if result.exit_code != 0 {
                    return Ok(ModuleResult::complete(
                        stdout_parts.join("\n"),
                        stderr_parts.join("\n"),
                        result.exit_code,
                        false, // Operation failed, no change
                    ));
                }
            } else {
                stdout_parts.push(format!("File {} already absent", params.path));
            }
        }
        "directory" => {
            if exists {
                stdout_parts.push(format!("Directory {} already exists", params.path));
            } else {
                let result = create_directory(connection, &params.path)?;
                stdout_parts.push(result.stdout);
                if !result.stderr.is_empty() {
                    stderr_parts.push(result.stderr);
                }
                if result.exit_code == 0 {
                    changed = true;
                }
                if result.exit_code != 0 {
                    return Ok(ModuleResult::complete(
                        stdout_parts.join("\n"),
                        stderr_parts.join("\n"),
                        result.exit_code,
                        false, // Operation failed, no change
                    ));
                }
            }
        }
        "file" => {
            let mut file_changed = false;

            if !exists {
                let result = create_file(connection, &params.path, params.content.as_deref())?;
                stdout_parts.push(result.stdout);
                if !result.stderr.is_empty() {
                    stderr_parts.push(result.stderr);
                }
                if result.exit_code == 0 {
                    file_changed = true;
                }
                if result.exit_code != 0 {
                    return Ok(ModuleResult::complete(
                        stdout_parts.join("\n"),
                        stderr_parts.join("\n"),
                        result.exit_code,
                        false, // Operation failed, no change
                    ));
                }
            } else if let Some(content) = &params.content {
                // Check if content needs to be updated
                let content_changed = check_content_changed(connection, &params.path, content)?;
                if content_changed {
                    if params.backup {
                        let backup_result = create_backup(connection, &params.path)?;
                        stdout_parts.push(backup_result.stdout);
                        if !backup_result.stderr.is_empty() {
                            stderr_parts.push(backup_result.stderr);
                        }
                    }

                    let result = update_file_content(connection, &params.path, content)?;
                    stdout_parts.push(result.stdout);
                    if !result.stderr.is_empty() {
                        stderr_parts.push(result.stderr);
                    }
                    if result.exit_code == 0 {
                        file_changed = true;
                    }
                    if result.exit_code != 0 {
                        return Ok(ModuleResult::complete(
                            stdout_parts.join("\n"),
                            stderr_parts.join("\n"),
                            result.exit_code,
                            false, // Operation failed, no change
                        ));
                    }
                } else {
                    stdout_parts.push(format!("File {} content already correct", params.path));
                }
            } else {
                stdout_parts.push(format!("File {} already exists", params.path));
            }

            if file_changed {
                changed = true;
            }
        }
        "link" => {
            if let Some(src) = &params.src {
                if exists {
                    stdout_parts.push(format!("Symlink {} already exists", params.path));
                } else {
                    let result = create_symlink(connection, src, &params.path)?;
                    stdout_parts.push(result.stdout);
                    if !result.stderr.is_empty() {
                        stderr_parts.push(result.stderr);
                    }
                    if result.exit_code == 0 {
                        changed = true;
                    }
                    if result.exit_code != 0 {
                        return Ok(ModuleResult::complete(
                            stdout_parts.join("\n"),
                            stderr_parts.join("\n"),
                            result.exit_code,
                            false, // Operation failed, no change
                        ));
                    }
                }
            }
        }
        _ => unreachable!("State validation should prevent this"),
    }

    // Apply ownership and permissions if specified
    if params.state != "absent" && exists || changed {
        if let Some(mode) = &params.mode {
            let result = set_file_mode(connection, &params.path, mode)?;
            if !result.stdout.is_empty() {
                stdout_parts.push(result.stdout);
            }
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                    changed, // Use current changed state
                ));
            }
        }

        if let Some(owner) = &params.owner {
            let result = set_file_owner(connection, &params.path, owner)?;
            if !result.stdout.is_empty() {
                stdout_parts.push(result.stdout);
            }
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                    changed, // Use current changed state
                ));
            }
        }

        if let Some(group) = &params.group {
            let result = set_file_group(connection, &params.path, group)?;
            if !result.stdout.is_empty() {
                stdout_parts.push(result.stdout);
            }
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                    changed, // Use current changed state
                ));
            }
        }
    }

    Ok(ModuleResult::complete(
        stdout_parts.join("\n"),
        stderr_parts.join("\n"),
        0,       // Success
        changed, // Use the tracked changed state
    ))
}

/// Check if file or directory exists
fn check_file_exists(connection: &mut Connection, path: &str) -> mlua::Result<bool> {
    let (_, _, exit_code) = connection
        .cmd(&format!("[ -e '{}' ]", escape_shell_arg(path)))
        .map_err(|e| mlua::Error::RuntimeError(format!("File existence check failed: {e}")))?;
    Ok(exit_code == 0)
}

/// Remove file or directory
fn remove_file_or_directory(connection: &mut Connection, path: &str) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("rm -rf '{}'", escape_shell_arg(path)))
        .map_err(|e| mlua::Error::RuntimeError(format!("File removal failed: {e}")))?;

    Ok(ModuleResult::complete(
        if stdout.is_empty() {
            format!("Removed {path}")
        } else {
            stdout
        },
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Create directory
fn create_directory(connection: &mut Connection, path: &str) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("mkdir -p '{}'", escape_shell_arg(path)))
        .map_err(|e| mlua::Error::RuntimeError(format!("Directory creation failed: {e}")))?;

    Ok(ModuleResult::complete(
        if stdout.is_empty() {
            format!("Created directory {path}")
        } else {
            stdout
        },
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Create file with optional content
fn create_file(
    connection: &mut Connection,
    path: &str,
    content: Option<&str>,
) -> mlua::Result<ModuleResult> {
    let command = content.map_or_else(
        || format!("touch '{}'", escape_shell_arg(path)),
        |content| {
            format!(
                "cat > '{}' << 'EOF'\n{}\nEOF",
                escape_shell_arg(path),
                content
            )
        },
    );

    let (stdout, stderr, exit_code) = connection
        .cmd(&command)
        .map_err(|e| mlua::Error::RuntimeError(format!("File creation failed: {e}")))?;

    Ok(ModuleResult::complete(
        if stdout.is_empty() {
            format!("Created file {path}")
        } else {
            stdout
        },
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Check if file content has changed
fn check_content_changed(
    connection: &mut Connection,
    path: &str,
    new_content: &str,
) -> mlua::Result<bool> {
    let (current_content, _, exit_code) = connection
        .cmd(&format!("cat '{}'", escape_shell_arg(path)))
        .map_err(|e| mlua::Error::RuntimeError(format!("Content check failed: {e}")))?;

    if exit_code != 0 {
        return Ok(true); // If we can't read the file, assume it needs updating
    }

    Ok(current_content.trim() != new_content.trim())
}

/// Create backup of existing file
fn create_backup(connection: &mut Connection, path: &str) -> mlua::Result<ModuleResult> {
    let backup_path = format!("{path}.backup");
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!(
            "cp '{}' '{}'",
            escape_shell_arg(path),
            escape_shell_arg(&backup_path)
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Backup creation failed: {e}")))?;

    Ok(ModuleResult::complete(
        if stdout.is_empty() {
            format!("Created backup at {backup_path}")
        } else {
            stdout
        },
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Update file content
fn update_file_content(
    connection: &mut Connection,
    path: &str,
    content: &str,
) -> mlua::Result<ModuleResult> {
    let command = format!(
        "cat > '{}' << 'EOF'\n{}\nEOF",
        escape_shell_arg(path),
        content
    );

    let (stdout, stderr, exit_code) = connection
        .cmd(&command)
        .map_err(|e| mlua::Error::RuntimeError(format!("Content update failed: {e}")))?;

    Ok(ModuleResult::complete(
        if stdout.is_empty() {
            format!("Updated content of {path}")
        } else {
            stdout
        },
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Create symlink
fn create_symlink(
    connection: &mut Connection,
    src: &str,
    dest: &str,
) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!(
            "ln -s '{}' '{}'",
            escape_shell_arg(src),
            escape_shell_arg(dest)
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Symlink creation failed: {e}")))?;

    Ok(ModuleResult::complete(
        if stdout.is_empty() {
            format!("Created symlink {dest} -> {src}")
        } else {
            stdout
        },
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Set file mode (permissions)
fn set_file_mode(
    connection: &mut Connection,
    path: &str,
    mode: &str,
) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("chmod {} '{}'", mode, escape_shell_arg(path)))
        .map_err(|e| mlua::Error::RuntimeError(format!("Mode setting failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0,
    ))
}

/// Set file owner
fn set_file_owner(
    connection: &mut Connection,
    path: &str,
    owner: &str,
) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!(
            "chown '{}' '{}'",
            escape_shell_arg(owner),
            escape_shell_arg(path)
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Owner setting failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0,
    ))
}

/// Set file group
fn set_file_group(
    connection: &mut Connection,
    path: &str,
    group: &str,
) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!(
            "chgrp '{}' '{}'",
            escape_shell_arg(group),
            escape_shell_arg(path)
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Group setting failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0,
    ))
}

/// Escape shell arguments to prevent injection
fn escape_shell_arg(arg: &str) -> String {
    arg.replace('\'', "'\"'\"'")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_file_v2_create_file() -> mlua::Result<()> {
        let lua = create_lua()?;
        let file_fn = file_v2(&lua)?;

        // Test creating a file in /tmp
        let params = lua.create_table()?;
        params.set("path", "/tmp/test_modulesv2_file")?;
        params.set("content", "test content")?;
        params.set("mode", "0644")?;

        let result: Table = file_fn.call(params)?;

        // Check that the function executed successfully
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        // The exit code should be 0 for successful file operations
        assert_eq!(result.get::<i32>("exit_code")?, 0);

        Ok(())
    }

    #[test]
    fn test_file_v2_missing_path_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;
        let file_fn = file_v2(&lua)?;

        // Test with missing path parameter
        let params = lua.create_table()?;
        params.set("content", "test")?;

        let result: mlua::Result<Table> = file_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("file module requires 'path' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("file module requires 'path' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_file_v2_invalid_state() -> mlua::Result<()> {
        let lua = create_lua()?;
        let file_fn = file_v2(&lua)?;

        // Test with invalid state
        let params = lua.create_table()?;
        params.set("path", "/tmp/test")?;
        params.set("state", "invalid_state")?;

        let result: mlua::Result<Table> = file_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("Invalid state: invalid_state"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("Invalid state: invalid_state"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_file_v2_link_without_src() -> mlua::Result<()> {
        let lua = create_lua()?;
        let file_fn = file_v2(&lua)?;

        // Test link state without src parameter
        let params = lua.create_table()?;
        params.set("path", "/tmp/test_link")?;
        params.set("state", "link")?;

        let result: mlua::Result<Table> = file_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("'src' parameter is required when state is 'link'"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("'src' parameter is required when state is 'link'"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_validate_mode() {
        // Test valid modes
        assert!(validate_mode("0644").is_ok());
        assert!(validate_mode("0755").is_ok());
        assert!(validate_mode("0600").is_ok());

        // Test invalid modes
        assert!(validate_mode("644").is_err()); // Missing leading 0
        assert!(validate_mode("0888").is_err()); // Invalid octal digit
        assert!(validate_mode("0abc").is_err()); // Non-numeric
        assert!(validate_mode("064").is_err()); // Too short
    }

    #[test]
    fn test_validate_path() {
        // Test valid paths
        assert!(validate_path("/tmp/test").is_ok());
        assert!(validate_path("/home/user/file.txt").is_ok());
        assert!(validate_path("relative/path").is_ok());

        // Test invalid paths
        assert!(validate_path("/tmp/test\0").is_err()); // Null byte
        assert!(validate_path(&"x".repeat(5000)).is_err()); // Too long
    }

    #[test]
    fn test_escape_shell_arg() {
        assert_eq!(escape_shell_arg("simple"), "simple");
        assert_eq!(escape_shell_arg("with'quote"), "with'\"'\"'quote");
        assert_eq!(
            escape_shell_arg("multiple'quotes'here"),
            "multiple'\"'\"'quotes'\"'\"'here"
        );
    }

    #[test]
    fn test_file_parameters_extraction() -> mlua::Result<()> {
        let lua = create_lua()?;

        let params = lua.create_table()?;
        params.set("path", "/tmp/test")?;
        params.set("state", "directory")?;
        params.set("mode", "0755")?;
        params.set("owner", "root")?;
        params.set("backup", true)?;

        let file_params = extract_file_parameters(&params)?;

        assert_eq!(file_params.path, "/tmp/test");
        assert_eq!(file_params.state, "directory");
        assert_eq!(file_params.mode, Some("0755".to_string()));
        assert_eq!(file_params.owner, Some("root".to_string()));
        assert!(file_params.backup);

        Ok(())
    }
}
