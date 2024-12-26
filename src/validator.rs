use mlua::{chunk, Error::RuntimeError, ExternalResult, Integer, Lua, Table, Value};

pub fn validate_host(lua: &Lua, host: Value) -> mlua::Result<Table> {
    if !host.is_table() {
        return Err(RuntimeError(format!("Host is not a table.")));
    }

    let address = host.as_table().unwrap().get::<Value>("address")?;
    if address.is_nil() {
        return Err(RuntimeError(format!("Host address is empty.")));
    }
    if !address.is_string() {
        return Err(RuntimeError(format!("Host address is invalid.")));
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

pub fn validate_task(lua: &Lua, task: Value) -> mlua::Result<Table> {
    if !task.is_table() {
        return Err(RuntimeError("Task is not a table.".to_string()));
    }

    let task = task.as_table().unwrap();
    if task.get::<Value>(1)?.is_nil() {
        return Err(RuntimeError("Task is invalid.".to_string()));
    }

    validate_module(lua, task.get::<Value>(1)?).into_lua_err()?;

    Ok(task.to_owned())
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

// Tests
#[cfg(test)]
mod tests {
    use crate::modules::cmd;

    use super::*;

    #[test]
    fn test_validate_host_valid() {
        let lua = Lua::new();
        let host = lua.create_table().unwrap();
        host.set("address", "127.0.0.1").unwrap();
        host.set("port", 22).unwrap();

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_host_not_table() {
        let lua = Lua::new();
        let result = super::validate_host(&lua, mlua::Value::Nil);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Host is not a table."
        );
    }

    #[test]
    fn test_validate_host_missing_address() {
        let lua = Lua::new();
        let host = lua.create_table().unwrap();
        host.set("port", 22).unwrap();

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Host address is empty."
        );
    }

    #[test]
    fn test_validate_host_invalid_address_type() {
        let lua = Lua::new();
        let host = lua.create_table().unwrap();
        host.set("address", 123).unwrap();
        host.set("port", 22).unwrap();

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Host address is invalid."
        );
    }

    #[test]
    fn test_validate_host_valid_port() {
        let lua = Lua::new();
        let host = lua.create_table().unwrap();
        host.set("address", "127.0.0.1").unwrap();
        host.set("port", 22).unwrap();

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_host_invalid_port_type() {
        let lua = Lua::new();
        let host = lua.create_table().unwrap();
        host.set("address", "127.0.0.1").unwrap();
        host.set("port", "22").unwrap();

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .starts_with("runtime error: Port is not an integer."),);
    }

    #[test]
    fn test_validate_host_port_out_of_range() {
        let lua = Lua::new();
        let host = lua.create_table().unwrap();
        host.set("address", "127.0.0.1").unwrap();
        host.set("port", 65536).unwrap();

        let result = super::validate_host(&lua, mlua::Value::Table(host));
        assert!(result.is_err());
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .starts_with("runtime error: Port is out of range."),);
    }

    #[test]
    fn test_validate_port_valid_min() {
        let lua = Lua::new();
        let result = super::validate_port(&lua, mlua::Value::Integer(0));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_validate_port_valid_max() {
        let lua = Lua::new();
        let result = super::validate_port(&lua, mlua::Value::Integer(65535));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 65535);
    }

    #[test]
    fn test_validate_port_invalid_negative() {
        let lua = Lua::new();
        let result = super::validate_port(&lua, mlua::Value::Integer(-1));
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Port is out of range."
        );
    }

    #[test]
    fn test_validate_port_invalid_too_large() {
        let lua = Lua::new();
        let result = super::validate_port(&lua, mlua::Value::Integer(65536));
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Port is out of range."
        );
    }

    #[test]
    fn test_validate_task_valid() {
        let lua = Lua::new();
        let task = lua.create_table().unwrap();
        let module = lua.create_table().unwrap();
        module.set("name", "cmd").unwrap();
        task.set(1, module).unwrap();

        let result = super::validate_task(&lua, mlua::Value::Table(task));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_task_not_table() {
        let lua = Lua::new();
        let result = super::validate_task(&lua, mlua::Value::Nil);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Task is not a table."
        );
    }

    #[test]
    fn test_validate_task_empty() {
        let lua = Lua::new();
        let task = lua.create_table().unwrap();

        let result = super::validate_task(&lua, mlua::Value::Table(task));
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Task is invalid."
        );
    }

    #[test]
    fn test_validate_module_valid_string() {
        let lua = Lua::new();
        let komandan = lua.create_table().unwrap();

        let komandan_module = lua
            .load(chunk! {
                    local KomandanModule = {}

            function KomandanModule:new(data)
                local o = setmetatable({}, { __index = self })
                o.name = data.name
                o.script = data.script
                return o
            end

            function KomandanModule:run_module()
            self:run()
            end

            return KomandanModule
                })
            .eval::<Table>()
            .unwrap();

        komandan.set("KomandanModule", komandan_module).unwrap();

        let modules = lua.create_table().unwrap();
        modules
            .set("cmd", lua.create_function(cmd).unwrap())
            .unwrap();
        komandan.set("modules", modules).unwrap();
        lua.globals().set("komandan", komandan).unwrap();

        let result =
            super::validate_module(&lua, mlua::Value::String(lua.create_string("ls").unwrap()));
        eprint!("result: {:#?}\n", result.clone().err());
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_module_valid_table() {
        let lua = Lua::new();
        let module = lua.create_table().unwrap();
        module.set("cmd", "ls").unwrap();

        let result = super::validate_module(&lua, mlua::Value::Table(module));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_module_invalid() {
        let lua = Lua::new();
        let result = super::validate_module(&lua, mlua::Value::Nil);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "runtime error: Module is invalid"
        );
    }
}
