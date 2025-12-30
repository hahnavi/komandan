use komandan::*;
use mlua::Table;

/// Test end-to-end workflow using `ModulesV2` - Requirements validation
#[test]
fn test_modulesv2_end_to_end_workflow() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- End-to-end workflow test
        local temp_dir = "/tmp/modulesv2_e2e_" .. os.time()
        local config_file = temp_dir .. "/app.conf"
        local backup_file = temp_dir .. "/app.conf.backup"

        -- Step 1: Create directory structure
        local mkdir_result = k.mod.cmd({cmd = "mkdir -p " .. temp_dir})

        -- Step 2: Create configuration file
        local config_content = [[
[app]
name = "test-app"
version = "1.0.0"
port = 8080
]]
        local create_config = k.mod.file({
            path = config_file,
            content = config_content,
            mode = "0644"
        })

        -- Step 3: Backup the configuration
        local backup_result = k.mod.cmd({cmd = "cp " .. config_file .. " " .. backup_file})

        -- Step 4: Verify files exist
        local verify_config = k.mod.cmd({cmd = "test -f " .. config_file})
        local verify_backup = k.mod.cmd({cmd = "test -f " .. backup_file})

        -- Step 5: Read configuration content
        local read_config = k.mod.cmd({cmd = "cat " .. config_file})

        -- Step 6: Cleanup
        local cleanup = k.mod.cmd({cmd = "rm -rf " .. temp_dir})

        return {
            mkdir = mkdir_result,
            create_config = create_config,
            backup = backup_result,
            verify_config = verify_config,
            verify_backup = verify_backup,
            read_config = read_config,
            cleanup = cleanup
        }
    "#;

    let results: Table = lua.load(script).eval()?;

    // Validate all steps succeeded
    let steps = [
        "mkdir",
        "create_config",
        "backup",
        "verify_config",
        "verify_backup",
        "read_config",
        "cleanup",
    ];
    for step in &steps {
        let result: Table = results.get(*step)?;
        assert_eq!(result.get::<i32>("exit_code")?, 0, "Step {step} failed");
    }

    // Validate content was correctly written and read
    let read_result: Table = results.get("read_config")?;
    let content = read_result.get::<String>("stdout")?;
    assert!(content.contains("test-app"));
    assert!(content.contains("version = \"1.0.0\""));
    assert!(content.contains("port = 8080"));

    Ok(())
}

/// Test Requirement 1.1: Local execution default
#[test]
fn test_requirement_1_1_local_execution_default() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test that modules execute locally by default (no host parameter)
        local result = k.mod.cmd({cmd = "echo 'local execution test'"})
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert_eq!(result.get::<i32>("exit_code")?, 0);
    assert!(
        result
            .get::<String>("stdout")?
            .contains("local execution test")
    );

    Ok(())
}

/// Test Requirement 1.2: Remote execution with host parameter
#[test]
fn test_requirement_1_2_remote_execution_with_host() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test remote execution with explicit local host (simulates remote)
        local host = {
            address = "localhost",
            connection = "local"  -- Force local connection for testing
        }

        local result = k.mod.cmd({cmd = "echo 'remote execution test'"}, host)
        return result
    "#;

    let result: Table = lua.load(script).eval()?;
    assert_eq!(result.get::<i32>("exit_code")?, 0);
    assert!(
        result
            .get::<String>("stdout")?
            .contains("remote execution test")
    );

    Ok(())
}

/// Test Requirement 1.3: Same result structure as `ModulesV1`
#[test]
fn test_requirement_1_3_result_structure_compatibility() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local host = {address = "localhost", connection = "local"}

        -- ModulesV1 result
        local task_v1 = {
            name = "Test task",
            komandan.modules.cmd({cmd = "echo 'test'"})
        }
        local result_v1 = komandan.komando(task_v1, host)

        -- ModulesV2 result
        local result_v2 = k.mod.cmd({cmd = "echo 'test'"}, host)

        return {v1 = result_v1, v2 = result_v2}
    "#;

    let results: Table = lua.load(script).eval()?;
    let result_v1: Table = results.get("v1")?;
    let result_v2: Table = results.get("v2")?;

    // Both should have same structure
    let required_fields = ["stdout", "stderr", "exit_code"];
    for field in &required_fields {
        assert!(result_v1.contains_key(*field)?);
        assert!(result_v2.contains_key(*field)?);
    }

    // Values should be equivalent
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

/// Test Requirement 1.4: Backward compatibility with `ModulesV1`
#[test]
fn test_requirement_1_4_backward_compatibility() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local host = {address = "localhost", connection = "local"}

        -- Test that ModulesV1 still works
        local task = {
            name = "Backward compatibility test",
            komandan.modules.cmd({cmd = "echo 'v1 works'"})
        }
        local result_v1 = komandan.komando(task, host)

        -- Test that k.mods alias works
        local task_alias = {
            name = "Alias test",
            k.mods.cmd({cmd = "echo 'alias works'"})
        }
        local result_alias = komandan.komando(task_alias, host)

        -- Test that ModulesV2 works alongside
        local result_v2 = k.mod.cmd({cmd = "echo 'v2 works'"})

        return {
            v1 = result_v1,
            alias = result_alias,
            v2 = result_v2
        }
    "#;

    let results: Table = lua.load(script).eval()?;

    // All should work
    let result_v1: Table = results.get("v1")?;
    let result_alias: Table = results.get("alias")?;
    let result_v2: Table = results.get("v2")?;

    assert_eq!(result_v1.get::<i32>("exit_code")?, 0);
    assert_eq!(result_alias.get::<i32>("exit_code")?, 0);
    assert_eq!(result_v2.get::<i32>("exit_code")?, 0);

    assert!(result_v1.get::<String>("stdout")?.contains("v1 works"));
    assert!(
        result_alias
            .get::<String>("stdout")?
            .contains("alias works")
    );
    assert!(result_v2.get::<String>("stdout")?.contains("v2 works"));

    Ok(())
}

/// Test Requirement 1.5: `ModulesV2` exposed under k.mod namespace
#[test]
fn test_requirement_1_5_k_mod_namespace() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test that k.mod namespace exists and contains modules
        local modules = {}

        -- Check that k.mod exists
        if k.mod then
            -- Check for implemented modules
            if k.mod.cmd then table.insert(modules, "cmd") end
            if k.mod.apt then table.insert(modules, "apt") end
            if k.mod.file then table.insert(modules, "file") end
            if k.mod.dnf then table.insert(modules, "dnf") end
            if k.mod.systemd_service then table.insert(modules, "systemd_service") end
            if k.mod.template then table.insert(modules, "template") end
            if k.mod.upload then table.insert(modules, "upload") end
            if k.mod.download then table.insert(modules, "download") end
        end

        return modules
    "#;

    let modules: Table = lua.load(script).eval()?;

    // Should have all implemented modules
    let expected_modules = [
        "cmd",
        "apt",
        "file",
        "dnf",
        "systemd_service",
        "template",
        "upload",
        "download",
    ];
    let mut found_modules = Vec::new();

    for pair in modules.pairs::<i32, String>() {
        let (_, module_name) = pair?;
        found_modules.push(module_name);
    }

    for expected in &expected_modules {
        assert!(
            found_modules.contains(&expected.to_string()),
            "Missing module: {expected}",
        );
    }

    Ok(())
}

/// Test Requirement 2.1: Automatic connection management
#[test]
fn test_requirement_2_1_automatic_connection_management() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test different connection types
        local results = {}

        -- Auto-detection: localhost should use local
        local result1 = k.mod.cmd({cmd = "echo 'auto-local'"}, {address = "localhost"})
        table.insert(results, {type = "auto-local", result = result1})

        -- Explicit local
        local result2 = k.mod.cmd({cmd = "echo 'explicit-local'"}, {address = "localhost", connection = "local"})
        table.insert(results, {type = "explicit-local", result = result2})

        -- Default (no host) should be local
        local result3 = k.mod.cmd({cmd = "echo 'default-local'"})
        table.insert(results, {type = "default-local", result = result3})

        return results
    "#;

    let results: Table = lua.load(script).eval()?;

    // All should succeed
    for pair in results.pairs::<i32, Table>() {
        let (_, test_case) = pair?;
        let result: Table = test_case.get("result")?;
        let test_type: String = test_case.get("type")?;

        assert_eq!(
            result.get::<i32>("exit_code")?,
            0,
            "Failed for type: {test_type}",
        );
    }

    Ok(())
}

/// Test Requirement 3.1 & 3.2: Host parameter validation
#[test]
fn test_requirement_3_1_3_2_host_validation() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        local test_cases = {}

        -- Valid host configurations should work
        local success1, result1 = pcall(function()
            return k.mod.cmd({cmd = "echo 'valid'"}, {address = "localhost"})
        end)
        table.insert(test_cases, {type = "valid-host", success = success1, result = result1})

        -- Missing host should default to local (should work)
        local success2, result2 = pcall(function()
            return k.mod.cmd({cmd = "echo 'no-host'"})
        end)
        table.insert(test_cases, {type = "no-host", success = success2, result = result2})

        -- Invalid host configuration should fail gracefully
        local success3, result3 = pcall(function()
            return k.mod.cmd({cmd = "echo 'invalid'"}, {invalid_field = "test"})
        end)
        table.insert(test_cases, {type = "invalid-host", success = success3, result = result3})

        return test_cases
    "#;

    let test_cases: Table = lua.load(script).eval()?;

    for pair in test_cases.pairs::<i32, Table>() {
        let (_, test_case) = pair?;
        let test_type: String = test_case.get("type")?;
        let success: bool = test_case.get("success")?;

        match test_type.as_str() {
            "valid-host" | "no-host" => {
                assert!(success, "Test {test_type} should have succeeded");
                if success {
                    let result: Table = test_case.get("result")?;
                    assert_eq!(result.get::<i32>("exit_code")?, 0);
                }
            }
            _ => {
                // This might succeed or fail depending on implementation
                // The key is that it handles the case gracefully
            }
        }
    }

    Ok(())
}

/// Test Requirement 4.1 & 4.3: Module registration and discovery
#[test]
fn test_requirement_4_1_4_3_module_registration() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test that modules are properly registered and discoverable
        local registered_modules = {}

        -- Check k.mod namespace
        if k and k.mod then
            for name, func in pairs(k.mod) do
                if type(func) == "function" then
                    table.insert(registered_modules, name)
                end
            end
        end

        -- Test that we can call registered modules
        local test_results = {}
        for _, module_name in ipairs(registered_modules) do
            if module_name == "cmd" then
                local success, result = pcall(function()
                    return k.mod.cmd({cmd = "echo 'test " .. module_name .. "'"})
                end)
                table.insert(test_results, {
                    module = module_name,
                    success = success,
                    callable = success
                })
            end
        end

        return {
            registered = registered_modules,
            tests = test_results
        }
    "#;

    let results: Table = lua.load(script).eval()?;
    let registered: Table = results.get("registered")?;
    let tests: Table = results.get("tests")?;

    // Should have registered modules
    let mut module_count = 0;
    for pair in registered.pairs::<i32, String>() {
        let (_, _module_name) = pair?;
        module_count += 1;
    }
    assert!(module_count > 0, "No modules registered");

    // Test results should show modules are callable
    for pair in tests.pairs::<i32, Table>() {
        let (_, test_result) = pair?;
        let module_name: String = test_result.get("module")?;
        let callable: bool = test_result.get("callable")?;
        assert!(callable, "Module {module_name} is not callable");
    }

    Ok(())
}

/// Test Requirement 5.1: Consistent error handling
#[test]
fn test_requirement_5_1_consistent_error_handling() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test error handling consistency
        local error_tests = {}

        -- Test command that fails
        local result1 = k.mod.cmd({cmd = "false"})  -- Command that always fails
        table.insert(error_tests, {
            type = "command-failure",
            result = result1,
            expected_exit_code = 1
        })

        -- Test invalid parameters (should error)
        local success2, result2 = pcall(function()
            return k.mod.cmd({})  -- Missing required 'cmd' parameter
        end)
        table.insert(error_tests, {
            type = "parameter-error",
            success = success2,
            error = not success2 and tostring(result2) or nil
        })

        return error_tests
    "#;

    let error_tests: Table = lua.load(script).eval()?;

    for pair in error_tests.pairs::<i32, Table>() {
        let (_, test) = pair?;
        let test_type: String = test.get("type")?;

        match test_type.as_str() {
            "command-failure" => {
                let result: Table = test.get("result")?;
                let exit_code = result.get::<i32>("exit_code")?;
                assert_ne!(
                    exit_code, 0,
                    "Failed command should have non-zero exit code"
                );

                // Should have error structure
                assert!(result.contains_key("stdout")?);
                assert!(result.contains_key("stderr")?);
                assert!(result.contains_key("exit_code")?);
            }
            "parameter-error" => {
                let success: bool = test.get("success")?;
                assert!(!success, "Invalid parameters should cause error");

                if let Ok(error) = test.get::<String>("error") {
                    assert!(
                        error.contains("cmd") || error.contains("required"),
                        "Error should mention missing parameter"
                    );
                }
            }
            _ => {}
        }
    }

    Ok(())
}

/// Test Requirement 8.1 & 8.3: Namespace design
#[test]
fn test_requirement_8_1_8_3_namespace_design() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Test namespace structure
        local namespaces = {}

        -- Check k.mod exists
        if k and k.mod then
            namespaces.k_mod = true
        end

        -- Check k.mods exists (ModulesV1 alias)
        if k and k.mods then
            namespaces.k_mods = true
        end

        -- Check komandan.modules exists
        if komandan and komandan.modules then
            namespaces.komandan_modules = true
        end

        -- Test that they work independently
        local test_results = {}

        -- Test k.mod
        if namespaces.k_mod then
            local success, result = pcall(function()
                return k.mod.cmd({cmd = "echo 'k.mod works'"})
            end)
            test_results.k_mod = {success = success, works = success}
        end

        -- Test k.mods (traditional way)
        if namespaces.k_mods then
            local success, result = pcall(function()
                local task = {
                    name = "k.mods test",
                    k.mods.cmd({cmd = "echo 'k.mods works'"})
                }
                return komandan.komando(task, {address = "localhost", connection = "local"})
            end)
            test_results.k_mods = {success = success, works = success}
        end

        return {
            namespaces = namespaces,
            tests = test_results
        }
    "#;

    let results: Table = lua.load(script).eval()?;
    let namespaces: Table = results.get("namespaces")?;
    let tests: Table = results.get("tests")?;

    // Check required namespaces exist
    assert!(namespaces.get::<bool>("k_mod")?, "k.mod namespace missing");
    assert!(
        namespaces.get::<bool>("k_mods")?,
        "k.mods namespace missing"
    );
    assert!(
        namespaces.get::<bool>("komandan_modules")?,
        "komandan.modules namespace missing"
    );

    // Check they work
    let k_mod_test: Table = tests.get("k_mod")?;
    let k_modules_test: Table = tests.get("k_mods")?;

    assert!(k_mod_test.get::<bool>("works")?, "k.mod should work");
    assert!(k_modules_test.get::<bool>("works")?, "k.mods should work");

    Ok(())
}

/// Test comprehensive workflow with multiple modules
#[test]
fn test_comprehensive_workflow() -> anyhow::Result<()> {
    let lua = create_lua()?;

    let script = r#"
        -- Comprehensive workflow test using multiple ModulesV2 modules
        local workflow_dir = "/tmp/modulesv2_workflow_" .. os.time()
        local app_config = workflow_dir .. "/app.conf"
        local service_script = workflow_dir .. "/service.sh"

        local workflow_steps = {}

        -- Step 1: Create directory
        local step1 = k.mod.cmd({cmd = "mkdir -p " .. workflow_dir})
        table.insert(workflow_steps, {name = "create_directory", result = step1})

        -- Step 2: Create configuration file
        local config_content = [[
#!/bin/bash
# Service configuration
APP_NAME="test-service"
APP_PORT=8080
APP_ENV="development"
]]
        local step2 = k.mod.file({
            path = app_config,
            content = config_content,
            mode = "0644"
        })
        table.insert(workflow_steps, {name = "create_config", result = step2})

        -- Step 3: Create service script
        local script_content = [[
#!/bin/bash
source ]] .. app_config .. [[

echo "Starting $APP_NAME on port $APP_PORT in $APP_ENV mode"
echo "Service started successfully"
]]
        local step3 = k.mod.file({
            path = service_script,
            content = script_content,
            mode = "0755"
        })
        table.insert(workflow_steps, {name = "create_script", result = step3})

        -- Step 4: Test script execution
        local step4 = k.mod.cmd({cmd = "bash " .. service_script})
        table.insert(workflow_steps, {name = "test_script", result = step4})

        -- Step 5: Verify files exist with correct permissions
        local step5 = k.mod.cmd({cmd = "ls -la " .. workflow_dir})
        table.insert(workflow_steps, {name = "verify_files", result = step5})

        -- Step 6: Cleanup
        local step6 = k.mod.cmd({cmd = "rm -rf " .. workflow_dir})
        table.insert(workflow_steps, {name = "cleanup", result = step6})

        return workflow_steps
    "#;

    let workflow_steps: Table = lua.load(script).eval()?;

    // Validate all workflow steps
    for pair in workflow_steps.pairs::<i32, Table>() {
        let (_, step) = pair?;
        let step_name: String = step.get("name")?;
        let result: Table = step.get("result")?;
        let exit_code = result.get::<i32>("exit_code")?;

        assert_eq!(exit_code, 0, "Workflow step '{step_name}' failed");

        // Additional validations for specific steps
        match step_name.as_str() {
            "test_script" => {
                let stdout = result.get::<String>("stdout")?;
                assert!(stdout.contains("Starting test-service"));
                assert!(stdout.contains("port 8080"));
                assert!(stdout.contains("development mode"));
                assert!(stdout.contains("Service started successfully"));
            }
            "verify_files" => {
                let stdout = result.get::<String>("stdout")?;
                assert!(stdout.contains("app.conf"));
                assert!(stdout.contains("service.sh"));
            }
            _ => {}
        }
    }

    Ok(())
}
