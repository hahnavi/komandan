use mlua::{chunk, ExternalResult, Lua, Table};

pub fn cmd(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let cmd = params.get::<String>("cmd")?;

    let base_module = super::base_module(&lua);
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "cmd" })

            function module:run()
                module.ssh:cmd($cmd)
            end

            return module
        })
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
