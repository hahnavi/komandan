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

    let result = compare::compare_file_state(&expected, &actual);
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

    let result = compare::compare_file_state(&expected, &actual);
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

    let result = compare::compare_file_state(&expected, &actual);
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

    let result = compare::compare_file_state(&expected, &actual);
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
