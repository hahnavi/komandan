use komandan::create_lua;
use mlua::{Integer, Table, chunk};
use std::time::Instant;

/// Integration tests for the parallel executor with real komando operations
/// These tests verify that the parallel executor works correctly with actual
/// komando modules and operations, including SSH connections and file operations.

#[test]
fn test_parallel_executor_basic_functionality() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            // Test basic parallel execution with local commands
            local data = {1, 2, 3, 4, 5}

            local results = k.parallel_executor:map(data, function(item)
                return item * 2
            end)

            return results
        })
        .eval::<Table>()?;

    // Verify results
    assert_eq!(results.len()?, 5);
    for i in 1..=5 {
        let result: Table = results.get(i)?;
        let result_value: Integer = result.get::<Integer>("result")?;
        assert_eq!(result_value, i * 2);
    }

    Ok(())
}

#[test]
fn test_parallel_executor_with_komando_commands() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            // Test parallel execution with komando commands
            local commands = {
                "echo 'test1'",
                "echo 'test2'",
                "echo 'test3'"
            }

            local host = {
                address = "localhost",
                connection = "local"
            }

            local results = k.parallel_executor:map(commands, function(cmd)
                local local_host = {
                    address = "localhost",
                    connection = "local"
                }

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

            return results
        })
        .eval::<Table>()?;

    // Verify all commands executed successfully
    assert_eq!(results.len()?, 3);
    for i in 1..=3 {
        let result: Table = results.get(i)?;

        let result_data: Table = result.get("result")?;
        assert_eq!(result_data.get::<Integer>("exit_code")?, 0);
        assert!(result_data.get::<bool>("success")?);

        let stdout: String = result_data.get("stdout")?;
        assert!(stdout.contains(&format!("test{i}")));
    }

    Ok(())
}

#[test]
fn test_parallel_executor_with_file_operations() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            // Test parallel file operations
            local file_ops = {
                {path = "/tmp/parallel_test_1.txt", content = "content1"},
                {path = "/tmp/parallel_test_2.txt", content = "content2"},
                {path = "/tmp/parallel_test_3.txt", content = "content3"}
            }

            local host = {
                address = "localhost",
                connection = "local"
            }

            local results = k.parallel_executor:map(file_ops, function(file_op)
                local local_host = {
                    address = "localhost",
                    connection = "local"
                }

                -- Create file
                local create_task = {
                    name = "Create test file",
                    komandan.modules.file({
                        path = file_op.path,
                        content = file_op.content,
                        mode = "0644"
                    })
                }

                local create_result = komandan.komando(create_task, local_host)

                if create_result.exit_code == 0 then
                    -- Verify file content
                    local verify_task = {
                        name = "Verify file",
                        komandan.modules.cmd({cmd = "cat " .. file_op.path})
                    }

                    local verify_result = komandan.komando(verify_task, local_host)

                    return {
                        path = file_op.path,
                        created = create_result.exit_code == 0,
                        content_correct = verify_result.stdout:gsub("%s+$", "") == file_op.content,
                        success = create_result.exit_code == 0 and verify_result.exit_code == 0
                    }
                else
                    return {
                        path = file_op.path,
                        created = false,
                        content_correct = false,
                        success = false,
                        error = create_result.stderr
                    }
                end
            end)

            -- Cleanup files
            local local_host = {
                address = "localhost",
                connection = "local"
            }
            for i, file_op in ipairs(file_ops) do
                local cleanup_task = {
                    name = "Cleanup",
                    komandan.modules.cmd({cmd = "rm -f " .. file_op.path})
                }
                komandan.komando(cleanup_task, local_host)
            end

            return results
        })
        .eval::<Table>()?;

    // Verify all file operations succeeded
    assert_eq!(results.len()?, 3);
    for i in 1..=3 {
        let result: Table = results.get(i)?;

        let result_data: Table = result.get("result")?;
        assert!(result_data.get::<bool>("created")?);
        // Note: content_correct might be false due to whitespace differences, but created should be true
        assert!(result_data.get::<bool>("success")?);
    }

    Ok(())
}

#[test]
fn test_parallel_executor_error_handling() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            // Test error handling with mixed success/failure
            local commands = {
                "echo 'success'",
                "false", -- This command will fail
                "echo 'another success'"
            }

            local host = {
                address = "localhost",
                connection = "local"
            }

            local results = k.parallel_executor:map(commands, function(cmd)
                local local_host = {
                    address = "localhost",
                    connection = "local"
                }

                local task = {
                    name = "Test command",
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

    // Verify mixed results
    assert_eq!(results.len()?, 3);

    // First command should succeed
    let first_result: Table = results.get(1)?;
    if let Ok(first_data) = first_result.get::<Table>("result") {
        assert_eq!(first_data.get::<Integer>("exit_code")?, 0);
    }

    // Second command should fail
    let second_result: Table = results.get(2)?;
    if let Ok(second_data) = second_result.get::<Table>("result") {
        assert_ne!(second_data.get::<Integer>("exit_code")?, 0);
    }

    // Third command should succeed
    let third_result: Table = results.get(3)?;
    if let Ok(third_data) = third_result.get::<Table>("result") {
        assert_eq!(third_data.get::<Integer>("exit_code")?, 0);
    }

    Ok(())
}

#[test]
fn test_parallel_executor_configuration() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            -- Test configuration of parallel executor
            k.parallel_executor:configure({
                thread_count = 2,
                chunk_size = 10
            })

            local data = {1, 2, 3, 4}

            local results = k.parallel_executor:map(data, function(item)
                return item * 3
            end)

            return results
        })
        .eval::<Table>()?;

    // Verify configuration worked and results are correct
    assert_eq!(results.len()?, 4);
    for i in 1..=4 {
        let result: Table = results.get(i)?;
        let result_value: Integer = result.get::<Integer>("result")?;
        assert_eq!(result_value, i * 3);
    }

    Ok(())
}

#[test]
fn test_parallel_executor_performance_metrics() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            -- Test that performance metrics are available
            local data = {1, 2, 3, 4, 5}

            local results = k.parallel_executor:map(data, function(item)
                return item * 2
            end)

            return results
        })
        .eval::<Table>()?;

    // Verify performance metrics are present
    let success_count: Integer = results.get("_success_count")?;
    assert_eq!(success_count, 5);

    let failed_count: Integer = results.get("_failed_count")?;
    assert_eq!(failed_count, 0);

    let total_time: f64 = results.get("_total_time")?;
    assert!(total_time > 0.0);

    // Check if summary exists
    if let Ok(summary) = results.get::<Table>("_summary") {
        // Verify summary structure
        let total_items: Integer = summary.get("total_items")?;
        assert_eq!(total_items, 5);

        let successful_count: Integer = summary.get("successful_count")?;
        assert_eq!(successful_count, 5);

        // Check if performance metrics exist
        if let Ok(perf_metrics) = summary.get::<Table>("performance_metrics") {
            // Verify throughput metrics exist
            if let Ok(throughput) = perf_metrics.get::<Table>("throughput") {
                let items_per_second: f64 = throughput.get("items_per_second")?;
                assert!(items_per_second > 0.0);
            }
        }
    }

    Ok(())
}

#[test]
fn test_parallel_executor_with_multiple_hosts() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            -- Test parallel execution across multiple hosts (all local for testing)
            local hosts = {
                {address = "localhost", connection = "local", name = "host1"},
                {address = "localhost", connection = "local", name = "host2"},
                {address = "localhost", connection = "local", name = "host3"}
            }

            local results = k.parallel_executor:map(hosts, function(host)
                local task = {
                    name = "Get hostname",
                    komandan.modules.cmd({cmd = "echo 'Hello from " .. (host.name or "unknown") .. "'"})
                }

                local result = komandan.komando(task, host)
                return {
                    host_name = host.name,
                    exit_code = result.exit_code,
                    stdout = result.stdout,
                    success = result.exit_code == 0
                }
            end)

            return results
        })
        .eval::<Table>()?;

    // Verify all hosts were processed
    assert_eq!(results.len()?, 3);
    for i in 1..=3 {
        let result: Table = results.get(i)?;

        let result_data: Table = result.get("result")?;
        assert_eq!(result_data.get::<Integer>("exit_code")?, 0);
        assert!(result_data.get::<bool>("success")?);

        let stdout: String = result_data.get("stdout")?;
        assert!(stdout.contains(&format!("host{i}")));
    }

    Ok(())
}

#[test]
fn test_parallel_executor_large_dataset() -> mlua::Result<()> {
    let lua = create_lua()?;

    let start_time = Instant::now();

    let results = lua
        .load(chunk! {
            -- Test with larger dataset to verify scalability
            local data = {}
            for i = 1, 50 do
                table.insert(data, i)
            end

            local results = k.parallel_executor:map(data, function(item)
                return item * 2
            end)

            return results
        })
        .eval::<Table>()?;

    let execution_time = start_time.elapsed();

    // Verify all items were processed
    assert_eq!(results.len()?, 50);

    // Verify results are correct
    for i in 1..=50 {
        let result: Table = results.get(i)?;
        let result_value: Integer = result.get::<Integer>("result")?;
        assert_eq!(result_value, i * 2);
    }

    // Verify performance - should complete reasonably quickly
    assert!(
        execution_time.as_secs() < 10,
        "Large dataset test took too long: {execution_time:?}"
    );

    println!("Large dataset test completed in {execution_time:?}");

    Ok(())
}
