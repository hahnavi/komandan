use crate::checks::base::{
    CheckResult, ExecutionContext, execution, shell_escape,
    validation::{validate_optional_bool, validate_optional_string, validate_required_string},
};
use anyhow::{Context, Result};
use mlua::{Lua, MultiValue, Table};

mod compare;

#[cfg(test)]
mod tests;

/// Check service function for validating systemd service states
///
/// This function validates systemd service states (active/inactive, enabled/disabled) without
/// modifying the service. It supports both local and remote execution via SSH.
///
/// # Parameters
/// - `name` (required): Service name to validate
/// - `state` (optional): Expected service state ("active" or "inactive")
/// - `enabled` (optional): Expected enabled status (true/false)
///
/// # Returns
/// A Lua table with:
/// - `ok`: Boolean indicating validation success/failure
/// - `actual`: Table with current service properties
/// - `error`: Optional error message
///
/// # Examples
/// ```lua
/// -- Local service check
/// local result = komandan.check.service({
///     name = "nginx",
///     state = "active",
///     enabled = true
/// })
///
/// -- Remote service check
/// local result = komandan.check.service({
///     name = "postgresql",
///     state = "active"
/// }, host_table)
/// ```
pub fn check_service(lua: &Lua, args: MultiValue) -> mlua::Result<Table> {
    // Parse arguments: (params, optional_host_table)
    let mut args_iter = args.into_iter();
    let params = args_iter
        .next()
        .ok_or_else(|| mlua::Error::RuntimeError("Missing parameters".to_string()))?
        .as_table()
        .ok_or_else(|| mlua::Error::RuntimeError("Parameters must be a table".to_string()))?
        .clone();

    let host_table = match args_iter.next() {
        None | Some(mlua::Value::Nil) => None,
        Some(mlua::Value::Table(table)) => Some(table),
        Some(other) => {
            return Err(mlua::Error::RuntimeError(format!(
                "optional host argument must be a table, got {}",
                other.type_name()
            )));
        }
    };

    // Execute the service validation
    let result = execute_service_validation(lua, &params, host_table.as_ref());

    // Convert result to Lua table
    result.to_lua_table(lua)
}

/// Execute service validation logic
fn execute_service_validation(
    lua: &Lua,
    params: &Table,
    host_table: Option<&Table>,
) -> CheckResult {
    // 1. Extract and validate parameters
    let service_params = match extract_service_parameters(params) {
        Ok(params) => params,
        Err(e) => {
            return CheckResult::parameter_error(&e.to_string(), "service parameters");
        }
    };

    // 2. Determine execution context
    let context = ExecutionContext::from_host_table(host_table);

    // 3. Query actual service state
    let actual_state = query_service_state(lua, &context, &service_params.name);

    compare::compare_service_state(&service_params, &actual_state)
}

/// Service validation parameters
#[derive(Debug, Clone)]
struct ServiceParameters {
    name: String,
    state: Option<String>,
    enabled: Option<bool>,
}

/// Extract and validate service parameters from Lua table
fn extract_service_parameters(params: &Table) -> Result<ServiceParameters> {
    // Required parameter: name
    let name = validate_required_string(params, "name")?;

    // Optional parameters
    let state = validate_optional_string(params, "state")?;
    let enabled = validate_optional_bool(params, "enabled")?;

    // Validate service name format
    validate_service_name(&name).with_context(|| format!("Invalid service name: {name}"))?;

    // Validate state value if provided
    if let Some(ref state_str) = state {
        validate_service_state(state_str)
            .with_context(|| format!("Invalid service state: {state_str}"))?;
    }

    Ok(ServiceParameters {
        name,
        state,
        enabled,
    })
}

/// Validate service name format
fn validate_service_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("Service name cannot be empty");
    }

    // Service names should not contain spaces or special characters that could cause issues
    if name.contains(' ') {
        anyhow::bail!("Service name cannot contain spaces");
    }

    // Check for potentially dangerous characters
    if name.contains([
        '/', '\\', '|', '&', ';', '`', '$', '(', ')', '<', '>', '"', '\'',
    ]) {
        anyhow::bail!("Service name contains invalid characters");
    }

    Ok(())
}

/// Validate service state value
fn validate_service_state(state: &str) -> Result<()> {
    match state {
        "active" | "inactive" => Ok(()),
        _ => anyhow::bail!("Service state must be 'active' or 'inactive'"),
    }
}

/// Actual service state information
#[derive(Debug, Clone)]
pub struct ServiceState {
    exists: bool,
    state: Option<String>,
    enabled: Option<bool>,
    error: Option<String>,
}

/// Query actual service state using systemctl commands
fn query_service_state(lua: &Lua, context: &ExecutionContext, service_name: &str) -> ServiceState {
    // First check if service exists by checking its load state
    let load_state_command = format!(
        "systemctl show '{}' --property=LoadState --value",
        shell_escape(service_name)
    );

    let result = execution::execute_command_with_error_handling(
        lua,
        context,
        &load_state_command,
        "Service load state check",
    );

    if result.error.is_some() {
        return ServiceState {
            exists: false,
            state: None,
            enabled: None,
            error: result.error,
        };
    }

    if !result.ok {
        return ServiceState {
            exists: false,
            state: None,
            enabled: None,
            error: result.error,
        };
    }

    let load_state = result
        .actual
        .get("stdout")
        .map_or("", |s: &String| s.trim());

    if load_state == "not-found" {
        return ServiceState {
            exists: false,
            state: None,
            enabled: None,
            error: None,
        };
    }

    // Service exists, get its active state
    compare::query_service_active_and_enabled_state(lua, context, service_name)
}
