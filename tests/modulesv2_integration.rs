use komandan::*;
use mlua::Table;
use std::env;

/// Test `ModulesV2` basic functionality with local connections
#[test]
fn test_modulesv2_local_execution() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test cmd module with local execution
    let script = r#"
        local result = k.mod.cmd({cmd = "echo 'ModulesV2 test'"})
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert_eq!(result.get::<i32>("exit_code")?, 0);
    assert!(result.get::<String>("stdout")?.contains("ModulesV2 test"));

    Ok(())
}

/// Test `ModulesV2` with explicit local host configuration
#[test]
fn test_modulesv2_explicit_local_host() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local host = {
            address = "localhost",
            connection = "local"
        }

        local result = k.mod.cmd({cmd = "echo 'explicit local'"}, host)
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert_eq!(result.get::<i32>("exit_code")?, 0);
    assert!(result.get::<String>("stdout")?.contains("explicit local"));

    Ok(())
}

/// Test `ModulesV2` file operations
#[test]
fn test_modulesv2_file_operations() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local temp_file = "/tmp/modulesv2_test_" .. os.time()

        -- Create file using ModulesV2
        local create_result = k.mod.file({
            path = temp_file,
            content = "ModulesV2 file test\n",
            mode = "0644"
        })

        -- Verify file was created
        local verify_result = k.mod.cmd({cmd = "cat " .. temp_file})

        -- Cleanup
        k.mod.cmd({cmd = "rm -f " .. temp_file})

        return {
            create = create_result,
            verify = verify_result
        }
    "#;

    let results: Table = lua.load(script).eval()?;
    let create_result: Table = results.get("create")?;
    let verify_result: Table = results.get("verify")?;

    assert_eq!(create_result.get::<i32>("exit_code")?, 0);
    assert_eq!(verify_result.get::<i32>("exit_code")?, 0);
    assert!(
        verify_result
            .get::<String>("stdout")?
            .contains("ModulesV2 file test")
    );

    Ok(())
}

/// Test `ModulesV2` apt module (if available)
#[test]
fn test_modulesv2_apt_module() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test apt module with a safe operation (update cache)
    let script = r#"
        -- Test apt module availability
        local result = k.mod.apt({
            package = "curl",
            state = "present",
            update_cache = false  -- Don't actually update cache in tests
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    // The result might fail if not running as root, but the module should execute
    assert!(result.contains_key("exit_code")?);
    assert!(result.contains_key("stdout")?);
    assert!(result.contains_key("stderr")?);

    Ok(())
}

/// Test `ModulesV2` `systemd_service` module
#[test]
fn test_modulesv2_systemd_service_module() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test systemd_service module with status check (safe operation)
        local result = k.mod.systemd_service({
            name = "ssh",
            action = "status"
        })
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    // The result might vary depending on system, but module should execute
    assert!(result.contains_key("exit_code")?);
    assert!(result.contains_key("stdout")?);
    assert!(result.contains_key("stderr")?);

    Ok(())
}

/// Test `ModulesV2` with SSH connections (if SSH test environment is available)
#[test]
fn test_modulesv2_ssh_execution() -> anyhow::Result<()> {
    // Skip if SSH test environment not available
    if env::var("KOMANDAN_SSH_TEST").is_err() {
        return Ok(());
    }

    let lua = create_lua()?;

    let script = r#"
        local host = {
            name = "ssh-test-server",
            address = "127.0.0.1",
            port = 22,
            user = "usertest",
            private_key_file = os.getenv("HOME") .. "/.ssh/id_ed25519",
            connection = "ssh"
        }

        local result = k.mod.cmd({cmd = "echo 'ModulesV2 SSH test'"}, host)
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert_eq!(result.get::<i32>("exit_code")?, 0);
    assert!(
        result
            .get::<String>("stdout")?
            .contains("ModulesV2 SSH test")
    );

    Ok(())
}

/// Test interaction between `ModulesV1` and `ModulesV2`
#[test]
fn test_modulesv1_modulesv2_coexistence() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local host = {
            address = "localhost",
            connection = "local"
        }

        -- Test ModulesV1 (traditional approach)
        local task_v1 = {
            name = "ModulesV1 test",
            komandan.modules.cmd({
                cmd = "echo 'ModulesV1 works'"
            })
        }
        local result_v1 = komandan.komando(task_v1, host)

        -- Test ModulesV2 (direct approach)
        local result_v2 = k.mod.cmd({cmd = "echo 'ModulesV2 works'"}, host)

        -- Test k.mods alias (should be same as komandan.modules)
        -- Use the traditional komando approach for k.mods
        local task_alias = {
            name = "k.mods alias test",
            k.mods.cmd({cmd = "echo 'k.mods alias works'"})
        }
        local result_alias = komandan.komando(task_alias, host)

        return {
            v1 = result_v1,
            v2 = result_v2,
            alias = result_alias
        }
    "#;

    let results: Table = lua.load(script).eval()?;
    let result_v1: Table = results.get("v1")?;
    let result_v2: Table = results.get("v2")?;
    let result_alias: Table = results.get("alias")?;

    // All should succeed
    assert_eq!(result_v1.get::<i32>("exit_code")?, 0);
    assert_eq!(result_v2.get::<i32>("exit_code")?, 0);
    assert_eq!(result_alias.get::<i32>("exit_code")?, 0);

    // Check outputs
    assert!(
        result_v1
            .get::<String>("stdout")?
            .contains("ModulesV1 works")
    );
    assert!(
        result_v2
            .get::<String>("stdout")?
            .contains("ModulesV2 works")
    );
    assert!(
        result_alias
            .get::<String>("stdout")?
            .contains("k.mods alias works")
    );

    Ok(())
}

/// Test `ModulesV2` error handling
#[test]
fn test_modulesv2_error_handling() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test with invalid command
    let script = r#"
        local result = k.mod.cmd({cmd = "nonexistent_command_12345"})
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    // Should have non-zero exit code
    assert_ne!(result.get::<i32>("exit_code")?, 0);
    assert!(result.contains_key("stderr")?);

    Ok(())
}

/// Test `ModulesV2` parameter validation
#[test]
fn test_modulesv2_parameter_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test with missing required parameter
    let script = r"
        local success, error = pcall(function()
            return k.mod.cmd({})  -- Missing 'cmd' parameter
        end)
        return {success = success, error = tostring(error)}
    ";

    let result: Table = lua.load(script).eval()?;
    assert!(!result.get::<bool>("success")?);
    let error_msg = result.get::<String>("error")?;
    assert!(error_msg.contains("cmd") || error_msg.contains("required"));

    Ok(())
}

/// Test `ModulesV2` with various host configurations
#[test]
fn test_modulesv2_host_configurations() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local test_cases = {
            -- Auto-detection: localhost should use local
            {
                host = {address = "localhost"},
                expected_success = true
            },
            -- Auto-detection: 127.0.0.1 should use local
            {
                host = {address = "127.0.0.1"},
                expected_success = true
            },
            -- Explicit local connection
            {
                host = {address = "localhost", connection = "local"},
                expected_success = true
            }
        }

        local results = {}
        for i, test_case in ipairs(test_cases) do
            local success, result = pcall(function()
                return k.mod.cmd({cmd = "echo 'test " .. i .. "'"}, test_case.host)
            end)

            table.insert(results, {
                test_index = i,
                success = success,
                result = success and result or nil,
                error = not success and tostring(result) or nil,
                expected_success = test_case.expected_success
            })
        end

        return results
    "#;

    let results: Table = lua.load(script).eval()?;

    // Check each test case
    for pair in results.pairs::<i32, Table>() {
        let (_, test_result) = pair?;
        let success = test_result.get::<bool>("success")?;
        let expected_success = test_result.get::<bool>("expected_success")?;
        let test_index = test_result.get::<i32>("test_index")?;

        if expected_success {
            assert!(success, "Test case {test_index} should have succeeded");
            if success {
                let result: Table = test_result.get("result")?;
                assert_eq!(result.get::<i32>("exit_code")?, 0);
            }
        }
    }

    Ok(())
}

/// Test `ModulesV2` result structure compatibility with `ModulesV1`
#[test]
fn test_modulesv2_result_structure_compatibility() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local host = {address = "localhost", connection = "local"}

        -- Get result from ModulesV1
        local task_v1 = {
            name = "Test task",
            komandan.modules.cmd({cmd = "echo 'test'"})
        }
        local result_v1 = komandan.komando(task_v1, host)

        -- Get result from ModulesV2
        local result_v2 = k.mod.cmd({cmd = "echo 'test'"}, host)

        return {
            v1 = result_v1,
            v2 = result_v2
        }
    "#;

    let results: Table = lua.load(script).eval()?;
    let result_v1: Table = results.get("v1")?;
    let result_v2: Table = results.get("v2")?;

    // Both should have the same structure
    let required_fields = ["stdout", "stderr", "exit_code"];

    for field in &required_fields {
        assert!(
            result_v1.contains_key(*field)?,
            "ModulesV1 missing field: {field}",
        );
        assert!(
            result_v2.contains_key(*field)?,
            "ModulesV2 missing field: {field}",
        );
    }

    // Values should be compatible types
    assert_eq!(
        result_v1.get::<i32>("exit_code")?,
        result_v2.get::<i32>("exit_code")?
    );
    assert_eq!(
        result_v1.get::<String>("stdout")?,
        result_v2.get::<String>("stdout")?
    );

    Ok(())
}

/// Test `ModulesV2` with multiple modules in sequence
#[test]
fn test_modulesv2_multiple_modules_sequence() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local temp_file = "/tmp/modulesv2_sequence_test_" .. os.time()

        -- Step 1: Create file
        local step1 = k.mod.file({
            path = temp_file,
            content = "sequence test\n"
        })

        -- Step 2: Verify file exists
        local step2 = k.mod.cmd({cmd = "test -f " .. temp_file})

        -- Step 3: Read file content
        local step3 = k.mod.cmd({cmd = "cat " .. temp_file})

        -- Step 4: Cleanup
        local step4 = k.mod.cmd({cmd = "rm -f " .. temp_file})

        return {
            step1 = step1,
            step2 = step2,
            step3 = step3,
            step4 = step4
        }
    "#;

    let results: Table = lua.load(script).eval()?;

    // All steps should succeed
    let step1: Table = results.get("step1")?;
    let step2: Table = results.get("step2")?;
    let step3: Table = results.get("step3")?;
    let step4: Table = results.get("step4")?;

    assert_eq!(step1.get::<i32>("exit_code")?, 0);
    assert_eq!(step2.get::<i32>("exit_code")?, 0);
    assert_eq!(step3.get::<i32>("exit_code")?, 0);
    assert_eq!(step4.get::<i32>("exit_code")?, 0);

    // Step 3 should contain the file content
    assert!(step3.get::<String>("stdout")?.contains("sequence test"));

    Ok(())
}
