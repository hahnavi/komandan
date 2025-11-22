use mlua::{Error::RuntimeError, ExternalResult, Integer, Lua, Table, Value, chunk};

pub fn validate_host(lua: &Lua, host: Value) -> mlua::Result<Table> {
    let Value::Table(host_table) = host else {
        return Err(RuntimeError("Host is not a table.".to_string()));
    };

    let address = host_table.get::<Value>("address")?;
    if address.is_nil() {
        return Err(RuntimeError("Host address is empty.".to_string()));
    }
    if !address.is_string() {
        return Err(RuntimeError("Host address is invalid.".to_string()));
    }

    let port = host_table.get::<Value>("port")?;
    if !port.is_nil() {
        validate_port(lua, &port)?;
    }

    Ok(host_table)
}

fn validate_port(_: &Lua, port: &Value) -> mlua::Result<Integer> {
    match port {
        Value::Integer(port_value) => {
            if !(0..=65535).contains(port_value) {
                return Err(RuntimeError("Port is out of range.".to_string()));
            }
            Ok(*port_value)
        }
        _ => Err(RuntimeError("Port is not an integer.".to_string())),
    }
}

pub fn validate_task(lua: &Lua, task: Value) -> mlua::Result<Table> {
    let Value::Table(task_table) = task else {
        return Err(RuntimeError("Task is not a table.".to_string()));
    };

    if task_table.get::<Value>(1)?.is_nil() {
        return Err(RuntimeError("Task is invalid.".to_string()));
    }

    validate_module(lua, task_table.get::<Value>(1)?).into_lua_err()?;

    Ok(task_table)
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

    Ok(module
        .as_table()
        .ok_or_else(|| RuntimeError("Module is not a table".to_string()))?
        .to_owned())
}

// Tests
#[cfg(test)]
mod tests {
    use crate::create_lua;

    #[test]
    fn test_validate_host_valid() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "127.0.0.1")?;
        host.set("port", 22)?;

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_validate_host_not_table() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = super::validate_host(&lua, mlua::Value::Nil);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Host is not a table.");
        }
        Ok(())
    }

    #[test]
    fn test_validate_host_missing_address() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("port", 22)?;

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Host address is empty.");
        }
        Ok(())
    }

    #[test]
    fn test_validate_host_invalid_address_type() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", 123)?;
        host.set("port", 22)?;

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Host address is invalid.");
        }
        Ok(())
    }

    #[test]
    fn test_validate_host_valid_port() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "127.0.0.1")?;
        host.set("port", 22)?;

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_validate_host_invalid_port_type() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "127.0.0.1")?;
        host.set("port", "22")?;

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .starts_with("runtime error: Port is not an integer.")
            );
        }
        Ok(())
    }

    #[test]
    fn test_validate_host_port_out_of_range() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "127.0.0.1")?;
        host.set("port", 65536)?;

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                e.to_string()
                    .starts_with("runtime error: Port is out of range.")
            );
        }
        Ok(())
    }

    #[test]
    fn test_validate_port_valid_min() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = super::validate_port(&lua, &mlua::Value::Integer(0));
        assert!(result.is_ok());
        assert_eq!(result?, 0);
        Ok(())
    }

    #[test]
    fn test_validate_port_valid_max() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = super::validate_port(&lua, &mlua::Value::Integer(65535));
        assert!(result.is_ok());
        assert_eq!(result?, 65535);
        Ok(())
    }

    #[test]
    fn test_validate_port_invalid_negative() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = super::validate_port(&lua, &mlua::Value::Integer(-1));
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Port is out of range.");
        }
        Ok(())
    }

    #[test]
    fn test_validate_port_invalid_too_large() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = super::validate_port(&lua, &mlua::Value::Integer(65536));
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Port is out of range.");
        }
        Ok(())
    }

    #[test]
    fn test_validate_task_valid() -> mlua::Result<()> {
        let lua = create_lua()?;
        let task = lua.create_table()?;
        let module = lua.create_table()?;
        module.set("name", "cmd")?;
        task.set(1, module)?;

        let result = super::validate_task(&lua, mlua::Value::Table(task));
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_validate_task_not_table() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = super::validate_task(&lua, mlua::Value::Nil);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Task is not a table.");
        }
        Ok(())
    }

    #[test]
    fn test_validate_task_empty() -> mlua::Result<()> {
        let lua = create_lua()?;
        let task = lua.create_table()?;

        let result = super::validate_task(&lua, mlua::Value::Table(task));
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Task is invalid.");
        }
        Ok(())
    }

    #[test]
    fn test_validate_module_valid_string() -> mlua::Result<()> {
        let lua = create_lua()?;

        let result = super::validate_module(&lua, mlua::Value::String(lua.create_string("ls")?));
        eprintln!("result: {:#?}", result.clone().err());
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_validate_module_valid_table() -> mlua::Result<()> {
        let lua = create_lua()?;
        let module = lua.create_table()?;
        module.set("cmd", "ls")?;

        let result = super::validate_module(&lua, mlua::Value::Table(module));
        assert!(result.is_ok());
        Ok(())
    }

    #[test]
    fn test_validate_module_invalid() -> mlua::Result<()> {
        let lua = create_lua()?;
        let result = super::validate_module(&lua, mlua::Value::Nil);
        assert!(result.is_err());
        if let Err(e) = result {
            assert_eq!(e.to_string(), "runtime error: Module is invalid");
        }
        Ok(())
    }
}
