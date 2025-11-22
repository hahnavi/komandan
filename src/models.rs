use std::collections::HashMap;

use mlua::{Error, FromLua, IntoLua, Lua, LuaSerdeExt, UserData, Value};
use serde::{Deserialize, Serialize};

use crate::ssh::ElevationMethod;

#[derive(Clone, Debug)]
pub struct Host {
    name: Option<String>,
    address: String,
    port: Option<u16>,
    user: Option<String>,
    key_check: Option<bool>,
    private_key_file: Option<String>,
    private_key_pass: Option<String>,
    password: Option<String>,
    elevate: Option<bool>,
    elevation_method: Option<ElevationMethod>,
    as_user: Option<String>,
    env: Option<HashMap<String, String>>,
}

impl FromLua for Host {
    fn from_lua(lua_value: Value, _: &Lua) -> mlua::Result<Self> {
        let table = lua_value
            .as_table()
            .ok_or_else(|| Error::external("Value is not a table"))?;
        Ok(Self {
            name: table.get("name")?,
            address: table.get("address")?,
            port: table.get("port")?,
            user: table.get("user")?,
            key_check: table.get("host_key_check")?,
            private_key_file: table.get("private_key_file")?,
            private_key_pass: table.get("private_key_pass")?,
            password: table.get("password")?,
            elevate: table.get("elevate")?,
            elevation_method: table.get::<String>("elevation_method").ok().and_then(|s| {
                match s.as_str() {
                    "none" => Some(ElevationMethod::None),
                    "sudo" => Some(ElevationMethod::Sudo),
                    "su" => Some(ElevationMethod::Su),
                    _ => None,
                }
            }),
            as_user: table.get("as_user")?,
            env: table.get("env")?,
        })
    }
}

impl IntoLua for Host {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        if let Some(name) = self.name {
            table.set("name", name)?;
        }
        table.set("address", self.address)?;
        if let Some(port) = self.port {
            table.set("port", port)?;
        }
        if let Some(user) = self.user {
            table.set("user", user)?;
        }
        if let Some(key_check) = self.key_check {
            table.set("host_key_check", key_check)?;
        }
        if let Some(private_key_file) = self.private_key_file {
            table.set("private_key_file", private_key_file)?;
        }
        if let Some(private_key_pass) = self.private_key_pass {
            table.set("private_key_pass", private_key_pass)?;
        }
        if let Some(password) = self.password {
            table.set("password", password)?;
        }
        if let Some(elevate) = self.elevate {
            table.set("elevate", elevate)?;
        }
        if let Some(elevation_method) = self.elevation_method {
            match elevation_method {
                ElevationMethod::None => table.set("elevation_method", "none")?,
                ElevationMethod::Sudo => table.set("elevation_method", "sudo")?,
                ElevationMethod::Su => table.set("elevation_method", "su")?,
            }
        }
        if let Some(as_user) = self.as_user {
            table.set("as_user", as_user)?;
        }
        if let Some(env) = self.env {
            table.set("env", env)?;
        }
        Ok(Value::Table(table))
    }
}

#[derive(Clone, Debug)]
pub struct Task {
    name: Option<String>,
    module: Module,
    ignore_exit_code: Option<bool>,
    elevate: Option<bool>,
    elevation_method: Option<ElevationMethod>,
    as_user: Option<String>,
    env: Option<HashMap<String, String>>,
}

impl FromLua for Task {
    fn from_lua(lua_value: Value, lua: &Lua) -> mlua::Result<Self> {
        let table = lua_value
            .as_table()
            .ok_or_else(|| Error::external("Value is not a table"))?;
        Ok(Self {
            name: table.get("name")?,
            module: Module::from_lua(table.get(1)?, lua)?,
            ignore_exit_code: table.get("ignore_exit_code")?,
            elevate: table.get("elevate")?,
            elevation_method: table.get::<String>("elevation_method").ok().and_then(|s| {
                match s.as_str() {
                    "none" => Some(ElevationMethod::None),
                    "sudo" => Some(ElevationMethod::Sudo),
                    "su" => Some(ElevationMethod::Su),
                    _ => None,
                }
            }),
            as_user: table.get("as_user")?,
            env: table.get("env")?,
        })
    }
}

impl IntoLua for Task {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        if let Some(name) = self.name {
            table.set("name", name)?;
        }
        table.set(1, self.module.into_lua(lua)?)?;
        if let Some(ignore_exit_code) = self.ignore_exit_code {
            table.set("ignore_exit_code", ignore_exit_code)?;
        }
        if let Some(elevate) = self.elevate {
            table.set("elevate", elevate)?;
        }
        if let Some(elevation_method) = self.elevation_method {
            match elevation_method {
                ElevationMethod::None => table.set("elevation_method", "none")?,
                ElevationMethod::Sudo => table.set("elevation_method", "sudo")?,
                ElevationMethod::Su => table.set("elevation_method", "su")?,
            }
        }
        if let Some(as_user) = self.as_user {
            table.set("as_user", as_user)?;
        }
        if let Some(env) = self.env {
            table.set("env", env)?;
        }
        Ok(Value::Table(table))
    }
}

#[derive(Clone, Debug)]
pub struct Module {
    functions: HashMap<String, Vec<u8>>,
    others: HashMap<String, String>,
}

impl FromLua for Module {
    fn from_lua(value: Value, _: &Lua) -> mlua::Result<Self> {
        let table = value
            .as_table()
            .ok_or_else(|| Error::external("Value is not a table"))?;
        let mut functions: HashMap<String, Vec<u8>> = HashMap::new();
        let mut others: HashMap<String, String> = HashMap::new();
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair?;
            if value.is_function() {
                functions.insert(
                    key.to_string()?,
                    value
                        .as_function()
                        .ok_or_else(|| Error::external("Value is not a function"))?
                        .dump(true),
                );
            } else {
                others.insert(
                    key.to_string()?,
                    serde_json::to_string(&value).map_err(Error::external)?,
                );
            }
        }
        Ok(Self { functions, others })
    }
}

impl IntoLua for Module {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        for (key, value) in &self.functions {
            table.set(key.as_str(), lua.load(value).into_function()?)?;
        }
        for (key, value) in &self.others {
            let json: serde_json::Value = serde_json::from_str(value).map_err(Error::external)?;
            table.set(key.as_str(), lua.to_value(&json)?)?;
        }
        Ok(Value::Table(table))
    }
}

#[derive(Serialize, Deserialize)]
pub struct KomandoResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
    changed: bool,
}

impl UserData for KomandoResult {}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;
    use std::collections::HashMap;

    #[test]
    fn test_host_from_lua() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("address", "127.0.0.1")?;
        let host = Host::from_lua(Value::Table(table.clone()), &lua)?;
        assert_eq!(host.address, "127.0.0.1");
        assert_eq!(host.name, None);

        table.set("name", "test")?;
        table.set("port", 22)?;
        table.set("user", "user")?;
        table.set("private_key_file", "/path/to/key")?;
        table.set("private_key_pass", "pass")?;
        table.set("password", "password")?;
        table.set("elevate", true)?;
        table.set("elevation_method", "sudo")?;
        table.set("as_user", "root")?;
        let mut env = HashMap::new();
        env.insert("key".to_string(), "value".to_string());
        table.set("env", env.clone())?;

        let host = Host::from_lua(Value::Table(table), &lua)?;
        assert_eq!(host.address, "127.0.0.1");
        assert_eq!(host.name, Some("test".to_string()));
        assert_eq!(host.port, Some(22));
        assert_eq!(host.user, Some("user".to_string()));
        assert_eq!(host.private_key_file, Some("/path/to/key".to_string()));
        assert_eq!(host.private_key_pass, Some("pass".to_string()));
        assert_eq!(host.password, Some("password".to_string()));
        assert_eq!(host.elevate, Some(true));
        assert_eq!(host.elevation_method, Some(ElevationMethod::Sudo));
        assert_eq!(host.as_user, Some("root".to_string()));
        assert_eq!(host.env, Some(env));
        Ok(())
    }

    #[test]
    fn test_host_into_lua() -> mlua::Result<()> {
        let lua = Lua::new();
        let mut env = HashMap::new();
        env.insert("key".to_string(), "value".to_string());
        let host = Host {
            name: Some("test".to_string()),
            address: "127.0.0.1".to_string(),
            port: Some(22),
            user: Some("user".to_string()),
            key_check: None,
            private_key_file: Some("/path/to/key".to_string()),
            private_key_pass: Some("pass".to_string()),
            password: Some("password".to_string()),
            elevate: Some(true),
            elevation_method: Some(ElevationMethod::Sudo),
            as_user: Some("root".to_string()),
            env: Some(env.clone()),
        };

        let table = host
            .into_lua(&lua)?
            .as_table()
            .ok_or_else(|| Error::external("Value is not a table"))?
            .clone();
        assert_eq!(table.get::<String>("address")?, "127.0.0.1");
        assert_eq!(table.get::<String>("name")?, "test");
        assert_eq!(table.get::<u16>("port")?, 22);
        assert_eq!(table.get::<String>("user")?, "user");
        assert_eq!(table.get::<String>("private_key_file")?, "/path/to/key");
        assert_eq!(table.get::<String>("private_key_pass")?, "pass");
        assert_eq!(table.get::<String>("password")?, "password");
        assert!(table.get::<bool>("elevate")?);
        assert_eq!(table.get::<String>("elevation_method")?, "sudo");
        assert_eq!(table.get::<String>("as_user")?, "root");
        assert_eq!(table.get::<HashMap<String, String>>("env")?, env);
        Ok(())
    }

    #[test]
    fn test_task_from_lua() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        let module_table = lua.create_table()?;
        module_table.set("command", "echo 'hello'")?;
        table.set(1, module_table)?;
        let task = Task::from_lua(Value::Table(table.clone()), &lua)?;
        assert_eq!(task.name, None);
        assert!(task.module.functions.is_empty());
        assert_eq!(
            task.module.others.get("command"),
            Some(&"\"echo 'hello'\"".to_string())
        );

        table.set("name", "test")?;
        table.set("ignore_exit_code", true)?;
        table.set("elevate", true)?;
        table.set("elevation_method", "sudo")?;
        table.set("as_user", "root")?;
        let mut env = HashMap::new();
        env.insert("key".to_string(), "value".to_string());
        table.set("env", env.clone())?;

        let task = Task::from_lua(Value::Table(table), &lua)?;
        assert_eq!(task.name, Some("test".to_string()));
        assert_eq!(task.ignore_exit_code, Some(true));
        assert_eq!(task.elevate, Some(true));
        assert_eq!(task.elevation_method, Some(ElevationMethod::Sudo));
        assert_eq!(task.as_user, Some("root".to_string()));
        assert_eq!(task.env, Some(env));
        Ok(())
    }

    #[test]
    fn test_module_from_lua() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("command", "echo 'hello'")?;
        let module = Module::from_lua(Value::Table(table.clone()), &lua)?;
        assert!(module.functions.is_empty());
        assert_eq!(
            module.others.get("command"),
            Some(&"\"echo 'hello'\"".to_string())
        );

        let function = lua.load("return 1").into_function()?;
        table.set("test_func", function)?;

        let module = Module::from_lua(Value::Table(table), &lua)?;
        assert_eq!(module.functions.len(), 1);
        assert_eq!(
            module.others.get("command"),
            Some(&"\"echo 'hello'\"".to_string())
        );
        Ok(())
    }
}
