use crate::checks::base::{ExecutionContext, execution, shell_escape};
use mlua::Lua;

use super::PackageState;

/// Query package state using DNF/RPM
pub(super) fn query_dnf_package_state(
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
