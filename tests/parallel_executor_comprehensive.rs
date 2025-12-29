use komandan::create_lua;
use mlua::{Integer, Table, chunk};

#[test]
fn test_parallel_executor_comprehensive() -> mlua::Result<()> {
    let lua = create_lua()?;

    // Test 1: Basic functionality
    println!("=== Test 1: Basic functionality ===");
    let basic_results = lua
        .load(chunk! {
            local data = {1, 2, 3}
            local results = k.parallel_executor:map(data, function(item)
                return item * 2
            end)
            return results
        })
        .eval::<Table>()?;

    assert_eq!(basic_results.len()?, 3);
    for i in 1..=3 {
        let result: Table = basic_results.get(i)?;
        assert!(result.get::<bool>("success")?);
        let result_value: Integer = result.get::<Integer>("result")?;
        assert_eq!(result_value, i * 2);
    }
    println!("✓ Basic functionality test passed");

    // Test 2: Komando operations with error handling
    println!("=== Test 2: Komando operations ===");
    let komando_results = lua
        .load(chunk! {
            local commands = {"echo 'test1'", "echo 'test2'"}
            local host = {address = "localhost", connection = "local"}

            local success, results = pcall(function()
                return k.parallel_executor:map(commands, function(cmd)
                    local local_host = {address = "localhost", connection = "local"}
                    local task = {
                        name = "Test command",
                        komandan.modules.cmd({cmd = cmd})
                    }
                    local result = komandan.komando(task, local_host)
                    return {
                        command = cmd,
                        exit_code = result.exit_code,
                        stdout = result.stdout,
                        success = result.exit_code == 0
                    }
                end)
            end)

            if not success then
                error("Parallel executor failed: " .. tostring(results))
            end

            return results
        })
        .eval::<Table>()?;

    assert_eq!(komando_results.len()?, 2);
    for i in 1..=2 {
        let result: Table = komando_results.get(i)?;
        if !result.get::<bool>("success")? {
            // Print error information
            if let Ok(error) = result.get::<String>("error") {
                println!("Result {i} failed with error: {error}");
            }
            panic!("Result {i} should have succeeded");
        }

        let result_data: Table = result.get("result")?;
        assert_eq!(result_data.get::<Integer>("exit_code")?, 0);
        assert!(result_data.get::<bool>("success")?);

        let stdout: String = result_data.get("stdout")?;
        assert!(stdout.contains(&format!("test{i}")));
    }
    println!("✓ Komando operations test passed");

    // Test 3: Error handling
    println!("=== Test 3: Error handling ===");
    let error_results = lua
        .load(chunk! {
            local commands = {"echo 'success'", "false", "echo 'another success'"}

            local results = k.parallel_executor:map(commands, function(cmd)
                local local_host = {address = "localhost", connection = "local"}
                local task = {
                    name = "Error test command",
                    komandan.modules.cmd({cmd = cmd})
                }
                local result = komandan.komando(task, local_host)
                return {
                    command = cmd,
                    exit_code = result.exit_code,
                    success = result.exit_code == 0
                }
            end)

            return results
        })
        .eval::<Table>()?;

    assert_eq!(error_results.len()?, 3);

    // First should succeed
    let result1: Table = error_results.get(1)?;
    assert!(result1.get::<bool>("success")?);
    let result1_data: Table = result1.get("result")?;
    assert_eq!(result1_data.get::<Integer>("exit_code")?, 0);

    // Second should fail
    let result2: Table = error_results.get(2)?;
    if result2.get::<bool>("success")? {
        // If the parallel execution succeeded, check the command result
        let result2_data: Table = result2.get("result")?;
        assert_ne!(result2_data.get::<Integer>("exit_code")?, 0);
    } else {
        // If the parallel execution failed, that's also acceptable for a failing command
        println!("Command 2 failed at parallel execution level (expected)");
    }

    // Third should succeed
    let result3: Table = error_results.get(3)?;
    assert!(result3.get::<bool>("success")?);
    let result3_data: Table = result3.get("result")?;
    assert_eq!(result3_data.get::<Integer>("exit_code")?, 0);

    println!("✓ Error handling test passed");

    // Test 4: Performance metrics
    println!("=== Test 4: Performance metrics ===");
    let perf_results = lua
        .load(chunk! {
            local data = {1, 2, 3, 4, 5}
            local results = k.parallel_executor:map(data, function(item)
                return item * 2
            end)
            return results
        })
        .eval::<Table>()?;

    // Check metadata
    let success_count: Integer = perf_results.get("_success_count")?;
    assert_eq!(success_count, 5);

    let failed_count: Integer = perf_results.get("_failed_count")?;
    assert_eq!(failed_count, 0);

    let total_time: f64 = perf_results.get("_total_time")?;
    assert!(total_time > 0.0);

    println!("✓ Performance metrics test passed");

    println!("=== All comprehensive tests passed! ===");
    Ok(())
}
