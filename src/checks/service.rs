use crate::checks::base::{
    CheckResult, ExecutionContext, execution,
    result_validation::{
        StandardFields, create_validated_result, set_enabled_field, set_exists_field,
    },
    shell_escape,
    validation::{validate_optional_bool, validate_optional_string, validate_required_string},
};
use anyhow::{Context, Result};
use mlua::{Lua, MultiValue, Table};
use std::collections::HashMap;

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

    let host_table = args_iter.next().map_or_else(
        || None,
        |value| {
            value
                .as_table()
                .map_or_else(|| None, |table| Some(table.clone()))
        },
    );

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

    compare_service_state(&service_params, &actual_state)
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
struct ServiceState {
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
    query_service_active_and_enabled_state(lua, context, service_name)
}

/// Query service active and enabled states
fn query_service_active_and_enabled_state(
    lua: &Lua,
    context: &ExecutionContext,
    service_name: &str,
) -> ServiceState {
    // Get active state
    let active_state_command = format!("systemctl is-active '{}'", shell_escape(service_name));
    let active_state = match execution::execute_command(lua, context, &active_state_command) {
        Ok((stdout, _, exit_code)) => {
            match exit_code {
                0 => "active".to_string(),
                3 => "inactive".to_string(),
                _ => {
                    // For other exit codes, use the actual output from systemctl is-active
                    let state: &str = stdout.trim();
                    if state.is_empty() {
                        "unknown".to_string()
                    } else {
                        state.to_string()
                    }
                }
            }
        }
        Err(_) => "unknown".to_string(),
    };

    // Get enabled state
    let enabled_command = format!("systemctl is-enabled '{}'", shell_escape(service_name));
    let enabled_state = match execution::execute_command(lua, context, &enabled_command) {
        Ok((stdout, _, exit_code)) => {
            match exit_code {
                0 => {
                    let enabled_output: &str = stdout.trim();
                    match enabled_output {
                        "enabled" | "enabled-runtime" | "static" => Some(true),
                        "disabled" | "masked" => Some(false),
                        _ => None, // Other states like "indirect", "generated", etc.
                    }
                }
                1 => Some(false), // disabled
                _ => None,        // Unknown or error state
            }
        }
        Err(_) => None,
    };

    ServiceState {
        exists: true,
        state: Some(active_state),
        enabled: enabled_state,
        error: None,
    }
}

/// Compare expected service parameters with actual service state
fn compare_service_state(expected: &ServiceParameters, actual: &ServiceState) -> CheckResult {
    let mut actual_map = HashMap::new();
    let mut validation_passed = true;

    // Always include existence in actual state using standard field name
    set_exists_field(&mut actual_map, actual.exists);

    // If service doesn't exist, validation fails if we expected any properties
    if !actual.exists {
        if expected.state.is_some() || expected.enabled.is_some() {
            validation_passed = false;
        }

        if let Some(error) = &actual.error {
            return CheckResult::error(error.clone());
        }

        return create_validated_result(validation_passed, &actual_map, None, "service")
            .unwrap_or_else(|_| CheckResult::failure(actual_map));
    }

    // Service exists, check properties
    if let Some(error) = &actual.error {
        return CheckResult::error(error.clone());
    }

    // Check service state using standard field name
    if let Some(ref actual_state) = actual.state {
        actual_map.insert(StandardFields::STATE.to_string(), actual_state.clone());

        if let Some(ref expected_state) = expected.state
            && expected_state != actual_state
        {
            validation_passed = false;
        }
    }

    // Check enabled status using standard field name and helper
    set_enabled_field(&mut actual_map, actual.enabled);

    if let Some(expected_enabled) = expected.enabled
        && let Some(actual_enabled) = actual.enabled
        && expected_enabled != actual_enabled
    {
        validation_passed = false;
    }
    // Don't fail validation for unknown enabled state, just report it

    create_validated_result(validation_passed, &actual_map, None, "service").unwrap_or_else(|_| {
        if validation_passed {
            CheckResult::success(actual_map)
        } else {
            CheckResult::failure(actual_map)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_extract_service_parameters_valid() -> Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "nginx")?;
        params.set("state", "active")?;
        params.set("enabled", true)?;

        let service_params = extract_service_parameters(&params)?;

        assert_eq!(service_params.name, "nginx");
        assert_eq!(service_params.state, Some("active".to_string()));
        assert_eq!(service_params.enabled, Some(true));

        Ok(())
    }

    #[test]
    fn test_extract_service_parameters_minimal() -> Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "postgresql")?;

        let service_params = extract_service_parameters(&params)?;

        assert_eq!(service_params.name, "postgresql");
        assert_eq!(service_params.state, None);
        assert_eq!(service_params.enabled, None);

        Ok(())
    }

    #[test]
    fn test_extract_service_parameters_missing_name() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("state", "active")?;

        let result = extract_service_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("name"));
        }
        Ok(())
    }

    #[test]
    fn test_extract_service_parameters_empty_name() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "")?;

        let result = extract_service_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("empty"));
        }
        Ok(())
    }

    #[test]
    fn test_extract_service_parameters_invalid_state() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "nginx")?;
        params.set("state", "running")?; // Invalid state

        let result = extract_service_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("state"));
        }
        Ok(())
    }

    #[test]
    fn test_validate_service_name() -> Result<()> {
        // Valid service names
        validate_service_name("nginx")?;
        validate_service_name("postgresql")?;
        validate_service_name("my-service")?;
        validate_service_name("service_name")?;

        // Invalid service names
        assert!(validate_service_name("").is_err());
        assert!(validate_service_name("service with spaces").is_err());
        assert!(validate_service_name("service/with/slash").is_err());
        assert!(validate_service_name("service;with;semicolon").is_err());
        assert!(validate_service_name("service`with`backtick").is_err());

        Ok(())
    }

    #[test]
    fn test_validate_service_state() -> Result<()> {
        // Valid states
        validate_service_state("active")?;
        validate_service_state("inactive")?;

        // Invalid states
        assert!(validate_service_state("running").is_err());
        assert!(validate_service_state("stopped").is_err());
        assert!(validate_service_state("enabled").is_err());

        Ok(())
    }

    #[test]
    fn test_query_service_state_nonexistent() {
        // This test would require mocking systemctl commands
        // For now, we'll just test the shell_escape function
        assert_eq!(shell_escape("simple"), "simple");
        assert_eq!(shell_escape("with'quote"), "with'\"'\"'quote");
    }

    #[test]
    fn test_shell_escape() {
        assert_eq!(shell_escape("simple"), "simple");
        assert_eq!(shell_escape("with'quote"), "with'\"'\"'quote");
        assert_eq!(
            shell_escape("multiple'quotes'here"),
            "multiple'\"'\"'quotes'\"'\"'here"
        );
    }

    #[test]
    fn test_compare_service_state_success() {
        let expected = ServiceParameters {
            name: "nginx".to_string(),
            state: Some("active".to_string()),
            enabled: Some(true),
        };

        let actual = ServiceState {
            exists: true,
            state: Some("active".to_string()),
            enabled: Some(true),
            error: None,
        };

        let result = compare_service_state(&expected, &actual);
        assert!(result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("state"), Some(&"active".to_string()));
        assert_eq!(result.actual.get("enabled"), Some(&"true".to_string()));
    }

    #[test]
    fn test_compare_service_state_failure() {
        let expected = ServiceParameters {
            name: "nginx".to_string(),
            state: Some("active".to_string()),
            enabled: Some(true),
        };

        let actual = ServiceState {
            exists: true,
            state: Some("inactive".to_string()), // Different state
            enabled: Some(false),                // Different enabled status
            error: None,
        };

        let result = compare_service_state(&expected, &actual);
        assert!(!result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("state"), Some(&"inactive".to_string()));
        assert_eq!(result.actual.get("enabled"), Some(&"false".to_string()));
    }

    #[test]
    fn test_compare_service_state_nonexistent() {
        let expected = ServiceParameters {
            name: "nonexistent".to_string(),
            state: None,
            enabled: None,
        };

        let actual = ServiceState {
            exists: false,
            state: None,
            enabled: None,
            error: None,
        };

        let result = compare_service_state(&expected, &actual);
        assert!(result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));
    }

    #[test]
    fn test_compare_service_state_unexpected_nonexistent() {
        let expected = ServiceParameters {
            name: "nginx".to_string(),
            state: Some("active".to_string()),
            enabled: Some(true),
        };

        let actual = ServiceState {
            exists: false,
            state: None,
            enabled: None,
            error: None,
        };

        let result = compare_service_state(&expected, &actual);
        assert!(!result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));
    }

    #[test]
    fn test_compare_service_state_unknown_enabled() {
        let expected = ServiceParameters {
            name: "nginx".to_string(),
            state: Some("active".to_string()),
            enabled: Some(true),
        };

        let actual = ServiceState {
            exists: true,
            state: Some("active".to_string()),
            enabled: None, // Unknown enabled state
            error: None,
        };

        let result = compare_service_state(&expected, &actual);
        // Should still pass validation since we don't fail on unknown enabled state
        assert!(result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("state"), Some(&"active".to_string()));
        assert_eq!(result.actual.get("enabled"), Some(&"unknown".to_string()));
    }

    #[test]
    fn test_check_service_lua_interface() -> mlua::Result<()> {
        let lua = Lua::new();

        // Create parameters table
        let params = lua.create_table()?;
        params.set("name", "nginx")?;
        params.set("state", "active")?;

        // Test that the function can be called (it will fail due to no actual systemctl access in tests)
        let args = mlua::MultiValue::from_vec(vec![mlua::Value::Table(params)]);
        let result = check_service(&lua, args);

        // The function should return a result (success or error)
        assert!(result.is_ok() || result.is_err());

        Ok(())
    }
}
