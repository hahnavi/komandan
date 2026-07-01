use komandan::create_lua;
use mlua::chunk;

#[test]
fn test_debug_parallel_executor() -> mlua::Result<()> {
    let lua = create_lua()?;

    let result = lua
        .load(chunk! {
            print("=== Debug Info ===")
            print("k type:", type(k))
            if k then
                print("k.parallel_executor type:", type(k.parallel_executor))
                if k.parallel_executor then
                    print("k.parallel_executor.map type:", type(k.parallel_executor.map))
                    print("k.parallel_executor.configure type:", type(k.parallel_executor.configure))
                end
            end

            print("komandan type:", type(komandan))
            if komandan then
                print("komandan.parallel_executor type:", type(komandan.parallel_executor))
            end

            -- Try to access global parallel executor
            local success, result = pcall(function()
                return k.parallel_executor:map({1}, function(x) return x end)
            end)

            print("Parallel executor call success:", success)
            if not success then
                print("Error:", result)
            end

            return success
        })
        .eval::<bool>()?;

    println!("Parallel executor call result: {result}");

    Ok(())
}
