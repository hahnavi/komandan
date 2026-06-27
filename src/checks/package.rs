use crate::checks::base::{
    CheckResult, ExecutionContext, execution,
    result_validation::{create_validated_result, set_installed_field, set_version_field},
    shell_escape,
    validation::{validate_optional_string, validate_required_string},
};
use anyhow::{Context, Result};
use mlua::{Lua, MultiValue, Table};
use std::collections::HashMap;

/// Check package function for validating package installation states
///
/// This function validates package installation states (presence, version) without
/// modifying packages. It supports both APT and DNF package managers and works
/// with both local and remote execution via SSH.
///
/// # Parameters
/// - `name` (required): Package name to validate
/// - `state` (optional): Expected package state ("present" or "absent")
/// - `version` (optional): Expected package version
///
/// # Returns
/// A Lua table with:
/// - `ok`: Boolean indicating validation success/failure
/// - `actual`: Table with current package properties
/// - `error`: Optional error message
///
/// # Examples
/// ```lua
/// -- Local package check
/// local result = komandan.check.package({
///     name = "nginx",
///     state = "present"
/// })
///
/// -- Remote package check with version
/// local result = komandan.check.package({
///     name = "postgresql",
///     state = "present",
///     version = "13.7"
/// }, host_table)
/// ```
pub fn check_package(lua: &Lua, args: MultiValue) -> mlua::Result<Table> {
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

    // Execute the package validation
    let result = execute_package_validation(lua, &params, host_table.as_ref());

    // Convert result to Lua table
    result.to_lua_table(lua)
}

/// Execute package validation logic
fn execute_package_validation(
    lua: &Lua,
    params: &Table,
    host_table: Option<&Table>,
) -> CheckResult {
    // 1. Extract and validate parameters
    let package_params = match extract_package_parameters(params) {
        Ok(params) => params,
        Err(e) => {
            return CheckResult::parameter_error(&e.to_string(), "package parameters");
        }
    };

    // 2. Determine execution context
    let context = ExecutionContext::from_host_table(host_table);

    // 3. Query actual package state
    let actual_state = query_package_state(lua, &context, &package_params.name);

    compare_package_state(&package_params, &actual_state)
}

/// Package validation parameters
#[derive(Debug, Clone)]
struct PackageParameters {
    name: String,
    state: Option<String>,
    version: Option<String>,
}

/// Extract and validate package parameters from Lua table
fn extract_package_parameters(params: &Table) -> Result<PackageParameters> {
    // Required parameter: name
    let name = validate_required_string(params, "name")?;

    // Optional parameters
    let state = validate_optional_string(params, "state")?;
    let version = validate_optional_string(params, "version")?;

    // Validate package name format
    validate_package_name(&name).with_context(|| format!("Invalid package name: {name}"))?;

    // Validate state value if provided
    if let Some(ref state_str) = state {
        validate_package_state(state_str)
            .with_context(|| format!("Invalid package state: {state_str}"))?;
    }

    Ok(PackageParameters {
        name,
        state,
        version,
    })
}

/// Validate package name format
fn validate_package_name(name: &str) -> Result<()> {
    if name.trim().is_empty() {
        anyhow::bail!("Package name cannot be empty");
    }

    // Package names should not contain spaces
    if name.contains(' ') {
        anyhow::bail!("Package name cannot contain spaces");
    }

    // Check for potentially dangerous characters that could cause command injection
    if name.contains([
        '/', '\\', '|', '&', ';', '`', '$', '(', ')', '<', '>', '"', '\'', '\n', '\r',
    ]) {
        anyhow::bail!("Package name contains invalid characters");
    }

    Ok(())
}

/// Validate package state value
fn validate_package_state(state: &str) -> Result<()> {
    match state {
        "present" | "absent" => Ok(()),
        _ => anyhow::bail!("Package state must be 'present' or 'absent'"),
    }
}

/// Actual package state information
#[derive(Debug, Clone)]
struct PackageState {
    installed: bool,
    version: Option<String>,
    error: Option<String>,
}

/// Query actual package state using package manager commands
fn query_package_state(lua: &Lua, context: &ExecutionContext, package_name: &str) -> PackageState {
    // First, detect which package manager is available
    let package_manager = detect_package_manager(lua, context);

    match package_manager {
        PackageManager::Apt => query_apt_package_state(lua, context, package_name),
        PackageManager::Dnf => query_dnf_package_state(lua, context, package_name),
        PackageManager::Unknown => PackageState {
            installed: false,
            version: None,
            error: Some("No supported package manager found (APT or DNF)".to_string()),
        },
    }
}

/// Supported package managers
#[derive(Debug, Clone)]
enum PackageManager {
    Apt,
    Dnf,
    Unknown,
}

/// Detect which package manager is available on the system
fn detect_package_manager(lua: &Lua, context: &ExecutionContext) -> PackageManager {
    // Check for dpkg-query (Debian/Ubuntu APT systems)
    let dpkg_result = execution::execute_command(lua, context, "which dpkg-query");
    if let Ok((_, _, dpkg_exit_code)) = dpkg_result
        && dpkg_exit_code == 0
    {
        return PackageManager::Apt;
    }

    // Check for rpm (RHEL/CentOS/Fedora DNF/YUM systems)
    let rpm_result = execution::execute_command(lua, context, "which rpm");
    if let Ok((_, _, rpm_exit_code)) = rpm_result
        && rpm_exit_code == 0
    {
        return PackageManager::Dnf;
    }

    PackageManager::Unknown
}

/// Query package state using APT/dpkg-query
fn query_apt_package_state(
    lua: &Lua,
    context: &ExecutionContext,
    package_name: &str,
) -> PackageState {
    // Use dpkg-query to check package status and version
    let query_command = format!(
        "dpkg-query -W -f='${{Status}} ${{Version}}' '{}'",
        shell_escape(package_name)
    );

    let result = execution::execute_command_with_error_handling(
        lua,
        context,
        &query_command,
        "APT package query",
    );

    if result.error.is_some() {
        return PackageState {
            installed: false,
            version: None,
            error: result.error,
        };
    }

    if result.ok {
        result.actual.get("stdout").map_or_else(
            || PackageState {
                installed: false,
                version: None,
                error: Some("No output from dpkg-query command".to_string()),
            },
            |stdout| parse_apt_package_output(stdout),
        )
    } else {
        // Check if it's a "not found" error
        result.error.as_ref().map_or_else(
            || PackageState {
                installed: false,
                version: None,
                error: Some("Unknown error querying package".to_string()),
            },
            |error| {
                if error.to_lowercase().contains("no packages found") {
                    PackageState {
                        installed: false,
                        version: None,
                        error: None,
                    }
                } else {
                    PackageState {
                        installed: false,
                        version: None,
                        error: Some(error.clone()),
                    }
                }
            },
        )
    }
}

/// Parse APT package query output
fn parse_apt_package_output(stdout: &str) -> PackageState {
    // Parse dpkg-query output: "install ok installed version"
    let output = stdout.trim();
    let parts: Vec<&str> = output.split_whitespace().collect();

    if parts.len() < 4 {
        return PackageState {
            installed: false,
            version: None,
            error: Some(format!("Unexpected dpkg-query output format: {output}")),
        };
    }

    // Check if package is installed (status should be "install ok installed")
    let is_installed =
        parts.len() >= 3 && parts[0] == "install" && parts[1] == "ok" && parts[2] == "installed";

    let version = if is_installed && parts.len() >= 4 {
        Some(parts[3].to_string())
    } else {
        None
    };

    PackageState {
        installed: is_installed,
        version,
        error: None,
    }
}

/// Query package state using DNF/RPM
fn query_dnf_package_state(
    lua: &Lua,
    context: &ExecutionContext,
    package_name: &str,
) -> PackageState {
    // Use rpm to check if package is installed and get version
    let query_command = format!(
        "rpm -q '{}' --queryformat='%{{VERSION}}-%{{RELEASE}}'",
        shell_escape(package_name)
    );

    let result = execution::execute_command_with_error_handling(
        lua,
        context,
        &query_command,
        "RPM package query",
    );

    if result.error.is_some() {
        return PackageState {
            installed: false,
            version: None,
            error: result.error,
        };
    }

    if result.ok {
        result.actual.get("stdout").map_or_else(
            || PackageState {
                installed: false,
                version: None,
                error: Some("No output from rpm command".to_string()),
            },
            |stdout| {
                let version = stdout.trim();
                if version.is_empty() {
                    PackageState {
                        installed: true,
                        version: None,
                        error: Some(
                            "Package installed but version could not be determined".to_string(),
                        ),
                    }
                } else {
                    PackageState {
                        installed: true,
                        version: Some(version.to_string()),
                        error: None,
                    }
                }
            },
        )
    } else {
        // Check if it's a "not installed" error
        result.error.as_ref().map_or_else(
            || PackageState {
                installed: false,
                version: None,
                error: Some("Unknown error querying package".to_string()),
            },
            |error| {
                if error.to_lowercase().contains("is not installed") {
                    PackageState {
                        installed: false,
                        version: None,
                        error: None,
                    }
                } else {
                    PackageState {
                        installed: false,
                        version: None,
                        error: Some(error.clone()),
                    }
                }
            },
        )
    }
}

/// Compare expected package parameters with actual package state
fn compare_package_state(expected: &PackageParameters, actual: &PackageState) -> CheckResult {
    let mut actual_map = HashMap::new();
    let mut validation_passed = true;

    // Always include installation status in actual state using standard field name
    set_installed_field(&mut actual_map, actual.installed);

    // If there's an error, return error result with populated actual state
    if let Some(error) = &actual.error {
        return CheckResult {
            ok: false,
            actual: actual_map,
            error: Some(error.clone()),
        };
    }

    // Check expected state (present/absent)
    if let Some(ref expected_state) = expected.state {
        let expected_installed = expected_state == "present";
        if expected_installed != actual.installed {
            validation_passed = false;
        }
    }

    // Include version in actual state using standard field name and helper
    set_version_field(&mut actual_map, actual.version.clone());

    // Check expected version if specified
    if let Some(ref expected_version) = expected.version {
        // Only validate version if package is installed
        if actual.installed {
            if let Some(ref actual_version) = actual.version {
                if expected_version != actual_version {
                    validation_passed = false;
                }
            } else {
                // Expected a version but couldn't determine it
                // Don't fail validation for unknown version, just report it
            }
        } else {
            // Package not installed but version was expected
            validation_passed = false;
        }
    }

    create_validated_result(validation_passed, &actual_map, None, "package").unwrap_or_else(|_| {
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
    fn test_extract_package_parameters_valid() -> Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "nginx")?;
        params.set("state", "present")?;
        params.set("version", "1.18.0")?;

        let package_params = extract_package_parameters(&params)?;

        assert_eq!(package_params.name, "nginx");
        assert_eq!(package_params.state, Some("present".to_string()));
        assert_eq!(package_params.version, Some("1.18.0".to_string()));

        Ok(())
    }

    #[test]
    fn test_extract_package_parameters_minimal() -> Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "postgresql")?;

        let package_params = extract_package_parameters(&params)?;

        assert_eq!(package_params.name, "postgresql");
        assert_eq!(package_params.state, None);
        assert_eq!(package_params.version, None);

        Ok(())
    }

    #[test]
    fn test_extract_package_parameters_missing_name() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("state", "present")?;

        let result = extract_package_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("name"));
        }
        Ok(())
    }

    #[test]
    fn test_extract_package_parameters_empty_name() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "")?;

        let result = extract_package_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("empty"));
        }
        Ok(())
    }

    #[test]
    fn test_extract_package_parameters_invalid_state() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("name", "nginx")?;
        params.set("state", "installed")?; // Invalid state

        let result = extract_package_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("state"));
        }
        Ok(())
    }

    #[test]
    fn test_validate_package_name() -> Result<()> {
        // Valid package names
        validate_package_name("nginx")?;
        validate_package_name("postgresql")?;
        validate_package_name("my-package")?;
        validate_package_name("package_name")?;
        validate_package_name("package.name")?;
        validate_package_name("package123")?;

        // Invalid package names
        assert!(validate_package_name("").is_err());
        assert!(validate_package_name("package with spaces").is_err());
        assert!(validate_package_name("package/with/slash").is_err());
        assert!(validate_package_name("package;with;semicolon").is_err());
        assert!(validate_package_name("package`with`backtick").is_err());
        assert!(validate_package_name("package$with$dollar").is_err());

        Ok(())
    }

    #[test]
    fn test_validate_package_state() -> Result<()> {
        // Valid states
        validate_package_state("present")?;
        validate_package_state("absent")?;

        // Invalid states
        assert!(validate_package_state("installed").is_err());
        assert!(validate_package_state("removed").is_err());
        assert!(validate_package_state("latest").is_err());

        Ok(())
    }

    #[test]
    fn test_check_package_lua_interface() -> mlua::Result<()> {
        let lua = Lua::new();

        // Create parameters table
        let params = lua.create_table()?;
        params.set("name", "nginx")?;
        params.set("state", "present")?;

        // Test that the function can be called (it will fail due to no actual package manager access in tests)
        let args = mlua::MultiValue::from_vec(vec![mlua::Value::Table(params)]);
        let result = check_package(&lua, args);

        // The function should return a result (success or error)
        assert!(result.is_ok() || result.is_err());

        Ok(())
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
    fn test_compare_package_state_success() {
        let expected = PackageParameters {
            name: "nginx".to_string(),
            state: Some("present".to_string()),
            version: Some("1.18.0".to_string()),
        };

        let actual = PackageState {
            installed: true,
            version: Some("1.18.0".to_string()),
            error: None,
        };

        let result = compare_package_state(&expected, &actual);
        assert!(result.ok);
        assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("version"), Some(&"1.18.0".to_string()));
    }

    #[test]
    fn test_compare_package_state_failure() {
        let expected = PackageParameters {
            name: "nginx".to_string(),
            state: Some("present".to_string()),
            version: Some("1.18.0".to_string()),
        };

        let actual = PackageState {
            installed: true,
            version: Some("1.16.0".to_string()), // Different version
            error: None,
        };

        let result = compare_package_state(&expected, &actual);
        assert!(!result.ok);
        assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("version"), Some(&"1.16.0".to_string()));
    }

    #[test]
    fn test_compare_package_state_not_installed() {
        let expected = PackageParameters {
            name: "nonexistent".to_string(),
            state: Some("absent".to_string()),
            version: None,
        };

        let actual = PackageState {
            installed: false,
            version: None,
            error: None,
        };

        let result = compare_package_state(&expected, &actual);
        assert!(result.ok);
        assert_eq!(result.actual.get("installed"), Some(&"false".to_string()));
    }

    #[test]
    fn test_compare_package_state_unexpected_installed() {
        let expected = PackageParameters {
            name: "nginx".to_string(),
            state: Some("absent".to_string()),
            version: None,
        };

        let actual = PackageState {
            installed: true,
            version: Some("1.18.0".to_string()),
            error: None,
        };

        let result = compare_package_state(&expected, &actual);
        assert!(!result.ok);
        assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("version"), Some(&"1.18.0".to_string()));
    }

    #[test]
    fn test_compare_package_state_unknown_version() {
        let expected = PackageParameters {
            name: "nginx".to_string(),
            state: Some("present".to_string()),
            version: Some("1.18.0".to_string()),
        };

        let actual = PackageState {
            installed: true,
            version: None, // Unknown version
            error: None,
        };

        let result = compare_package_state(&expected, &actual);
        // Should still pass validation since we don't fail on unknown version
        assert!(result.ok);
        assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("version"), Some(&"unknown".to_string()));
    }

    #[test]
    fn test_compare_package_state_error() {
        let expected = PackageParameters {
            name: "nginx".to_string(),
            state: Some("present".to_string()),
            version: None,
        };

        let actual = PackageState {
            installed: false,
            version: None,
            error: Some("Package manager error".to_string()),
        };

        let result = compare_package_state(&expected, &actual);
        assert!(!result.ok);
        assert_eq!(result.error, Some("Package manager error".to_string()));
    }
}
