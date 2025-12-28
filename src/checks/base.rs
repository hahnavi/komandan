use anyhow::{Context, Result};
use mlua::{Lua, Table};
use serde::Serialize;
use std::collections::HashMap;

/// Error categories for check functions
#[derive(Debug, Clone)]
pub enum CheckErrorCategory {
    /// Parameter validation errors (missing required params, invalid values)
    ParameterValidation,
    /// SSH connection errors (connection timeout, auth failure, network issues)
    SshConnection,
    /// Permission errors (insufficient privileges, access denied)
    Permission,
    /// Command execution errors (command failed, unexpected output)
    CommandExecution,
    /// System errors (out of memory, disk full, etc.)
    System,
}

impl CheckErrorCategory {
    /// Get a human-readable description of the error category
    pub const fn description(&self) -> &'static str {
        match self {
            Self::ParameterValidation => "Parameter validation failed",
            Self::SshConnection => "SSH connection failed",
            Self::Permission => "Permission denied",
            Self::CommandExecution => "Command execution failed",
            Self::System => "System error",
        }
    }
}

/// Structured error information for check functions
#[derive(Debug, Clone)]
pub struct CheckError {
    /// Error category for programmatic handling
    pub category: CheckErrorCategory,
    /// Human-readable error message
    pub message: String,
    /// Optional context information
    pub context: Option<String>,
    /// Optional suggestions for fixing the error
    pub suggestion: Option<String>,
}

/// Base structure for check function results
#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    /// Whether the validation passed (true) or failed (false)
    pub ok: bool,
    /// Actual state information from the target system
    pub actual: HashMap<String, String>,
    /// Optional error message when validation cannot be performed
    pub error: Option<String>,
}

impl CheckResult {
    /// Create a successful check result
    pub const fn success(actual: HashMap<String, String>) -> Self {
        Self {
            ok: true,
            actual,
            error: None,
        }
    }

    /// Create a failed check result
    pub const fn failure(actual: HashMap<String, String>) -> Self {
        Self {
            ok: false,
            actual,
            error: None,
        }
    }

    /// Create an error check result from a simple error message
    pub fn error(error_message: String) -> Self {
        Self {
            ok: false,
            actual: HashMap::new(),
            error: Some(error_message),
        }
    }

    /// Create an error check result from a structured `CheckError`
    pub fn from_check_error(check_error: &CheckError) -> Self {
        let mut error_message = format!(
            "{}: {}",
            check_error.category.description(),
            check_error.message
        );

        if let Some(context) = &check_error.context {
            use std::fmt::Write;
            let _ = write!(error_message, " (Context: {context})");
        }

        if let Some(suggestion) = &check_error.suggestion {
            use std::fmt::Write;
            let _ = write!(error_message, " Suggestion: {suggestion}");
        }

        Self {
            ok: false,
            actual: HashMap::new(),
            error: Some(error_message),
        }
    }

    /// Create an error result for parameter validation failures
    pub fn parameter_error(message: &str, parameter: &str) -> Self {
        Self::from_check_error(&CheckError {
            category: CheckErrorCategory::ParameterValidation,
            message: message.to_string(),
            context: Some(format!("Parameter: {parameter}")),
            suggestion: Some("Check the parameter documentation for valid values".to_string()),
        })
    }

    /// Create an error result for SSH connection failures
    pub fn ssh_error(message: &str, host: Option<&str>) -> Self {
        let context = host.map(|h| format!("Host: {h}"));
        let suggestion =
            Some("Verify SSH connectivity, credentials, and host configuration".to_string());

        Self::from_check_error(&CheckError {
            category: CheckErrorCategory::SshConnection,
            message: message.to_string(),
            context,
            suggestion,
        })
    }

    /// Create an error result for permission failures
    pub fn permission_error(message: &str, resource: &str) -> Self {
        Self::from_check_error(&CheckError {
            category: CheckErrorCategory::Permission,
            message: message.to_string(),
            context: Some(format!("Resource: {resource}")),
            suggestion: Some(
                "Check user permissions or consider using privilege escalation".to_string(),
            ),
        })
    }

    /// Create an error result for command execution failures
    pub fn command_error(message: &str, command: &str, exit_code: i32) -> Self {
        Self::from_check_error(&CheckError {
            category: CheckErrorCategory::CommandExecution,
            message: message.to_string(),
            context: Some(format!("Command: {command}, Exit code: {exit_code}")),
            suggestion: Some("Check command syntax and system state".to_string()),
        })
    }

    /// Create an error result for system errors
    pub fn system_error(message: &str) -> Self {
        Self::from_check_error(&CheckError {
            category: CheckErrorCategory::System,
            message: message.to_string(),
            context: None,
            suggestion: Some("Check system resources and try again".to_string()),
        })
    }

    /// Convert `CheckResult` to a Lua table
    pub fn to_lua_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let table = lua.create_table()?;
        table.set("ok", self.ok)?;

        // Convert actual state to Lua table
        let actual_table = lua.create_table()?;
        for (key, value) in &self.actual {
            actual_table.set(key.as_str(), value.clone())?;
        }
        table.set("actual", actual_table)?;

        if let Some(error) = &self.error {
            table.set("error", error.clone())?;
        }

        Ok(table)
    }
}

/// Execution context for check functions
#[derive(Debug, Clone)]
pub enum ExecutionContext {
    /// Execute commands locally
    Local,
    /// Execute commands via SSH using the provided host configuration
    Remote(Table),
}

impl ExecutionContext {
    /// Create execution context from optional host table
    pub fn from_host_table(host_table: Option<&Table>) -> Self {
        host_table.map_or_else(|| Self::Local, |host| Self::Remote(host.clone()))
    }
}

/// Common parameter validation functions
pub mod validation {
    use super::{Context, Result};
    use mlua::Table;

    /// Validate that a required string parameter exists and is not empty
    pub fn validate_required_string(params: &Table, param_name: &str) -> Result<String> {
        let value = params
            .get::<String>(param_name)
            .with_context(|| format!("Parameter '{param_name}' is required"))?;

        if value.trim().is_empty() {
            anyhow::bail!("Parameter '{param_name}' cannot be empty");
        }

        Ok(value)
    }

    /// Validate an optional string parameter
    pub fn validate_optional_string(params: &Table, param_name: &str) -> Result<Option<String>> {
        match params.get::<Option<String>>(param_name)? {
            Some(value) if value.trim().is_empty() => Ok(None),
            other => Ok(other),
        }
    }

    /// Validate an optional boolean parameter
    pub fn validate_optional_bool(params: &Table, param_name: &str) -> Result<Option<bool>> {
        Ok(params.get::<Option<bool>>(param_name)?)
    }

    /// Validate file mode format (4-digit octal)
    pub fn validate_file_mode(mode: &str) -> Result<()> {
        if mode.len() != 4 || !mode.starts_with('0') {
            anyhow::bail!("File mode must be a 4-digit octal number (e.g., '0644')");
        }

        for c in mode.chars().skip(1) {
            if !('0'..='7').contains(&c) {
                anyhow::bail!("File mode must contain only octal digits (0-7)");
            }
        }

        Ok(())
    }
}

/// Result structure validation functions
pub mod result_validation {
    use super::{CheckResult, HashMap, Result};
    use std::collections::HashSet;

    /// Standard field names that should be used consistently across all check modules
    pub struct StandardFields;

    impl StandardFields {
        /// Fields that indicate existence or presence
        pub const EXISTS: &'static str = "exists";
        pub const INSTALLED: &'static str = "installed";

        /// Fields for file properties
        pub const MODE: &'static str = "mode";
        pub const OWNER: &'static str = "owner";
        pub const GROUP: &'static str = "group";

        /// Fields for service properties
        pub const STATE: &'static str = "state";
        pub const ENABLED: &'static str = "enabled";

        /// Fields for package properties
        pub const VERSION: &'static str = "version";

        /// Fields for command output
        pub const STDOUT: &'static str = "stdout";
        pub const STDERR: &'static str = "stderr";
        pub const EXIT_CODE: &'static str = "exit_code";
    }

    /// Validate that a `CheckResult` has the required structure
    pub fn validate_check_result_structure(result: &CheckResult) -> Result<()> {
        // Validate that ok field is present (it's always present in our struct)
        // This is guaranteed by the type system

        // Validate that actual field is present (it's always present in our struct)
        // This is guaranteed by the type system

        // Validate that if ok is false and there's no actual data, there should be an error
        if !result.ok && result.actual.is_empty() && result.error.is_none() {
            anyhow::bail!(
                "CheckResult with ok=false must have either actual data or an error message"
            );
        }

        Ok(())
    }

    /// Validate that field names in actual data follow standard conventions
    pub fn validate_field_naming(
        actual: &HashMap<String, String>,
        module_type: &str,
    ) -> Result<()> {
        let valid_fields = get_valid_fields_for_module(module_type);

        for field_name in actual.keys() {
            if !valid_fields.contains(field_name.as_str()) {
                anyhow::bail!(
                    "Invalid field name '{field_name}' for {module_type} module. Valid fields: {valid_fields:?}"
                );
            }
        }

        Ok(())
    }

    /// Get valid field names for a specific module type
    fn get_valid_fields_for_module(module_type: &str) -> HashSet<&'static str> {
        let mut fields = HashSet::new();

        // Common fields for all modules
        fields.insert(StandardFields::STDOUT);
        fields.insert(StandardFields::STDERR);
        fields.insert(StandardFields::EXIT_CODE);

        match module_type {
            "file" => {
                fields.insert(StandardFields::EXISTS);
                fields.insert(StandardFields::MODE);
                fields.insert(StandardFields::OWNER);
                fields.insert(StandardFields::GROUP);
            }
            "service" => {
                fields.insert(StandardFields::EXISTS);
                fields.insert(StandardFields::STATE);
                fields.insert(StandardFields::ENABLED);
            }
            "package" => {
                fields.insert(StandardFields::INSTALLED);
                fields.insert(StandardFields::VERSION);
            }
            _ => {
                // For unknown modules, allow any field names
                // This provides flexibility for future modules
                return HashSet::new();
            }
        }

        fields
    }

    /// Create a validated `CheckResult` with consistent field naming
    pub fn create_validated_result(
        ok: bool,
        actual: &HashMap<String, String>,
        error: Option<String>,
        module_type: &str,
    ) -> Result<CheckResult> {
        let result = error.map_or_else(
            || {
                if ok {
                    CheckResult::success(actual.clone())
                } else {
                    CheckResult::failure(actual.clone())
                }
            },
            CheckResult::error,
        );
        // Validate the result structure
        validate_check_result_structure(&result)?;

        // Validate field naming if we have actual data
        if !actual.is_empty() {
            validate_field_naming(actual, module_type)?;
        }

        Ok(result)
    }

    /// Helper to ensure consistent boolean field representation
    pub fn format_boolean_field(value: bool) -> String {
        if value {
            "true".to_string()
        } else {
            "false".to_string()
        }
    }

    /// Helper to ensure consistent existence field representation
    pub fn set_exists_field(actual: &mut HashMap<String, String>, exists: bool) {
        actual.insert(
            StandardFields::EXISTS.to_string(),
            format_boolean_field(exists),
        );
    }

    /// Helper to ensure consistent installed field representation
    pub fn set_installed_field(actual: &mut HashMap<String, String>, installed: bool) {
        actual.insert(
            StandardFields::INSTALLED.to_string(),
            format_boolean_field(installed),
        );
    }

    /// Helper to ensure consistent enabled field representation
    pub fn set_enabled_field(actual: &mut HashMap<String, String>, enabled: Option<bool>) {
        let value = match enabled {
            Some(true) => "true".to_string(),
            Some(false) => "false".to_string(),
            None => "unknown".to_string(),
        };
        actual.insert(StandardFields::ENABLED.to_string(), value);
    }

    /// Helper to set version field with proper handling of None values
    pub fn set_version_field(actual: &mut HashMap<String, String>, version: Option<String>) {
        let value = version.unwrap_or_else(|| "unknown".to_string());
        actual.insert(StandardFields::VERSION.to_string(), value);
    }
}

/// Common command execution functions
pub mod execution {
    use super::{CheckResult, Context, ExecutionContext, HashMap, Lua, Result};
    use crate::connection::{Connection, create_connection};
    use mlua::{Table, Value};

    /// Execute a command in the given execution context with comprehensive error handling
    pub fn execute_command(
        lua: &Lua,
        context: &ExecutionContext,
        command: &str,
    ) -> Result<(String, String, i32)> {
        match context {
            ExecutionContext::Local => execute_local_command(lua, command),
            ExecutionContext::Remote(host_table) => {
                execute_remote_command(lua, host_table, command)
            }
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

        let connection =
            create_connection(lua, &Value::Table(host_table.clone())).map_err(|e| {
                anyhow::anyhow!("Failed to create SSH connection to {host_address}: {e}")
            })?;

        execute_command_with_connection(&connection, command).with_context(|| {
            format!("Remote command execution failed on {host_address}: {command}")
        })
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
                        // For "not found" errors, return empty actual state rather than error
                        let mut actual = HashMap::new();
                        actual.insert("exists".to_string(), "false".to_string());
                        CheckResult::success(actual)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;
    use std::collections::HashMap;

    #[test]
    fn test_check_result_success() {
        let mut actual = HashMap::new();
        actual.insert("exists".to_string(), "true".to_string());
        actual.insert("mode".to_string(), "0644".to_string());

        let result = CheckResult::success(actual.clone());
        assert!(result.ok);
        assert_eq!(result.actual.len(), 2);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_check_result_failure() {
        let mut actual = HashMap::new();
        actual.insert("exists".to_string(), "false".to_string());

        let result = CheckResult::failure(actual.clone());
        assert!(!result.ok);
        assert_eq!(result.actual.len(), 1);
        assert!(result.error.is_none());
    }

    #[test]
    fn test_check_result_error() {
        let result = CheckResult::error("Connection failed".to_string());
        assert!(!result.ok);
        assert!(result.actual.is_empty());
        assert_eq!(result.error, Some("Connection failed".to_string()));
    }

    #[test]
    fn test_check_result_parameter_error() {
        let result = CheckResult::parameter_error("Invalid value", "mode");
        assert!(!result.ok);
        assert!(result.actual.is_empty());
        assert!(result.error.is_some());
        if let Some(error_msg) = result.error {
            assert!(error_msg.contains("Parameter validation failed"));
            assert!(error_msg.contains("Invalid value"));
            assert!(error_msg.contains("Parameter: mode"));
            assert!(error_msg.contains("Suggestion:"));
        }
    }

    #[test]
    fn test_check_result_ssh_error() {
        let result = CheckResult::ssh_error("Connection timeout", Some("example.com"));
        assert!(!result.ok);
        assert!(result.actual.is_empty());
        assert!(result.error.is_some());
        if let Some(error_msg) = result.error {
            assert!(error_msg.contains("SSH connection failed"));
            assert!(error_msg.contains("Connection timeout"));
            assert!(error_msg.contains("Host: example.com"));
            assert!(error_msg.contains("Suggestion:"));
        }
    }

    #[test]
    fn test_check_result_permission_error() {
        let result = CheckResult::permission_error("Access denied", "/etc/shadow");
        assert!(!result.ok);
        assert!(result.actual.is_empty());
        assert!(result.error.is_some());
        if let Some(error_msg) = result.error {
            assert!(error_msg.contains("Permission denied"));
            assert!(error_msg.contains("Access denied"));
            assert!(error_msg.contains("Resource: /etc/shadow"));
            assert!(error_msg.contains("privilege escalation"));
        }
    }

    #[test]
    fn test_check_result_command_error() {
        let result = CheckResult::command_error("Command not found", "nonexistent-cmd", 127);
        assert!(!result.ok);
        assert!(result.actual.is_empty());
        assert!(result.error.is_some());
        if let Some(error_msg) = result.error {
            assert!(error_msg.contains("Command execution failed"));
            assert!(error_msg.contains("Command not found"));
            assert!(error_msg.contains("Command: nonexistent-cmd"));
            assert!(error_msg.contains("Exit code: 127"));
        }
    }

    #[test]
    fn test_check_result_system_error() {
        let result = CheckResult::system_error("Out of memory");
        assert!(!result.ok);
        assert!(result.actual.is_empty());
        assert!(result.error.is_some());
        if let Some(error_msg) = result.error {
            assert!(error_msg.contains("System error"));
            assert!(error_msg.contains("Out of memory"));
            assert!(error_msg.contains("system resources"));
        }
    }

    #[test]
    fn test_check_result_to_lua_table() -> mlua::Result<()> {
        let lua = Lua::new();
        let mut actual = HashMap::new();
        actual.insert("exists".to_string(), "true".to_string());
        actual.insert("mode".to_string(), "0644".to_string());

        let result = CheckResult::success(actual);
        let table = result.to_lua_table(&lua)?;

        assert!(table.get::<bool>("ok")?);
        let actual_table = table.get::<Table>("actual")?;
        assert_eq!(actual_table.get::<String>("exists")?, "true");
        assert_eq!(actual_table.get::<String>("mode")?, "0644");

        Ok(())
    }

    #[test]
    fn test_check_result_to_lua_table_with_error() -> mlua::Result<()> {
        let lua = Lua::new();
        let result = CheckResult::error("Test error".to_string());
        let table = result.to_lua_table(&lua)?;

        assert!(!table.get::<bool>("ok")?);
        assert_eq!(table.get::<String>("error")?, "Test error");

        Ok(())
    }

    #[test]
    fn test_execution_context_from_host_table() -> mlua::Result<()> {
        let lua = Lua::new();
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;

        let context = ExecutionContext::from_host_table(Some(&host_table));
        matches!(context, ExecutionContext::Remote(_));

        let context = ExecutionContext::from_host_table(None);
        matches!(context, ExecutionContext::Local);

        Ok(())
    }

    #[test]
    fn test_validate_required_string() -> anyhow::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("test_param", "test_value")?;

        let result = validation::validate_required_string(&params, "test_param")?;
        assert_eq!(result, "test_value");

        // Test missing parameter
        let result = validation::validate_required_string(&params, "missing_param");
        assert!(result.is_err());

        // Test empty parameter
        params.set("empty_param", "")?;
        let result = validation::validate_required_string(&params, "empty_param");
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_validate_file_mode() -> anyhow::Result<()> {
        // Valid modes
        validation::validate_file_mode("0644")?;
        validation::validate_file_mode("0755")?;
        validation::validate_file_mode("0600")?;

        // Invalid modes
        assert!(validation::validate_file_mode("644").is_err());
        assert!(validation::validate_file_mode("0888").is_err());
        assert!(validation::validate_file_mode("abcd").is_err());

        Ok(())
    }

    #[test]
    fn test_execute_command_local_context() -> Result<()> {
        let lua = Lua::new();
        let context = ExecutionContext::Local;

        // Test a simple command that should work on any system
        let (stdout, _stderr, exit_code) =
            execution::execute_command(&lua, &context, "echo 'test'")?;

        assert_eq!(exit_code, 0);
        assert_eq!(stdout.trim(), "test");

        Ok(())
    }

    #[test]
    fn test_execute_command_remote_context_localhost() -> Result<()> {
        let lua = Lua::new();
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;
        host_table.set("connection", "local")?; // Force local even though we're using remote context

        let context = ExecutionContext::Remote(host_table);

        // Test a simple command that should work on any system
        let (stdout, _stderr, exit_code) =
            execution::execute_command(&lua, &context, "echo 'remote_test'")?;

        assert_eq!(exit_code, 0);
        assert_eq!(stdout.trim(), "remote_test");

        Ok(())
    }

    #[test]
    fn test_error_categorization() {
        // Test permission error detection
        assert!(execution::is_permission_error("Permission denied"));
        assert!(execution::is_permission_error("Access denied"));
        assert!(execution::is_permission_error("Operation not permitted"));
        assert!(!execution::is_permission_error("File not found"));

        // Test not found error detection
        assert!(execution::is_not_found_error("No such file or directory"));
        assert!(execution::is_not_found_error("not found"));
        assert!(execution::is_not_found_error("does not exist"));
        assert!(execution::is_not_found_error("no packages found"));
        assert!(!execution::is_not_found_error("Permission denied"));

        // Test SSH error detection
        assert!(execution::is_ssh_error("SSH connection failed"));
        assert!(execution::is_ssh_error("Connection timeout"));
        assert!(execution::is_ssh_error("Authentication failed"));
        assert!(!execution::is_ssh_error("File not found"));
    }

    #[test]
    fn test_execute_command_with_error_handling_success() {
        let lua = Lua::new();
        let context = ExecutionContext::Local;

        let result = execution::execute_command_with_error_handling(
            &lua,
            &context,
            "echo 'success'",
            "Test operation",
        );

        assert!(result.ok);
        if let Some(stdout) = result.actual.get("stdout") {
            assert!(stdout.contains("success"));
        }
        assert!(result.error.is_none());
    }

    #[test]
    fn test_execute_command_with_error_handling_not_found() {
        let lua = Lua::new();
        let context = ExecutionContext::Local;

        let result = execution::execute_command_with_error_handling(
            &lua,
            &context,
            "cat /nonexistent/file/that/does/not/exist",
            "File read check",
        );

        // Should return success with exists=false for not found errors
        assert!(result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));
        assert!(result.error.is_none());
    }

    #[test]
    fn test_result_validation_structure() -> Result<()> {
        use std::collections::HashMap;

        // Test valid result structure
        let mut actual = HashMap::new();
        actual.insert("exists".to_string(), "true".to_string());
        let result = CheckResult::success(actual);
        result_validation::validate_check_result_structure(&result)?;

        // Test invalid result structure (ok=false, no actual data, no error)
        let empty_actual = HashMap::new();
        let invalid_result = CheckResult {
            ok: false,
            actual: empty_actual,
            error: None,
        };
        assert!(result_validation::validate_check_result_structure(&invalid_result).is_err());

        Ok(())
    }

    #[test]
    fn test_field_naming_validation() -> Result<()> {
        use std::collections::HashMap;

        // Test valid field names for file module
        let mut file_actual = HashMap::new();
        file_actual.insert("exists".to_string(), "true".to_string());
        file_actual.insert("mode".to_string(), "0644".to_string());
        file_actual.insert("owner".to_string(), "root".to_string());
        result_validation::validate_field_naming(&file_actual, "file")?;

        // Test invalid field name for file module
        let mut invalid_actual = HashMap::new();
        invalid_actual.insert("invalid_field".to_string(), "value".to_string());
        assert!(result_validation::validate_field_naming(&invalid_actual, "file").is_err());

        // Test valid field names for service module
        let mut service_actual = HashMap::new();
        service_actual.insert("exists".to_string(), "true".to_string());
        service_actual.insert("state".to_string(), "active".to_string());
        service_actual.insert("enabled".to_string(), "true".to_string());
        result_validation::validate_field_naming(&service_actual, "service")?;

        // Test valid field names for package module
        let mut package_actual = HashMap::new();
        package_actual.insert("installed".to_string(), "true".to_string());
        package_actual.insert("version".to_string(), "1.0.0".to_string());
        result_validation::validate_field_naming(&package_actual, "package")?;

        Ok(())
    }

    #[test]
    fn test_standard_field_helpers() {
        use std::collections::HashMap;

        let mut actual = HashMap::new();

        // Test boolean field formatting
        assert_eq!(result_validation::format_boolean_field(true), "true");
        assert_eq!(result_validation::format_boolean_field(false), "false");

        // Test exists field helper
        result_validation::set_exists_field(&mut actual, true);
        assert_eq!(actual.get("exists"), Some(&"true".to_string()));

        // Test installed field helper
        result_validation::set_installed_field(&mut actual, false);
        assert_eq!(actual.get("installed"), Some(&"false".to_string()));

        // Test enabled field helper
        result_validation::set_enabled_field(&mut actual, Some(true));
        assert_eq!(actual.get("enabled"), Some(&"true".to_string()));

        result_validation::set_enabled_field(&mut actual, None);
        assert_eq!(actual.get("enabled"), Some(&"unknown".to_string()));

        // Test version field helper
        result_validation::set_version_field(&mut actual, Some("1.0.0".to_string()));
        assert_eq!(actual.get("version"), Some(&"1.0.0".to_string()));

        result_validation::set_version_field(&mut actual, None);
        assert_eq!(actual.get("version"), Some(&"unknown".to_string()));
    }

    #[test]
    fn test_create_validated_result() -> Result<()> {
        use std::collections::HashMap;

        // Test creating a valid success result
        let mut actual = HashMap::new();
        actual.insert("exists".to_string(), "true".to_string());
        actual.insert("mode".to_string(), "0644".to_string());

        let result = result_validation::create_validated_result(true, &actual, None, "file")?;
        assert!(result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("mode"), Some(&"0644".to_string()));

        // Test creating a valid failure result
        let mut actual = HashMap::new();
        actual.insert("exists".to_string(), "false".to_string());

        let result = result_validation::create_validated_result(false, &actual, None, "file")?;
        assert!(!result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));

        // Test creating an error result
        let actual = HashMap::new();
        let result = result_validation::create_validated_result(
            false,
            &actual,
            Some("Test error".to_string()),
            "file",
        )?;
        assert!(!result.ok);
        assert_eq!(result.error, Some("Test error".to_string()));

        Ok(())
    }
}
