use super::*;
use anyhow::Result;
use mlua::{Lua, Table};
use std::collections::HashMap;

#[test]
fn test_check_result_success() {
    let mut actual = HashMap::new();
    actual.insert("exists".to_string(), "true".to_string());
    actual.insert("mode".to_string(), "0644".to_string());

    let result = CheckResult::success(actual.clone());
    assert!(result.ok);
    assert_eq!(result.actual.len(), 2);
    assert!(result.error.is_none());
}

#[test]
fn test_check_result_failure() {
    let mut actual = HashMap::new();
    actual.insert("exists".to_string(), "false".to_string());

    let result = CheckResult::failure(actual.clone());
    assert!(!result.ok);
    assert_eq!(result.actual.len(), 1);
    assert!(result.error.is_none());
}

#[test]
fn test_check_result_error() {
    let result = CheckResult::error("Connection failed".to_string());
    assert!(!result.ok);
    assert!(result.actual.is_empty());
    assert_eq!(result.error, Some("Connection failed".to_string()));
}

#[test]
fn test_check_result_parameter_error() {
    let result = CheckResult::parameter_error("Invalid value", "mode");
    assert!(!result.ok);
    assert!(result.actual.is_empty());
    assert!(result.error.is_some());
    if let Some(error_msg) = result.error {
        assert!(error_msg.contains("Parameter validation failed"));
        assert!(error_msg.contains("Invalid value"));
        assert!(error_msg.contains("Parameter: mode"));
        assert!(error_msg.contains("Suggestion:"));
    }
}

#[test]
fn test_check_result_ssh_error() {
    let result = CheckResult::ssh_error("Connection timeout", Some("example.com"));
    assert!(!result.ok);
    assert!(result.actual.is_empty());
    assert!(result.error.is_some());
    if let Some(error_msg) = result.error {
        assert!(error_msg.contains("SSH connection failed"));
        assert!(error_msg.contains("Connection timeout"));
        assert!(error_msg.contains("Host: example.com"));
        assert!(error_msg.contains("Suggestion:"));
    }
}

#[test]
fn test_check_result_permission_error() {
    let result = CheckResult::permission_error("Access denied", "/etc/shadow");
    assert!(!result.ok);
    assert!(result.actual.is_empty());
    assert!(result.error.is_some());
    if let Some(error_msg) = result.error {
        assert!(error_msg.contains("Permission denied"));
        assert!(error_msg.contains("Access denied"));
        assert!(error_msg.contains("Resource: /etc/shadow"));
        assert!(error_msg.contains("privilege escalation"));
    }
}

#[test]
fn test_check_result_command_error() {
    let result = CheckResult::command_error("Command not found", "nonexistent-cmd", 127);
    assert!(!result.ok);
    assert!(result.actual.is_empty());
    assert!(result.error.is_some());
    if let Some(error_msg) = result.error {
        assert!(error_msg.contains("Command execution failed"));
        assert!(error_msg.contains("Command not found"));
        assert!(error_msg.contains("Command: nonexistent-cmd"));
        assert!(error_msg.contains("Exit code: 127"));
    }
}

#[test]
fn test_check_result_system_error() {
    let result = CheckResult::system_error("Out of memory");
    assert!(!result.ok);
    assert!(result.actual.is_empty());
    assert!(result.error.is_some());
    if let Some(error_msg) = result.error {
        assert!(error_msg.contains("System error"));
        assert!(error_msg.contains("Out of memory"));
        assert!(error_msg.contains("system resources"));
    }
}

#[test]
fn test_check_result_to_lua_table() -> mlua::Result<()> {
    let lua = Lua::new();
    let mut actual = HashMap::new();
    actual.insert("exists".to_string(), "true".to_string());
    actual.insert("mode".to_string(), "0644".to_string());

    let result = CheckResult::success(actual);
    let table = result.to_lua_table(&lua)?;

    assert!(table.get::<bool>("ok")?);
    let actual_table = table.get::<Table>("actual")?;
    assert_eq!(actual_table.get::<String>("exists")?, "true");
    assert_eq!(actual_table.get::<String>("mode")?, "0644");

    Ok(())
}

#[test]
fn test_check_result_to_lua_table_with_error() -> mlua::Result<()> {
    let lua = Lua::new();
    let result = CheckResult::error("Test error".to_string());
    let table = result.to_lua_table(&lua)?;

    assert!(!table.get::<bool>("ok")?);
    assert_eq!(table.get::<String>("error")?, "Test error");

    Ok(())
}

#[test]
fn test_execution_context_from_host_table() -> mlua::Result<()> {
    let lua = Lua::new();
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;

    let context = ExecutionContext::from_host_table(Some(&host_table));
    matches!(context, ExecutionContext::Remote(_));

    let context = ExecutionContext::from_host_table(None);
    matches!(context, ExecutionContext::Local);

    Ok(())
}

#[test]
fn test_validate_required_string() -> anyhow::Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("test_param", "test_value")?;

    let result = validation::validate_required_string(&params, "test_param")?;
    assert_eq!(result, "test_value");

    // Test missing parameter
    let result = validation::validate_required_string(&params, "missing_param");
    assert!(result.is_err());

    // Test empty parameter
    params.set("empty_param", "")?;
    let result = validation::validate_required_string(&params, "empty_param");
    assert!(result.is_err());

    Ok(())
}

#[test]
fn test_validate_file_mode() -> anyhow::Result<()> {
    // Valid modes
    validation::validate_file_mode("0644")?;
    validation::validate_file_mode("0755")?;
    validation::validate_file_mode("0600")?;

    // Invalid modes
    assert!(validation::validate_file_mode("644").is_err());
    assert!(validation::validate_file_mode("0888").is_err());
    assert!(validation::validate_file_mode("abcd").is_err());

    Ok(())
}

#[test]
fn test_execute_command_local_context() -> Result<()> {
    let lua = Lua::new();
    let context = ExecutionContext::Local;

    // Test a simple command that should work on any system
    let (stdout, _stderr, exit_code) = execution::execute_command(&lua, &context, "echo 'test'")?;

    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "test");

    Ok(())
}

#[test]
fn test_execute_command_remote_context_localhost() -> Result<()> {
    let lua = Lua::new();
    let host_table = lua.create_table()?;
    host_table.set("address", "localhost")?;
    host_table.set("connection", "local")?; // Force local even though we're using remote context

    let context = ExecutionContext::Remote(host_table);

    // Test a simple command that should work on any system
    let (stdout, _stderr, exit_code) =
        execution::execute_command(&lua, &context, "echo 'remote_test'")?;

    assert_eq!(exit_code, 0);
    assert_eq!(stdout.trim(), "remote_test");

    Ok(())
}

#[test]
fn test_error_categorization() {
    // Test permission error detection
    assert!(execution::is_permission_error("Permission denied"));
    assert!(execution::is_permission_error("Access denied"));
    assert!(execution::is_permission_error("Operation not permitted"));
    assert!(!execution::is_permission_error("File not found"));

    // Test not found error detection
    assert!(execution::is_not_found_error("No such file or directory"));
    assert!(execution::is_not_found_error("not found"));
    assert!(execution::is_not_found_error("does not exist"));
    assert!(execution::is_not_found_error("no packages found"));
    assert!(!execution::is_not_found_error("Permission denied"));

    // Test SSH error detection
    assert!(execution::is_ssh_error("SSH connection failed"));
    assert!(execution::is_ssh_error("Connection timeout"));
    assert!(execution::is_ssh_error("Authentication failed"));
    assert!(!execution::is_ssh_error("File not found"));
}

#[test]
fn test_execute_command_with_error_handling_success() {
    let lua = Lua::new();
    let context = ExecutionContext::Local;

    let result = execution::execute_command_with_error_handling(
        &lua,
        &context,
        "echo 'success'",
        "Test operation",
    );

    assert!(result.ok);
    if let Some(stdout) = result.actual.get("stdout") {
        assert!(stdout.contains("success"));
    }
    assert!(result.error.is_none());
}

#[test]
fn test_execute_command_with_error_handling_not_found() {
    let lua = Lua::new();
    let context = ExecutionContext::Local;

    let result = execution::execute_command_with_error_handling(
        &lua,
        &context,
        "cat /nonexistent/file/that/does/not/exist",
        "File read check",
    );

    // Not-found errors return success with an empty actual map. Callers
    // interpret the missing keys per their own semantics (file/service/package
    // checks each map absence to their own state).
    assert!(result.ok);
    assert!(!result.actual.contains_key("exists"));
    assert!(result.error.is_none());
}

#[test]
fn test_result_validation_structure() -> Result<()> {
    use std::collections::HashMap;

    // Test valid result structure
    let mut actual = HashMap::new();
    actual.insert("exists".to_string(), "true".to_string());
    let result = CheckResult::success(actual);
    result_validation::validate_check_result_structure(&result)?;

    // Test invalid result structure (ok=false, no actual data, no error)
    let empty_actual = HashMap::new();
    let invalid_result = CheckResult {
        ok: false,
        actual: empty_actual,
        error: None,
    };
    assert!(result_validation::validate_check_result_structure(&invalid_result).is_err());

    Ok(())
}

#[test]
fn test_field_naming_validation() -> Result<()> {
    use std::collections::HashMap;

    // Test valid field names for file module
    let mut file_actual = HashMap::new();
    file_actual.insert("exists".to_string(), "true".to_string());
    file_actual.insert("mode".to_string(), "0644".to_string());
    file_actual.insert("owner".to_string(), "root".to_string());
    result_validation::validate_field_naming(&file_actual, "file")?;

    // Test invalid field name for file module
    let mut invalid_actual = HashMap::new();
    invalid_actual.insert("invalid_field".to_string(), "value".to_string());
    assert!(result_validation::validate_field_naming(&invalid_actual, "file").is_err());

    // Test valid field names for service module
    let mut service_actual = HashMap::new();
    service_actual.insert("exists".to_string(), "true".to_string());
    service_actual.insert("state".to_string(), "active".to_string());
    service_actual.insert("enabled".to_string(), "true".to_string());
    result_validation::validate_field_naming(&service_actual, "service")?;

    // Test valid field names for package module
    let mut package_actual = HashMap::new();
    package_actual.insert("installed".to_string(), "true".to_string());
    package_actual.insert("version".to_string(), "1.0.0".to_string());
    result_validation::validate_field_naming(&package_actual, "package")?;

    Ok(())
}

#[test]
fn test_standard_field_helpers() {
    use std::collections::HashMap;

    let mut actual = HashMap::new();

    // Test boolean field formatting
    assert_eq!(result_validation::format_boolean_field(true), "true");
    assert_eq!(result_validation::format_boolean_field(false), "false");

    // Test exists field helper
    result_validation::set_exists_field(&mut actual, true);
    assert_eq!(actual.get("exists"), Some(&"true".to_string()));

    // Test installed field helper
    result_validation::set_installed_field(&mut actual, false);
    assert_eq!(actual.get("installed"), Some(&"false".to_string()));

    // Test enabled field helper
    result_validation::set_enabled_field(&mut actual, Some(true));
    assert_eq!(actual.get("enabled"), Some(&"true".to_string()));

    result_validation::set_enabled_field(&mut actual, None);
    assert_eq!(actual.get("enabled"), Some(&"unknown".to_string()));

    // Test version field helper
    result_validation::set_version_field(&mut actual, Some("1.0.0".to_string()));
    assert_eq!(actual.get("version"), Some(&"1.0.0".to_string()));

    result_validation::set_version_field(&mut actual, None);
    assert_eq!(actual.get("version"), Some(&"unknown".to_string()));
}

#[test]
fn test_create_validated_result() -> Result<()> {
    use std::collections::HashMap;

    // Test creating a valid success result
    let mut actual = HashMap::new();
    actual.insert("exists".to_string(), "true".to_string());
    actual.insert("mode".to_string(), "0644".to_string());

    let result = result_validation::create_validated_result(true, &actual, None, "file")?;
    assert!(result.ok);
    assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("mode"), Some(&"0644".to_string()));

    // Test creating a valid failure result
    let mut actual = HashMap::new();
    actual.insert("exists".to_string(), "false".to_string());

    let result = result_validation::create_validated_result(false, &actual, None, "file")?;
    assert!(!result.ok);
    assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));

    // Test creating an error result
    let actual = HashMap::new();
    let result = result_validation::create_validated_result(
        false,
        &actual,
        Some("Test error".to_string()),
        "file",
    )?;
    assert!(!result.ok);
    assert_eq!(result.error, Some("Test error".to_string()));

    Ok(())
}
