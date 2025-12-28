use anyhow::Result;
use komandan::connection::create_connection;
use komandan::create_lua;
use mlua::Value;
use std::time::Instant;

/// Performance tests for connection creation
/// These tests measure the performance of connection creation to ensure
/// no regression in timing or resource usage after refactoring.

#[test]
fn test_local_connection_creation_performance() -> Result<()> {
    let lua = create_lua()?;

    // Warm up
    for _ in 0..10 {
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;
        let _connection = create_connection(&lua, &Value::Table(host_table))?;
    }

    // Measure performance
    let iterations = 1000;
    let start = Instant::now();

    for _ in 0..iterations {
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;
        let _connection = create_connection(&lua, &Value::Table(host_table))?;
    }

    let duration = start.elapsed();
    let avg_duration = duration / iterations;

    println!(
        "Local connection creation: {iterations} iterations in {duration:?} (avg: {avg_duration:?})"
    );

    // Performance assertion - should be very fast for local connections
    // Allow up to 1ms per connection creation (very generous)
    assert!(
        avg_duration.as_millis() < 1,
        "Local connection creation too slow: {avg_duration:?}"
    );

    Ok(())
}

#[test]
fn test_connection_type_detection_performance() -> Result<()> {
    let lua = create_lua()?;

    // Test various localhost address types for performance
    let test_addresses = vec!["localhost", "127.0.0.1", "::1"];

    let iterations = 1000;
    let start = Instant::now();

    for _ in 0..iterations {
        for address in &test_addresses {
            let host_table = lua.create_table()?;
            host_table.set("address", *address)?;
            let _connection = create_connection(&lua, &Value::Table(host_table))?;
        }
    }

    let duration = start.elapsed();
    let total_operations = iterations * u32::try_from(test_addresses.len()).unwrap_or(0);
    let avg_duration = duration / total_operations;

    println!(
        "Connection type detection: {total_operations} operations in {duration:?} (avg: {avg_duration:?})"
    );

    // Performance assertion - connection type detection should be very fast
    assert!(
        avg_duration.as_micros() < 500,
        "Connection type detection too slow: {avg_duration:?}"
    );

    Ok(())
}

#[test]
fn test_memory_usage_stability() -> Result<()> {
    let lua = create_lua()?;

    // Create many connections to test for memory leaks
    let iterations = 5000;

    for i in 0..iterations {
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;

        let _connection = create_connection(&lua, &Value::Table(host_table))?;

        // Periodically print progress
        if i % 1000 == 0 {
            println!("Memory test progress: {i}/{iterations}");
        }
    }

    println!("Memory stability test completed: {iterations} connections created");

    // If we get here without running out of memory, the test passes
    Ok(())
}

#[test]
fn test_sequential_bulk_connection_creation() -> Result<()> {
    let lua = create_lua()?;
    let total_connections = 1000;

    let start = Instant::now();

    for i in 0..total_connections {
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;

        let _connection = create_connection(&lua, &Value::Table(host_table))?;

        if i % 100 == 0 {
            println!("Bulk creation progress: {i}/{total_connections}");
        }
    }

    let duration = start.elapsed();
    let avg_duration = duration / total_connections;

    println!(
        "Sequential bulk connection creation: {total_connections} connections in {duration:?} (avg: {avg_duration:?})"
    );

    // Performance assertion - bulk creation should be efficient
    assert!(
        avg_duration.as_millis() < 2,
        "Sequential bulk connection creation too slow: {avg_duration:?}"
    );

    Ok(())
}
