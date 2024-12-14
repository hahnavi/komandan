use mlua::{Lua, Table};

pub fn defaults(lua: &Lua) -> mlua::Result<Table> {
    let defaults = lua.create_table()?;

    defaults.set("port", 22)?;
    defaults.set("ignore_exit_code", false)?;
    defaults.set("elevate", false)?;
    defaults.set("elevation_method", "sudo")?;

    let env = lua.create_table()?;
    env.set("DEBIAN_FRONTEND", "noninteractive")?;
    defaults.set("env", env)?;

    Ok(defaults)
}
