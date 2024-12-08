use mlua::{chunk, Error::RuntimeError, Integer, Lua, Table, Value};

pub fn validate_host(lua: &Lua, host: Value) -> mlua::Result<Table> {
    if !host.is_table() {
        return Err(RuntimeError(format!("Host is not a table: {:?}.", host)));
    }

    let address = host.as_table().unwrap().get::<Value>("address")?;
    if address.is_nil() {
        return Err(RuntimeError(format!("Host address is empty: {:?}.", host)));
    }
    if !address.is_string() {
        return Err(RuntimeError(format!(
            "Host address is invalid: {:?}.",
            host
        )));
    }

    let port = host.as_table().unwrap().get::<Value>("port")?;
    if !port.is_nil() {
        lua.create_function(validate_port)?.call::<Integer>(port)?;
    }

    Ok(host.as_table().unwrap().to_owned())
}

fn validate_port(_: &Lua, port: Value) -> mlua::Result<Integer> {
    if !port.is_integer() {
        return Err(RuntimeError("Port is not an integer.".to_string()));
    }

    if !(port.as_integer().unwrap() >= 0 && port.as_integer().unwrap() <= 65535) {
        return Err(RuntimeError("Port is out of range.".to_string()));
    }

    Ok(port.as_integer().unwrap())
}

pub fn validate_task(_: &Lua, task: Value) -> mlua::Result<Table> {
    if !task.is_table() {
        return Err(RuntimeError("Task is not a table.".to_string()));
    }

    if task.as_table().unwrap().get::<Value>(1)?.is_nil() {
        return Err(RuntimeError("Task is invalid.".to_string()));
    }

    Ok(task.as_table().unwrap().to_owned())
}

pub fn validate_module(lua: &Lua, module: Value) -> mlua::Result<Table> {
    if module.is_string() {
        let module = lua
            .load(chunk! {
                return komandan.modules.cmd({ cmd = $module })
            })
            .eval::<Table>()?;

        return Ok(module);
    }

    if !module.is_table() {
        return Err(RuntimeError("Module is invalid".to_string()));
    }

    Ok(module.as_table().unwrap().to_owned())
}
