use mlua::{chunk, ExternalResult, Lua, Table};

pub fn cmd(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let cmd = params.get::<String>("cmd")?;
    let module = lua
        .load(chunk! {
            local module = komandan.KomandanModule:new({ name = "cmd" })

            function module:run()
                module.ssh:cmd($cmd)
            end

            return module
        })
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
