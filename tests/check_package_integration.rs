use komandan::*;

/// Integration tests for package check functionality
/// These tests validate the package check function with real package managers

#[test]
fn test_check_package_basic_functionality() -> anyhow::Result<()> {
    if std::env::var("KOMANDAN_SSH_TEST").is_err() {
        eprintln!("Skipping SSH integration test - set KOMANDAN_SSH_TEST=1 to enable");
        return Ok(());
    }
    let lua = create_lua()?;

    // Test basic package check functionality
    let script = r#"
        local host = {
            address = "127.0.0.1",
            user = "usertest",
            connection = "ssh"
        }

        -- Check for a package that should exist on most systems
        local result = komandan.check.package({
            name = "bash"
        }, host)

        -- Should return a valid result structure
        assert(type(result) == "table", "Result should be a table")
        assert(type(result.ok) == "boolean", "Result should have ok field")
        assert(type(result.actual) == "table", "Result should have actual field")

        return result
    "#;

    let result: mlua::Table = lua.load(script).eval()?;

    // Verify the result structure
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    let actual: mlua::Table = result.get("actual")?;
    assert!(actual.contains_key("installed")?);

    Ok(())
}

#[test]
fn test_check_package_with_state_validation() -> anyhow::Result<()> {
    if std::env::var("KOMANDAN_SSH_TEST").is_err() {
        eprintln!("Skipping SSH integration test - set KOMANDAN_SSH_TEST=1 to enable");
        return Ok(());
    }
    let lua = create_lua()?;

    // Test package state validation
    let script = r#"
        local host = {
            address = "127.0.0.1",
            user = "usertest",
            connection = "ssh"
        }

        -- Check for bash package presence
        local result = komandan.check.package({
            name = "bash",
            state = "present"
        }, host)

        -- bash should be present on most systems
        assert(type(result.ok) == "boolean", "Result should have ok field")

        return result
    "#;

    let result: mlua::Table = lua.load(script).eval()?;
    let actual: mlua::Table = result.get("actual")?;
    let installed: bool = actual.get("installed")?;

    // bash should be installed on most systems
    assert!(installed);

    Ok(())
}

#[test]
fn test_check_package_namespace_accessibility() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test that package check is accessible via komandan.check namespace
    let script = r#"
        -- Verify the function exists in the namespace
        assert(type(komandan.check.package) == "function", "komandan.check.package should be a function")

        return true
    "#;

    let result: bool = lua.load(script).eval()?;
    assert!(result);

    Ok(())
}
