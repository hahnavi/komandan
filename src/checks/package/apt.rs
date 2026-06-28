use crate::checks::base::{ExecutionContext, execution, shell_escape};
use mlua::Lua;

use super::PackageState;

/// Query package state using APT/dpkg-query
pub(super) fn query_apt_package_state(
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
            // No stdout means `execute_command_with_error_handling` classified
            // the stderr (e.g. "no packages found" / dpkg-query exit 1 for a
            // missing package) as a not-found condition and returned an empty
            // actual map. Treat that as absent rather than a hard error.
            || PackageState {
                installed: false,
                version: None,
                error: None,
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
pub(super) fn parse_apt_package_output(stdout: &str) -> PackageState {
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
