use mlua::{Lua, Table};

pub fn defaults(lua: &Lua) -> mlua::Result<Table> {
    let defaults = lua.create_table()?;

    defaults.set("port", 22)?;
    defaults.set("ignore_exit_code", false)?;

    Ok(defaults)
}
