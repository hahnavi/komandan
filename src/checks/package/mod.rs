use crate::checks::base::{
    CheckResult, ExecutionContext, execution,
    result_validation::{create_validated_result, set_installed_field, set_version_field},
    validation::{validate_optional_string, validate_required_string},
};
use anyhow::{Context, Result};
use mlua::{Lua, MultiValue, Table};
use std::collections::HashMap;

mod apt;
mod dnf;

#[cfg(test)]
mod tests;

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
pub struct PackageState {
    installed: bool,
    version: Option<String>,
    error: Option<String>,
}

/// Query actual package state using package manager commands
fn query_package_state(lua: &Lua, context: &ExecutionContext, package_name: &str) -> PackageState {
    // First, detect which package manager is available
    let package_manager = detect_package_manager(lua, context);

    match package_manager {
        PackageManager::Apt => apt::query_apt_package_state(lua, context, package_name),
        PackageManager::Dnf => dnf::query_dnf_package_state(lua, context, package_name),
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
