use mlua::Table;

use super::*;

pub fn collect_core_modules(lua: &mlua::Lua) -> mlua::Result<Table> {
    let modules = lua.create_table()?;
    modules.set("apt", lua.create_function(apt::apt)?)?;
    modules.set("cmd", lua.create_function(cmd::cmd)?)?;
    modules.set("download", lua.create_function(download::download)?)?;
    modules.set("lineinfile", lua.create_function(lineinfile::lineinfile)?)?;
    modules.set("script", lua.create_function(script::script)?)?;
    modules.set(
        "systemd_service",
        lua.create_function(systemd_service::systemd_service)?,
    )?;
    modules.set("template", lua.create_function(template::template)?)?;
    modules.set("upload", lua.create_function(upload::upload)?)?;
    Ok(modules)
}
