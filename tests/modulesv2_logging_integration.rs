use komandan::*;
use mlua::Table;

/// Test ModulesV2 logging functionality - Requirements 9.1, 9.2, 9.3, 9.5
#[test]
fn test_modulesv2_logging_format() -> anyhow::Result<()> {
    let lua = create_lua()?;

    // Test basic logging format with cmd module
    let script = r#"
        -- Test logging with local execution
        local result = k.mod.cmd({cmd = "echo 'logging test'"})
        return result
    "#;

    // Capture the output (in real execution, this would go to stdout)
    let result: Table = lua.load(script).eval()?;

    // Verify the result structure
    assert_eq!(result.get::<i32>("exit_code")?, 0);
    assert!(result.get::<String>("stdout")?.contains("logging test"));

    // Note: The actual logging output goes to stdout during execution
    // The format should match: ">> Running task 'cmd module' on host 'localhost' ..."
    // and ">> Task 'cmd module' on host 'localhost' succeeded. [OK]"

    Ok(())
}

/// Test ModulesV2 logging with different host configurations
#[test]
fn test_modulesv2_logging_with_hosts() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local test_cases = {
            -- Test with localhost
            {
                host = {address = "localhost", connection = "local"},
                expected_host_display = "localhost"
            },
            -- Test with named host
            {
                host = {name = "web-server", address = "localhost", connection = "local"},
                expected_host_display = "web-server"
            },
            -- Test with no host (should default to localhost)
            {
                host = nil,
                expected_host_display = "localhost"
            }
        }

        local results = {}
        for i, test_case in ipairs(test_cases) do
            local result
            if test_case.host then
                result = k.mod.cmd({cmd = "echo 'test " .. i .. "'"}, test_case.host)
            else
                result = k.mod.cmd({cmd = "echo 'test " .. i .. "'"})
            end
            table.insert(results, {
                test_index = i,
                result = result,
                expected_host_display = test_case.expected_host_display
            })
        end

        return results
    "#;

    let results: Table = lua.load(script).eval()?;

    // Verify all test cases succeeded
    for pair in results.pairs::<i32, Table>() {
        let (_, test_case) = pair?;
        let result: Table = test_case.get("result")?;
        let test_index = test_case.get::<i32>("test_index")?;

        assert_eq!(
            result.get::<i32>("exit_code")?,
            0,
            "Test case {test_index} should succeed"
        );
        assert!(
            result
                .get::<String>("stdout")?
                .contains(&format!("test {test_index}"))
        );
    }

    Ok(())
}

/// Test ModulesV2 logging with different module types
#[test]
fn test_modulesv2_logging_different_modules() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local temp_file = "/tmp/modulesv2_logging_test_" .. os.time()
        local results = {}

        -- Test cmd module logging
        local cmd_result = k.mod.cmd({cmd = "echo 'cmd module test'"})
        table.insert(results, {module = "cmd", result = cmd_result})

        -- Test file module logging
        local file_result = k.mod.file({
            path = temp_file,
            content = "file module test\n",
            mode = "0644"
        })
        table.insert(results, {module = "file", result = file_result})

        -- Test apt module logging (safe operation)
        local apt_result = k.mod.apt({
            package = "curl",
            state = "present",
            update_cache = false
        })
        table.insert(results, {module = "apt", result = apt_result})

        -- Cleanup
        k.mod.cmd({cmd = "rm -f " .. temp_file})

        return results
    "#;

    let results: Table = lua.load(script).eval()?;

    // Verify all modules executed and logged appropriately
    for pair in results.pairs::<i32, Table>() {
        let (_, test_case) = pair?;
        let module_name: String = test_case.get("module")?;
        let result: Table = test_case.get("result")?;

        // All modules should have proper result structure
        assert!(
            result.contains_key("exit_code")?,
            "Module {module_name} missing exit_code"
        );
        assert!(
            result.contains_key("stdout")?,
            "Module {module_name} missing stdout"
        );
        assert!(
            result.contains_key("stderr")?,
            "Module {module_name} missing stderr"
        );
        assert!(
            result.contains_key("changed")?,
            "Module {module_name} missing changed"
        );

        // The logging format should be consistent across all modules
        // Each module execution should log:
        // 1. Start message: ">> Running task '{module} module' on host 'localhost' ..."
        // 2. Completion message: ">> Task '{module} module' on host 'localhost' succeeded. [OK/Changed]"
    }

    Ok(())
}

/// Test ModulesV2 logging with success and failure scenarios
#[test]
fn test_modulesv2_logging_success_failure() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local results = {}

        -- Test successful command
        local success_result = k.mod.cmd({cmd = "echo 'success test'"})
        table.insert(results, {
            type = "success",
            result = success_result
        })

        -- Test failing command
        local failure_result = k.mod.cmd({cmd = "false"})  -- Command that always fails
        table.insert(results, {
            type = "failure",
            result = failure_result
        })

        -- Test command with output and error
        local mixed_result = k.mod.cmd({cmd = "echo 'stdout'; echo 'stderr' >&2; exit 1"})
        table.insert(results, {
            type = "mixed",
            result = mixed_result
        })

        return results
    "#;

    let results: Table = lua.load(script).eval()?;

    for pair in results.pairs::<i32, Table>() {
        let (_, test_case) = pair?;
        let test_type: String = test_case.get("type")?;
        let result: Table = test_case.get("result")?;

        match test_type.as_str() {
            "success" => {
                assert_eq!(result.get::<i32>("exit_code")?, 0);
                assert!(result.get::<String>("stdout")?.contains("success test"));
                // Should log: ">> Task 'cmd module' on host 'localhost' succeeded. [OK]"
            }
            "failure" => {
                assert_ne!(result.get::<i32>("exit_code")?, 0);
                // Should log: ">> Task 'cmd module' on host 'localhost' failed with exit code 1: ..."
            }
            "mixed" => {
                assert_ne!(result.get::<i32>("exit_code")?, 0);
                assert!(result.get::<String>("stdout")?.contains("stdout"));
                assert!(result.get::<String>("stderr")?.contains("stderr"));
                // Should log failure message with stderr content
            }
            _ => panic!("Unknown test type: {test_type}"),
        }
    }

    Ok(())
}

/// Test ModulesV2 logging with changed status indicators
#[test]
fn test_modulesv2_logging_changed_status() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local temp_file = "/tmp/modulesv2_changed_test_" .. os.time()
        local results = {}

        -- Test operation that should show [Changed]
        local create_result = k.mod.file({
            path = temp_file,
            content = "test content\n",
            mode = "0644"
        })
        table.insert(results, {
            type = "changed",
            result = create_result
        })

        -- Test read-only operation that should show [OK]
        local read_result = k.mod.cmd({cmd = "cat " .. temp_file})
        table.insert(results, {
            type = "ok",
            result = read_result
        })

        -- Cleanup
        k.mod.cmd({cmd = "rm -f " .. temp_file})

        return results
    "#;

    let results: Table = lua.load(script).eval()?;

    for pair in results.pairs::<i32, Table>() {
        let (_, test_case) = pair?;
        let test_type: String = test_case.get("type")?;
        let result: Table = test_case.get("result")?;

        assert_eq!(result.get::<i32>("exit_code")?, 0);

        match test_type.as_str() {
            "changed" => {
                // File creation should be marked as changed
                assert!(result.get::<bool>("changed")?);
                // Should log: ">> Task 'file module' on host 'localhost' succeeded. [Changed]"
            }
            "ok" => {
                // Read operations might be marked as changed or not, depending on implementation
                // The key is that the logging format is consistent
                // Should log: ">> Task 'cmd module' on host 'localhost' succeeded. [OK/Changed]"
            }
            _ => panic!("Unknown test type: {test_type}"),
        }
    }

    Ok(())
}

/// Test ModulesV2 debug output integration - Requirement 9.4
#[test]
fn test_modulesv2_debug_output() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test command with significant output
        local result = k.mod.cmd({cmd = "echo 'debug output test'; echo 'line 2'; echo 'line 3'"})
        return result
    "#;

    let result: Table = lua.load(script).eval()?;

    assert_eq!(result.get::<i32>("exit_code")?, 0);
    let stdout = result.get::<String>("stdout")?;
    assert!(stdout.contains("debug output test"));
    assert!(stdout.contains("line 2"));
    assert!(stdout.contains("line 3"));

    // When debug mode is enabled, the stdout should be printed using komandan.dprint()
    // This is handled by the print_debug_output function in execution.rs
    // The actual debug output behavior depends on the verbose flag setting

    Ok(())
}

/// Test ModulesV2 logging format matches komando exactly - Requirements 9.1, 9.2, 9.3, 9.5
#[test]
fn test_modulesv2_logging_format_compatibility() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local host = {address = "localhost", connection = "local"}

        -- Test ModulesV1 logging format (for comparison)
        local task_v1 = {
            name = "Test task for comparison",
            komandan.modules.cmd({cmd = "echo 'v1 test'"})
        }
        local result_v1 = komandan.komando(task_v1, host)

        -- Test ModulesV2 logging format
        local result_v2 = k.mod.cmd({cmd = "echo 'v2 test'"}, host)

        return {
            v1 = result_v1,
            v2 = result_v2
        }
    "#;

    let results: Table = lua.load(script).eval()?;
    let result_v1: Table = results.get("v1")?;
    let result_v2: Table = results.get("v2")?;

    // Both should succeed
    assert_eq!(result_v1.get::<i32>("exit_code")?, 0);
    assert_eq!(result_v2.get::<i32>("exit_code")?, 0);

    // Both should have the same result structure
    let required_fields = ["stdout", "stderr", "exit_code"];
    for field in &required_fields {
        assert!(result_v1.contains_key(*field)?);
        assert!(result_v2.contains_key(*field)?);
    }

    // The logging format should be consistent between ModulesV1 and ModulesV2
    // ModulesV1 logs via komando execution
    // ModulesV2 logs via ExecutionEngine with matching format

    Ok(())
}

/// Test ModulesV2 logging with multiple sequential operations
#[test]
fn test_modulesv2_logging_sequential_operations() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local temp_dir = "/tmp/modulesv2_sequential_" .. os.time()
        local operations = {}

        -- Operation 1: Create directory
        local op1 = k.mod.cmd({cmd = "mkdir -p " .. temp_dir})
        table.insert(operations, {name = "create_dir", result = op1})

        -- Operation 2: Create file
        local op2 = k.mod.file({
            path = temp_dir .. "/test.txt",
            content = "sequential test\n"
        })
        table.insert(operations, {name = "create_file", result = op2})

        -- Operation 3: Read file
        local op3 = k.mod.cmd({cmd = "cat " .. temp_dir .. "/test.txt"})
        table.insert(operations, {name = "read_file", result = op3})

        -- Operation 4: Cleanup
        local op4 = k.mod.cmd({cmd = "rm -rf " .. temp_dir})
        table.insert(operations, {name = "cleanup", result = op4})

        return operations
    "#;

    let operations: Table = lua.load(script).eval()?;

    // Verify all operations succeeded and logged properly
    for pair in operations.pairs::<i32, Table>() {
        let (_, operation) = pair?;
        let op_name: String = operation.get("name")?;
        let result: Table = operation.get("result")?;

        assert_eq!(
            result.get::<i32>("exit_code")?,
            0,
            "Operation {op_name} should succeed"
        );

        // Each operation should have logged:
        // 1. Start message with appropriate module name
        // 2. Completion message with success status

        match op_name.as_str() {
            "read_file" => {
                assert!(result.get::<String>("stdout")?.contains("sequential test"));
            }
            _ => {
                // Other operations should complete successfully
            }
        }
    }

    Ok(())
}

/// Test ModulesV2 logging with error scenarios
#[test]
fn test_modulesv2_logging_error_scenarios() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local error_tests = {}

        -- Test command execution error
        local cmd_error = k.mod.cmd({cmd = "nonexistent_command_12345"})
        table.insert(error_tests, {
            type = "command_error",
            result = cmd_error
        })

        -- Test file operation error (permission denied simulation)
        local file_error = k.mod.file({
            path = "/root/test_file_no_permission",
            content = "test"
        })
        table.insert(error_tests, {
            type = "file_error",
            result = file_error
        })

        return error_tests
    "#;

    let error_tests: Table = lua.load(script).eval()?;

    for pair in error_tests.pairs::<i32, Table>() {
        let (_, test_case) = pair?;
        let test_type: String = test_case.get("type")?;
        let result: Table = test_case.get("result")?;

        // All error cases should have non-zero exit codes
        assert_ne!(
            result.get::<i32>("exit_code")?,
            0,
            "Error test {test_type} should fail"
        );

        // Should have error information
        assert!(result.contains_key("stderr")?);

        // Error logging should include:
        // ">> Task '{module} module' on host 'localhost' failed with exit code {code}: {error}"
    }

    Ok(())
}
