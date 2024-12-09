use mlua::{chunk, ExternalResult, Lua, Table};

pub fn upload(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let src = params.get::<String>("src")?;
    let dst = params.get::<String>("dst")?;
    let module = lua
        .load(chunk! {
            local module = komandan.KomandanModule:new({ name = "upload" })

            function module:run()
                module.ssh:upload($src, $dst)
            end

            return module
        })
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
