use komandan::create_lua;
use mlua::{Integer, Table, chunk};

/// Simple test to verify parallel executor basic functionality
#[test]
fn test_simple_parallel_execution() -> mlua::Result<()> {
    let lua = create_lua()?;

    let results = lua
        .load(chunk! {
            local data = {1, 2, 3}
            local results = k.parallel_executor:map(data, function(item)
                return item * 2
            end)
            return results
        })
        .eval::<Table>()?;

    // Check that we got results
    assert!(results.len()? > 0);

    // Check first result structure
    let first_result: Table = results.get(1)?;

    // Print debug info
    println!("First result keys:");
    for pair in first_result.pairs::<mlua::Value, mlua::Value>() {
        let (key, value) = pair?;
        println!("  {key:?} = {value:?}");
    }

    // Basic assertions
    assert!(first_result.get::<bool>("success")?);
    let result_value: Integer = first_result.get::<Integer>("result")?;
    assert_eq!(result_value, 2);

    Ok(())
}

#[test]
fn test_parallel_executor_exists() -> mlua::Result<()> {
    let lua = create_lua()?;

    // Just verify that k.parallel_executor exists and has a map method
    let result = lua
        .load(chunk! {
            return type(k.parallel_executor) == "table" and
                   type(k.parallel_executor.map) == "function" and
                   type(k.parallel_executor.configure) == "function"
        })
        .eval::<bool>()?;

    assert!(
        result,
        "k.parallel_executor should exist and have map and configure methods"
    );

    Ok(())
}
