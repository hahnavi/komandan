use std::collections::HashMap;

use mlua::{Error, FromLua, IntoLua, Lua, LuaSerdeExt, UserData, Value};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};

use crate::ssh::ElevationMethod;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionType {
    Local,
    SSH,
}

impl std::str::FromStr for ConnectionType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "local" => Ok(Self::Local),
            "ssh" => Ok(Self::SSH),
            _ => Err(format!(
                "invalid connection type '{s}' (expected 'local' or 'ssh')"
            )),
        }
    }
}

impl ConnectionType {
    #[must_use]
    pub const fn as_str(&self) -> &str {
        match self {
            Self::Local => "local",
            Self::SSH => "ssh",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Host {
    name: Option<String>,
    address: String,
    port: Option<u16>,
    user: Option<String>,
    key_check: Option<bool>,
    private_key_file: Option<String>,
    private_key_pass: Option<SecretString>,
    password: Option<SecretString>,
    elevate: Option<bool>,
    elevation_method: Option<ElevationMethod>,
    as_user: Option<String>,
    env: Option<HashMap<String, String>>,
    connection: Option<ConnectionType>,
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
            private_key_pass: table
                .get::<Option<String>>("private_key_pass")?
                .map(|s| SecretString::new(s.into_boxed_str())),
            password: table
                .get::<Option<String>>("password")?
                .map(|s| SecretString::new(s.into_boxed_str())),
            elevate: table.get("elevate")?,
            elevation_method: table
                .get::<Option<String>>("elevation_method")?
                .map(|s| s.parse().map_err(Error::external))
                .transpose()?,
            as_user: table.get("as_user")?,
            env: table.get("env")?,
            connection: table
                .get::<Option<String>>("connection")?
                .map(|s| s.parse().map_err(Error::external))
                .transpose()?,
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
            table.set(
                "private_key_pass",
                private_key_pass.expose_secret().to_string(),
            )?;
        }
        if let Some(password) = self.password {
            table.set("password", password.expose_secret().to_string())?;
        }
        if let Some(elevate) = self.elevate {
            table.set("elevate", elevate)?;
        }
        if let Some(elevation_method) = self.elevation_method {
            table.set("elevation_method", elevation_method.to_string())?;
        }
        if let Some(as_user) = self.as_user {
            table.set("as_user", as_user)?;
        }
        if let Some(env) = self.env {
            table.set("env", env)?;
        }
        if let Some(connection) = self.connection {
            table.set("connection", connection.as_str())?;
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
            elevation_method: table
                .get::<Option<String>>("elevation_method")?
                .map(|s| s.parse().map_err(Error::external))
                .transpose()?,
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
            table.set("elevation_method", elevation_method.to_string())?;
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
    others: HashMap<String, serde_json::Value>,
}

impl FromLua for Module {
    fn from_lua(value: Value, lua: &Lua) -> mlua::Result<Self> {
        let table = value
            .as_table()
            .ok_or_else(|| Error::external("Value is not a table"))?;
        let mut functions: HashMap<String, Vec<u8>> = HashMap::new();
        let mut others: HashMap<String, serde_json::Value> = HashMap::new();
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
                others.insert(key.to_string()?, lua.from_value(value)?);
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
            table.set(key.as_str(), lua.to_value(value)?)?;
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

#[derive(Debug, Serialize, Deserialize)]
pub struct KomandanConfig {
    pub name: String,
    pub version: String,
    pub main: String,
    #[serde(default)]
    pub defaults: DefaultsConfig,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct DefaultsConfig {
    pub hosts: Option<String>,
    #[serde(flatten)]
    pub other: HashMap<String, String>,
}

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
        assert_eq!(
            host.private_key_pass
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            Some("pass".to_string())
        );
        assert_eq!(
            host.password
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            Some("password".to_string())
        );
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
            private_key_pass: Some(SecretString::new("pass".to_string().into_boxed_str())),
            password: Some(SecretString::new("password".to_string().into_boxed_str())),
            elevate: Some(true),
            elevation_method: Some(ElevationMethod::Sudo),
            as_user: Some("root".to_string()),
            env: Some(env.clone()),
            connection: None,
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
    fn test_host_debug_redacts_secrets() {
        let host = Host {
            name: None,
            address: "127.0.0.1".to_string(),
            port: None,
            user: None,
            key_check: None,
            private_key_file: None,
            private_key_pass: Some(SecretString::new(
                "super_secret_passphrase".to_string().into_boxed_str(),
            )),
            password: Some(SecretString::new(
                "super_secret_password".to_string().into_boxed_str(),
            )),
            elevate: None,
            elevation_method: None,
            as_user: None,
            env: None,
            connection: None,
        };
        let debug = format!("{host:?}");
        assert!(
            !debug.contains("super_secret_passphrase"),
            "private_key_pass leaked into Debug: {debug}"
        );
        assert!(
            !debug.contains("super_secret_password"),
            "password leaked into Debug: {debug}"
        );
        assert!(
            debug.contains("[REDACTED]"),
            "expected [REDACTED] marker in Debug: {debug}"
        );
    }

    #[test]
    fn test_host_invalid_connection_errors() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("address", "127.0.0.1")?;
        table.set("connection", "bogus")?;
        let result = Host::from_lua(Value::Table(table), &lua);
        let msg = match result {
            Err(e) => e.to_string(),
            Ok(_) => {
                return Err(Error::external(
                    "invalid connection value should error, but Host parsed successfully",
                ));
            }
        };
        assert!(
            msg.contains("invalid connection type"),
            "error should mention invalid connection type: {msg}"
        );
        Ok(())
    }

    #[test]
    fn test_host_invalid_elevation_method_errors() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("address", "127.0.0.1")?;
        table.set("elevation_method", "definitely-not-real")?;
        let result = Host::from_lua(Value::Table(table), &lua);
        assert!(
            result.is_err(),
            "invalid elevation_method value should error, but Host parsed successfully"
        );
        Ok(())
    }

    #[test]
    fn test_host_non_string_elevation_method_errors() -> mlua::Result<()> {
        // Non-string elevation_method (e.g. a number) must surface as a type
        // error rather than silently becoming None.
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("address", "127.0.0.1")?;
        table.set("elevation_method", 42)?;
        let result = Host::from_lua(Value::Table(table), &lua);
        assert!(
            result.is_err(),
            "non-string elevation_method should error, but Host parsed successfully"
        );
        Ok(())
    }

    #[test]
    fn test_host_non_string_connection_errors() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("address", "127.0.0.1")?;
        table.set("connection", 99)?;
        let result = Host::from_lua(Value::Table(table), &lua);
        assert!(
            result.is_err(),
            "non-string connection value should error, but Host parsed successfully"
        );
        Ok(())
    }

    #[test]
    fn test_task_invalid_elevation_method_errors() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        let module_table = lua.create_table()?;
        module_table.set("command", "echo hi")?;
        table.set(1, module_table)?;
        table.set("elevation_method", "nope")?;
        let result = Task::from_lua(Value::Table(table), &lua);
        assert!(
            result.is_err(),
            "invalid task elevation_method should error, but Task parsed successfully"
        );
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
            Some(&serde_json::json!("echo 'hello'"))
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
            Some(&serde_json::json!("echo 'hello'"))
        );

        let function = lua.load("return 1").into_function()?;
        table.set("test_func", function)?;

        let module = Module::from_lua(Value::Table(table), &lua)?;
        assert_eq!(module.functions.len(), 1);
        assert_eq!(
            module.others.get("command"),
            Some(&serde_json::json!("echo 'hello'"))
        );
        Ok(())
    }

    #[test]
    fn test_module_round_trip_nested_mixed() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("string_val", "hello")?;
        table.set("number_val", 42)?;
        table.set("bool_val", true)?;
        let nested = lua.create_table()?;
        nested.set("inner", "world")?;
        nested.set("count", 7)?;
        table.set("nested", nested)?;

        // FromLua captures the table into a Module with no JSON-string intermediate.
        let module = Module::from_lua(Value::Table(table), &lua)?;
        assert!(module.functions.is_empty());
        assert_eq!(
            module.others.get("string_val"),
            Some(&serde_json::json!("hello"))
        );
        assert_eq!(
            module.others.get("bool_val"),
            Some(&serde_json::json!(true))
        );

        // Round-trip Module -> IntoLua -> Lua table; nested + mixed values preserved.
        let round_tripped = module
            .into_lua(&lua)?
            .as_table()
            .ok_or_else(|| Error::external("round-tripped value is not a table"))?
            .clone();
        assert_eq!(round_tripped.get::<String>("string_val")?, "hello");
        assert_eq!(round_tripped.get::<i64>("number_val")?, 42);
        assert!(round_tripped.get::<bool>("bool_val")?);
        let inner: mlua::Table = round_tripped.get("nested")?;
        assert_eq!(inner.get::<String>("inner")?, "world");
        assert_eq!(inner.get::<i64>("count")?, 7);
        Ok(())
    }
}
