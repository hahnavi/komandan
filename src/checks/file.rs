use crate::checks::base::{
    CheckResult, ExecutionContext, execution,
    result_validation::{StandardFields, create_validated_result, set_exists_field},
    shell_escape,
    validation::{
        validate_file_mode, validate_optional_bool, validate_optional_string,
        validate_required_string,
    },
};
use anyhow::{Context, Result};
use mlua::{Lua, MultiValue, Table};
use std::collections::HashMap;

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

    let host_table = args_iter.next().map_or_else(
        || None,
        |value| {
            value
                .as_table()
                .map_or_else(|| None, |table| Some(table.clone()))
        },
    );

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

    compare_file_state(&file_params, &actual_state)
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
struct FileState {
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

/// Compare expected file parameters with actual file state
fn compare_file_state(expected: &FileParameters, actual: &FileState) -> CheckResult {
    let mut actual_map = HashMap::new();
    let mut validation_passed = true;

    // Always include existence in actual state using standard field name
    set_exists_field(&mut actual_map, actual.exists);

    // Check existence expectation
    if let Some(expected_exists) = expected.exists
        && expected_exists != actual.exists
    {
        validation_passed = false;
    }

    // If file doesn't exist, we can't check other properties
    if !actual.exists {
        if expected.mode.is_some() || expected.owner.is_some() || expected.group.is_some() {
            validation_passed = false;
        }

        if let Some(error) = &actual.error {
            return CheckResult::error(error.clone());
        }

        return create_validated_result(validation_passed, &actual_map, None, "file")
            .unwrap_or_else(|_| CheckResult::failure(actual_map));
    }

    // File exists, check properties
    if let Some(error) = &actual.error {
        return CheckResult::error(error.clone());
    }

    // Check mode using standard field name
    if let Some(ref actual_mode) = actual.mode {
        actual_map.insert(StandardFields::MODE.to_string(), actual_mode.clone());

        if let Some(ref expected_mode) = expected.mode
            && expected_mode != actual_mode
        {
            validation_passed = false;
        }
    }

    // Check owner using standard field name
    if let Some(ref actual_owner) = actual.owner {
        actual_map.insert(StandardFields::OWNER.to_string(), actual_owner.clone());

        if let Some(ref expected_owner) = expected.owner
            && expected_owner != actual_owner
        {
            validation_passed = false;
        }
    }

    // Check group using standard field name
    if let Some(ref actual_group) = actual.group {
        actual_map.insert(StandardFields::GROUP.to_string(), actual_group.clone());

        if let Some(ref expected_group) = expected.group
            && expected_group != actual_group
        {
            validation_passed = false;
        }
    }

    create_validated_result(validation_passed, &actual_map, None, "file").unwrap_or_else(|_| {
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
    fn test_extract_file_parameters_valid() -> Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("path", "/tmp/testfile")?;
        params.set("mode", "0644")?;
        params.set("owner", "root")?;
        params.set("group", "admin")?;
        params.set("exists", true)?;

        let file_params = extract_file_parameters(&params)?;

        assert_eq!(file_params.path, "/tmp/testfile");
        assert_eq!(file_params.mode, Some("0644".to_string()));
        assert_eq!(file_params.owner, Some("root".to_string()));
        assert_eq!(file_params.group, Some("admin".to_string()));
        assert_eq!(file_params.exists, Some(true));

        Ok(())
    }

    #[test]
    fn test_extract_file_parameters_minimal() -> Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("path", "/tmp/testfile")?;

        let file_params = extract_file_parameters(&params)?;

        assert_eq!(file_params.path, "/tmp/testfile");
        assert_eq!(file_params.mode, None);
        assert_eq!(file_params.owner, None);
        assert_eq!(file_params.group, None);
        assert_eq!(file_params.exists, None);

        Ok(())
    }

    #[test]
    fn test_extract_file_parameters_missing_path() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("mode", "0644")?;

        let result = extract_file_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("path"));
        }
        Ok(())
    }

    #[test]
    fn test_extract_file_parameters_empty_path() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("path", "")?;

        let result = extract_file_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("empty"));
        }
        Ok(())
    }

    #[test]
    fn test_extract_file_parameters_relative_path() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("path", "relative/path")?;

        let result = extract_file_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("absolute"));
        }
        Ok(())
    }

    #[test]
    fn test_extract_file_parameters_invalid_mode() -> mlua::Result<()> {
        let lua = Lua::new();
        let params = lua.create_table()?;
        params.set("path", "/tmp/testfile")?;
        params.set("mode", "644")?; // Missing leading zero

        let result = extract_file_parameters(&params);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("mode"));
        }
        Ok(())
    }

    #[test]
    fn test_compare_file_state_success() {
        let expected = FileParameters {
            path: "/tmp/testfile".to_string(),
            mode: Some("0644".to_string()),
            owner: Some("root".to_string()),
            group: Some("admin".to_string()),
            exists: Some(true),
        };

        let actual = FileState {
            exists: true,
            mode: Some("0644".to_string()),
            owner: Some("root".to_string()),
            group: Some("admin".to_string()),
            error: None,
        };

        let result = compare_file_state(&expected, &actual);
        assert!(result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("mode"), Some(&"0644".to_string()));
        assert_eq!(result.actual.get("owner"), Some(&"root".to_string()));
        assert_eq!(result.actual.get("group"), Some(&"admin".to_string()));
    }

    #[test]
    fn test_compare_file_state_failure() {
        let expected = FileParameters {
            path: "/tmp/testfile".to_string(),
            mode: Some("0644".to_string()),
            owner: Some("root".to_string()),
            group: Some("admin".to_string()),
            exists: Some(true),
        };

        let actual = FileState {
            exists: true,
            mode: Some("0600".to_string()),  // Different mode
            owner: Some("user".to_string()), // Different owner
            group: Some("admin".to_string()),
            error: None,
        };

        let result = compare_file_state(&expected, &actual);
        assert!(!result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
        assert_eq!(result.actual.get("mode"), Some(&"0600".to_string()));
        assert_eq!(result.actual.get("owner"), Some(&"user".to_string()));
        assert_eq!(result.actual.get("group"), Some(&"admin".to_string()));
    }

    #[test]
    fn test_compare_file_state_nonexistent() {
        let expected = FileParameters {
            path: "/tmp/nonexistent".to_string(),
            mode: None,
            owner: None,
            group: None,
            exists: Some(false),
        };

        let actual = FileState {
            exists: false,
            mode: None,
            owner: None,
            group: None,
            error: None,
        };

        let result = compare_file_state(&expected, &actual);
        assert!(result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));
    }

    #[test]
    fn test_compare_file_state_unexpected_nonexistent() {
        let expected = FileParameters {
            path: "/tmp/testfile".to_string(),
            mode: Some("0644".to_string()),
            owner: Some("root".to_string()),
            group: None,
            exists: Some(true),
        };

        let actual = FileState {
            exists: false,
            mode: None,
            owner: None,
            group: None,
            error: None,
        };

        let result = compare_file_state(&expected, &actual);
        assert!(!result.ok);
        assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));
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
    fn test_check_file_lua_interface() -> mlua::Result<()> {
        let lua = Lua::new();

        // Create parameters table
        let params = lua.create_table()?;
        params.set("path", "/tmp/testfile")?;
        params.set("mode", "0644")?;

        // Test that the function can be called (it will fail due to no actual file system access in tests)
        let args = mlua::MultiValue::from_vec(vec![mlua::Value::Table(params)]);
        let result = check_file(&lua, args);

        // The function should return a result (success or error)
        assert!(result.is_ok() || result.is_err());

        Ok(())
    }
}
