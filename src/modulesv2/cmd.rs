//! # `ModulesV2` Command Module
//!
//! The `cmd` module provides command execution functionality for `ModulesV2`.
//! It supports both local and remote execution with automatic connection management.
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution
//! local result = k.mod.cmd({cmd = "echo 'Hello World'"})
//!
//! -- Remote execution
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.cmd({cmd = "echo 'Hello World'"}, host)
//! ```
//!
//! ## Parameters
//!
//! - `cmd` (string, required): The command to execute
//!
//! ## Return Value
//!
//! Returns a table with:
//! - `stdout`: Command output
//! - `stderr`: Error output
//! - `exit_code`: Exit code (0 for success)
//! - `changed`: Boolean indicating if the command succeeded

use super::execution::{ExecutionEngine, ModuleResult};
use super::factory::create_modulev2_function;
use mlua::Lua;

/// Check if a command is read-only (unlikely to change system state)
///
/// This is a heuristic to determine if a command would likely change system state.
/// Read-only commands are less likely to change the system.
///
/// # Arguments
/// * `command` - The command string to analyze
///
/// # Returns
/// * `bool` - true if the command appears to be read-only
fn is_readonly_command(command: &str) -> bool {
    let readonly_patterns = [
        "echo", "cat", "ls", "pwd", "whoami", "id", "date", "uptime", "ps", "top", "df", "du",
        "free", "uname", "hostname", "which", "whereis", "find", "grep", "awk", "sed -n", "head",
        "tail", "wc", "sort", "uniq", "cut", "tr", "test", "[", "[[",
    ];

    let command_lower = command.to_lowercase();
    readonly_patterns.iter().any(|&pattern| {
        // Check if command starts with pattern followed by space or is exactly the pattern
        command_lower == pattern || command_lower.starts_with(&format!("{pattern} "))
    })
}

/// Create the `cmd_v2` function for `ModulesV2`
///
/// This function creates a ModulesV2-compatible command execution module that supports
/// both local and remote execution patterns.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - The configured `cmd_v2` function
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - Command execution fails
pub fn cmd_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
    create_modulev2_function(lua, "cmd", |lua, params, host| {
        ExecutionEngine::execute_module(lua, "cmd", &params, host, |connection, params| {
            // Extract and validate the command parameter
            let command = params.get::<String>("cmd").map_err(|_| {
                mlua::Error::RuntimeError("cmd module requires 'cmd' parameter".to_string())
            })?;

            // Validate that command is not empty
            if command.trim().is_empty() {
                return Err(mlua::Error::RuntimeError(
                    "cmd parameter cannot be empty".to_string(),
                ));
            }

            // Execute the command using the connection
            let (stdout, stderr, exit_code) = connection
                .cmd(&command)
                .map_err(|e| mlua::Error::RuntimeError(format!("Command execution failed: {e}")))?;

            // For cmd module, we assume successful commands make changes
            // unless they are clearly read-only operations
            let changed = if exit_code == 0 {
                !is_readonly_command(&command)
            } else {
                false // Failed commands don't change system state
            };

            Ok(ModuleResult::complete(stdout, stderr, exit_code, changed))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;
    use mlua::Table;

    #[test]
    fn test_cmd_v2_local_execution() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test successful command execution
        let params = lua.create_table()?;
        params.set("cmd", "echo 'test output'")?;

        let result: Table = cmd_fn.call(params)?;

        assert_eq!(result.get::<i32>("exit_code")?, 0);
        assert!(result.get::<String>("stdout")?.contains("test output"));
        assert!(!result.get::<bool>("changed")?); // echo is read-only

        Ok(())
    }

    #[test]
    fn test_cmd_v2_remote_execution() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with explicit local connection (simulating remote)
        let params = lua.create_table()?;
        params.set("cmd", "echo 'remote test'")?;

        let host = lua.create_table()?;
        host.set("address", "localhost")?;
        host.set("connection", "local")?; // Force local for testing

        let result: Table = cmd_fn.call((params, host))?;

        assert_eq!(result.get::<i32>("exit_code")?, 0);
        assert!(result.get::<String>("stdout")?.contains("remote test"));
        assert!(!result.get::<bool>("changed")?); // echo is read-only

        Ok(())
    }

    #[test]
    fn test_cmd_v2_missing_cmd_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with missing cmd parameter
        let params = lua.create_table()?;
        // Don't set cmd parameter

        let result: mlua::Result<Table> = cmd_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("cmd module requires 'cmd' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("cmd module requires 'cmd' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_cmd_v2_empty_cmd_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with empty cmd parameter
        let params = lua.create_table()?;
        params.set("cmd", "")?;

        let result: mlua::Result<Table> = cmd_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("cmd parameter cannot be empty"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("cmd parameter cannot be empty"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_cmd_v2_whitespace_only_cmd() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with whitespace-only cmd parameter
        let params = lua.create_table()?;
        params.set("cmd", "   \t\n   ")?;

        let result: mlua::Result<Table> = cmd_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("cmd parameter cannot be empty"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("cmd parameter cannot be empty"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_cmd_v2_failing_command() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with a command that should fail
        let params = lua.create_table()?;
        params.set("cmd", "false")?; // Command that always returns exit code 1

        let result: Table = cmd_fn.call(params)?;

        assert_eq!(result.get::<i32>("exit_code")?, 1);
        assert!(!result.get::<bool>("changed")?); // Should be false for non-zero exit code

        Ok(())
    }

    #[test]
    fn test_cmd_v2_readonly_command() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with a read-only command
        let params = lua.create_table()?;
        params.set("cmd", "echo 'test output'")?;

        let result: Table = cmd_fn.call(params)?;

        assert_eq!(result.get::<i32>("exit_code")?, 0);
        assert!(result.get::<String>("stdout")?.contains("test output"));
        assert!(!result.get::<bool>("changed")?); // Should be false for read-only command

        Ok(())
    }

    #[test]
    fn test_cmd_v2_write_command() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with a write command
        let params = lua.create_table()?;
        params.set("cmd", "touch /tmp/test_file")?;

        let result: Table = cmd_fn.call(params)?;

        assert_eq!(result.get::<i32>("exit_code")?, 0);
        assert!(result.get::<bool>("changed")?); // Should be true for write command

        Ok(())
    }

    #[test]
    fn test_cmd_v2_command_with_output_and_error() -> mlua::Result<()> {
        let lua = create_lua()?;
        let cmd_fn = cmd_v2(&lua)?;

        // Test with a command that produces both stdout and stderr
        let params = lua.create_table()?;
        params.set("cmd", "echo 'stdout message' && echo 'stderr message' >&2")?;

        let result: Table = cmd_fn.call(params)?;

        assert_eq!(result.get::<i32>("exit_code")?, 0);
        assert!(result.get::<String>("stdout")?.contains("stdout message"));
        assert!(result.get::<String>("stderr")?.contains("stderr message"));
        assert!(!result.get::<bool>("changed")?); // echo is read-only

        Ok(())
    }
}
