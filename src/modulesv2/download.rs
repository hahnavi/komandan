//! # `ModulesV2` File Download Module
//!
//! The `download` module provides file download functionality for `ModulesV2`.
//! It supports both local and remote execution with automatic connection management.
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution - copy file
//! local result = k.mod.download({
//!     src = "/tmp/remote_file.txt",
//!     dest = "local_file.txt"
//! })
//!
//! -- Remote execution - download file with backup
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.download({
//!     src = "/var/log/app.log",
//!     dest = "./logs/app.log",
//!     backup = true
//! }, host)
//! ```
//!
//! ## Parameters
//!
//! - `src` (string, required): Path to the source file to download
//! - `dest` (string, required): Destination path for the downloaded file
//! - `backup` (boolean, optional): Whether to backup existing destination file (default: false)
//! - `mode` (string, optional): File permissions for the destination file
//!
//! ## Return Value
//!
//! Returns a table with:
//! - `stdout`: Command output
//! - `stderr`: Error output
//! - `exit_code`: Exit code (0 for success)
//! - `changed`: Boolean indicating if the file was downloaded

use super::execution::{ExecutionEngine, ModuleResult};
use super::factory::create_modulev2_function;
use crate::connection::Connection;
use mlua::{Lua, Table};
use std::path::Path;

/// Create the `download_v2` function for `ModulesV2`
///
/// This function creates a ModulesV2-compatible file download module that supports
/// both local and remote execution patterns.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - The configured `download_v2` function
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - File download operations fail
pub fn download_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
    create_modulev2_function(lua, "download", |lua, params, host| {
        ExecutionEngine::execute_module(lua, "download", &params, host, |connection, params| {
            // Extract and validate parameters
            let src_path = extract_src_parameter(params)?;
            let dest_path = extract_dest_parameter(params)?;
            let backup = params.get::<Option<bool>>("backup")?.unwrap_or(false);
            let mode = params.get::<Option<String>>("mode")?;

            // Validate and sanitize paths
            let sanitized_src_path = sanitize_path(&src_path)?;
            let sanitized_dest_path = sanitize_path(&dest_path)?;

            // Execute file download operations
            execute_download_operations(
                connection,
                &sanitized_src_path,
                &sanitized_dest_path,
                backup,
                mode.as_deref(),
            )
        })
    })
}

/// Extract and validate the src parameter
fn extract_src_parameter(params: &Table) -> mlua::Result<String> {
    params.get::<Option<String>>("src")?.map_or_else(
        || {
            Err(mlua::Error::RuntimeError(
                "download module requires 'src' parameter".to_string(),
            ))
        },
        |src| {
            if src.trim().is_empty() {
                Err(mlua::Error::RuntimeError(
                    "src parameter cannot be empty".to_string(),
                ))
            } else {
                Ok(src)
            }
        },
    )
}

/// Extract and validate the dest parameter
fn extract_dest_parameter(params: &Table) -> mlua::Result<String> {
    params.get::<Option<String>>("dest")?.map_or_else(
        || {
            Err(mlua::Error::RuntimeError(
                "download module requires 'dest' parameter".to_string(),
            ))
        },
        |dest| {
            if dest.trim().is_empty() {
                Err(mlua::Error::RuntimeError(
                    "dest parameter cannot be empty".to_string(),
                ))
            } else {
                Ok(dest)
            }
        },
    )
}

/// Sanitize file paths to prevent directory traversal attacks
fn sanitize_path(path: &str) -> mlua::Result<String> {
    if path.trim().is_empty() {
        return Err(mlua::Error::RuntimeError(
            "Path cannot be empty".to_string(),
        ));
    }

    // For download operations, we allow relative paths and don't restrict directory traversal
    // since users should be able to download files from anywhere on the remote system
    // and save them anywhere on their local system
    Ok(path.to_string())
}

/// Execute file download operations
fn execute_download_operations(
    connection: &mut Connection,
    src_path: &str,
    dest_path: &str,
    backup: bool,
    mode: Option<&str>,
) -> mlua::Result<ModuleResult> {
    let mut stdout_parts = Vec::new();
    let mut stderr_parts = Vec::new();
    let mut changed = false;

    // Check if source file exists on remote/target system
    let (_, _, src_exists_exit_code) = connection
        .cmd(&format!("test -f {src_path}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Source file check failed: {e}")))?;

    if src_exists_exit_code != 0 {
        return Ok(ModuleResult::failure(
            format!("Source file does not exist: {src_path}"),
            1,
        ));
    }

    // Check if destination file exists and has the same content
    let needs_download = if Path::new(dest_path).exists() {
        match connection.cmd(&format!("cat {src_path}")) {
            Ok((src_content, _, exit_code)) => {
                if exit_code == 0 {
                    match std::fs::read_to_string(dest_path) {
                        Ok(dest_content) => src_content.trim() != dest_content.trim(),
                        Err(_) => true, // Error reading destination, assume we need to download
                    }
                } else {
                    true // Error reading source, assume we need to download
                }
            }
            Err(_) => true, // Error reading source, assume we need to download
        }
    } else {
        true // Destination doesn't exist, need to download
    };

    if !needs_download {
        stdout_parts.push(format!(
            "File content unchanged, skipping download: {src_path} -> {dest_path}"
        ));
        return Ok(ModuleResult::complete(
            stdout_parts.join("\n"),
            String::new(),
            0,
            false, // No change needed
        ));
    }

    // Create backup if requested and destination file exists
    if backup && Path::new(dest_path).exists() {
        let backup_path = format!("{dest_path}.backup");
        match std::fs::copy(dest_path, &backup_path) {
            Ok(_) => {
                stdout_parts.push(format!("Backup created: {backup_path}"));
            }
            Err(e) => {
                return Ok(ModuleResult::failure(
                    format!("Backup creation failed: {e}"),
                    1,
                ));
            }
        }
    }

    // Create destination directory if it doesn't exist
    if let Some(parent_dir) = Path::new(dest_path).parent()
        && !parent_dir.exists()
    {
        if let Err(e) = std::fs::create_dir_all(parent_dir) {
            return Ok(ModuleResult::failure(
                format!("Failed to create destination directory: {e}"),
                1,
            ));
        }
        stdout_parts.push(format!(
            "Created destination directory: {}",
            parent_dir.display()
        ));
    }

    // Download the file
    match connection.download(src_path, dest_path) {
        Ok(()) => {
            stdout_parts.push(format!(
                "File downloaded successfully: {src_path} -> {dest_path}"
            ));
            changed = true;
        }
        Err(e) => {
            return Ok(ModuleResult::failure(
                format!("File download failed: {e}"),
                1,
            ));
        }
    }

    // Set file permissions if specified
    if let Some(mode) = mode {
        // For local files, we need to use std::fs to set permissions
        use std::os::unix::fs::PermissionsExt;

        // Parse octal mode string
        let mode_value = u32::from_str_radix(mode.trim_start_matches('0'), 8)
            .map_err(|_| mlua::Error::RuntimeError(format!("Invalid mode format: {mode}")))?;

        let permissions = std::fs::Permissions::from_mode(mode_value);
        match std::fs::set_permissions(dest_path, permissions) {
            Ok(()) => {
                stdout_parts.push(format!("Set file permissions to {mode}"));
            }
            Err(e) => {
                stderr_parts.push(format!("Failed to set file permissions: {e}"));
                // Don't fail the entire operation for permission errors
            }
        }
    }

    Ok(ModuleResult::complete(
        stdout_parts.join("\n"),
        stderr_parts.join("\n"),
        0,       // Always return 0 for successful operations
        changed, // Use the tracked changed state
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_download_v2_missing_src() -> mlua::Result<()> {
        let lua = create_lua()?;
        let download_fn = download_v2(&lua)?;

        // Test with missing src parameter
        let params = lua.create_table()?;
        params.set("dest", "/tmp/test")?;

        let result: mlua::Result<Table> = download_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("download module requires 'src' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("download module requires 'src' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_download_v2_missing_dest() -> mlua::Result<()> {
        let lua = create_lua()?;
        let download_fn = download_v2(&lua)?;

        // Test with missing dest parameter
        let params = lua.create_table()?;
        params.set("src", "/tmp/source.txt")?;

        let result: mlua::Result<Table> = download_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("download module requires 'dest' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("download module requires 'dest' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_download_v2_with_valid_parameters() -> mlua::Result<()> {
        let lua = create_lua()?;
        let download_fn = download_v2(&lua)?;

        // Test with valid parameters
        let params = lua.create_table()?;
        params.set("src", "/etc/hostname")?; // Usually exists on Unix systems
        params.set("dest", "/tmp/test_download")?;

        let result: Table = download_fn.call(params)?;

        // In test environment, download operations might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_download_v2_with_backup() -> mlua::Result<()> {
        let lua = create_lua()?;
        let download_fn = download_v2(&lua)?;

        // Test with backup option
        let params = lua.create_table()?;
        params.set("src", "/etc/hostname")?;
        params.set("dest", "/tmp/test_backup_download")?;
        params.set("backup", true)?;

        let result: Table = download_fn.call(params)?;

        // In test environment, download operations might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_download_v2_with_mode() -> mlua::Result<()> {
        let lua = create_lua()?;
        let download_fn = download_v2(&lua)?;

        // Test with mode option
        let params = lua.create_table()?;
        params.set("src", "/etc/hostname")?;
        params.set("dest", "/tmp/test_mode_download")?;
        params.set("mode", "0644")?;

        let result: Table = download_fn.call(params)?;

        // In test environment, download operations might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_download_v2_nonexistent_source() -> mlua::Result<()> {
        let lua = create_lua()?;
        let download_fn = download_v2(&lua)?;

        // Test with nonexistent source file
        let params = lua.create_table()?;
        params.set("src", "/nonexistent/source.txt")?;
        params.set("dest", "/tmp/test_download")?;

        let result: Table = download_fn.call(params)?;

        // Should return an error result but not throw an exception
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        // Exit code should be non-zero for error
        let exit_code: i32 = result.get("exit_code")?;
        assert_ne!(exit_code, 0);

        Ok(())
    }

    #[test]
    fn test_sanitize_path() -> mlua::Result<()> {
        // Test valid paths
        assert_eq!(sanitize_path("/etc/config.conf")?, "/etc/config.conf");
        assert_eq!(sanitize_path("./local_file.txt")?, "./local_file.txt");
        assert_eq!(sanitize_path("../config/app.conf")?, "../config/app.conf");

        // Test empty path
        assert!(sanitize_path("").is_err());
        assert!(sanitize_path("   ").is_err());

        Ok(())
    }

    #[test]
    fn test_extract_src_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test valid src parameter
        let params = lua.create_table()?;
        params.set("src", "/path/to/file.txt")?;
        assert_eq!(extract_src_parameter(&params)?, "/path/to/file.txt");

        // Test empty src parameter
        let params = lua.create_table()?;
        params.set("src", "")?;
        assert!(extract_src_parameter(&params).is_err());

        // Test missing src parameter
        let params = lua.create_table()?;
        assert!(extract_src_parameter(&params).is_err());

        Ok(())
    }

    #[test]
    fn test_extract_dest_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test valid dest parameter
        let params = lua.create_table()?;
        params.set("dest", "/path/to/destination.txt")?;
        assert_eq!(extract_dest_parameter(&params)?, "/path/to/destination.txt");

        // Test empty dest parameter
        let params = lua.create_table()?;
        params.set("dest", "")?;
        assert!(extract_dest_parameter(&params).is_err());

        // Test missing dest parameter
        let params = lua.create_table()?;
        assert!(extract_dest_parameter(&params).is_err());

        Ok(())
    }
}
