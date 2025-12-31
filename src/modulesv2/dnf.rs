//! # `ModulesV2` DNF Package Management Module
//!
//! The `dnf` module provides package management functionality for `ModulesV2` on Red Hat-based systems.
//! It supports both local and remote execution with automatic connection management.
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution - install package
//! local result = k.mod.dnf({package = "nginx", state = "present"})
//!
//! -- Remote execution - remove package
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.dnf({package = "nginx", state = "absent"}, host)
//!
//! -- Install with cache update
//! local result = k.mod.dnf({
//!     package = "nginx",
//!     state = "present",
//!     update_cache = true
//! })
//!
//! -- Upgrade all packages
//! local result = k.mod.dnf({action = "upgrade"})
//! ```
//!
//! ## Parameters
//!
//! - `package` (string or table, optional): Package name(s) to manage
//! - `state` (string, optional): Package state - "present", "absent", or "latest" (default: "present")
//! - `action` (string, optional): Action to perform - "install", "remove", "update", "upgrade", "autoremove"
//! - `update_cache` (boolean, optional): Whether to update package cache (default: false)
//! - `install_weak_deps` (boolean, optional): Whether to install weak dependencies (default: true)
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

/// Create the `dnf_v2` function for `ModulesV2`
///
/// This function creates a ModulesV2-compatible DNF package management module that supports
/// both local and remote execution patterns.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - The configured `dnf_v2` function
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - Package management operations fail
pub fn dnf_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
    create_modulev2_function(lua, "dnf", |lua, params, host| {
        ExecutionEngine::execute_module(lua, "dnf", &params, host, |connection, params| {
            // Extract and validate parameters
            let package = extract_package_parameter(params)?;
            let action = extract_action_parameter(params, package.as_ref())?;
            let update_cache = params.get::<Option<bool>>("update_cache")?.unwrap_or(false);
            let install_weak_deps = params
                .get::<Option<bool>>("install_weak_deps")?
                .unwrap_or(true);

            // Validate action parameter
            validate_action(&action)?;

            // Sanitize package names if provided
            let sanitized_package = if let Some(pkg) = package {
                Some(sanitize_package_names(pkg)?)
            } else {
                None
            };

            // Execute DNF operations
            execute_dnf_operations(
                connection,
                sanitized_package.as_ref(),
                &action,
                update_cache,
                install_weak_deps,
            )
        })
    })
}

/// Extract and validate the package parameter
fn extract_package_parameter(params: &Table) -> mlua::Result<Option<PackageSpec>> {
    match params.get::<Value>("package")? {
        Value::String(pkg) => Ok(Some(PackageSpec::Single(pkg.to_str()?.to_string()))),
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
            Ok(Some(PackageSpec::Multiple(packages)))
        }
        Value::Nil => Ok(None),
        _ => Err(mlua::Error::RuntimeError(
            "package parameter must be a string or table of strings".to_string(),
        )),
    }
}

/// Extract and validate the action parameter
fn extract_action_parameter(params: &Table, package: Option<&PackageSpec>) -> mlua::Result<String> {
    let action = params.get::<Option<String>>("action")?;
    let state = params.get::<Option<String>>("state")?;

    // Determine action based on parameters
    match (action, state, package) {
        (Some(action), None, _) => Ok(action),
        (None, Some(state), _) => match state.as_str() {
            "present" => Ok("install".to_string()),
            "absent" => Ok("remove".to_string()),
            "latest" => Ok("update".to_string()),
            _ => Err(mlua::Error::RuntimeError(format!(
                "Invalid state: {state}. Valid states are: present, absent, latest"
            ))),
        },
        (None, None, Some(_)) => Ok("install".to_string()), // Default action when package is provided
        (None, None, None) => Err(mlua::Error::RuntimeError(
            "Either 'action' or 'package' parameter is required".to_string(),
        )),
        (Some(_), Some(_), _) => Err(mlua::Error::RuntimeError(
            "Cannot specify both 'action' and 'state' parameters".to_string(),
        )),
    }
}

/// Validate the action parameter
fn validate_action(action: &str) -> mlua::Result<()> {
    match action {
        "install" | "remove" | "update" | "upgrade" | "autoremove" => Ok(()),
        _ => Err(mlua::Error::RuntimeError(format!(
            "Invalid action: {action}. Valid actions are: install, remove, update, upgrade, autoremove"
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

/// Execute DNF operations based on parameters
fn execute_dnf_operations(
    connection: &mut Connection,
    package_spec: Option<&PackageSpec>,
    action: &str,
    update_cache: bool,
    install_weak_deps: bool,
) -> mlua::Result<ModuleResult> {
    let mut stdout_parts = Vec::new();
    let mut stderr_parts = Vec::new();
    // Update cache if requested
    let mut _changed = if update_cache {
        let (cache_stdout, cache_stderr, cache_exit_code) = connection
            .cmd("dnf makecache")
            .map_err(|e| mlua::Error::RuntimeError(format!("Cache update failed: {e}")))?;

        stdout_parts.push(cache_stdout);
        if !cache_stderr.is_empty() {
            stderr_parts.push(cache_stderr);
        }

        if cache_exit_code != 0 {
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                cache_exit_code,
                false, // Cache update failed, no change
            ));
        }

        // Cache update always indicates change
        true
    } else {
        false
    };

    // Execute package operations based on action
    match action {
        "install" => {
            if let Some(package_spec) = package_spec {
                let result = install_packages(connection, package_spec, install_weak_deps)?;
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
                        false, // Operation failed, no change
                    ));
                }
            } else {
                return Err(mlua::Error::RuntimeError(
                    "package parameter is required for install action".to_string(),
                ));
            }
        }
        "remove" => {
            if let Some(package_spec) = package_spec {
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
                        false, // Operation failed, no change
                    ));
                }
            } else {
                return Err(mlua::Error::RuntimeError(
                    "package parameter is required for remove action".to_string(),
                ));
            }
        }
        "update" => {
            if let Some(package_spec) = package_spec {
                let result = update_packages(connection, package_spec)?;
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
                        false, // Operation failed, no change
                    ));
                }
            } else {
                return Err(mlua::Error::RuntimeError(
                    "package parameter is required for update action".to_string(),
                ));
            }
        }
        "upgrade" => {
            let result = upgrade_all_packages(connection)?;
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
                    false, // Operation failed, no change
                ));
            }
        }
        "autoremove" => {
            let result = autoremove_packages(connection)?;
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
                    false, // Operation failed, no change
                ));
            }
        }
        _ => unreachable!("Action validation should prevent this"),
    }

    Ok(ModuleResult::complete(
        stdout_parts.join("\n"),
        stderr_parts.join("\n"),
        0,        // Always return 0 for successful operations
        _changed, // Use the tracked changed state
    ))
}

/// Install packages
fn install_packages(
    connection: &mut Connection,
    package_spec: &PackageSpec,
    install_weak_deps: bool,
) -> mlua::Result<ModuleResult> {
    let packages_str = format!("{package_spec}");

    // First check if packages are already installed
    if is_packages_installed(connection, package_spec)? {
        return Ok(ModuleResult::success_with_changed(
            format!("Package(s) {packages_str} already installed"),
            false, // No change if already installed
        ));
    }

    let mut cmd = format!("dnf install -y {packages_str}");
    if !install_weak_deps {
        cmd.push_str(" --setopt=install_weak_deps=False");
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&cmd)
        .map_err(|e| mlua::Error::RuntimeError(format!("Package installation failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Remove packages
fn remove_packages(
    connection: &mut Connection,
    package_spec: &PackageSpec,
) -> mlua::Result<ModuleResult> {
    let packages_str = format!("{package_spec}");

    // First check if any packages are installed
    if !is_packages_installed(connection, package_spec)? {
        return Ok(ModuleResult::success_with_changed(
            format!("Package(s) {packages_str} already removed"),
            false, // No change if already removed
        ));
    }

    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("dnf remove -y {packages_str}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Package removal failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0, // Changed if successful
    ))
}

/// Update specific packages to latest version
fn update_packages(
    connection: &mut Connection,
    package_spec: &PackageSpec,
) -> mlua::Result<ModuleResult> {
    let packages_str = format!("{package_spec}");

    let (stdout, stderr, exit_code) = connection
        .cmd(&format!("dnf update -y {packages_str}"))
        .map_err(|e| mlua::Error::RuntimeError(format!("Package update failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0,
    ))
}

/// Upgrade all packages
fn upgrade_all_packages(connection: &mut Connection) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd("dnf upgrade -y")
        .map_err(|e| mlua::Error::RuntimeError(format!("System upgrade failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0,
    ))
}

/// Remove unused packages
fn autoremove_packages(connection: &mut Connection) -> mlua::Result<ModuleResult> {
    let (stdout, stderr, exit_code) = connection
        .cmd("dnf autoremove -y")
        .map_err(|e| mlua::Error::RuntimeError(format!("Autoremove failed: {e}")))?;

    Ok(ModuleResult::complete(
        stdout,
        stderr,
        exit_code,
        exit_code == 0,
    ))
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
                    "dnf repoquery --installed --whatprovides {pkg} 2>/dev/null"
                ))
                .map_err(|e| mlua::Error::RuntimeError(format!("Package check failed: {e}")))?;
            Ok(exit_code == 0)
        }
        PackageSpec::Multiple(packages) => {
            // For multiple packages, check if all are installed
            for pkg in packages {
                let (_, _, exit_code) = connection
                    .cmd(&format!(
                        "dnf repoquery --installed --whatprovides {pkg} 2>/dev/null"
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
    if output.contains("Nothing to do") {
        return false;
    }

    output.contains("Installing")
        || output.contains("Upgrading")
        || output.contains("Removing")
        || output.contains("Complete!")
        || output.contains("Installed:")
        || output.contains("Upgraded:")
        || output.contains("Removed:")
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
    fn test_dnf_v2_local_execution() -> mlua::Result<()> {
        let lua = create_lua()?;
        let dnf_fn = dnf_v2(&lua)?;

        // Test with a package that should be available on most systems
        let params = lua.create_table()?;
        params.set("package", "coreutils")?; // Usually already installed
        params.set("action", "install")?;

        let result: Table = dnf_fn.call(params)?;

        // In test environment, dnf commands might fail, so we just check that the function works
        // The exit code might be non-zero due to lack of dnf or permissions
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_dnf_v2_missing_package_for_install() -> mlua::Result<()> {
        let lua = create_lua()?;
        let dnf_fn = dnf_v2(&lua)?;

        // Test with missing package parameter for install action
        let params = lua.create_table()?;
        params.set("action", "install")?;

        let result: mlua::Result<Table> = dnf_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("package parameter is required for install action"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("package parameter is required for install action"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_dnf_v2_invalid_action() -> mlua::Result<()> {
        let lua = create_lua()?;
        let dnf_fn = dnf_v2(&lua)?;

        // Test with invalid action
        let params = lua.create_table()?;
        params.set("package", "test-package")?;
        params.set("action", "invalid_action")?;

        let result: mlua::Result<Table> = dnf_fn.call(params);
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
    fn test_dnf_v2_upgrade_action() -> mlua::Result<()> {
        let lua = create_lua()?;
        let dnf_fn = dnf_v2(&lua)?;

        // Test upgrade action without package parameter
        let params = lua.create_table()?;
        params.set("action", "upgrade")?;

        let result: Table = dnf_fn.call(params)?;

        // In test environment, dnf commands might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_dnf_v2_multiple_packages() -> mlua::Result<()> {
        let lua = create_lua()?;
        let dnf_fn = dnf_v2(&lua)?;

        // Test with multiple packages
        let packages = lua.create_table()?;
        packages.set(1, "coreutils")?;
        packages.set(2, "findutils")?;

        let params = lua.create_table()?;
        params.set("package", packages)?;
        params.set("action", "install")?;

        let result: Table = dnf_fn.call(params)?;

        // In test environment, dnf commands might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_dnf_v2_state_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;
        let dnf_fn = dnf_v2(&lua)?;

        // Test with state parameter instead of action
        let params = lua.create_table()?;
        params.set("package", "test-package")?;
        params.set("state", "present")?;

        let result: Table = dnf_fn.call(params)?;

        // In test environment, dnf commands might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_dnf_v2_conflicting_parameters() -> mlua::Result<()> {
        let lua = create_lua()?;
        let dnf_fn = dnf_v2(&lua)?;

        // Test with both action and state parameters
        let params = lua.create_table()?;
        params.set("package", "test-package")?;
        params.set("action", "install")?;
        params.set("state", "present")?;

        let result: mlua::Result<Table> = dnf_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("Cannot specify both 'action' and 'state' parameters"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("Cannot specify both 'action' and 'state' parameters"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

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
        assert_eq!(format!("{single}"), "nginx");

        let multiple = PackageSpec::Multiple(vec![
            "nginx".to_string(),
            "httpd".to_string(),
            "mariadb-server".to_string(),
        ]);
        assert_eq!(format!("{multiple}"), "nginx httpd mariadb-server");
    }

    #[test]
    fn test_validate_action() {
        // Test valid actions
        assert!(validate_action("install").is_ok());
        assert!(validate_action("remove").is_ok());
        assert!(validate_action("update").is_ok());
        assert!(validate_action("upgrade").is_ok());
        assert!(validate_action("autoremove").is_ok());

        // Test invalid action
        assert!(validate_action("invalid").is_err());
    }

    #[test]
    fn test_is_package_operation_changed() {
        // Test output indicating changes
        assert!(is_package_operation_changed("Installing : nginx"));
        assert!(is_package_operation_changed("Upgrading : kernel"));
        assert!(is_package_operation_changed("Removing : old-package"));
        assert!(is_package_operation_changed("Complete!"));
        assert!(is_package_operation_changed("Installed: nginx"));

        // Test output indicating no changes
        assert!(!is_package_operation_changed("Nothing to do"));
        assert!(!is_package_operation_changed(
            "Last metadata expiration check"
        ));
    }
}
