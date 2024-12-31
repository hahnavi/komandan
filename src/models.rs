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
        let table = lua_value.as_table().unwrap();
        Ok(Host {
            name: table.get("name")?,
            address: table.get("address")?,
            port: table.get("port")?,
            user: table.get("user")?,
            private_key_file: table.get("private_key_file")?,
            private_key_pass: table.get("private_key_pass")?,
            password: table.get("password")?,
            elevate: table.get("elevate")?,
            elevation_method: match table.get::<String>("elevation_method") {
                Ok(elevation_method) => match elevation_method.as_str() {
                    "none" => Some(ElevationMethod::None),
                    "sudo" => Some(ElevationMethod::Sudo),
                    "su" => Some(ElevationMethod::Su),
                    _ => None,
                },
                Err(_) => None,
            },
            as_user: table.get("as_user")?,
            env: table.get("env")?,
        })
    }
}

impl IntoLua for Host {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        if self.name.is_some() {
            table.set("name", self.name.unwrap())?;
        }
        table.set("address", self.address)?;
        if self.port.is_some() {
            table.set("port", self.port.unwrap())?;
        }
        if self.user.is_some() {
            table.set("user", self.user.unwrap())?;
        }
        if self.private_key_file.is_some() {
            table.set("private_key_file", self.private_key_file.unwrap())?;
        }
        if self.private_key_pass.is_some() {
            table.set("private_key_pass", self.private_key_pass.unwrap())?;
        }
        if self.password.is_some() {
            table.set("password", self.password.unwrap())?;
        }
        if self.elevate.is_some() {
            table.set("elevate", self.elevate.unwrap())?;
        }
        if self.elevation_method.is_some() {
            match self.elevation_method.unwrap() {
                ElevationMethod::None => table.set("elevation_method", "none")?,
                ElevationMethod::Sudo => table.set("elevation_method", "sudo")?,
                ElevationMethod::Su => table.set("elevation_method", "su")?,
            }
        }
        if self.as_user.is_some() {
            table.set("as_user", self.as_user.unwrap())?;
        }
        if self.env.is_some() {
            table.set("env", self.env.unwrap())?;
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
        let table = lua_value.as_table().unwrap();
        Ok(Task {
            name: table.get("name")?,
            module: Module::from_lua(table.get(1)?, lua)?,
            ignore_exit_code: table.get("ignore_exit_code")?,
            elevate: table.get("elevate")?,
            elevation_method: match table.get::<String>("elevation_method") {
                Ok(elevation_method) => match elevation_method.as_str() {
                    "none" => Some(ElevationMethod::None),
                    "sudo" => Some(ElevationMethod::Sudo),
                    "su" => Some(ElevationMethod::Su),
                    _ => None,
                },
                Err(_) => None,
            },
            as_user: table.get("as_user")?,
            env: table.get("env")?,
        })
    }
}

impl IntoLua for Task {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        if self.name.is_some() {
            table.set("name", self.name.unwrap())?;
        }
        table.set(1, self.module.into_lua(lua)?)?;
        if self.ignore_exit_code.is_some() {
            table.set("ignore_exit_code", self.ignore_exit_code.unwrap())?;
        }
        if self.elevate.is_some() {
            table.set("elevate", self.elevate.unwrap())?;
        }
        if self.elevation_method.is_some() {
            match self.elevation_method.unwrap() {
                ElevationMethod::None => table.set("elevation_method", "none")?,
                ElevationMethod::Sudo => table.set("elevation_method", "sudo")?,
                ElevationMethod::Su => table.set("elevation_method", "su")?,
            }
        }
        if self.as_user.is_some() {
            table.set("as_user", self.as_user.unwrap())?;
        }
        if self.env.is_some() {
            table.set("env", self.env.unwrap())?;
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
        let table = value.as_table().unwrap();
        let mut functions: HashMap<String, Vec<u8>> = HashMap::new();
        let mut others: HashMap<String, String> = HashMap::new();
        for pair in table.pairs::<Value, Value>() {
            let (key, value) = pair.unwrap();
            if value.is_function() {
                functions.insert(key.to_string()?, value.as_function().unwrap().dump(true));
            } else {
                others.insert(
                    key.to_string()?,
                    serde_json::to_string(&value).map_err(Error::external)?,
                );
            }
        }
        Ok(Module { functions, others })
    }
}

impl IntoLua for Module {
    fn into_lua(self, lua: &Lua) -> mlua::Result<Value> {
        let table = lua.create_table()?;
        self.functions.iter().for_each(|(key, value)| {
            table
                .set(key.as_str(), lua.load(value).into_function().unwrap())
                .unwrap();
        });
        self.others.iter().for_each(|(key, value)| {
            let json: serde_json::Value = serde_json::from_str(value).unwrap();
            table
                .set(key.as_str(), lua.to_value(&json).unwrap())
                .unwrap();
        });
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
    fn test_host_from_lua() {
        let lua = Lua::new();
        let table = lua.create_table().unwrap();
        table.set("address", "127.0.0.1").unwrap();
        let host = Host::from_lua(Value::Table(table.clone()), &lua).unwrap();
        assert_eq!(host.address, "127.0.0.1");
        assert_eq!(host.name, None);

        table.set("name", "test").unwrap();
        table.set("port", 22).unwrap();
        table.set("user", "user").unwrap();
        table.set("private_key_file", "/path/to/key").unwrap();
        table.set("private_key_pass", "pass").unwrap();
        table.set("password", "password").unwrap();
        table.set("elevate", true).unwrap();
        table.set("elevation_method", "sudo").unwrap();
        table.set("as_user", "root").unwrap();
        let mut env = HashMap::new();
        env.insert("key".to_string(), "value".to_string());
        table.set("env", env.clone()).unwrap();

        let host = Host::from_lua(Value::Table(table), &lua).unwrap();
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
    }

    #[test]
    fn test_host_into_lua() {
        let lua = Lua::new();
        let mut env = HashMap::new();
        env.insert("key".to_string(), "value".to_string());
        let host = Host {
            name: Some("test".to_string()),
            address: "127.0.0.1".to_string(),
            port: Some(22),
            user: Some("user".to_string()),
            private_key_file: Some("/path/to/key".to_string()),
            private_key_pass: Some("pass".to_string()),
            password: Some("password".to_string()),
            elevate: Some(true),
            elevation_method: Some(ElevationMethod::Sudo),
            as_user: Some("root".to_string()),
            env: Some(env.clone()),
        };

        let table = host.into_lua(&lua).unwrap().as_table().unwrap().clone();
        assert_eq!(table.get::<String>("address").unwrap(), "127.0.0.1");
        assert_eq!(table.get::<String>("name").unwrap(), "test");
        assert_eq!(table.get::<u16>("port").unwrap(), 22);
        assert_eq!(table.get::<String>("user").unwrap(), "user");
        assert_eq!(
            table.get::<String>("private_key_file").unwrap(),
            "/path/to/key"
        );
        assert_eq!(table.get::<String>("private_key_pass").unwrap(), "pass");
        assert_eq!(table.get::<String>("password").unwrap(), "password");
        assert_eq!(table.get::<bool>("elevate").unwrap(), true);
        assert_eq!(table.get::<String>("elevation_method").unwrap(), "sudo");
        assert_eq!(table.get::<String>("as_user").unwrap(), "root");
        assert_eq!(table.get::<HashMap<String, String>>("env").unwrap(), env);
    }

    #[test]
    fn test_task_from_lua() {
        let lua = Lua::new();
        let table = lua.create_table().unwrap();
        let module_table = lua.create_table().unwrap();
        module_table.set("command", "echo 'hello'").unwrap();
        table.set(1, module_table).unwrap();
        let task = Task::from_lua(Value::Table(table.clone()), &lua).unwrap();
        assert_eq!(task.name, None);
        assert!(task.module.functions.is_empty());
        assert_eq!(
            task.module.others.get("command").unwrap().clone(),
            "\"echo 'hello'\""
        );

        table.set("name", "test").unwrap();
        table.set("ignore_exit_code", true).unwrap();
        table.set("elevate", true).unwrap();
        table.set("elevation_method", "sudo").unwrap();
        table.set("as_user", "root").unwrap();
        let mut env = HashMap::new();
        env.insert("key".to_string(), "value".to_string());
        table.set("env", env.clone()).unwrap();

        let task = Task::from_lua(Value::Table(table), &lua).unwrap();
        assert_eq!(task.name, Some("test".to_string()));
        assert_eq!(task.ignore_exit_code, Some(true));
        assert_eq!(task.elevate, Some(true));
        assert_eq!(task.elevation_method, Some(ElevationMethod::Sudo));
        assert_eq!(task.as_user, Some("root".to_string()));
        assert_eq!(task.env, Some(env));
    }

    #[test]
    fn test_module_from_lua() {
        let lua = Lua::new();
        let table = lua.create_table().unwrap();
        table.set("command", "echo 'hello'").unwrap();
        let module = Module::from_lua(Value::Table(table.clone()), &lua).unwrap();
        assert!(module.functions.is_empty());
        assert_eq!(
            module.others.get("command").unwrap().clone(),
            "\"echo 'hello'\""
        );

        let function = lua.load("return 1").into_function().unwrap();
        table.set("test_func", function).unwrap();

        let module = Module::from_lua(Value::Table(table), &lua).unwrap();
        assert_eq!(module.functions.len(), 1);
        assert_eq!(
            module.others.get("command").unwrap().clone(),
            "\"echo 'hello'\""
        );
    }
}
