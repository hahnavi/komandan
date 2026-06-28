use komandan::create_lua;
use mlua::Table;

/// Comprehensive unit tests for parameter validation across all check functions
/// Tests required parameter validation, invalid parameter handling, and error messages
/// Requirements: 4.5

#[test]
fn test_check_file_parameter_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test missing required parameter (path)
    let script = r#"
        local result = komandan.check.file({
            mode = "0644"
            -- missing path parameter
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("path"),);

    // Test empty path parameter
    let script = r#"
        local result = komandan.check.file({
            path = ""
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("empty") || error.to_lowercase().contains("path"),);

    // Test relative path (should be absolute)
    let script = r#"
        local result = komandan.check.file({
            path = "relative/path"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("absolute"),);

    // Test invalid file mode format
    let script = r#"
        local result = komandan.check.file({
            path = "/tmp/test",
            mode = "644"  -- Missing leading zero
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("mode") || error.to_lowercase().contains("octal"));

    // Test invalid octal digits in mode
    let script = r#"
        local result = komandan.check.file({
            path = "/tmp/test",
            mode = "0888"  -- Invalid octal digit
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("mode") || error.to_lowercase().contains("octal"),);

    Ok(())
}

#[test]
fn test_check_service_parameter_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test missing required parameter (name)
    let script = r#"
        local result = komandan.check.service({
            state = "active"
            -- missing name parameter
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("name"),);

    // Test empty service name
    let script = r#"
        local result = komandan.check.service({
            name = ""
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("empty") || error.to_lowercase().contains("name"),);

    // Test service name with spaces
    let script = r#"
        local result = komandan.check.service({
            name = "service with spaces"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("space") || error.to_lowercase().contains("invalid"),);

    // Test service name with dangerous characters
    let script = r#"
        local result = komandan.check.service({
            name = "service;rm -rf /"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("invalid") || error.to_lowercase().contains("character"),);

    // Test invalid service state
    let script = r#"
        local result = komandan.check.service({
            name = "nginx",
            state = "running"  -- Invalid state (should be "active" or "inactive")
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(
        error.to_lowercase().contains("state")
            || error.to_lowercase().contains("active")
            || error.to_lowercase().contains("inactive"),
    );

    // Test invalid enabled parameter type (should be boolean)
    let script = r#"
        local result = komandan.check.service({
            name = "nginx",
            enabled = "yes"  -- Should be boolean
        })
        return result
    "#;

    let _result: Table = lua.load(script).eval()?;
    // This might succeed if Lua converts the string, but let's check the behavior
    // The validation should happen at the Rust level

    Ok(())
}

#[test]
fn test_check_package_parameter_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test missing required parameter (name)
    let script = r#"
        local result = komandan.check.package({
            state = "present"
            -- missing name parameter
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("name"),);

    // Test empty package name
    let script = r#"
        local result = komandan.check.package({
            name = ""
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("empty") || error.to_lowercase().contains("name"),);

    // Test package name with spaces
    let script = r#"
        local result = komandan.check.package({
            name = "package with spaces"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("space") || error.to_lowercase().contains("invalid"),);

    // Test package name with dangerous characters
    let script = r#"
        local result = komandan.check.package({
            name = "package;rm -rf /"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(error.to_lowercase().contains("invalid") || error.to_lowercase().contains("character"),);

    // Test invalid package state
    let script = r#"
        local result = komandan.check.package({
            name = "nginx",
            state = "installed"  -- Invalid state (should be "present" or "absent")
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    assert!(
        error.to_lowercase().contains("state")
            || error.to_lowercase().contains("present")
            || error.to_lowercase().contains("absent"),
    );

    Ok(())
}

#[test]
fn test_parameter_type_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test non-table parameter for file check
    let script = r#"
        local result = komandan.check.file("not a table")
        return result
    "#;

    // This should cause a Lua error since we expect a table
    let result = lua.load(script).eval::<Table>();
    assert!(
        result.is_err(),
        "Should fail when parameters are not a table"
    );

    // Test nil parameter
    let script = r"
        local result = komandan.check.file(nil)
        return result
    ";

    let result = lua.load(script).eval::<Table>();
    assert!(result.is_err(), "Should fail when parameters are nil");

    Ok(())
}

#[test]
fn test_parameter_conversion_and_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test boolean parameter conversion for file exists
    let script = r#"
        local result = komandan.check.file({
            path = "/tmp/test",
            exists = "true"  -- String instead of boolean
        })
        return result
    "#;

    let _result: Table = lua.load(script).eval()?;
    // This might succeed if Lua/mlua converts the string to boolean
    // The behavior depends on the implementation

    // Test numeric parameter where string expected
    let script = r"
        local result = komandan.check.file({
            path = 123  -- Number instead of string
        })
        return result
    ";

    let _result = lua.load(script).eval::<Table>();
    // This should either fail at the Lua level or be converted to string

    Ok(())
}

#[test]
fn test_comprehensive_error_messages() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test that error messages are descriptive and helpful
    let script = r#"
        local result = komandan.check.file({
            path = "relative/path",
            mode = "invalid_mode",
            owner = "",
            group = ""
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("ok")?);
    assert!(result.contains_key("error")?);

    let error = result.get::<String>("error")?;
    // Error should be descriptive and mention the specific issue
    assert!(!error.is_empty(), "Error message should not be empty");
    assert!(error.len() > 10,);

    Ok(())
}

#[test]
fn test_edge_case_parameter_values() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test very long parameter values
    let long_path = "/".to_string() + &"a".repeat(1000);
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{long_path}"
        }})
        return result
    "#
    );

    let _result: Table = lua.load(&script).eval()?;
    // Should handle long paths gracefully (either accept or reject with clear error)

    // Test unicode characters in parameters
    let script = r#"
        local result = komandan.check.file({
            path = "/tmp/测试文件",
            owner = "用户"
        })
        return result
    "#;

    let _result: Table = lua.load(script).eval()?;
    // Should handle unicode characters appropriately

    // Test special characters that might cause issues
    let script = r#"
        local result = komandan.check.file({
            path = "/tmp/file with\nnewline"
        })
        return result
    "#;

    let _result: Table = lua.load(script).eval()?;
    // Should handle or reject special characters appropriately

    Ok(())
}

#[test]
fn test_parameter_validation_consistency() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test that all check functions handle missing parameters consistently
    let test_cases = vec![
        ("komandan.check.file({})", "path"),
        ("komandan.check.service({})", "name"),
        ("komandan.check.package({})", "name"),
    ];

    for (script, expected_param) in test_cases {
        let result: Table = lua.load(script).eval()?;
        assert!(!result.get::<bool>("ok")?, "Script {script} should fail");
        assert!(
            result.contains_key("error")?,
            "Script {script} should have error"
        );

        let error = result.get::<String>("error")?;
        assert!(error.to_lowercase().contains(expected_param));
    }

    Ok(())
}
