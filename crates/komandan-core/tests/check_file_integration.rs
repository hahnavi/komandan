use komandan::create_lua;
use std::fs;
use tempfile::TempDir;

#[test]
fn test_check_file_integration_local() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Create a temporary file for testing
    let temp_dir = TempDir::new()?;
    let test_file = temp_dir.path().join("test_file.txt");
    fs::write(&test_file, "test content")?;

    // Set file permissions to 644
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&test_file)?.permissions();
        perms.set_mode(0o644);
        fs::set_permissions(&test_file, perms)?;
    }

    let test_file_path = test_file.to_string_lossy().to_string();

    // Test file existence check
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}",
            exists = true
        }})
        return result
    "#
    );

    let result: mlua::Table = lua.load(&script).eval()?;

    // Verify the result structure
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    let ok = result.get::<bool>("ok")?;
    assert!(ok, "File existence check should pass");

    let actual = result.get::<mlua::Table>("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "true");

    // Test file mode check (Unix only)
    #[cfg(unix)]
    {
        let script = format!(
            r#"
            local result = komandan.check.file({{
                path = "{test_file_path}",
                mode = "0644"
            }})
            return result
        "#
        );

        let result: mlua::Table = lua.load(&script).eval()?;
        let ok = result.get::<bool>("ok")?;
        assert!(ok, "File mode check should pass");

        let actual = result.get::<mlua::Table>("actual")?;
        assert_eq!(actual.get::<String>("mode")?, "0644");
    }

    Ok(())
}

#[test]
fn test_check_file_integration_nonexistent() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test nonexistent file
    let script = r#"
        local result = komandan.check.file({
            path = "/tmp/nonexistent_file_12345",
            exists = false
        })
        return result
    "#;

    let result: mlua::Table = lua.load(script).eval()?;

    let ok = result.get::<bool>("ok")?;
    assert!(
        ok,
        "Nonexistent file check should pass when expecting false"
    );

    let actual = result.get::<mlua::Table>("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "false");

    Ok(())
}

#[test]
fn test_check_file_integration_failure() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test expecting file to exist when it doesn't
    let script = r#"
        local result = komandan.check.file({
            path = "/tmp/nonexistent_file_12345",
            exists = true
        })
        return result
    "#;

    let result: mlua::Table = lua.load(script).eval()?;

    let ok = result.get::<bool>("ok")?;
    assert!(
        !ok,
        "Check should fail when expecting file to exist but it doesn't"
    );

    let actual = result.get::<mlua::Table>("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "false");

    Ok(())
}

#[test]
fn test_check_file_integration_k_alias() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test using k.check alias
    let temp_dir = TempDir::new()?;
    let test_file = temp_dir.path().join("test_file.txt");
    fs::write(&test_file, "test content")?;

    let test_file_path = test_file.to_string_lossy().to_string();

    let script = format!(
        r#"
        local result = k.check.file({{
            path = "{test_file_path}",
            exists = true
        }})
        return result
    "#
    );

    let result: mlua::Table = lua.load(&script).eval()?;

    let ok = result.get::<bool>("ok")?;
    assert!(ok, "File check using k.check alias should work");

    Ok(())
}

#[test]
fn test_check_file_integration_parameter_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test missing required parameter
    let script = r#"
        local result = komandan.check.file({
            mode = "0644"
            -- missing path parameter
        })
        return result
    "#;

    let result: mlua::Table = lua.load(script).eval()?;

    // The function should return a result table, not throw an error
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("error")?);

    let ok = result.get::<bool>("ok")?;
    assert!(
        !ok,
        "Should return ok=false when required parameter is missing"
    );

    let error = result.get::<String>("error")?;
    assert!(
        error.contains("path"),
        "Error should mention missing path parameter, got: {error}"
    );

    Ok(())
}
