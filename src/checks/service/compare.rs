use crate::checks::base::{
    CheckResult, ExecutionContext, execution,
    result_validation::{
        StandardFields, create_validated_result, set_enabled_field, set_exists_field,
    },
    shell_escape,
};
use mlua::Lua;
use std::collections::HashMap;

use super::{ServiceParameters, ServiceState};

/// Query service active and enabled states
pub(super) fn query_service_active_and_enabled_state(
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
pub(super) fn compare_service_state(
    expected: &ServiceParameters,
    actual: &ServiceState,
) -> CheckResult {
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
            return create_validated_result(false, &actual_map, Some(error.clone()), "service")
                .unwrap_or_else(|_| CheckResult::failure(actual_map));
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

    match (expected.enabled, actual.enabled) {
        // Explicit expectation cannot be verified because the actual enabled
        // state is unknown — that is a validation failure, not a silent pass.
        (Some(_), None) => validation_passed = false,
        // Explicit expectation unmet.
        (Some(expected_enabled), Some(actual_enabled)) if expected_enabled != actual_enabled => {
            validation_passed = false;
        }
        // All other cases (no explicit expectation, or expectation matches) pass.
        _ => {}
    }

    create_validated_result(validation_passed, &actual_map, None, "service").unwrap_or_else(|_| {
        if validation_passed {
            CheckResult::success(actual_map)
        } else {
            CheckResult::failure(actual_map)
        }
    })
}
