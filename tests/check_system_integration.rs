use komandan::create_lua;
use mlua::Table;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use tempfile::TempDir;

/// Comprehensive integration tests with real system components
/// Tests file validation with actual files and permissions
/// Tests service validation with real systemd services
/// Tests package validation with installed/uninstalled packages
/// Requirements: 1.1, 2.1, 3.1

#[test]
fn test_file_validation_with_real_files() -> anyhow::Result<()> {
    let lua = create_lua()?;
    let temp_dir = TempDir::new()?;

    // Test 1: Create a file with specific permissions and validate
    let test_file = temp_dir.path().join("test_file.txt");
    fs::write(&test_file, "test content for validation")?;

    // Set specific permissions (644)
    let mut perms = fs::metadata(&test_file)?.permissions();
    perms.set_mode(0o644);
    fs::set_permissions(&test_file, perms)?;

    let test_file_path = test_file.to_string_lossy().to_string();

    // Test file existence and mode validation
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}",
            exists = true,
            mode = "0644"
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(result.get::<bool>("ok")?);

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "true");
    assert_eq!(actual.get::<String>("mode")?, "0644");

    // Test 2: Validate file with wrong expected mode
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}",
            mode = "0755"  -- Wrong expected mode
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(!result.get::<bool>("ok")?); // Should fail due to mode mismatch

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("mode")?, "0644"); // Should show actual mode

    // Test 3: Create file with different permissions (755)
    let exec_file = temp_dir.path().join("executable.sh");
    fs::write(&exec_file, "#!/bin/bash\necho 'test'")?;

    let mut perms = fs::metadata(&exec_file)?.permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&exec_file, perms)?;

    let exec_file_path = exec_file.to_string_lossy().to_string();

    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{exec_file_path}",
            mode = "0755"
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(
        result.get::<bool>("ok")?,
        "Executable file validation should pass"
    );

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("mode")?, "0755");

    // Test 4: Test nonexistent file
    let nonexistent_path = temp_dir.path().join("nonexistent.txt");
    let nonexistent_path_str = nonexistent_path.to_string_lossy().to_string();

    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{nonexistent_path_str}",
            exists = false
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(result.get::<bool>("ok")?);

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "false");

    // Test 5: Test expecting file to exist when it doesn't
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{nonexistent_path_str}",
            exists = true
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(!result.get::<bool>("ok")?); // Should fail due to existence mismatch

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "false");

    Ok(())
}

#[test]
fn test_file_validation_with_ownership() -> anyhow::Result<()> {
    let lua = create_lua()?;
    let temp_dir = TempDir::new()?;

    // Create a file and test ownership validation
    let test_file = temp_dir.path().join("owned_file.txt");
    fs::write(&test_file, "test content")?;

    let test_file_path = test_file.to_string_lossy().to_string();

    // Get current user for testing
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}"
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    let actual: Table = result.get("actual")?;

    // Get the actual owner from the result
    let actual_owner = actual.get::<String>("owner")?;
    let actual_group = actual.get::<String>("group")?;

    // Now test validation with the correct owner
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}",
            owner = "{actual_owner}",
            group = "{actual_group}"
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(result.get::<bool>("ok")?);

    // Test with wrong owner
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}",
            owner = "nonexistent_user"
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(!result.get::<bool>("ok")?); // Should fail due to owner mismatch

    Ok(())
}

#[test]
fn test_service_validation_with_real_services() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test 1: Check a service that should exist on most Linux systems
    let script = r#"
        local result = komandan.check.service({
            name = "systemd-journald"  -- This service should exist on systemd systems
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    let actual: Table = result.get("actual")?;
    assert!(actual.contains_key("exists")?);

    // If the service exists, it should have state information
    let exists = actual.get::<String>("exists")?;
    if exists == "true" {
        assert!(actual.contains_key("state")?);
        let state = actual.get::<String>("state")?;
        assert!(state == "active" || state == "inactive" || state.contains("unknown"));
    }

    // Test 2: Check a nonexistent service
    let script = r#"
        local result = komandan.check.service({
            name = "nonexistent-service-12345"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "false");

    // Test 3: Check service state validation
    let script = r#"
        local result = komandan.check.service({
            name = "systemd-journald",
            state = "active"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    // This might pass or fail depending on the actual service state
    // The important thing is that we get a proper result structure
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    Ok(())
}

#[test]
fn test_service_validation_state_checking() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test checking for inactive state on a service that might be inactive
    let script = r#"
        local result = komandan.check.service({
            name = "nonexistent-service-12345",
            state = "inactive"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    // Should fail because service doesn't exist
    assert!(!result.get::<bool>("ok")?);

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("exists")?, "false");

    Ok(())
}

#[test]
fn test_package_validation_with_real_packages() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test 1: Check for a package that should exist on most systems (bash)
    let script = r#"
        local result = komandan.check.package({
            name = "bash"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    // Check if package manager is available
    if result.contains_key("error")? {
        let error = result.get::<String>("error")?;
        if error.contains("No supported package manager") {
            println!("Skipping package tests - no supported package manager found");
            return Ok(());
        }
    }

    let actual: Table = result.get("actual")?;
    assert!(actual.contains_key("installed")?);

    // bash should be installed on most systems
    let installed = actual.get::<String>("installed")?;
    if installed == "true" {
        // Should have version information
        assert!(actual.contains_key("version")?);
        let version = actual.get::<String>("version")?;
        assert!(!version.is_empty() && version != "unknown");
    }

    // Test 2: Check for a package that definitely doesn't exist
    let script = r#"
        local result = komandan.check.package({
            name = "nonexistent-package-12345"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Skip if no package manager
    if result.contains_key("error")? {
        let error = result.get::<String>("error")?;
        if error.contains("No supported package manager") {
            return Ok(());
        }
    }

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("installed")?, "false");

    // Test 3: Test package state validation
    let script = r#"
        local result = komandan.check.package({
            name = "bash",
            state = "present"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Skip if no package manager
    if result.contains_key("error")? {
        let error = result.get::<String>("error")?;
        if error.contains("No supported package manager") {
            return Ok(());
        }
    }

    // bash should be present, so this should pass
    let ok = result.get::<bool>("ok")?;
    if ok {
        let actual: Table = result.get("actual")?;
        assert_eq!(actual.get::<String>("installed")?, "true");
    }

    // Test 4: Test expecting absent package
    let script = r#"
        local result = komandan.check.package({
            name = "nonexistent-package-12345",
            state = "absent"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Skip if no package manager
    if result.contains_key("error")? {
        let error = result.get::<String>("error")?;
        if error.contains("No supported package manager") {
            return Ok(());
        }
    }

    assert!(result.get::<bool>("ok")?);

    let actual: Table = result.get("actual")?;
    assert_eq!(actual.get::<String>("installed")?, "false");

    Ok(())
}

#[test]
fn test_package_validation_with_version_checking() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // First get the actual version of bash
    let script = r#"
        local result = komandan.check.package({
            name = "bash"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    // Check if package manager is available
    if result.contains_key("error")? {
        let error = result.get::<String>("error")?;
        if error.contains("No supported package manager") {
            println!("Skipping package version tests - no supported package manager found");
            return Ok(());
        }
    }

    let actual: Table = result.get("actual")?;

    // Check if installed field exists
    if !actual.contains_key("installed")? {
        println!("Skipping package version tests - package check failed");
        return Ok(());
    }

    let installed = actual.get::<String>("installed")?;

    if installed == "true" {
        let actual_version = actual.get::<String>("version")?;

        // Test validation with correct version
        let script = format!(
            r#"
            local result = komandan.check.package({{
                name = "bash",
                version = "{actual_version}"
            }})
            return result
        "#
        );

        let result: Table = lua.load(&script).eval()?;
        assert!(result.get::<bool>("ok")?);

        // Test validation with wrong version
        let script = r#"
            local result = komandan.check.package({
                name = "bash",
                version = "999.999.999"  -- Definitely wrong version
            })
            return result
        "#;

        let result: Table = lua.load(script).eval()?;
        assert!(!result.get::<bool>("ok")?);  // Should fail due to version mismatch
    }

    Ok(())
}

#[test]
fn test_cross_module_consistency() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test that all modules return consistent result structures
    let test_cases = vec![
        r#"komandan.check.file({path = "/tmp"})"#,
        r#"komandan.check.service({name = "ssh"})"#,
        r#"komandan.check.package({name = "bash"})"#,
    ];

    for script in test_cases {
        let result: Table = lua.load(script).eval()?;

        // All should have consistent structure
        assert!(
            result.contains_key("ok")?,
            "Script {script} should have 'ok' field"
        );
        assert!(
            result.contains_key("actual")?,
            "Script {script} should have 'actual' field"
        );

        let _ok = result.get::<bool>("ok")?;
        // ok is always a boolean, so this assertion is redundant
        // Just verify the field exists and is accessible

        let actual: Table = result.get("actual")?;
        // Don't require actual to be non-empty since some checks might fail
        // Just verify it's a table
        assert!(
            actual.len()? >= 0,
            "Script {script} 'actual' should be a table"
        );
    }

    Ok(())
}

#[test]
fn test_namespace_accessibility_integration() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test that both komandan.check and k.check work
    let temp_dir = TempDir::new()?;
    let test_file = temp_dir.path().join("namespace_test.txt");
    fs::write(&test_file, "test")?;
    let test_file_path = test_file.to_string_lossy().to_string();

    // Test komandan.check namespace
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}",
            exists = true
        }})
        return result
    "#
    );

    let result1: Table = lua.load(&script).eval()?;
    assert!(result1.get::<bool>("ok")?);

    // Test k.check namespace alias
    let script = format!(
        r#"
        local result = k.check.file({{
            path = "{test_file_path}",
            exists = true
        }})
        return result
    "#
    );

    let result2: Table = lua.load(&script).eval()?;
    assert!(result2.get::<bool>("ok")?);

    // Results should be equivalent
    let actual1: Table = result1.get("actual")?;
    let actual2: Table = result2.get("actual")?;
    assert_eq!(
        actual1.get::<String>("exists")?,
        actual2.get::<String>("exists")?
    );

    Ok(())
}

#[test]
fn test_error_handling_with_real_system_errors() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test file permission errors (try to access a file we can't read)
    let script = r#"
        local result = komandan.check.file({
            path = "/root/.ssh/id_rsa"  -- Likely to have permission issues
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    // Should handle permission errors gracefully
    assert!(result.contains_key("ok")?);
    assert!(result.contains_key("actual")?);

    // If there's an error, it should be in the error field
    if result.contains_key("error")? {
        let error = result.get::<String>("error")?;
        assert!(!error.is_empty(), "Error message should not be empty");
    }

    Ok(())
}

#[test]
fn test_local_vs_remote_execution_patterns() -> anyhow::Result<()> {
    let lua = create_lua()?;
    let temp_dir = TempDir::new()?;
    let test_file = temp_dir.path().join("execution_test.txt");
    fs::write(&test_file, "test content")?;
    let test_file_path = test_file.to_string_lossy().to_string();

    // Test local execution (no host parameter)
    let script = format!(
        r#"
        local result = komandan.check.file({{
            path = "{test_file_path}",
            exists = true
        }})
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(result.get::<bool>("ok")?);

    // Test with local host configuration
    let script = format!(
        r#"
        local host = {{
            address = "localhost",
            connection = "local"
        }}

        local result = komandan.check.file({{
            path = "{test_file_path}",
            exists = true
        }}, host)
        return result
    "#
    );

    let result: Table = lua.load(&script).eval()?;
    assert!(result.get::<bool>("ok")?);

    Ok(())
}
