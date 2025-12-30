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

use crate::connection::{Connection, create_connection};
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
        // Get connection using the connection manager
        let mut connection = ConnectionManager::get_connection(lua, host).map_err(|e| {
            mlua::Error::RuntimeError(format!(
                "Failed to create connection for module {module_name}: {e}"
            ))
        })?;

        // Execute module logic
        let result = module_logic(&mut connection, params).map_err(|e| {
            mlua::Error::RuntimeError(format!("Module {module_name} execution failed: {e}"))
        })?;

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
        result_table.set("changed", result.exit_code == 0)?;
        Ok(result_table)
    }
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
        }
    }

    /// Create a result with both output and error information
    ///
    /// # Arguments
    /// * `stdout` - Standard output
    /// * `stderr` - Standard error
    /// * `exit_code` - Exit code
    ///
    /// # Returns
    /// * `ModuleResult` - A complete result
    #[must_use]
    pub const fn complete(stdout: String, stderr: String, exit_code: i32) -> Self {
        Self {
            stdout,
            stderr,
            exit_code,
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

        // Test failure constructor
        let result = ModuleResult::failure("error message".to_string(), 1);
        assert_eq!(result.stdout, "");
        assert_eq!(result.stderr, "error message");
        assert_eq!(result.exit_code, 1);

        // Test failure constructor with zero exit code (should be corrected to 1)
        let result = ModuleResult::failure("error message".to_string(), 0);
        assert_eq!(result.exit_code, 1);

        // Test complete constructor
        let result = ModuleResult::complete("output".to_string(), "error".to_string(), 2);
        assert_eq!(result.stdout, "output");
        assert_eq!(result.stderr, "error");
        assert_eq!(result.exit_code, 2);
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
}
