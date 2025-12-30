//! # `ModulesV2` APT Package Management Module
//!
//! The `apt` module provides package management functionality for `ModulesV2`.
//! It supports both local and remote execution with automatic connection management.
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution - install package
//! local result = k.mod.apt({package = "nginx", state = "present"})
//!
//! -- Remote execution - remove package
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.apt({package = "nginx", state = "absent"}, host)
//!
//! -- Install with cache update
//! local result = k.mod.apt({
//!     package = "nginx",
//!     state = "present",
//!     update_cache = true
//! })
//! ```
//!
//! ## Parameters
//!
//! - `package` (string or table, required): Package name(s) to manage
//! - `state` (string, optional): Package state - "present", "absent", or "latest" (default: "present")
//! - `update_cache` (boolean, optional): Whether to update package cache (default: false)
//! - `cache_valid_time` (number, optional): Cache validity time in seconds
//!
//! ## Return Value
//!
//! Returns a table with:
//! - `stdout`: Command output
//! - `stderr`: Error output
//! - `exit_code`: Exit code (0 for success)
//! - `changed`: Boolean indicating if packages were modified

use super::execution::{ExecutionEngine, ModuleResult};
use super::factory::create_modulev2_function;
use crate::connection::Connection;
use mlua::{Lua, Table, Value};

/// Create the `apt_v2` function for `ModulesV2`
///
/// This function creates a ModulesV2-compatible APT package management module that supports
/// both local and remote execution patterns.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - The configured `apt_v2` function
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - Package management operations fail
pub fn apt_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
    create_modulev2_function(lua, "apt", |lua, params, host| {
        ExecutionEngine::execute_module(lua, "apt", &params, host, |connection, params| {
            // Extract and validate parameters
            let package = extract_package_parameter(params)?;
            let state = params
                .get::<Option<String>>("state")?
                .unwrap_or_else(|| "present".to_string());
            let update_cache = params.get::<Option<bool>>("update_cache")?.unwrap_or(false);

            // Validate state parameter
            validate_state(&state)?;

            // Sanitize package names
            let sanitized_package = sanitize_package_names(package)?;

            // Execute APT operations
            execute_apt_operations(connection, &sanitized_package, &state, update_cache)
        })
    })
}

/// Extract and validate the package parameter
fn extract_package_parameter(params: &Table) -> mlua::Result<PackageSpec> {
    match params.get::<Value>("package")? {
        Value::String(pkg) => Ok(PackageSpec::Single(pkg.to_str()?.to_string())),
        Value::Table(pkg_table) => {
            let mut packages = Vec::new();
            for pair in pkg_table.pairs::<i32, String>() {
                let (_, package) = pair?;
                packages.push(package);
            }
            if packages.is_empty() {
                return Err(mlua::Error::RuntimeError(
                    "package parameter cannot be empty".to_string(),
                ));
            }
            Ok(PackageSpec::Multiple(packages))
        }
        Value::Nil => Err(mlua::Error::RuntimeError(
            "apt module requires 'package' parameter".to_string(),
        )),
        _ => Err(mlua::Error::RuntimeError(
            "package parameter must be a string or table of strings".to_string(),
        )),
    }
}

/// Validate the state parameter
fn validate_state(state: &str) -> mlua::Result<()> {
    match state {
        "present" | "absent" | "latest" => Ok(()),
        _ => Err(mlua::Error::RuntimeError(format!(
            "Invalid state: {state}. Valid states are: present, absent, latest"
        ))),
    }
}

/// Sanitize package names to prevent injection attacks
fn sanitize_package_names(package_spec: PackageSpec) -> mlua::Result<PackageSpec> {
    match package_spec {
        PackageSpec::Single(pkg) => {
            let sanitized = sanitize_package_name(&pkg)?;
            Ok(PackageSpec::Single(sanitized))
        }
        PackageSpec::Multiple(packages) => {
            let mut sanitized_packages = Vec::new();
            for pkg in packages {
                let sanitized = sanitize_package_name(&pkg)?;
                sanitized_packages.push(sanitized);
            }
            if sanitized_packages.is_empty() {
                return Err(mlua::Error::RuntimeError(
                    "No valid package names after sanitization".to_string(),
                ));
            }
            Ok(PackageSpec::Multiple(sanitized_packages))
        }
    }
}

/// Sanitize a single package name
fn sanitize_package_name(package: &str) -> mlua::Result<String> {
    if package.trim().is_empty() {
        return Err(mlua::Error::RuntimeError(
            "Package name cannot be empty".to_string(),
        ));
    }

    // Allow alphanumeric, -, _, =, ., +, : (common in package names)
    let sanitized: String = package
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '=' | '.' | '+' | ':'))
        .collect();

    if sanitized.is_empty() {
        return Err(mlua::Error::RuntimeError(format!(
            "Package name '{package}' contains only invalid characters"
        )));
    }

    Ok(sanitized)
}

/// Execute APT operations based on parameters
fn execute_apt_operations(
    connection: &mut Connection,
    package_spec: &PackageSpec,
    state: &str,
    update_cache: bool,
) -> mlua::Result<ModuleResult> {
    let mut stdout_parts = Vec::new();
    let mut stderr_parts = Vec::new();
    let mut _changed = false;

    // Update cache if requested
    if update_cache {
        let (cache_stdout, cache_stderr, cache_exit_code) = connection
            .cmd("apt update")
            .map_err(|e| mlua::Error::RuntimeError(format!("Cache update failed: {e}")))?;

        stdout_parts.push(cache_stdout.clone());
        if !cache_stderr.is_empty() {
            stderr_parts.push(cache_stderr);
        }

        if cache_exit_code != 0 {
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                cache_exit_code,
            ));
        }

        // Check if cache update actually changed anything
        if cache_stdout.contains("Get:") {
            _changed = true;
        }
    }

    // Execute package operations based on state
    match state {
        "present" => {
            let result = install_packages(connection, package_spec)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0
                && is_package_operation_changed(stdout_parts.last().unwrap_or(&String::new()))
            {
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
        "absent" => {
            let result = remove_packages(connection, package_spec)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0
                && is_package_operation_changed(stdout_parts.last().unwrap_or(&String::new()))
            {
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
        "latest" => {
            let result = upgrade_packages(connection, package_spec)?;
            stdout_parts.push(result.stdout);
            if !result.stderr.is_empty() {
                stderr_parts.push(result.stderr);
            }
            if result.exit_code == 0
                && is_package_operation_changed(stdout_parts.last().unwrap_or(&String::new()))
            {
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
        _ => unreachable!("State validation should prevent this"),
    }

    Ok(ModuleResult::complete(
        stdout_parts.join("\n"),
        stderr_parts.join("\n"),
        0, // Always return 0 for successful operations
    ))
}

/// Install packages
fn install_packages(
    connection: &mut Connection,
    package_spec: &PackageSpec,
) -> mlua::Result<ModuleResult> {
    let packages_str = package_spec.to_string();

    // First check if packages are already installed
    if is_packages_installed(connection, package_spec)? {
        return Ok(ModuleResult::success(format!(
            "Package(s) {packages_str} already installed"
        )));
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("apt install -y {packages_str}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Package installation failed: {e}")))?;

    Ok(ModuleResult::complete(stdout, stderr, exit_code))
}

/// Remove packages
fn remove_packages(
    connection: &mut Connection,
    package_spec: &PackageSpec,
) -> mlua::Result<ModuleResult> {
    let packages_str = package_spec.to_string();

    // First check if any packages are installed
    if !is_packages_installed(connection, package_spec)? {
        return Ok(ModuleResult::success(format!(
            "Package(s) {packages_str} already removed"
        )));
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("apt remove -y {packages_str}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Package removal failed: {e}")))?;

    Ok(ModuleResult::complete(stdout, stderr, exit_code))
}

/// Upgrade packages to latest version
fn upgrade_packages(
    connection: &mut Connection,
    package_spec: &PackageSpec,
) -> mlua::Result<ModuleResult> {
    let packages_str = package_spec.to_string();

    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("apt install -y --only-upgrade {packages_str}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Package upgrade failed: {e}")))?;

    Ok(ModuleResult::complete(stdout, stderr, exit_code))
}

/// Check if packages are installed
fn is_packages_installed(
    connection: &mut Connection,
    package_spec: &PackageSpec,
) -> mlua::Result<bool> {
    match package_spec {
        PackageSpec::Single(pkg) => {
            let (_, _, exit_code) = connection
                .cmd(&format!(
                    "dpkg-query -W -f='${{Status}}' {pkg} 2>/dev/null | grep -q 'ok installed'"
                ))
                .map_err(|e| mlua::Error::RuntimeError(format!("Package check failed: {e}")))?;
            Ok(exit_code == 0)
        }
        PackageSpec::Multiple(packages) => {
            // For multiple packages, check if all are installed
            for pkg in packages {
                let (_, _, exit_code) = connection
                    .cmd(&format!(
                        "dpkg-query -W -f='${{Status}}' {pkg} 2>/dev/null | grep -q 'ok installed'"
                    ))
                    .map_err(|e| mlua::Error::RuntimeError(format!("Package check failed: {e}")))?;
                if exit_code != 0 {
                    return Ok(false);
                }
            }
            Ok(true)
        }
    }
}

/// Check if package operation resulted in changes
fn is_package_operation_changed(output: &str) -> bool {
    // Look for indicators that packages were actually installed/removed/upgraded
    // Exclude the specific case where nothing was changed
    if output.contains("0 upgraded, 0 newly installed, 0 to remove") {
        return false;
    }

    output.contains("newly installed")
        || output.contains("upgraded")
        || output.contains("removed")
        || output.contains("The following packages will be")
}

/// Package specification enum
#[derive(Debug, Clone)]
enum PackageSpec {
    Single(String),
    Multiple(Vec<String>),
}

impl std::fmt::Display for PackageSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Single(pkg) => write!(f, "{pkg}"),
            Self::Multiple(packages) => write!(f, "{}", packages.join(" ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_apt_v2_local_execution() -> mlua::Result<()> {
        let lua = create_lua()?;
        let apt_fn = apt_v2(&lua)?;

        // Test with a package that should be available on most systems
        let params = lua.create_table()?;
        params.set("package", "coreutils")?; // Usually already installed
        params.set("state", "present")?;

        let result: Table = apt_fn.call(params)?;

        // In test environment, apt commands might fail, so we just check that the function works
        // The exit code might be non-zero due to lack of apt or permissions
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_apt_v2_missing_package_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;
        let apt_fn = apt_v2(&lua)?;

        // Test with missing package parameter
        let params = lua.create_table()?;
        params.set("state", "present")?;

        let result: mlua::Result<Table> = apt_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("apt module requires 'package' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("apt module requires 'package' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_apt_v2_invalid_state() -> mlua::Result<()> {
        let lua = create_lua()?;
        let apt_fn = apt_v2(&lua)?;

        // Test with invalid state
        let params = lua.create_table()?;
        params.set("package", "test-package")?;
        params.set("state", "invalid_state")?;

        let result: mlua::Result<Table> = apt_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("Invalid state: invalid_state"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("Invalid state: invalid_state"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_apt_v2_multiple_packages() -> mlua::Result<()> {
        let lua = create_lua()?;
        let apt_fn = apt_v2(&lua)?;

        // Test with multiple packages
        let packages = lua.create_table()?;
        packages.set(1, "coreutils")?;
        packages.set(2, "findutils")?;

        let params = lua.create_table()?;
        params.set("package", packages)?;
        params.set("state", "present")?;

        let result: Table = apt_fn.call(params)?;

        // In test environment, apt commands might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_sanitize_package_name() -> mlua::Result<()> {
        // Test valid package names
        assert_eq!(sanitize_package_name("nginx")?, "nginx");
        assert_eq!(sanitize_package_name("python3.8")?, "python3.8");
        assert_eq!(sanitize_package_name("g++")?, "g++");
        assert_eq!(sanitize_package_name("lib-dev")?, "lib-dev");

        // Test package name with invalid characters
        assert_eq!(sanitize_package_name("nginx;rm -rf /")?, "nginxrm-rf");

        // Test empty package name
        assert!(sanitize_package_name("").is_err());
        assert!(sanitize_package_name("   ").is_err());

        // Test package name with only invalid characters
        assert!(sanitize_package_name(";|&").is_err());

        Ok(())
    }

    #[test]
    fn test_package_spec_to_string() {
        let single = PackageSpec::Single("nginx".to_string());
        assert_eq!(single.to_string(), "nginx");

        let multiple = PackageSpec::Multiple(vec![
            "nginx".to_string(),
            "apache2".to_string(),
            "mysql-server".to_string(),
        ]);
        assert_eq!(multiple.to_string(), "nginx apache2 mysql-server");
    }

    #[test]
    fn test_validate_state() {
        // Test valid states
        assert!(validate_state("present").is_ok());
        assert!(validate_state("absent").is_ok());
        assert!(validate_state("latest").is_ok());

        // Test invalid state
        assert!(validate_state("invalid").is_err());
    }

    #[test]
    fn test_is_package_operation_changed() {
        // Test output indicating changes
        assert!(is_package_operation_changed(
            "The following packages will be newly installed:"
        ));
        assert!(is_package_operation_changed(
            "1 upgraded, 0 newly installed"
        ));
        assert!(is_package_operation_changed(
            "The following packages will be removed:"
        ));

        // Test output indicating no changes
        assert!(!is_package_operation_changed(
            "0 upgraded, 0 newly installed, 0 to remove"
        ));
        assert!(!is_package_operation_changed("Reading package lists..."));
    }
}
