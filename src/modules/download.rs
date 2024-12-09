use mlua::{chunk, ExternalResult, Lua, Table};

pub fn download(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let src = params.get::<String>("src")?;
    let dst = params.get::<String>("dst")?;
    let module = lua
        .load(chunk! {
            local module = komandan.KomandanModule:new({ name = "download" })

            function module:run()
                module.ssh:download($src, $dst)
            end

            return module
        })
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
