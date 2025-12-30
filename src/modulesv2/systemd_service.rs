//! # `ModulesV2` Systemd Service Management Module
//!
//! The `systemd_service` module provides service management functionality for `ModulesV2`.
//! It supports both local and remote execution with automatic connection management.
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution - start service
//! local result = k.mod.systemd_service({name = "nginx", action = "start"})
//!
//! -- Remote execution - restart service
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.systemd_service({name = "nginx", action = "restart"}, host)
//!
//! -- Enable service with daemon reload
//! local result = k.mod.systemd_service({
//!     name = "myapp",
//!     action = "enable",
//!     daemon_reload = true
//! })
//! ```
//!
//! ## Parameters
//!
//! - `name` (string, required): Service name to manage
//! - `action` (string, optional): Action to perform - "start", "stop", "restart", "reload", "enable", "disable" (default: "start")
//! - `daemon_reload` (boolean, optional): Whether to reload systemd daemon before action (default: false)
//! - `force` (boolean, optional): Whether to force the action (default: false)
//!
//! ## Return Value
//!
//! Returns a table with:
//! - `stdout`: Command output
//! - `stderr`: Error output
//! - `exit_code`: Exit code (0 for success)
//! - `changed`: Boolean indicating if service state was modified

use super::execution::{ExecutionEngine, ModuleResult};
use super::factory::create_modulev2_function;
use crate::connection::Connection;
use mlua::{Lua, Table};

/// Create the `systemd_service_v2` function for `ModulesV2`
///
/// This function creates a ModulesV2-compatible systemd service management module that supports
/// both local and remote execution patterns.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - The configured `systemd_service_v2` function
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - Service management operations fail
pub fn systemd_service_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
    create_modulev2_function(lua, "systemd_service", |lua, params, host| {
        ExecutionEngine::execute_module(
            lua,
            "systemd_service",
            &params,
            host,
            |connection, params| {
                // Extract and validate parameters
                let service_name = extract_service_name(params)?;
                let action = params
                    .get::<Option<String>>("action")?
                    .unwrap_or_else(|| "start".to_string());
                let daemon_reload = params
                    .get::<Option<bool>>("daemon_reload")?
                    .unwrap_or(false);
                let force = params.get::<Option<bool>>("force")?.unwrap_or(false);

                // Validate action parameter
                validate_action(&action)?;

                // Sanitize service name
                let sanitized_service_name = sanitize_service_name(&service_name)?;

                // Execute systemd service operations
                execute_systemd_operations(
                    connection,
                    &sanitized_service_name,
                    &action,
                    daemon_reload,
                    force,
                )
            },
        )
    })
}

/// Extract and validate the service name parameter
fn extract_service_name(params: &Table) -> mlua::Result<String> {
    params.get::<Option<String>>("name")?.map_or_else(
        || {
            Err(mlua::Error::RuntimeError(
                "systemd_service module requires 'name' parameter".to_string(),
            ))
        },
        |name| {
            if name.trim().is_empty() {
                Err(mlua::Error::RuntimeError(
                    "Service name cannot be empty".to_string(),
                ))
            } else {
                Ok(name)
            }
        },
    )
}

/// Validate the action parameter
fn validate_action(action: &str) -> mlua::Result<()> {
    match action {
        "start" | "stop" | "restart" | "reload" | "enable" | "disable" | "status" => Ok(()),
        _ => Err(mlua::Error::RuntimeError(format!(
            "Invalid action: {action}. Valid actions are: start, stop, restart, reload, enable, disable, status"
        ))),
    }
}

/// Sanitize service name to prevent injection attacks
fn sanitize_service_name(service_name: &str) -> mlua::Result<String> {
    if service_name.trim().is_empty() {
        return Err(mlua::Error::RuntimeError(
            "Service name cannot be empty".to_string(),
        ));
    }

    // Allow alphanumeric, -, _, ., @ (common in service names)
    let sanitized: String = service_name
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | '@'))
        .collect();

    if sanitized.is_empty() {
        return Err(mlua::Error::RuntimeError(format!(
            "Service name '{service_name}' contains only invalid characters"
        )));
    }

    Ok(sanitized)
}

/// Execute systemd service operations based on parameters
fn execute_systemd_operations(
    connection: &mut Connection,
    service_name: &str,
    action: &str,
    daemon_reload: bool,
    force: bool,
) -> mlua::Result<ModuleResult> {
    let mut stdout_parts = Vec::new();
    let mut stderr_parts = Vec::new();
    // Reload systemd daemon if requested
    let mut _changed = if daemon_reload {
        let (reload_stdout, reload_stderr, reload_exit_code) = connection
            .cmd("systemctl --no-ask-password daemon-reload")
            .map_err(|e| mlua::Error::RuntimeError(format!("Daemon reload failed: {e}")))?;

        stdout_parts.push(reload_stdout);
        if !reload_stderr.is_empty() {
            stderr_parts.push(reload_stderr);
        }

        if reload_exit_code != 0 {
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                reload_exit_code,
            ));
        }

        // Daemon reload always indicates change
        true
    } else {
        false
    };

    // Execute service action
    match action {
        "start" => {
            let result = start_service(connection, service_name)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0 && result.changed {
                _changed = true;
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                ));
            }
        }
        "stop" => {
            let result = stop_service(connection, service_name)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0 && result.changed {
                _changed = true;
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                ));
            }
        }
        "restart" => {
            let result = restart_service(connection, service_name)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0 {
                _changed = true; // Restart always indicates change
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                ));
            }
        }
        "reload" => {
            let result = reload_service(connection, service_name)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0 {
                _changed = true; // Reload always indicates change
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                ));
            }
        }
        "enable" => {
            let result = enable_service(connection, service_name, force)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0 && result.changed {
                _changed = true;
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                ));
            }
        }
        "disable" => {
            let result = disable_service(connection, service_name, force)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0 && result.changed {
                _changed = true;
            }
            if result.exit_code != 0 {
                return Ok(ModuleResult::complete(
                    stdout_parts.join("\n"),
                    stderr_parts.join("\n"),
                    result.exit_code,
                ));
            }
        }
        "status" => {
            let result = status_service(connection, service_name)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            // Status never changes anything, just return the exit code
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                result.exit_code,
            ));
        }
        _ => unreachable!("Action validation should prevent this"),
    }

    Ok(ModuleResult::complete(
        stdout_parts.join("\n"),
        stderr_parts.join("\n"),
        0, // Always return 0 for successful operations
    ))
}

/// Start a service
fn start_service(connection: &mut Connection, service_name: &str) -> mlua::Result<ServiceResult> {
    // Check if service is already active
    let (_, _, is_active_exit_code) = connection
        .cmd(&format!(
            "systemctl --no-ask-password is-active {service_name}"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service status check failed: {e}")))?;

    if is_active_exit_code == 0 {
        return Ok(ServiceResult {
            stdout: format!("Service {service_name} is already active"),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
        });
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("systemctl --no-ask-password start {service_name}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service start failed: {e}")))?;

    Ok(ServiceResult {
        stdout,
        stderr,
        exit_code,
        changed: exit_code == 0,
    })
}

/// Stop a service
fn stop_service(connection: &mut Connection, service_name: &str) -> mlua::Result<ServiceResult> {
    // Check if service is already inactive
    let (_, _, is_active_exit_code) = connection
        .cmd(&format!(
            "systemctl --no-ask-password is-active {service_name}"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service status check failed: {e}")))?;

    if is_active_exit_code != 0 {
        return Ok(ServiceResult {
            stdout: format!("Service {service_name} is already inactive"),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
        });
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("systemctl --no-ask-password stop {service_name}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service stop failed: {e}")))?;

    Ok(ServiceResult {
        stdout,
        stderr,
        exit_code,
        changed: exit_code == 0,
    })
}

/// Restart a service
fn restart_service(connection: &mut Connection, service_name: &str) -> mlua::Result<ServiceResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!(
            "systemctl --no-ask-password restart {service_name}"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service restart failed: {e}")))?;

    Ok(ServiceResult {
        stdout,
        stderr,
        exit_code,
        changed: exit_code == 0, // Restart always indicates change if successful
    })
}

/// Reload a service
fn reload_service(connection: &mut Connection, service_name: &str) -> mlua::Result<ServiceResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!(
            "systemctl --no-ask-password reload {service_name}"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service reload failed: {e}")))?;

    Ok(ServiceResult {
        stdout,
        stderr,
        exit_code,
        changed: exit_code == 0, // Reload always indicates change if successful
    })
}

/// Enable a service
fn enable_service(
    connection: &mut Connection,
    service_name: &str,
    force: bool,
) -> mlua::Result<ServiceResult> {
    // Check if service is already enabled
    let (_, _, is_enabled_exit_code) = connection
        .cmd(&format!(
            "systemctl --no-ask-password is-enabled {service_name}"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service enabled check failed: {e}")))?;

    if is_enabled_exit_code == 0 {
        return Ok(ServiceResult {
            stdout: format!("Service {service_name} is already enabled"),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
        });
    }

    let mut cmd = format!("systemctl --no-ask-password enable {service_name}");
    if force {
        cmd.push_str(" --force");
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&cmd)
        .map_err(|e| mlua::Error::RuntimeError(format!("Service enable failed: {e}")))?;

    Ok(ServiceResult {
        stdout,
        stderr,
        exit_code,
        changed: exit_code == 0,
    })
}

/// Disable a service
fn disable_service(
    connection: &mut Connection,
    service_name: &str,
    force: bool,
) -> mlua::Result<ServiceResult> {
    // Check if service is already disabled
    let (_, _, is_enabled_exit_code) = connection
        .cmd(&format!(
            "systemctl --no-ask-password is-enabled {service_name}"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service enabled check failed: {e}")))?;

    if is_enabled_exit_code != 0 {
        return Ok(ServiceResult {
            stdout: format!("Service {service_name} is already disabled"),
            stderr: String::new(),
            exit_code: 0,
            changed: false,
        });
    }

    let mut cmd = format!("systemctl --no-ask-password disable {service_name}");
    if force {
        cmd.push_str(" --force");
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&cmd)
        .map_err(|e| mlua::Error::RuntimeError(format!("Service disable failed: {e}")))?;

    Ok(ServiceResult {
        stdout,
        stderr,
        exit_code,
        changed: exit_code == 0,
    })
}

/// Get service status
fn status_service(connection: &mut Connection, service_name: &str) -> mlua::Result<ServiceResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd(&format!(
            "systemctl --no-ask-password status {service_name}"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Service status check failed: {e}")))?;

    Ok(ServiceResult {
        stdout,
        stderr,
        exit_code,
        changed: false, // Status never changes anything
    })
}

/// Service operation result
#[derive(Debug)]
struct ServiceResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
    changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_systemd_service_v2_local_execution() -> mlua::Result<()> {
        let lua = create_lua()?;
        let service_fn = systemd_service_v2(&lua)?;

        // Test with status action which should not require privileges
        let params = lua.create_table()?;
        params.set("name", "nonexistent-test-service")?;
        params.set("action", "status")?;

        let result: Table = service_fn.call(params)?;

        // Verify the result structure is correct
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        // Status action should never report changed=true
        assert!(!result.get::<bool>("changed")?);

        Ok(())
    }

    #[test]
    fn test_systemd_service_v2_missing_name() -> mlua::Result<()> {
        let lua = create_lua()?;
        let service_fn = systemd_service_v2(&lua)?;

        // Test with missing name parameter
        let params = lua.create_table()?;
        params.set("action", "start")?;

        let result: mlua::Result<Table> = service_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("systemd_service module requires 'name' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("systemd_service module requires 'name' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_systemd_service_v2_invalid_action() -> mlua::Result<()> {
        let lua = create_lua()?;
        let service_fn = systemd_service_v2(&lua)?;

        // Test with invalid action
        let params = lua.create_table()?;
        params.set("name", "test-service")?;
        params.set("action", "invalid_action")?;

        let result: mlua::Result<Table> = service_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("Invalid action: invalid_action"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("Invalid action: invalid_action"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_systemd_service_v2_default_action() -> mlua::Result<()> {
        let lua = create_lua()?;
        let service_fn = systemd_service_v2(&lua)?;

        // Test default action (should be "start") with a non-existent service
        let params = lua.create_table()?;
        params.set("name", "nonexistent-test-service")?;
        // No action specified - should default to "start"

        let result: Table = service_fn.call(params)?;

        // Should return proper structure even if service doesn't exist
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_systemd_service_v2_with_daemon_reload() -> mlua::Result<()> {
        let lua = create_lua()?;
        let service_fn = systemd_service_v2(&lua)?;

        // Test daemon_reload parameter with a non-existent service
        let params = lua.create_table()?;
        params.set("name", "nonexistent-test-service")?;
        params.set("action", "enable")?;
        params.set("daemon_reload", true)?;

        let result: Table = service_fn.call(params)?;

        // Should return proper structure even if service doesn't exist
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_systemd_service_v2_with_force() -> mlua::Result<()> {
        let lua = create_lua()?;
        let service_fn = systemd_service_v2(&lua)?;

        // Test force parameter with a non-existent service
        let params = lua.create_table()?;
        params.set("name", "nonexistent-test-service")?;
        params.set("action", "enable")?;
        params.set("force", true)?;

        let result: Table = service_fn.call(params)?;

        // Should return proper structure even if service doesn't exist
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_sanitize_service_name() -> mlua::Result<()> {
        // Test valid service names
        assert_eq!(sanitize_service_name("nginx")?, "nginx");
        assert_eq!(sanitize_service_name("my-service")?, "my-service");
        assert_eq!(sanitize_service_name("app_service")?, "app_service");
        assert_eq!(sanitize_service_name("service.socket")?, "service.socket");
        assert_eq!(
            sanitize_service_name("user@1000.service")?,
            "user@1000.service"
        );

        // Test service name with invalid characters
        assert_eq!(sanitize_service_name("nginx;rm -rf /")?, "nginxrm-rf");

        // Test empty service name
        assert!(sanitize_service_name("").is_err());
        assert!(sanitize_service_name("   ").is_err());

        // Test service name with only invalid characters
        assert!(sanitize_service_name(";|&").is_err());

        Ok(())
    }

    #[test]
    fn test_validate_action() {
        // Test valid actions
        assert!(validate_action("start").is_ok());
        assert!(validate_action("stop").is_ok());
        assert!(validate_action("restart").is_ok());
        assert!(validate_action("reload").is_ok());
        assert!(validate_action("enable").is_ok());
        assert!(validate_action("disable").is_ok());
        assert!(validate_action("status").is_ok());

        // Test invalid action
        assert!(validate_action("invalid").is_err());
    }

    #[test]
    fn test_extract_service_name() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test valid service name
        let params = lua.create_table()?;
        params.set("name", "nginx")?;
        assert_eq!(extract_service_name(&params)?, "nginx");

        // Test empty service name
        let params = lua.create_table()?;
        params.set("name", "")?;
        assert!(extract_service_name(&params).is_err());

        // Test missing service name
        let params = lua.create_table()?;
        assert!(extract_service_name(&params).is_err());

        Ok(())
    }
}
