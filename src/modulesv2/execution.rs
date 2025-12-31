//! # `ModulesV2` Execution Engine
//!
//! The execution engine handles module execution with proper error handling,
//! result formatting, and resource cleanup. It provides a unified interface
//! for executing modules with both local and remote connections.
//!
//! ## Key Features
//!
//! - **Connection Management**: Automatic connection creation and cleanup
//! - **Error Handling**: Comprehensive error handling with troubleshooting guidance
//! - **Result Formatting**: Consistent result structure compatible with `ModulesV1`
//! - **Resource Cleanup**: Proper cleanup of connections and resources
//!
//! ## Architecture
//!
//! The execution engine consists of:
//! - `ConnectionManager`: Handles connection creation using the connection factory
//! - `ExecutionEngine`: Manages module execution and result processing
//! - `ModuleResult`: Standardized result structure for module outputs

use crate::args::Args;
use crate::connection::{Connection, create_connection};
use crate::util::dprint;
use clap::Parser;
use mlua::{Lua, Table, Value};

/// Connection manager for `ModulesV2`
///
/// Provides a simplified interface for creating connections based on host configuration.
/// Uses the existing connection factory to maintain consistency with the rest of Komandan.
pub struct ConnectionManager;

impl ConnectionManager {
    /// Get a connection based on host configuration
    ///
    /// Creates either a local or SSH connection based on the host parameter.
    /// If no host is provided, defaults to local execution.
    ///
    /// # Arguments
    /// * `lua` - The Lua context for validation and connection creation
    /// * `host` - Optional host configuration table
    ///
    /// # Returns
    /// * `mlua::Result<Connection>` - A configured connection ready for use
    ///
    /// # Errors
    /// Returns an error if:
    /// - Host validation fails
    /// - Connection creation fails
    /// - Authentication setup fails
    pub fn get_connection(lua: &Lua, host: Option<Table>) -> mlua::Result<Connection> {
        if let Some(host_table) = host {
            // Remote execution - use provided host
            create_connection(lua, &Value::Table(host_table))
        } else {
            // Local execution - create localhost host configuration
            let localhost = lua.create_table()?;
            localhost.set("address", "localhost")?;
            localhost.set("connection", "local")?;
            create_connection(lua, &Value::Table(localhost))
        }
    }
}

/// Execution engine for `ModulesV2` modules
///
/// Handles the complete execution lifecycle including connection management,
/// module execution, error handling, and result formatting.
pub struct ExecutionEngine;

impl ExecutionEngine {
    /// Execute a module with proper error handling and result formatting
    ///
    /// This is the main entry point for module execution. It handles:
    /// - Connection creation and management
    /// - Module logic execution
    /// - Error handling and reporting
    /// - Result formatting and cleanup
    /// - Logging and progress reporting
    /// - Dry run mode detection and handling
    ///
    /// # Arguments
    /// * `lua` - The Lua context
    /// * `module_name` - Name of the module being executed (for error reporting)
    /// * `params` - Module parameters table
    /// * `host` - Optional host configuration for remote execution
    /// * `module_logic` - The actual module implementation function
    ///
    /// # Returns
    /// * `mlua::Result<Table>` - Formatted result table compatible with `ModulesV1`
    ///
    /// # Errors
    /// Returns an error if:
    /// - Connection creation fails
    /// - Module logic execution fails
    /// - Result formatting fails
    pub fn execute_module<F>(
        lua: &Lua,
        module_name: &str,
        params: &Table,
        host: Option<Table>,
        module_logic: F,
    ) -> mlua::Result<Table>
    where
        F: FnOnce(&mut Connection, &Table) -> mlua::Result<ModuleResult>,
    {
        // Generate display names for logging
        let host_display = generate_host_display(&host);
        let task_display = generate_task_display(module_name);

        // Print start message
        print_task_start(&task_display, &host_display);

        // Get connection using the connection manager
        let mut connection = ConnectionManager::get_connection(lua, host).map_err(|e| {
            mlua::Error::RuntimeError(format!(
                "Failed to create connection for module {module_name}: {e}"
            ))
        })?;

        // Check dry run mode
        let dry_run = Args::parse().flags.dry_run;

        let result = if dry_run {
            execute_dry_run_logic(
                module_name,
                params,
                &mut connection,
                &task_display,
                &host_display,
            )?
        } else {
            // Execute module logic
            let result = module_logic(&mut connection, params).map_err(|e| {
                mlua::Error::RuntimeError(format!("Module {module_name} execution failed: {e}"))
            })?;

            // Print debug output if enabled
            print_debug_output(lua, &result.stdout);

            result
        };

        // Print completion message
        print_task_completion(&task_display, &host_display, &result);

        // Format result as Lua table compatible with ModulesV1
        Self::format_result(lua, result)
    }

    /// Format module result as Lua table
    ///
    /// Creates a result table that is compatible with `ModulesV1` format,
    /// ensuring consistency across the Komandan ecosystem.
    ///
    /// # Arguments
    /// * `lua` - The Lua context for table creation
    /// * `result` - The module result to format
    ///
    /// # Returns
    /// * `mlua::Result<Table>` - Formatted result table
    ///
    /// # Errors
    /// Returns an error if table creation or field setting fails
    fn format_result(lua: &Lua, result: ModuleResult) -> mlua::Result<Table> {
        let result_table = lua.create_table()?;
        result_table.set("stdout", result.stdout)?;
        result_table.set("stderr", result.stderr)?;
        result_table.set("exit_code", result.exit_code)?;
        result_table.set("changed", result.changed)?;
        Ok(result_table)
    }
}

/// Generate a display name for the host
///
/// Creates a human-readable host identifier for logging purposes.
/// Uses the host name if available, otherwise falls back to the address.
///
/// # Arguments
/// * `host` - Optional host configuration table
///
/// # Returns
/// * `String` - Display name for the host
fn generate_host_display(host: &Option<Table>) -> String {
    if let Some(host_table) = host {
        // Try to get the name first, then fall back to address
        if let Ok(name) = host_table.get::<String>("name") {
            if !name.is_empty() {
                return name;
            }
        }

        if let Ok(address) = host_table.get::<String>("address") {
            return address;
        }
    }

    "localhost".to_string()
}

/// Generate a display name for the task
///
/// Creates a human-readable task identifier for logging purposes.
///
/// # Arguments
/// * `module_name` - Name of the module being executed
///
/// # Returns
/// * `String` - Display name for the task
fn generate_task_display(module_name: &str) -> String {
    format!("{module_name} module")
}

/// Print task start message
///
/// Prints a message indicating that a task has started execution.
/// Format matches komando's logging format.
///
/// # Arguments
/// * `task_display` - Display name for the task
/// * `host_display` - Display name for the host
fn print_task_start(task_display: &str, host_display: &str) {
    println!(">> Running task '{task_display}' on host '{host_display}' ...");
}

/// Print task completion message
///
/// Prints a message indicating task completion with success/failure status.
/// Format matches komando's logging format.
///
/// # Arguments
/// * `task_display` - Display name for the task
/// * `host_display` - Display name for the host
/// * `result` - The module execution result
fn print_task_completion(task_display: &str, host_display: &str, result: &ModuleResult) {
    if result.exit_code != 0 {
        println!(
            ">> Task '{task_display}' on host '{host_display}' failed with exit code {}: {}",
            result.exit_code, result.stderr
        );
    } else {
        let state = if result.changed { "[Changed]" } else { "[OK]" };
        println!(">> Task '{task_display}' on host '{host_display}' succeeded. {state}");
    }
}

/// Print debug output using komandan.dprint()
///
/// Prints stdout from module execution when debug mode is enabled.
/// Uses the existing dprint utility function to respect verbose flag.
///
/// # Arguments
/// * `lua` - The Lua context for dprint
/// * `stdout` - The stdout content to print
///
/// # Errors
/// Logs any errors from dprint but doesn't propagate them
fn print_debug_output(lua: &Lua, stdout: &str) {
    if !stdout.is_empty() {
        if let Ok(stdout_value) = lua.create_string(stdout) {
            if let Err(e) = dprint(lua, Value::String(stdout_value)) {
                eprintln!("Warning: Failed to print debug output: {e}");
            }
        }
    }
}

/// Execute dry run logic for modules
///
/// Handles dry run mode execution for ModulesV2 modules. Since most modules
/// don't have specific dry run implementations, this function prints a warning
/// message and returns a dry run result with appropriate changed flag based on
/// the module type and parameters.
///
/// # Arguments
/// * `module_name` - Name of the module being executed
/// * `params` - Module parameters table
/// * `connection` - Connection to the target host (unused in dry run)
/// * `task_display` - Display name for the task (for logging)
/// * `host_display` - Display name for the host (for logging)
///
/// # Returns
/// * `mlua::Result<ModuleResult>` - Dry run result with appropriate messaging
///
/// # Errors
/// Returns an error if result creation fails
fn execute_dry_run_logic(
    module_name: &str,
    params: &Table,
    _connection: &mut Connection,
    task_display: &str,
    host_display: &str,
) -> mlua::Result<ModuleResult> {
    // Generate module-specific dry run message and determine changed status
    let (dry_run_message, would_change) = generate_dry_run_message(module_name, params)?;

    // Print dry run warning message with module-specific information
    println!(
        "[[ Task '{task_display}' on host '{host_display}' does not support dry-run. Assuming 'changed' is {}. ]]",
        would_change
    );

    // Print what would be executed
    println!("   Would execute: {dry_run_message}");

    // Return dry run result with module-specific information
    Ok(ModuleResult::complete(
        format!("Dry run: {dry_run_message}"),
        String::new(),
        0,
        would_change,
    ))
}

/// Generate module-specific dry run message and determine if changes would occur
///
/// Analyzes the module type and parameters to provide meaningful dry run output
/// and make an educated guess about whether the operation would result in changes.
///
/// # Arguments
/// * `module_name` - Name of the module being executed
/// * `params` - Module parameters table
///
/// # Returns
/// * `mlua::Result<(String, bool)>` - Tuple of (dry_run_message, would_change)
///
/// # Errors
/// Returns an error if parameter extraction fails
fn generate_dry_run_message(module_name: &str, params: &Table) -> mlua::Result<(String, bool)> {
    match module_name {
        "cmd" => {
            let command = params
                .get::<String>("cmd")
                .unwrap_or_else(|_| "unknown command".to_string());
            let message = format!("execute command: {command}");
            // Commands typically change system state unless they're read-only
            let would_change = !is_readonly_command(&command);
            Ok((message, would_change))
        }
        "apt" => {
            let package = extract_package_display(params);
            let state = params
                .get::<String>("state")
                .unwrap_or_else(|_| "present".to_string());
            let update_cache = params.get::<bool>("update_cache").unwrap_or(false);

            let mut message_parts = Vec::new();
            if update_cache {
                message_parts.push("update package cache".to_string());
            }
            message_parts.push(format!("manage package(s) {package} (state: {state})"));

            let message = message_parts.join(", ");
            // Package operations typically change system state
            let would_change = true;
            Ok((message, would_change))
        }
        "dnf" => {
            let package = extract_package_display(params);
            let state = params
                .get::<String>("state")
                .unwrap_or_else(|_| "present".to_string());
            let message = format!("manage package(s) {package} with DNF (state: {state})");
            // Package operations typically change system state
            let would_change = true;
            Ok((message, would_change))
        }
        "file" => {
            let path = params
                .get::<String>("path")
                .unwrap_or_else(|_| "unknown path".to_string());
            let content = params.get::<Option<String>>("content").unwrap_or(None);
            let state = params
                .get::<String>("state")
                .unwrap_or_else(|_| "present".to_string());

            let message = if let Some(content) = content {
                format!(
                    "create/update file {path} with content ({} bytes)",
                    content.len()
                )
            } else {
                format!("manage file {path} (state: {state})")
            };
            // File operations typically change system state unless removing non-existent files
            let would_change = state != "absent"; // Assume file exists for dry run
            Ok((message, would_change))
        }
        "systemd_service" => {
            let name = params
                .get::<String>("name")
                .unwrap_or_else(|_| "unknown service".to_string());
            let action = params
                .get::<String>("action")
                .unwrap_or_else(|_| "start".to_string());
            let message = format!("manage systemd service {name} (action: {action})");
            // Service operations typically change system state
            let would_change = true;
            Ok((message, would_change))
        }
        "template" => {
            let src = params
                .get::<String>("src")
                .unwrap_or_else(|_| "unknown template".to_string());
            let dest = params
                .get::<String>("dest")
                .unwrap_or_else(|_| "unknown destination".to_string());
            let message = format!("render template {src} to {dest}");
            // Template rendering typically changes system state
            let would_change = true;
            Ok((message, would_change))
        }
        "upload" => {
            let src = params
                .get::<String>("src")
                .unwrap_or_else(|_| "unknown source".to_string());
            let dest = params
                .get::<String>("dest")
                .unwrap_or_else(|_| "unknown destination".to_string());
            let message = format!("upload file from {src} to {dest}");
            // File uploads typically change system state
            let would_change = true;
            Ok((message, would_change))
        }
        "download" => {
            let url = params
                .get::<String>("url")
                .unwrap_or_else(|_| "unknown URL".to_string());
            let dest = params
                .get::<String>("dest")
                .unwrap_or_else(|_| "unknown destination".to_string());
            let message = format!("download from {url} to {dest}");
            // Downloads typically change system state
            let would_change = true;
            Ok((message, would_change))
        }
        _ => {
            // Generic fallback for unknown modules
            let message = format!("execute {module_name} module with provided parameters");
            // Unknown modules are assumed to change system state for safety
            let would_change = true;
            Ok((message, would_change))
        }
    }
}

/// Extract package display string from parameters
///
/// Handles both single package strings and arrays of packages.
///
/// # Arguments
/// * `params` - Module parameters table
///
/// # Returns
/// * `String` - Display string for package(s)
fn extract_package_display(params: &Table) -> String {
    match params.get::<mlua::Value>("package") {
        Ok(mlua::Value::String(pkg)) => pkg
            .to_str()
            .map_or_else(|_| "unknown package".to_string(), |s| s.to_string()),
        Ok(mlua::Value::Table(pkg_table)) => {
            let mut packages = Vec::new();
            for pair in pkg_table.pairs::<i32, String>() {
                if let Ok((_, package)) = pair {
                    packages.push(package);
                }
            }
            if packages.is_empty() {
                "unknown packages".to_string()
            } else if packages.len() == 1 {
                packages[0].clone()
            } else {
                format!("[{}]", packages.join(", "))
            }
        }
        _ => "unknown package".to_string(),
    }
}

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

/// Standardized result structure for `ModulesV2` modules
///
/// This structure provides a consistent format for module results that is
/// compatible with `ModulesV1` and the existing Komandan ecosystem.
#[derive(Debug, Clone)]
pub struct ModuleResult {
    /// Standard output from the module execution
    pub stdout: String,
    /// Standard error output from the module execution
    pub stderr: String,
    /// Exit code from the module execution (0 = success, non-zero = failure)
    pub exit_code: i32,
    /// Whether the operation made changes to the system
    pub changed: bool,
}

impl ModuleResult {
    /// Create a successful result
    ///
    /// # Arguments
    /// * `stdout` - The output from successful execution
    ///
    /// # Returns
    /// * `ModuleResult` - A result indicating success
    #[must_use]
    pub const fn success(stdout: String) -> Self {
        Self {
            stdout,
            stderr: String::new(),
            exit_code: 0,
            changed: true, // Assume changed by default for successful operations
        }
    }

    /// Create a successful result with explicit changed status
    ///
    /// # Arguments
    /// * `stdout` - The output from successful execution
    /// * `changed` - Whether the operation made changes
    ///
    /// # Returns
    /// * `ModuleResult` - A result indicating success
    #[must_use]
    pub const fn success_with_changed(stdout: String, changed: bool) -> Self {
        Self {
            stdout,
            stderr: String::new(),
            exit_code: 0,
            changed,
        }
    }

    /// Create a failure result
    ///
    /// # Arguments
    /// * `stderr` - The error message
    /// * `exit_code` - The exit code (should be non-zero)
    ///
    /// # Returns
    /// * `ModuleResult` - A result indicating failure
    #[must_use]
    pub const fn failure(stderr: String, exit_code: i32) -> Self {
        Self {
            stdout: String::new(),
            stderr,
            exit_code: if exit_code == 0 { 1 } else { exit_code },
            changed: false, // Failed operations don't change system state
        }
    }

    /// Create a result with both output and error information
    ///
    /// # Arguments
    /// * `stdout` - Standard output
    /// * `stderr` - Standard error
    /// * `exit_code` - Exit code
    /// * `changed` - Whether the operation made changes
    ///
    /// # Returns
    /// * `ModuleResult` - A complete result
    #[must_use]
    pub const fn complete(stdout: String, stderr: String, exit_code: i32, changed: bool) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
            changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_connection_manager_local() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test local connection (no host provided)
        let connection = ConnectionManager::get_connection(&lua, None)?;

        // Should create a local connection
        match connection {
            Connection::Local(_) => {}
            Connection::SSH(_) => panic!("Expected local connection"),
        }

        Ok(())
    }

    #[test]
    fn test_connection_manager_remote() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test remote connection
        let host = lua.create_table()?;
        host.set("address", "remote.example.com")?;
        host.set("user", "testuser")?;
        host.set("password", "testpass")?;

        // This should attempt to create an SSH connection
        // Note: This will fail in tests without actual SSH setup, but we can test the logic
        let result = ConnectionManager::get_connection(&lua, Some(host));

        // The result may fail due to no actual SSH server, but the connection type logic should work
        // We're mainly testing that the function doesn't panic and follows the right path
        match result {
            Ok(Connection::SSH(_)) | Err(_) => {} // Expected in test environment without SSH
            Ok(Connection::Local(_)) => panic!("Expected SSH connection for remote host"),
        }

        Ok(())
    }

    #[test]
    fn test_connection_manager_explicit_local() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test explicit local connection
        let host = lua.create_table()?;
        host.set("address", "remote.example.com")?;
        host.set("connection", "local")?; // Force local even for remote address

        let connection = ConnectionManager::get_connection(&lua, Some(host))?;

        // Should create a local connection despite remote address
        match connection {
            Connection::Local(_) => {}
            Connection::SSH(_) => panic!("Expected local connection when explicitly set"),
        }

        Ok(())
    }

    #[test]
    fn test_execution_engine_format_result() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test successful result formatting
        let result = ModuleResult::success("test output".to_string());
        let table = ExecutionEngine::format_result(&lua, result)?;

        assert_eq!(table.get::<String>("stdout")?, "test output");
        assert_eq!(table.get::<String>("stderr")?, "");
        assert_eq!(table.get::<i32>("exit_code")?, 0);
        assert!(table.get::<bool>("changed")?);

        // Test failure result formatting
        let result = ModuleResult::failure("test error".to_string(), 1);
        let table = ExecutionEngine::format_result(&lua, result)?;

        assert_eq!(table.get::<String>("stdout")?, "");
        assert_eq!(table.get::<String>("stderr")?, "test error");
        assert_eq!(table.get::<i32>("exit_code")?, 1);
        assert!(!table.get::<bool>("changed")?);

        Ok(())
    }

    #[test]
    fn test_execution_engine_execute_module() -> mlua::Result<()> {
        let lua = create_lua()?;

        let params = lua.create_table()?;
        params.set("test_param", "test_value")?;

        // Test successful module execution
        let result_table = ExecutionEngine::execute_module(
            &lua,
            "test_module",
            &params,
            None, // Local execution
            |_connection, params| {
                let test_param = params
                    .get::<String>("test_param")
                    .map_err(|e| mlua::Error::RuntimeError(format!("Missing test_param: {e}")))?;

                Ok(ModuleResult::success(format!("Processed: {test_param}")))
            },
        )?;

        assert_eq!(
            result_table.get::<String>("stdout")?,
            "Processed: test_value"
        );
        assert_eq!(result_table.get::<i32>("exit_code")?, 0);
        assert!(result_table.get::<bool>("changed")?);

        Ok(())
    }

    #[test]
    fn test_execution_engine_execute_module_failure() -> mlua::Result<()> {
        let lua = create_lua()?;

        let params = lua.create_table()?;

        // Test module execution failure
        let result = ExecutionEngine::execute_module(
            &lua,
            "test_module",
            &params,
            None, // Local execution
            |_connection, _params| {
                Err(mlua::Error::RuntimeError("Test module failure".to_string()))
            },
        );

        // Should return an error
        assert!(result.is_err());

        // Error message should include module name
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("test_module"));
                assert!(msg.contains("execution failed"));
            }
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_module_result_constructors() {
        // Test success constructor
        let result = ModuleResult::success("success output".to_string());
        assert_eq!(result.stdout, "success output");
        assert_eq!(result.stderr, "");
        assert_eq!(result.exit_code, 0);
        assert!(result.changed);

        // Test success with explicit changed status
        let result = ModuleResult::success_with_changed("no change output".to_string(), false);
        assert_eq!(result.stdout, "no change output");
        assert_eq!(result.stderr, "");
        assert_eq!(result.exit_code, 0);
        assert!(!result.changed);

        // Test failure constructor
        let result = ModuleResult::failure("error message".to_string(), 1);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "error message");
        assert_eq!(result.exit_code, 1);
        assert!(!result.changed);

        // Test failure constructor with zero exit code (should be corrected to 1)
        let result = ModuleResult::failure("error message".to_string(), 0);
        assert_eq!(result.exit_code, 1);
        assert!(!result.changed);

        // Test complete constructor
        let result = ModuleResult::complete("output".to_string(), "error".to_string(), 2, true);
        assert_eq!(result.stdout, "output");
        assert_eq!(result.stderr, "error");
        assert_eq!(result.exit_code, 2);
        assert!(result.changed);
    }

    #[test]
    fn test_execution_engine_with_host() -> mlua::Result<()> {
        let lua = create_lua()?;

        let params = lua.create_table()?;
        params.set("test_param", "remote_test")?;

        let host = lua.create_table()?;
        host.set("address", "localhost")?; // Use localhost to avoid SSH issues in tests
        host.set("connection", "local")?; // Force local connection

        // Test module execution with host
        let result_table = ExecutionEngine::execute_module(
            &lua,
            "test_module",
            &params,
            Some(host),
            |_connection, params| {
                let test_param = params
                    .get::<String>("test_param")
                    .map_err(|e| mlua::Error::RuntimeError(format!("Missing test_param: {e}")))?;

                Ok(ModuleResult::success(format!(
                    "Remote processed: {test_param}"
                )))
            },
        )?;

        assert_eq!(
            result_table.get::<String>("stdout")?,
            "Remote processed: remote_test"
        );
        assert_eq!(result_table.get::<i32>("exit_code")?, 0);

        Ok(())
    }

    #[test]
    fn test_generate_host_display() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test with no host (should default to localhost)
        let display = generate_host_display(&None);
        assert_eq!(display, "localhost");

        // Test with host name
        let host = lua.create_table()?;
        host.set("name", "web-server")?;
        host.set("address", "192.168.1.100")?;
        let display = generate_host_display(&Some(host));
        assert_eq!(display, "web-server");

        // Test with address but no name
        let host = lua.create_table()?;
        host.set("address", "192.168.1.100")?;
        let display = generate_host_display(&Some(host));
        assert_eq!(display, "192.168.1.100");

        // Test with empty name (should fall back to address)
        let host = lua.create_table()?;
        host.set("name", "")?;
        host.set("address", "192.168.1.100")?;
        let display = generate_host_display(&Some(host));
        assert_eq!(display, "192.168.1.100");

        Ok(())
    }

    #[test]
    fn test_generate_task_display() {
        let display = generate_task_display("cmd");
        assert_eq!(display, "cmd module");

        let display = generate_task_display("apt");
        assert_eq!(display, "apt module");
    }

    #[test]
    fn test_print_debug_output() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test with empty stdout (should not print anything)
        print_debug_output(&lua, "");

        // Test with stdout content
        print_debug_output(&lua, "test debug output");

        // The actual printing behavior depends on the verbose flag,
        // but we can at least verify the function doesn't panic
        Ok(())
    }

    #[test]
    fn test_execute_dry_run_logic() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("cmd", "echo 'test command'")?;

        // Create a local connection for testing
        let mut connection = ConnectionManager::get_connection(&lua, None)?;

        // Test dry run logic for cmd module
        let result =
            execute_dry_run_logic("cmd", &params, &mut connection, "cmd module", "localhost")?;

        // Verify dry run result
        assert_eq!(result.exit_code, 0);
        assert!(!result.changed); // echo command should be detected as read-only
        assert!(result.stdout.contains("Dry run"));
        assert!(
            result
                .stdout
                .contains("execute command: echo 'test command'")
        );
        assert!(result.stderr.is_empty());

        Ok(())
    }

    #[test]
    fn test_execute_dry_run_logic_apt() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("package", "nginx")?;
        params.set("state", "present")?;
        params.set("update_cache", true)?;

        let mut connection = ConnectionManager::get_connection(&lua, None)?;

        // Test dry run logic for apt module
        let result =
            execute_dry_run_logic("apt", &params, &mut connection, "apt module", "localhost")?;

        // Verify dry run result
        assert_eq!(result.exit_code, 0);
        assert!(result.changed); // apt operations should be detected as changing
        assert!(result.stdout.contains("Dry run"));
        assert!(result.stdout.contains("update package cache"));
        assert!(result.stdout.contains("manage package(s) nginx"));
        assert!(result.stderr.is_empty());

        Ok(())
    }

    #[test]
    fn test_execute_dry_run_logic_file() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;
        params.set("path", "/etc/test.conf")?;
        params.set("content", "test content")?;

        let mut connection = ConnectionManager::get_connection(&lua, None)?;

        // Test dry run logic for file module
        let result =
            execute_dry_run_logic("file", &params, &mut connection, "file module", "localhost")?;

        // Verify dry run result
        assert_eq!(result.exit_code, 0);
        assert!(result.changed); // file operations should be detected as changing
        assert!(result.stdout.contains("Dry run"));
        assert!(result.stdout.contains("create/update file /etc/test.conf"));
        assert!(result.stdout.contains("12 bytes")); // "test content" is 12 bytes
        assert!(result.stderr.is_empty());

        Ok(())
    }

    #[test]
    fn test_execute_dry_run_logic_unknown_module() -> mlua::Result<()> {
        let lua = create_lua()?;
        let params = lua.create_table()?;

        let mut connection = ConnectionManager::get_connection(&lua, None)?;

        // Test dry run logic for unknown module
        let result = execute_dry_run_logic(
            "unknown_module",
            &params,
            &mut connection,
            "unknown_module module",
            "localhost",
        )?;

        // Verify dry run result
        assert_eq!(result.exit_code, 0);
        assert!(result.changed); // unknown modules should assume changed for safety
        assert!(result.stdout.contains("Dry run"));
        assert!(result.stdout.contains("execute unknown_module module"));
        assert!(result.stderr.is_empty());

        Ok(())
    }

    #[test]
    fn test_generate_dry_run_message_cmd() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test read-only command
        let params = lua.create_table()?;
        params.set("cmd", "echo 'hello'")?;
        let (message, would_change) = generate_dry_run_message("cmd", &params)?;
        assert_eq!(message, "execute command: echo 'hello'");
        assert!(!would_change); // echo is read-only

        // Test write command
        let params = lua.create_table()?;
        params.set("cmd", "touch /tmp/test")?;
        let (message, would_change) = generate_dry_run_message("cmd", &params)?;
        assert_eq!(message, "execute command: touch /tmp/test");
        assert!(would_change); // touch changes system state

        Ok(())
    }

    #[test]
    fn test_generate_dry_run_message_apt() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test single package
        let params = lua.create_table()?;
        params.set("package", "nginx")?;
        params.set("state", "present")?;
        let (message, would_change) = generate_dry_run_message("apt", &params)?;
        assert_eq!(message, "manage package(s) nginx (state: present)");
        assert!(would_change);

        // Test with cache update
        let params = lua.create_table()?;
        params.set("package", "nginx")?;
        params.set("update_cache", true)?;
        let (message, would_change) = generate_dry_run_message("apt", &params)?;
        assert!(message.contains("update package cache"));
        assert!(message.contains("manage package(s) nginx"));
        assert!(would_change);

        Ok(())
    }

    #[test]
    fn test_extract_package_display() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test single package
        let params = lua.create_table()?;
        params.set("package", "nginx")?;
        let display = extract_package_display(&params);
        assert_eq!(display, "nginx");

        // Test multiple packages
        let packages = lua.create_table()?;
        packages.set(1, "nginx")?;
        packages.set(2, "apache2")?;
        let params = lua.create_table()?;
        params.set("package", packages)?;
        let display = extract_package_display(&params);
        assert_eq!(display, "[nginx, apache2]");

        // Test missing package
        let params = lua.create_table()?;
        let display = extract_package_display(&params);
        assert_eq!(display, "unknown package");

        Ok(())
    }

    #[test]
    fn test_is_readonly_command() {
        // Test read-only commands
        assert!(is_readonly_command("echo hello"));
        assert!(is_readonly_command("cat /etc/passwd"));
        assert!(is_readonly_command("ls -la"));
        assert!(is_readonly_command("pwd"));
        assert!(is_readonly_command("whoami"));
        assert!(is_readonly_command("ps aux"));
        assert!(is_readonly_command("grep pattern file"));

        // Test write commands
        assert!(!is_readonly_command("touch /tmp/test"));
        assert!(!is_readonly_command("rm -rf /tmp"));
        assert!(!is_readonly_command("mkdir /tmp/test"));
        assert!(!is_readonly_command("cp source dest"));
        assert!(!is_readonly_command("mv source dest"));
        assert!(!is_readonly_command("chmod 755 file"));
        assert!(!is_readonly_command("systemctl start nginx"));

        // Test edge cases
        assert!(is_readonly_command("ECHO hello")); // case insensitive
        assert!(!is_readonly_command("echo_command")); // must be word boundary
    }

    #[test]
    fn test_execution_engine_dry_run_mode() -> mlua::Result<()> {
        let lua = create_lua()?;

        let params = lua.create_table()?;
        params.set("test_param", "test_value")?;

        // Note: This test cannot easily test the actual dry run behavior
        // because Args::parse() reads from command line arguments.
        // In a real scenario, the dry run flag would be set via CLI.
        // We can test the dry run logic function separately above.

        // Test normal execution (non-dry-run)
        let result_table = ExecutionEngine::execute_module(
            &lua,
            "test_module",
            &params,
            None, // Local execution
            |_connection, params| {
                let test_param = params
                    .get::<String>("test_param")
                    .map_err(|e| mlua::Error::RuntimeError(format!("Missing test_param: {e}")))?;

                Ok(ModuleResult::success(format!("Processed: {test_param}")))
            },
        )?;

        // Should execute normally when not in dry run mode
        assert_eq!(
            result_table.get::<String>("stdout")?,
            "Processed: test_value"
        );
        assert_eq!(result_table.get::<i32>("exit_code")?, 0);
        assert!(result_table.get::<bool>("changed")?);

        Ok(())
    }
}
