use mlua::{ExternalResult, Lua, Table, chunk};

pub fn cmd(lua: &Lua, params: Table) -> mlua::Result<Table> {
    let base_module = super::base_module(lua)?;
    let module = lua
        .load(chunk! {
            local module = $base_module:new({ name = "cmd" })

            module.params = $params

            module.run = function(self)
                self.ssh:cmd(self.params.cmd)
            end

            return module
        })
        .set_name("cmd")
        .eval::<Table>()
        .into_lua_err()?;

    Ok(module)
}
