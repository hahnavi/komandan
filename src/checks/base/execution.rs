//! Common command execution functions

use super::{CheckResult, ExecutionContext};
use crate::connection::{Connection, create_connection};
use anyhow::{Context, Result};
use mlua::{Lua, Table, Value};
use std::collections::HashMap;

/// Execute a command in the given execution context with comprehensive error handling
pub fn execute_command(
    lua: &Lua,
    context: &ExecutionContext,
    command: &str,
) -> Result<(String, String, i32)> {
    match context {
        ExecutionContext::Local => execute_local_command(lua, command),
        ExecutionContext::Remote(host_table) => execute_remote_command(lua, host_table, command),
    }
}

/// Execute a command locally using the connection factory with error handling
fn execute_local_command(lua: &Lua, command: &str) -> Result<(String, String, i32)> {
    let host_table = lua
        .create_table()
        .context("Failed to create host table for local execution")?;
    host_table
        .set("address", "localhost")
        .context("Failed to set localhost address")?;
    host_table
        .set("connection", "local")
        .context("Failed to set local connection type")?;

    let connection = create_connection(lua, &Value::Table(host_table))
        .map_err(|e| anyhow::anyhow!("Failed to create local connection: {e}"))?;

    execute_command_with_connection(&connection, command)
        .with_context(|| format!("Local command execution failed: {command}"))
}

/// Execute a command remotely via SSH using the connection factory with error handling
fn execute_remote_command(
    lua: &Lua,
    host_table: &Table,
    command: &str,
) -> Result<(String, String, i32)> {
    // Extract host information for better error messages
    let host_address = host_table
        .get::<Option<String>>("address")
        .unwrap_or_default()
        .unwrap_or_else(|| "unknown".to_string());

    let connection = create_connection(lua, &Value::Table(host_table.clone()))
        .map_err(|e| anyhow::anyhow!("Failed to create SSH connection to {host_address}: {e}"))?;

    execute_command_with_connection(&connection, command)
        .with_context(|| format!("Remote command execution failed on {host_address}: {command}"))
}

/// Execute a command using the provided connection with error handling
fn execute_command_with_connection(
    connection: &Connection,
    command: &str,
) -> Result<(String, String, i32)> {
    let (stdout, stderr, exit_code) = connection
        .cmdq(command)
        .with_context(|| format!("Command execution failed: {command}"))?;

    Ok((stdout, stderr, exit_code))
}

/// Execute a command and handle common error scenarios
pub fn execute_command_with_error_handling(
    lua: &Lua,
    context: &ExecutionContext,
    command: &str,
    operation_description: &str,
) -> CheckResult {
    match execute_command(lua, context, command) {
        Ok((stdout, stderr, exit_code)) => {
            if exit_code == 0 {
                // Command succeeded, return stdout for further processing
                let mut actual = HashMap::new();
                actual.insert("stdout".to_string(), stdout);
                CheckResult::success(actual)
            } else {
                // Command failed, analyze the error
                let error_message = if !stderr.trim().is_empty() {
                    stderr.trim().to_string()
                } else if !stdout.trim().is_empty() {
                    stdout.trim().to_string()
                } else {
                    format!("Command failed with exit code {exit_code}")
                };

                // Categorize common error types
                if is_permission_error(&error_message) {
                    CheckResult::permission_error(&error_message, command)
                } else if is_not_found_error(&error_message) {
                    // For "not found" errors, return an empty actual map rather
                    // than a fabricated `exists=false`. Callers (file/package/
                    // service checks) interpret the missing key according to
                    // their own semantics; hardcoding `exists=false` here was
                    // wrong for package-style "is not installed" results.
                    CheckResult::success(HashMap::new())
                } else {
                    CheckResult::command_error(&error_message, command, exit_code)
                }
            }
        }
        Err(e) => {
            // Connection or system error
            let error_message = e.to_string();
            if is_ssh_error(&error_message) {
                let host = match context {
                    ExecutionContext::Remote(host_table) => host_table
                        .get::<Option<String>>("address")
                        .unwrap_or_default(),
                    ExecutionContext::Local => Some("localhost".to_string()),
                };
                CheckResult::ssh_error(&error_message, host.as_deref())
            } else {
                CheckResult::system_error(&format!("{operation_description}: {error_message}"))
            }
        }
    }
}

/// Check if an error message indicates a permission problem
pub fn is_permission_error(error_message: &str) -> bool {
    let error_lower = error_message.to_lowercase();
    error_lower.contains("permission denied")
        || error_lower.contains("access denied")
        || error_lower.contains("operation not permitted")
        || error_lower.contains("insufficient privileges")
        || error_lower.contains("not authorized")
}

/// Check if an error message indicates a "not found" condition
pub fn is_not_found_error(error_message: &str) -> bool {
    let error_lower = error_message.to_lowercase();
    error_lower.contains("no such file")
        || error_lower.contains("not found")
        || error_lower.contains("does not exist")
        || error_lower.contains("no packages found")
        || error_lower.contains("is not installed")
        || error_lower.contains("unit not found")
        || error_lower.contains("could not be found")
}

/// Check if an error message indicates an SSH connection problem
pub fn is_ssh_error(error_message: &str) -> bool {
    let error_lower = error_message.to_lowercase();
    error_lower.contains("ssh")
        || error_lower.contains("connection")
        || error_lower.contains("authentication")
        || error_lower.contains("host key")
        || error_lower.contains("network")
        || error_lower.contains("timeout")
}
