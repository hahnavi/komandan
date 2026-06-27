use super::*;

#[test]
fn test_executor_config_default() {
    let config = ExecutorConfig::default();
    assert_eq!(config.thread_count, None);
    assert_eq!(config.chunk_size, Some(100));
    assert_eq!(config.timeout_seconds, Some(300));
    assert_eq!(config.error_strategy, Some("continue".to_string()));
    assert_eq!(config.max_memory_mb, Some(512));
}

#[test]
fn test_executor_config_presets() {
    // Test small dataset preset
    let small_config = ExecutorConfig::for_small_datasets();
    assert_eq!(small_config.thread_count, Some(2));
    assert_eq!(small_config.chunk_size, Some(10));
    assert_eq!(small_config.timeout_seconds, Some(60));
    assert_eq!(small_config.max_memory_mb, Some(256));

    // Test large dataset preset
    let large_config = ExecutorConfig::for_large_datasets();
    assert_eq!(large_config.thread_count, None); // Use all cores
    assert_eq!(large_config.chunk_size, Some(500));
    assert_eq!(large_config.timeout_seconds, Some(600));
    assert_eq!(large_config.max_memory_mb, Some(1024));

    // Test I/O intensive preset
    let io_config = ExecutorConfig::for_io_intensive();
    assert!(io_config.thread_count.is_some_and(|count| count >= 4)); // At least 4 threads
    assert_eq!(io_config.chunk_size, Some(50));
    assert_eq!(io_config.timeout_seconds, Some(900));
    assert_eq!(io_config.max_memory_mb, Some(256));
}

#[test]
fn test_executor_config_effective_values() {
    let config = ExecutorConfig::default();

    // Test effective values
    assert!(config.effective_thread_count() > 0);
    assert_eq!(config.effective_chunk_size(), 100);
    assert_eq!(config.effective_timeout_seconds(), 300);
    assert_eq!(config.effective_error_strategy(), "continue");
    assert_eq!(config.effective_max_memory_mb(), 512);

    // Test with custom values
    let custom_config = ExecutorConfig {
        thread_count: Some(8),
        chunk_size: Some(200),
        timeout_seconds: Some(600),
        error_strategy: Some("fail_fast".to_string()),
        max_memory_mb: Some(1024),
    };

    assert_eq!(custom_config.effective_thread_count(), 8);
    assert_eq!(custom_config.effective_chunk_size(), 200);
    assert_eq!(custom_config.effective_timeout_seconds(), 600);
    assert_eq!(custom_config.effective_error_strategy(), "fail_fast");
    assert_eq!(custom_config.effective_max_memory_mb(), 1024);
}

#[test]
fn test_executor_creation() -> Result<()> {
    let executor = ParallelExecutor::new(None)?;
    assert!(executor.thread_count() > 0);
    Ok(())
}

#[test]
fn test_executor_creation_with_config() -> Result<()> {
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(50),
        timeout_seconds: Some(120),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(256),
    };
    let executor = ParallelExecutor::new(Some(config))?;
    assert_eq!(executor.thread_count(), 4);
    assert_eq!(executor.config().chunk_size, Some(50));
    assert_eq!(executor.config().timeout_seconds, Some(120));
    assert_eq!(
        executor.config().error_strategy,
        Some("continue".to_string())
    );
    assert_eq!(executor.config().max_memory_mb, Some(256));
    Ok(())
}

#[test]
fn test_config_validation() {
    // Test invalid thread count
    let config = ExecutorConfig {
        thread_count: Some(0),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };
    assert!(validate_config(&config).is_err());

    // Test excessive thread count
    let config = ExecutorConfig {
        thread_count: Some(2000),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };
    assert!(validate_config(&config).is_err());

    // Test invalid chunk size
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(0),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };
    assert!(validate_config(&config).is_err());

    // Test invalid timeout
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(0),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };
    assert!(validate_config(&config).is_err());

    // Test invalid error strategy
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("invalid_strategy".to_string()),
        max_memory_mb: Some(512),
    };
    assert!(validate_config(&config).is_err());

    // Test invalid memory limit
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(0),
    };
    assert!(validate_config(&config).is_err());

    // Test valid config
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };
    assert!(validate_config(&config).is_ok());

    // Test valid fail_fast strategy
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("fail_fast".to_string()),
        max_memory_mb: Some(512),
    };
    assert!(validate_config(&config).is_ok());
}

#[test]
fn test_global_executor() {
    let executor = global_executor();
    if let Ok(executor_guard) = executor.lock() {
        assert!(executor_guard.thread_count() > 0);
    } else {
        panic!("Failed to lock executor");
    }
}

#[test]
fn test_executor_configure() -> Result<()> {
    let mut executor = ParallelExecutor::new(None)?;
    let _original_count = executor.thread_count();

    let new_config = ExecutorConfig {
        thread_count: Some(2),
        chunk_size: Some(200),
        timeout_seconds: Some(600),
        error_strategy: Some("fail_fast".to_string()),
        max_memory_mb: Some(1024),
    };

    executor
        .configure(new_config)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    assert_eq!(executor.thread_count(), 2);
    assert_eq!(executor.config().chunk_size, Some(200));
    assert_eq!(executor.config().timeout_seconds, Some(600));
    assert_eq!(
        executor.config().error_strategy,
        Some("fail_fast".to_string())
    );
    assert_eq!(executor.config().max_memory_mb, Some(1024));

    Ok(())
}

#[test]
fn test_serialized_value_conversion() -> mlua::Result<()> {
    let lua = Lua::new();

    // Test basic types
    let nil_value = Value::Nil;
    let serialized = SerializedValue::from_lua_value(nil_value)?;
    let converted_back = serialized.to_lua_value(&lua)?;
    assert!(matches!(converted_back, Value::Nil));

    let bool_value = Value::Boolean(true);
    let serialized = SerializedValue::from_lua_value(bool_value)?;
    let converted_back = serialized.to_lua_value(&lua)?;
    assert!(matches!(converted_back, Value::Boolean(true)));

    let int_value = Value::Integer(42);
    let serialized = SerializedValue::from_lua_value(int_value)?;
    let converted_back = serialized.to_lua_value(&lua)?;
    assert!(matches!(converted_back, Value::Integer(42)));

    let string_value = Value::String(lua.create_string("test")?);
    let serialized = SerializedValue::from_lua_value(string_value)?;
    let converted_back = serialized.to_lua_value(&lua)?;
    if let Value::String(s) = converted_back {
        assert_eq!(s.to_str()?, "test");
    } else {
        panic!("Expected string value");
    }

    Ok(())
}

#[test]
fn test_lua_context_factory() -> Result<()> {
    let lua = LuaContextFactory::create_isolated_context()?;

    // Test that komandan global is available
    let komandan: Table = lua.globals().get("komandan")?;
    let _modules: Table = komandan.get("modules")?;
    let _check: Table = komandan.get("check")?;
    let _defaults = komandan.get::<mlua::Value>("defaults")?;
    let _komando_fn = komandan.get::<mlua::Function>("komando")?;

    // Test that 'k' alias is available
    let k: Table = lua.globals().get("k")?;
    let _k_mods: Table = k.get("mods")?;
    let _k_check: Table = k.get("check")?;
    let _k_komando_fn = k.get::<mlua::Function>("komando")?;

    // Test that modules are available
    let modules: Table = komandan.get("modules")?;
    assert!(modules.contains_key("cmd")?);
    assert!(modules.contains_key("file")?);
    assert!(modules.contains_key("apt")?);

    // Test that check functions are available
    let check: Table = komandan.get("check")?;
    assert!(check.contains_key("file")?);
    assert!(check.contains_key("service")?);
    assert!(check.contains_key("package")?);

    Ok(())
}

#[test]
fn test_function_serialization() -> mlua::Result<()> {
    let lua = Lua::new();

    // For now, let's test with a Lua-defined function instead of Rust function
    // Rust functions created with create_function may not serialize properly
    let lua_func_code = "return function(x) return x * 2 end";
    let lua_func: mlua::Function = lua.load(lua_func_code).eval()?;

    // Serialize the function
    let serialized = LuaContextFactory::serialize_function(&lua, &lua_func)?;
    assert!(
        !serialized.bytecode.is_empty(),
        "Lua function should have bytecode"
    );

    // Deserialize in a new context
    let new_lua = LuaContextFactory::create_isolated_context()?;
    let deserialized = LuaContextFactory::deserialize_function(&new_lua, &serialized)?;

    // Test that the deserialized function works
    let result: i32 = deserialized.call(5)?;
    assert_eq!(result, 10);

    Ok(())
}

#[test]
fn test_parallel_map_basic() -> mlua::Result<()> {
    let lua = Lua::new();
    let executor =
        ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

    // Create test data
    let data = lua.create_table()?;
    data.set(1, 10)?;
    data.set(2, 20)?;
    data.set(3, 30)?;

    // Create a Lua function that doubles the input (Rust functions don't serialize well)
    let func_code = "return function(x) return x * 2 end";
    let func: mlua::Function = lua.load(func_code).eval()?;

    // Execute parallel map
    let results = executor.map(&lua, &data, &func)?;

    // Check results
    assert_eq!(results.len()?, 3);

    for i in 1..=3 {
        let result_table: Table = results.get(i)?;
        let success: bool = result_table.get("success")?;

        if success {
            let result_value: i32 = result_table.get("result")?;
            let expected = i * 10 * 2; // Original values are 10, 20, 30, doubled
            assert_eq!(result_value, expected, "Result {i} should be {expected}");
        } else {
            // If there's an error, print it for debugging
            if let Ok(error) = result_table.get::<String>("error") {
                panic!("Result {i} failed with error: {error}");
            } else {
                panic!("Result {i} should be successful");
            }
        }
    }

    Ok(())
}

#[test]
fn test_error_handling_invalid_data() -> mlua::Result<()> {
    let lua = Lua::new();
    let executor =
        ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

    // Test empty data table
    let empty_data = lua.create_table()?;
    let func_code = "return function(x) return x * 2 end";
    let func: mlua::Function = lua.load(func_code).eval()?;

    let result = executor.map(&lua, &empty_data, &func);
    assert!(result.is_err());

    // Check that the error message is helpful
    if let Err(mlua::Error::RuntimeError(msg)) = result {
        assert!(msg.contains("Input data table is empty"));
        assert!(msg.contains("at least one element"));
    }

    Ok(())
}

#[test]
fn test_error_handling_function_failure() -> mlua::Result<()> {
    let lua = Lua::new();
    let executor =
        ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

    // Create test data with mixed types that will cause errors
    let data = lua.create_table()?;
    data.set(1, 10)?;
    data.set(2, "invalid")?; // This will cause a type error
    data.set(3, 30)?;

    // Create a function that will definitely fail with non-numbers
    let func_code =
        "return function(x) assert(type(x) == 'number', 'Expected number'); return x * 2 end";
    let func: mlua::Function = lua.load(func_code).eval()?;

    // Execute parallel map
    let results = executor.map(&lua, &data, &func)?;

    // Check that we have results for all items
    assert_eq!(results.len()?, 3);

    // Check summary shows mixed results
    let success_count: usize = results.get("_success_count")?;
    let failed_count: usize = results.get("_failed_count")?;

    println!("Success count: {success_count}, Failed count: {failed_count}");

    // Print individual results for debugging
    for i in 1..=3 {
        let result_table: Table = results.get(i)?;
        let success: bool = result_table.get("success")?;
        println!("Result {i}: success = {success}");
        if !success && let Ok(error_msg) = result_table.get::<String>("error") {
            println!("  Error: {error_msg}");
        }
    }

    assert_eq!(success_count, 2); // Items 1 and 3 should succeed
    assert_eq!(failed_count, 1); // Item 2 should fail

    // Check individual results
    let first_result: Table = results.get(1)?;
    assert!(first_result.get::<bool>("success")?);

    let second_result: Table = results.get(2)?;
    assert!(!second_result.get::<bool>("success")?);
    let error_msg: String = second_result.get("error")?;
    assert!(error_msg.contains("Function execution error"));

    let third_result: Table = results.get(3)?;
    assert!(third_result.get::<bool>("success")?);

    // Check execution summary
    let summary: Table = results.get("_summary")?;
    let total_items: usize = summary.get("total_items")?;
    assert_eq!(total_items, 3);

    let error_breakdown: Table = summary.get("error_breakdown")?;

    // Count error breakdown entries manually (Lua tables with string keys don't report len() correctly)
    let mut error_count = 0;
    for _pair in error_breakdown.pairs::<String, usize>() {
        error_count += 1;
    }

    // Should have at least one error category
    assert!(error_count > 0);

    Ok(())
}

#[test]
fn test_configuration_error_messages() {
    // Test thread count validation
    let result = validate_config(&ExecutorConfig {
        thread_count: Some(0),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    });

    assert!(result.is_err());
    if let Err(error) = result {
        let error_msg = error.to_string();
        assert!(error_msg.contains("thread_count must be greater than 0"));
    } else {
        panic!("Expected error result");
    }

    // Test chunk size validation
    let result = validate_config(&ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(0),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    });

    assert!(result.is_err());
    if let Err(error) = result {
        let error_msg = error.to_string();
        assert!(error_msg.contains("chunk_size must be greater than 0"));
    } else {
        panic!("Expected error result");
    }

    // Test excessive thread count
    let result = validate_config(&ExecutorConfig {
        thread_count: Some(2000),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    });

    assert!(result.is_err());
    if let Err(error) = result {
        let error_msg = error.to_string();
        assert!(error_msg.contains("exceeds maximum limit"));
    } else {
        panic!("Expected error result");
    }

    // Test invalid error strategy
    let result = validate_config(&ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("invalid".to_string()),
        max_memory_mb: Some(512),
    });

    assert!(result.is_err());
    if let Err(error) = result {
        let error_msg = error.to_string();
        assert!(error_msg.contains("Invalid error_strategy"));
    } else {
        panic!("Expected error result");
    }

    // Test timeout validation
    let result = validate_config(&ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(0),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    });

    assert!(result.is_err());
    if let Err(error) = result {
        let error_msg = error.to_string();
        assert!(error_msg.contains("timeout_seconds must be greater than 0"));
    } else {
        panic!("Expected error result");
    }

    // Test memory validation
    let result = validate_config(&ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(0),
    });

    assert!(result.is_err());
    if let Err(error) = result {
        let error_msg = error.to_string();
        assert!(error_msg.contains("max_memory_mb must be greater than 0"));
    } else {
        panic!("Expected error result");
    }
}

#[test]
fn test_configuration_combinations() {
    // Test configuration that should generate warnings (captured via stderr)
    let config = ExecutorConfig {
        thread_count: Some(20), // High thread count
        chunk_size: Some(5),    // Small chunk size
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };

    // This should succeed but generate warnings
    assert!(validate_config(&config).is_ok());

    // Test single thread with large chunk size
    let config = ExecutorConfig {
        thread_count: Some(1),
        chunk_size: Some(2000), // Large chunk size
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };

    // This should succeed but generate warnings
    assert!(validate_config(&config).is_ok());

    // Test reasonable configuration
    let config = ExecutorConfig {
        thread_count: Some(4),
        chunk_size: Some(100),
        timeout_seconds: Some(300),
        error_strategy: Some("continue".to_string()),
        max_memory_mb: Some(512),
    };

    assert!(validate_config(&config).is_ok());
}

#[test]
fn test_configuration_presets_validation() {
    // All presets should be valid
    assert!(validate_config(&ExecutorConfig::for_small_datasets()).is_ok());
    assert!(validate_config(&ExecutorConfig::for_large_datasets()).is_ok());
    assert!(validate_config(&ExecutorConfig::for_io_intensive()).is_ok());
    assert!(validate_config(&ExecutorConfig::default()).is_ok());
}

#[test]
fn test_execution_summary() -> mlua::Result<()> {
    let lua = Lua::new();
    let executor =
        ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

    // Create test data
    let data = lua.create_table()?;
    data.set(1, 10)?;
    data.set(2, 20)?;
    data.set(3, 30)?;

    let func_code = "return function(x) return x * 2 end";
    let func: mlua::Function = lua.load(func_code).eval()?;

    let results = executor.map(&lua, &data, &func)?;

    // Check summary information
    let summary: Table = results.get("_summary")?;

    let total_items: usize = summary.get("total_items")?;
    assert_eq!(total_items, 3);

    let successful_count: usize = summary.get("successful_count")?;
    assert_eq!(successful_count, 3);

    let failed_count: usize = summary.get("failed_count")?;
    assert_eq!(failed_count, 0);

    let success_rate: f64 = summary.get("success_rate")?;
    assert!((success_rate - 1.0).abs() < f64::EPSILON);

    // Check thread info
    let thread_info: Table = summary.get("thread_info")?;
    let threads_used: usize = thread_info.get("threads_used")?;
    assert!(threads_used > 0);

    let efficiency: f64 = thread_info.get("efficiency")?;
    assert!(efficiency > 0.0 && efficiency <= 1.0);

    // Check performance metrics
    let perf_metrics: Table = summary.get("performance_metrics")?;

    // Check throughput metrics
    let throughput: Table = perf_metrics.get("throughput")?;
    let items_per_second: f64 = throughput.get("items_per_second")?;
    assert!(items_per_second > 0.0);

    let speedup_factor: f64 = throughput.get("speedup_factor")?;
    assert!(speedup_factor > 0.0);

    let cpu_efficiency: f64 = throughput.get("cpu_efficiency")?;
    assert!((0.0..=1.0).contains(&cpu_efficiency));

    // Check memory usage
    let memory_usage: Table = perf_metrics.get("memory_usage")?;
    let peak_memory: f64 = memory_usage.get("peak_memory_per_thread_mb")?;
    assert!(peak_memory > 0.0);

    // Check connection stats (may be 0 for simple local operations)
    let connection_stats: Table = perf_metrics.get("connection_stats")?;
    let _connections_created: usize = connection_stats.get("connections_created")?;
    // For simple parallel map operations without actual connections, this may be 0

    // Check batching metrics
    let batching_metrics: Table = perf_metrics.get("batching_metrics")?;
    let batch_count: usize = batching_metrics.get("batch_count")?;
    assert!(batch_count > 0);

    Ok(())
}

#[test]
fn test_error_categorization() {
    // Test different error types are categorized correctly
    assert_eq!(
        ExecutionSummary::categorize_error("serialize failed"),
        "Serialization"
    );
    assert_eq!(
        ExecutionSummary::categorize_error("deserialize error"),
        "Serialization"
    );
    assert_eq!(
        ExecutionSummary::categorize_error("Lua context failed"),
        "Lua Context"
    );
    assert_eq!(
        ExecutionSummary::categorize_error("Function execution failed"),
        "Function Execution"
    );
    assert_eq!(
        ExecutionSummary::categorize_error("convert value failed"),
        "Type Conversion"
    );
    assert_eq!(ExecutionSummary::categorize_error("unknown error"), "Other");
}

#[test]
fn test_komando_availability_in_isolated_context() -> mlua::Result<()> {
    let lua = LuaContextFactory::create_isolated_context()?;

    // Test that komando function is available
    let komando_fn = lua.globals().get::<mlua::Function>("komando");
    match komando_fn {
        Ok(_) => println!("✓ komando function is available globally"),
        Err(e) => println!("✗ komando function not available globally: {e}"),
    }

    // Test via komandan table
    let komandan: Table = lua.globals().get("komandan")?;
    let komando_fn = komandan.get::<mlua::Function>("komando");
    match komando_fn {
        Ok(_) => println!("✓ komando function is available via komandan table"),
        Err(e) => println!("✗ komando function not available via komandan table: {e}"),
    }

    // Test via k table
    let k: Table = lua.globals().get("k")?;
    let komando_fn = k.get::<mlua::Function>("komando");
    match komando_fn {
        Ok(_) => println!("✓ komando function is available via k table"),
        Err(e) => println!("✗ komando function not available via k table: {e}"),
    }

    // Test a simple function that uses komando via komandan table
    let test_code = r#"
            local host = {
                address = "localhost",
                connection = "local"
            }

            local task = {
                name = "Test",
                komandan.modules.cmd({
                    cmd = "echo 'test'"
                })
            }

            return komandan.komando(task, host)
        "#;

    let result = lua.load(test_code).eval::<Table>();
    match result {
        Ok(result_table) => {
            let exit_code: i32 = result_table.get("exit_code")?;
            println!("✓ komando execution succeeded with exit code: {exit_code}");
        }
        Err(e) => {
            println!("✗ komando execution failed: {e}");
        }
    }

    Ok(())
}

#[test]
fn test_komando_integration_basic() -> mlua::Result<()> {
    let lua = Lua::new();
    let executor =
        ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

    // Create test data with host configurations
    let data = lua.create_table()?;

    // Create host configurations for local execution
    let host1 = lua.create_table()?;
    host1.set("address", "localhost")?;
    host1.set("connection", "local")?;
    data.set(1, host1)?;

    let host2 = lua.create_table()?;
    host2.set("address", "localhost")?;
    host2.set("connection", "local")?;
    data.set(2, host2)?;

    // Create a function that uses komando to execute a simple command
    let func_code = r#"
            return function(host)
                local task = {
                    name = "Test command",
                    komandan.modules.cmd({
                        cmd = "echo 'Hello from parallel komando'"
                    })
                }

                local result = komandan.komando(task, host)
                return {
                    host_address = host.address,
                    exit_code = result.exit_code,
                    stdout = result.stdout,
                    success = result.exit_code == 0
                }
            end
        "#;
    let func: mlua::Function = lua.load(func_code).eval()?;

    // Execute parallel map with komando integration
    let results = executor.map(&lua, &data, &func)?;

    // Check results
    assert_eq!(results.len()?, 2);

    // Verify both executions succeeded
    for i in 1..=2 {
        let result_table: Table = results.get(i)?;
        let success: bool = result_table.get("success")?;

        if success {
            let result_value: Table = result_table.get("result")?;
            let host_address: String = result_value.get("host_address")?;
            let exit_code: i32 = result_value.get("exit_code")?;
            let stdout: String = result_value.get("stdout")?;
            let task_success: bool = result_value.get("success")?;

            assert_eq!(host_address, "localhost");
            assert_eq!(exit_code, 0);
            assert!(stdout.contains("Hello from parallel komando"));
            assert!(task_success);
        } else {
            // If there's an error, print it for debugging
            if let Ok(error) = result_table.get::<String>("error") {
                panic!("Result {i} failed with error: {error}");
            } else {
                panic!("Result {i} should be successful");
            }
        }
    }

    // Check execution summary
    let success_count: usize = results.get("_success_count")?;
    let failed_count: usize = results.get("_failed_count")?;
    assert_eq!(success_count, 2);
    assert_eq!(failed_count, 0);

    Ok(())
}

#[test]
fn test_komando_integration_with_different_commands() -> mlua::Result<()> {
    let lua = Lua::new();
    let executor =
        ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

    // Create test data with different commands to execute
    let data = lua.create_table()?;
    data.set(1, "echo 'Command 1'")?;
    data.set(2, "echo 'Command 2'")?;
    data.set(3, "echo 'Command 3'")?;

    // Create a function that uses komando to execute different commands
    let func_code = r#"
            return function(cmd)
                local host = {
                    address = "localhost",
                    connection = "local"
                }

                local task = {
                    name = "Dynamic command",
                    komandan.modules.cmd({
                        cmd = cmd
                    })
                }

                local result = komandan.komando(task, host)
                return {
                    command = cmd,
                    exit_code = result.exit_code,
                    stdout = result.stdout:gsub("%s+$", ""), -- trim whitespace
                    success = result.exit_code == 0
                }
            end
        "#;
    let func: mlua::Function = lua.load(func_code).eval()?;

    // Execute parallel map
    let results = executor.map(&lua, &data, &func)?;

    // Check results
    assert_eq!(results.len()?, 3);

    // Verify each command executed correctly
    for i in 1..=3 {
        let result_table: Table = results.get(i)?;
        let success: bool = result_table.get("success")?;

        if success {
            let result_value: Table = result_table.get("result")?;
            let command: String = result_value.get("command")?;
            let exit_code: i32 = result_value.get("exit_code")?;
            let stdout: String = result_value.get("stdout")?;
            let task_success: bool = result_value.get("success")?;

            assert_eq!(exit_code, 0);
            assert!(task_success);

            // Verify the output matches the expected command
            match i {
                1 => {
                    assert_eq!(command, "echo 'Command 1'");
                    assert_eq!(stdout, "Command 1");
                }
                2 => {
                    assert_eq!(command, "echo 'Command 2'");
                    assert_eq!(stdout, "Command 2");
                }
                3 => {
                    assert_eq!(command, "echo 'Command 3'");
                    assert_eq!(stdout, "Command 3");
                }
                _ => panic!("Unexpected result index: {i}"),
            }
        } else if let Ok(error) = result_table.get::<String>("error") {
            panic!("Result {i} failed with error: {error}");
        } else {
            panic!("Result {i} should be successful");
        }
    }

    Ok(())
}

#[test]
fn test_komando_integration_error_handling() -> mlua::Result<()> {
    let lua = Lua::new();
    let executor =
        ParallelExecutor::new(None).map_err(|e| mlua::Error::RuntimeError(e.to_string()))?;

    // Create test data with commands that will fail
    let data = lua.create_table()?;
    data.set(1, "echo 'Success'")?;
    data.set(2, "false")?; // This command will fail
    data.set(3, "echo 'Another success'")?;

    // Create a function that uses komando with error handling
    let func_code = r#"
            return function(cmd)
                local host = {
                    address = "localhost",
                    connection = "local"
                }

                local task = {
                    name = "Test command with error handling",
                    komandan.modules.cmd({
                        cmd = cmd
                    }),
                    ignore_exit_code = true  -- Don't fail on non-zero exit codes
                }

                local result = komandan.komando(task, host)
                return {
                    command = cmd,
                    exit_code = result.exit_code,
                    stdout = result.stdout:gsub("%s+$", ""), -- trim whitespace
                    stderr = result.stderr or "",
                    success = result.exit_code == 0
                }
            end
        "#;
    let func: mlua::Function = lua.load(func_code).eval()?;

    // Execute parallel map
    let results = executor.map(&lua, &data, &func)?;

    // Check results
    assert_eq!(results.len()?, 3);

    // Check individual results
    let first_result: Table = results.get(1)?;
    assert!(first_result.get::<bool>("success")?);
    let result1_value: Table = first_result.get("result")?;
    assert_eq!(result1_value.get::<String>("stdout")?, "Success");
    assert_eq!(result1_value.get::<i32>("exit_code")?, 0);

    let second_result: Table = results.get(2)?;
    assert!(second_result.get::<bool>("success")?); // Function execution succeeded
    let result2_value: Table = second_result.get("result")?;
    assert!(!result2_value.get::<bool>("success")?); // But command failed
    assert_eq!(result2_value.get::<i32>("exit_code")?, 1);

    let third_result: Table = results.get(3)?;
    assert!(third_result.get::<bool>("success")?);
    let result3_value: Table = third_result.get("result")?;
    assert_eq!(result3_value.get::<String>("stdout")?, "Another success");
    assert_eq!(result3_value.get::<i32>("exit_code")?, 0);

    // Check execution summary - all function executions should succeed
    let success_count: usize = results.get("_success_count")?;
    let failed_count: usize = results.get("_failed_count")?;
    assert_eq!(success_count, 3);
    assert_eq!(failed_count, 0);

    Ok(())
}
