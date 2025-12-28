mod base;
mod file;
mod package;
mod service;

use mlua::{Lua, Table};

/// Collects all check functions into a table for the komandan.check namespace
pub fn collect_check_functions(lua: &Lua) -> mlua::Result<Table> {
    let check_functions = lua.create_table()?;

    // Add implemented check functions
    check_functions.set("file", lua.create_function(file::check_file)?)?;
    check_functions.set("service", lua.create_function(service::check_service)?)?;
    check_functions.set("package", lua.create_function(package::check_package)?)?;

    Ok(check_functions)
}
