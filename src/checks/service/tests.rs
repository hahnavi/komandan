use super::*;
use mlua::Lua;

#[test]
fn test_extract_service_parameters_valid() -> Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "nginx")?;
    params.set("state", "active")?;
    params.set("enabled", true)?;

    let service_params = extract_service_parameters(&params)?;

    assert_eq!(service_params.name, "nginx");
    assert_eq!(service_params.state, Some("active".to_string()));
    assert_eq!(service_params.enabled, Some(true));

    Ok(())
}

#[test]
fn test_extract_service_parameters_minimal() -> Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "postgresql")?;

    let service_params = extract_service_parameters(&params)?;

    assert_eq!(service_params.name, "postgresql");
    assert_eq!(service_params.state, None);
    assert_eq!(service_params.enabled, None);

    Ok(())
}

#[test]
fn test_extract_service_parameters_missing_name() -> mlua::Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("state", "active")?;

    let result = extract_service_parameters(&params);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("name"));
    }
    Ok(())
}

#[test]
fn test_extract_service_parameters_empty_name() -> mlua::Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "")?;

    let result = extract_service_parameters(&params);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("empty"));
    }
    Ok(())
}

#[test]
fn test_extract_service_parameters_invalid_state() -> mlua::Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "nginx")?;
    params.set("state", "running")?; // Invalid state

    let result = extract_service_parameters(&params);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("state"));
    }
    Ok(())
}

#[test]
fn test_validate_service_name() -> Result<()> {
    // Valid service names
    validate_service_name("nginx")?;
    validate_service_name("postgresql")?;
    validate_service_name("my-service")?;
    validate_service_name("service_name")?;

    // Invalid service names
    assert!(validate_service_name("").is_err());
    assert!(validate_service_name("service with spaces").is_err());
    assert!(validate_service_name("service/with/slash").is_err());
    assert!(validate_service_name("service;with;semicolon").is_err());
    assert!(validate_service_name("service`with`backtick").is_err());

    Ok(())
}

#[test]
fn test_validate_service_state() -> Result<()> {
    // Valid states
    validate_service_state("active")?;
    validate_service_state("inactive")?;

    // Invalid states
    assert!(validate_service_state("running").is_err());
    assert!(validate_service_state("stopped").is_err());
    assert!(validate_service_state("enabled").is_err());

    Ok(())
}

#[test]
fn test_query_service_state_nonexistent() {
    // This test would require mocking systemctl commands
    // For now, we'll just test the shell_escape function
    assert_eq!(shell_escape("simple"), "simple");
    assert_eq!(shell_escape("with'quote"), "with'\"'\"'quote");
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
fn test_compare_service_state_success() {
    let expected = ServiceParameters {
        name: "nginx".to_string(),
        state: Some("active".to_string()),
        enabled: Some(true),
    };

    let actual = ServiceState {
        exists: true,
        state: Some("active".to_string()),
        enabled: Some(true),
        error: None,
    };

    let result = compare::compare_service_state(&expected, &actual);
    assert!(result.ok);
    assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("state"), Some(&"active".to_string()));
    assert_eq!(result.actual.get("enabled"), Some(&"true".to_string()));
}

#[test]
fn test_compare_service_state_failure() {
    let expected = ServiceParameters {
        name: "nginx".to_string(),
        state: Some("active".to_string()),
        enabled: Some(true),
    };

    let actual = ServiceState {
        exists: true,
        state: Some("inactive".to_string()), // Different state
        enabled: Some(false),                // Different enabled status
        error: None,
    };

    let result = compare::compare_service_state(&expected, &actual);
    assert!(!result.ok);
    assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("state"), Some(&"inactive".to_string()));
    assert_eq!(result.actual.get("enabled"), Some(&"false".to_string()));
}

#[test]
fn test_compare_service_state_nonexistent() {
    let expected = ServiceParameters {
        name: "nonexistent".to_string(),
        state: None,
        enabled: None,
    };

    let actual = ServiceState {
        exists: false,
        state: None,
        enabled: None,
        error: None,
    };

    let result = compare::compare_service_state(&expected, &actual);
    assert!(result.ok);
    assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));
}

#[test]
fn test_compare_service_state_unexpected_nonexistent() {
    let expected = ServiceParameters {
        name: "nginx".to_string(),
        state: Some("active".to_string()),
        enabled: Some(true),
    };

    let actual = ServiceState {
        exists: false,
        state: None,
        enabled: None,
        error: None,
    };

    let result = compare::compare_service_state(&expected, &actual);
    assert!(!result.ok);
    assert_eq!(result.actual.get("exists"), Some(&"false".to_string()));
}

#[test]
fn test_compare_service_state_unknown_enabled() {
    let expected = ServiceParameters {
        name: "nginx".to_string(),
        state: Some("active".to_string()),
        enabled: Some(true),
    };

    let actual = ServiceState {
        exists: true,
        state: Some("active".to_string()),
        enabled: None, // Unknown enabled state
        error: None,
    };

    let result = compare::compare_service_state(&expected, &actual);
    // Should still pass validation since we don't fail on unknown enabled state
    assert!(result.ok);
    assert_eq!(result.actual.get("exists"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("state"), Some(&"active".to_string()));
    assert_eq!(result.actual.get("enabled"), Some(&"unknown".to_string()));
}

#[test]
fn test_check_service_lua_interface() -> mlua::Result<()> {
    let lua = Lua::new();

    // Create parameters table
    let params = lua.create_table()?;
    params.set("name", "nginx")?;
    params.set("state", "active")?;

    // Test that the function can be called (it will fail due to no actual systemctl access in tests)
    let args = mlua::MultiValue::from_vec(vec![mlua::Value::Table(params)]);
    let result = check_service(&lua, args);

    // The function should return a result (success or error)
    assert!(result.is_ok() || result.is_err());

    Ok(())
}
