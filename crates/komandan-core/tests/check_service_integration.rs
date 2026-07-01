use komandan::create_lua;
use mlua::Table;

#[test]
fn test_check_service_integration_local() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test local service check
        local result = komandan.check.service({
            name = "ssh"
        })

        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Should have basic structure
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    let actual: Table = result.get("actual")?;
    assert!(actual.contains_key("exists")?);

    Ok(())
}

#[test]
fn test_check_service_integration_parameter_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test parameter validation - missing name
        local result = komandan.check.service({
            state = "active"
        })

        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Should return a result table with ok=false and error message
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("error")?);

    let ok: bool = result.get("ok")?;
    assert!(
        !ok,
        "Should return ok=false when required parameter is missing"
    );

    let error: String = result.get("error")?;
    assert!(
        error.contains("name"),
        "Error should mention missing name parameter, got: {error}"
    );

    Ok(())
}

#[test]
fn test_check_service_integration_invalid_state() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test parameter validation - invalid state
        local result = komandan.check.service({
            name = "ssh",
            state = "running"  -- Invalid state
        })

        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Should return a result table with ok=false and error message
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("error")?);

    let ok: bool = result.get("ok")?;
    assert!(!ok, "Should return ok=false when parameter is invalid");

    let error: String = result.get("error")?;
    assert!(
        error.contains("state") || error.contains("running"),
        "Error should mention invalid state parameter, got: {error}"
    );

    Ok(())
}

#[test]
fn test_check_service_integration_nonexistent() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test with non-existent service
        local result = komandan.check.service({
            name = "nonexistent-service-12345",
            state = "active"
        })

        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Should return a result
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    let actual: Table = result.get("actual")?;
    let exists: String = actual.get("exists")?;

    // Service should not exist
    assert_eq!(exists, "false");

    // Should fail validation since we expected it to be active
    let ok: bool = result.get("ok")?;
    assert!(!ok);

    Ok(())
}

#[test]
fn test_check_service_integration_k_alias() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test k.check alias
        local result = k.check.service({
            name = "ssh"
        })

        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Should have basic structure
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    Ok(())
}
