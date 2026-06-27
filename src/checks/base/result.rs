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
