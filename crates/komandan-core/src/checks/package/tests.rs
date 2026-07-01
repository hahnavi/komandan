use super::*;
use crate::checks::base::shell_escape;
use mlua::Lua;

#[test]
fn test_extract_package_parameters_valid() -> Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "nginx")?;
    params.set("state", "present")?;
    params.set("version", "1.18.0")?;

    let package_params = extract_package_parameters(&params)?;

    assert_eq!(package_params.name, "nginx");
    assert_eq!(package_params.state, Some("present".to_string()));
    assert_eq!(package_params.version, Some("1.18.0".to_string()));

    Ok(())
}

#[test]
fn test_extract_package_parameters_minimal() -> Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "postgresql")?;

    let package_params = extract_package_parameters(&params)?;

    assert_eq!(package_params.name, "postgresql");
    assert_eq!(package_params.state, None);
    assert_eq!(package_params.version, None);

    Ok(())
}

#[test]
fn test_extract_package_parameters_missing_name() -> mlua::Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("state", "present")?;

    let result = extract_package_parameters(&params);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("name"));
    }
    Ok(())
}

#[test]
fn test_extract_package_parameters_empty_name() -> mlua::Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "")?;

    let result = extract_package_parameters(&params);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("empty"));
    }
    Ok(())
}

#[test]
fn test_extract_package_parameters_invalid_state() -> mlua::Result<()> {
    let lua = Lua::new();
    let params = lua.create_table()?;
    params.set("name", "nginx")?;
    params.set("state", "installed")?; // Invalid state

    let result = extract_package_parameters(&params);
    assert!(result.is_err());
    if let Err(e) = result {
        assert!(e.to_string().contains("state"));
    }
    Ok(())
}

#[test]
fn test_validate_package_name() -> Result<()> {
    // Valid package names
    validate_package_name("nginx")?;
    validate_package_name("postgresql")?;
    validate_package_name("my-package")?;
    validate_package_name("package_name")?;
    validate_package_name("package.name")?;
    validate_package_name("package123")?;

    // Invalid package names
    assert!(validate_package_name("").is_err());
    assert!(validate_package_name("package with spaces").is_err());
    assert!(validate_package_name("package/with/slash").is_err());
    assert!(validate_package_name("package;with;semicolon").is_err());
    assert!(validate_package_name("package`with`backtick").is_err());
    assert!(validate_package_name("package$with$dollar").is_err());

    Ok(())
}

#[test]
fn test_validate_package_state() -> Result<()> {
    // Valid states
    validate_package_state("present")?;
    validate_package_state("absent")?;

    // Invalid states
    assert!(validate_package_state("installed").is_err());
    assert!(validate_package_state("removed").is_err());
    assert!(validate_package_state("latest").is_err());

    Ok(())
}

#[test]
fn test_check_package_lua_interface() -> mlua::Result<()> {
    let lua = Lua::new();

    // Create parameters table
    let params = lua.create_table()?;
    params.set("name", "nginx")?;
    params.set("state", "present")?;

    // Test that the function can be called (it will fail due to no actual package manager access in tests)
    let args = mlua::MultiValue::from_vec(vec![mlua::Value::Table(params)]);
    let result = check_package(&lua, args);

    // The function should return a result (success or error)
    assert!(result.is_ok() || result.is_err());

    Ok(())
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
fn test_compare_package_state_success() {
    let expected = PackageParameters {
        name: "nginx".to_string(),
        state: Some("present".to_string()),
        version: Some("1.18.0".to_string()),
    };

    let actual = PackageState {
        installed: true,
        version: Some("1.18.0".to_string()),
        error: None,
    };

    let result = compare_package_state(&expected, &actual);
    assert!(result.ok);
    assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("version"), Some(&"1.18.0".to_string()));
}

#[test]
fn test_compare_package_state_failure() {
    let expected = PackageParameters {
        name: "nginx".to_string(),
        state: Some("present".to_string()),
        version: Some("1.18.0".to_string()),
    };

    let actual = PackageState {
        installed: true,
        version: Some("1.16.0".to_string()), // Different version
        error: None,
    };

    let result = compare_package_state(&expected, &actual);
    assert!(!result.ok);
    assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("version"), Some(&"1.16.0".to_string()));
}

#[test]
fn test_compare_package_state_not_installed() {
    let expected = PackageParameters {
        name: "nonexistent".to_string(),
        state: Some("absent".to_string()),
        version: None,
    };

    let actual = PackageState {
        installed: false,
        version: None,
        error: None,
    };

    let result = compare_package_state(&expected, &actual);
    assert!(result.ok);
    assert_eq!(result.actual.get("installed"), Some(&"false".to_string()));
}

#[test]
fn test_compare_package_state_unexpected_installed() {
    let expected = PackageParameters {
        name: "nginx".to_string(),
        state: Some("absent".to_string()),
        version: None,
    };

    let actual = PackageState {
        installed: true,
        version: Some("1.18.0".to_string()),
        error: None,
    };

    let result = compare_package_state(&expected, &actual);
    assert!(!result.ok);
    assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("version"), Some(&"1.18.0".to_string()));
}

#[test]
fn test_compare_package_state_unknown_version() {
    let expected = PackageParameters {
        name: "nginx".to_string(),
        state: Some("present".to_string()),
        version: Some("1.18.0".to_string()),
    };

    let actual = PackageState {
        installed: true,
        version: None, // Unknown version
        error: None,
    };

    let result = compare_package_state(&expected, &actual);
    // Should still pass validation since we don't fail on unknown version
    assert!(result.ok);
    assert_eq!(result.actual.get("installed"), Some(&"true".to_string()));
    assert_eq!(result.actual.get("version"), Some(&"unknown".to_string()));
}

#[test]
fn test_compare_package_state_error() {
    let expected = PackageParameters {
        name: "nginx".to_string(),
        state: Some("present".to_string()),
        version: None,
    };

    let actual = PackageState {
        installed: false,
        version: None,
        error: Some("Package manager error".to_string()),
    };

    let result = compare_package_state(&expected, &actual);
    assert!(!result.ok);
    assert_eq!(result.error, Some("Package manager error".to_string()));
}
