use crate::checks::base::{
    CheckResult, ExecutionContext, execution, shell_escape,
    validation::{
        validate_file_mode, validate_optional_bool, validate_optional_string,
        validate_required_string,
    },
};
use anyhow::{Context, Result};
use mlua::{Lua, MultiValue, Table};

mod compare;

#[cfg(test)]
mod tests;

/// Check file function for validating file properties
///
/// This function validates file properties (existence, mode, owner, group) without
/// modifying the file system. It supports both local and remote execution via SSH.
///
/// # Parameters
/// - `path` (required): File path to validate
/// - `mode` (optional): Expected file mode in octal format (e.g., "0644")
/// - `owner` (optional): Expected file owner
/// - `group` (optional): Expected file group
/// - `exists` (optional): Expected existence state (true/false)
///
/// # Returns
/// A Lua table with:
/// - `ok`: Boolean indicating validation success/failure
/// - `actual`: Table with current file properties
/// - `error`: Optional error message
///
/// # Examples
/// ```lua
/// -- Local file check
/// local result = komandan.check.file({
///     path = "/tmp/testfile",
///     mode = "0644",
///     owner = "root"
/// })
///
/// -- Remote file check
/// local result = komandan.check.file({
///     path = "/etc/nginx/nginx.conf",
///     exists = true
/// }, host_table)
/// ```
pub fn check_file(lua: &Lua, args: MultiValue) -> mlua::Result<Table> {
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

    // Execute the file validation
    let result = execute_file_validation(lua, &params, host_table.as_ref());

    // Convert result to Lua table
    result.to_lua_table(lua)
}

/// Execute file validation logic
fn execute_file_validation(lua: &Lua, params: &Table, host_table: Option<&Table>) -> CheckResult {
    // 1. Extract and validate parameters
    let file_params = match extract_file_parameters(params) {
        Ok(params) => params,
        Err(e) => {
            return CheckResult::parameter_error(&e.to_string(), "file parameters");
        }
    };

    // 2. Determine execution context
    let context = ExecutionContext::from_host_table(host_table);

    // 3. Query actual file state
    let actual_state = query_file_state(lua, &context, &file_params.path);

    compare::compare_file_state(&file_params, &actual_state)
}

/// File validation parameters
#[derive(Debug, Clone)]
struct FileParameters {
    path: String,
    mode: Option<String>,
    owner: Option<String>,
    group: Option<String>,
    exists: Option<bool>,
}

/// Extract and validate file parameters from Lua table
fn extract_file_parameters(params: &Table) -> Result<FileParameters> {
    // Required parameter: path
    let path = validate_required_string(params, "path")?;

    // Optional parameters
    let mode = validate_optional_string(params, "mode")?;
    let owner = validate_optional_string(params, "owner")?;
    let group = validate_optional_string(params, "group")?;
    let exists = validate_optional_bool(params, "exists")?;

    // Validate file mode format if provided
    if let Some(ref mode_str) = mode {
        validate_file_mode(mode_str).with_context(|| format!("Invalid file mode: {mode_str}"))?;
    }

    // Validate path is not empty and is absolute
    if path.trim().is_empty() {
        anyhow::bail!("File path cannot be empty");
    }

    if !path.starts_with('/') {
        anyhow::bail!("File path must be absolute (start with '/')");
    }

    Ok(FileParameters {
        path,
        mode,
        owner,
        group,
        exists,
    })
}

/// Actual file state information
#[derive(Debug, Clone)]
pub struct FileState {
    exists: bool,
    mode: Option<String>,
    owner: Option<String>,
    group: Option<String>,
    error: Option<String>,
}

/// Query actual file state using read-only commands
fn query_file_state(lua: &Lua, context: &ExecutionContext, file_path: &str) -> FileState {
    // First check if file exists
    let exists_command = format!("test -e '{}'", shell_escape(file_path));

    // Use raw execute_command for existence check since exit code 1 is normal (file doesn't exist)
    let exists_result = execution::execute_command(lua, context, &exists_command);

    match exists_result {
        Ok((_, _, exit_code)) => {
            if exit_code == 0 {
                // File exists, get detailed properties
                let stat_command = format!("stat -c '%a %U %G' '{}'", shell_escape(file_path));

                let stat_result = execution::execute_command_with_error_handling(
                    lua,
                    context,
                    &stat_command,
                    "File properties query",
                );

                if stat_result.error.is_some() {
                    // Error occurred during stat
                    return FileState {
                        exists: true,
                        mode: None,
                        owner: None,
                        group: None,
                        error: stat_result.error,
                    };
                }

                if stat_result.ok {
                    stat_result.actual.get("stdout").map_or_else(
                        || FileState {
                            exists: true,
                            mode: None,
                            owner: None,
                            group: None,
                            error: Some("No output from stat command".to_string()),
                        },
                        |stdout| parse_stat_output(stdout),
                    )
                } else {
                    FileState {
                        exists: true,
                        mode: None,
                        owner: None,
                        group: None,
                        error: stat_result.error,
                    }
                }
            } else {
                // File doesn't exist (exit code 1 from test -e)
                FileState {
                    exists: false,
                    mode: None,
                    owner: None,
                    group: None,
                    error: None,
                }
            }
        }
        Err(e) => {
            // Connection or system error during existence check
            FileState {
                exists: false,
                mode: None,
                owner: None,
                group: None,
                error: Some(e.to_string()),
            }
        }
    }
}

/// Parse stat command output into file properties
fn parse_stat_output(stdout: &str) -> FileState {
    // Parse stat output: "mode owner group"
    let parts: Vec<&str> = stdout.split_whitespace().collect();
    if parts.len() != 3 {
        return FileState {
            exists: true,
            mode: None,
            owner: None,
            group: None,
            error: Some(format!("Unexpected stat output format: {}", stdout.trim())),
        };
    }

    FileState {
        exists: true,
        mode: Some(format!("0{}", parts[0])), // Add leading zero for octal format
        owner: Some(parts[1].to_string()),
        group: Some(parts[2].to_string()),
        error: None,
    }
}
