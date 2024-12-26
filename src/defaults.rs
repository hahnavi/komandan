use mlua::UserData;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
};

static GLOBAL_STATE: OnceLock<Defaults> = OnceLock::new();

#[derive(Clone)]
pub struct Defaults {
    pub port: Arc<RwLock<u16>>,
    pub user: Arc<RwLock<Option<String>>>,
    pub private_key_file: Arc<RwLock<Option<String>>>,
    pub private_key_pass: Arc<RwLock<Option<String>>>,
    pub password: Arc<RwLock<Option<String>>>,
    pub ignore_exit_code: Arc<RwLock<bool>>,
    pub elevate: Arc<RwLock<bool>>,
    pub elevation_method: Arc<RwLock<String>>,
    pub as_user: Arc<RwLock<Option<String>>>,
    pub known_hosts_file: Arc<RwLock<String>>,
    pub host_key_check: Arc<RwLock<bool>>,
    pub env: Arc<RwLock<HashMap<String, String>>>,
}

impl Defaults {
    pub fn new() -> Self {
        let env = Arc::new(RwLock::new(HashMap::new()));
        match env.write() {
            Ok(mut env) => {
                env.insert("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string());
            }
            Err(_) => {}
        }

        Self {
            port: Arc::new(RwLock::new(22)),
            user: Arc::new(RwLock::new(None)),
            private_key_file: Arc::new(RwLock::new(None)),
            private_key_pass: Arc::new(RwLock::new(None)),
            password: Arc::new(RwLock::new(None)),
            ignore_exit_code: Arc::new(RwLock::new(false)),
            elevate: Arc::new(RwLock::new(false)),
            elevation_method: Arc::new(RwLock::new("sudo".to_string())),
            as_user: Arc::new(RwLock::new(None)),
            known_hosts_file: Arc::new(RwLock::new(format!("{}/.ssh/known_hosts", std::env::var("HOME").unwrap()))),
            host_key_check: Arc::new(RwLock::new(true)),
            env,
        }
    }

    pub fn global() -> Self {
        GLOBAL_STATE.get_or_init(|| Defaults::new()).clone()
    }
}

impl UserData for Defaults {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get_port", |_, this, ()| -> mlua::Result<u16> {
            match this.port.read() {
                Ok(port) => Ok(*port),
                Err(_) => {
                    return Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                }
            }
        });

        methods.add_method_mut("set_port", |_, this, new_port: u16| -> mlua::Result<()> {
            match this.port.write() {
                Ok(mut port) => {
                    *port = new_port;
                    Ok(())
                }
                Err(_) => {
                    return Err(mlua::Error::RuntimeError(
                        "Failed to acquire write lock".to_string(),
                    ))
                }
            }
        });

        methods.add_method("get_user", |_, this, ()| -> mlua::Result<Option<String>> {
            match this.user.read() {
                Ok(user) => Ok(user.clone()),
                Err(_) => {
                    return Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                }
            }
        });

        methods.add_method_mut(
            "set_user",
            |_, this, new_user: Option<String>| -> mlua::Result<()> {
                match this.user.write() {
                    Ok(mut user) => {
                        *user = new_user;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method(
            "get_private_key_file",
            |_, this, ()| -> mlua::Result<Option<String>> {
                match this.private_key_file.read() {
                    Ok(private_key_file) => Ok(private_key_file.clone()),
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method_mut(
            "set_private_key_file",
            |_, this, new_private_key_file: Option<String>| -> mlua::Result<()> {
                match this.private_key_file.write() {
                    Ok(mut private_key_file) => {
                        *private_key_file = new_private_key_file;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method(
            "get_private_key_pass",
            |_, this, ()| -> mlua::Result<Option<String>> {
                match this.private_key_pass.read() {
                    Ok(private_key_pass) => Ok(private_key_pass.clone()),
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method_mut(
            "set_private_key_pass",
            |_, this, new_private_key_pass: Option<String>| -> mlua::Result<()> {
                match this.private_key_pass.write() {
                    Ok(mut private_key_pass) => {
                        *private_key_pass = new_private_key_pass;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method(
            "get_password",
            |_, this, ()| -> mlua::Result<Option<String>> {
                match this.password.read() {
                    Ok(password) => Ok(password.clone()),
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method_mut(
            "set_password",
            |_, this, new_password: Option<String>| -> mlua::Result<()> {
                match this.password.write() {
                    Ok(mut password) => {
                        *password = new_password;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method(
            "get_ignore_exit_code",
            |_, this, ()| -> mlua::Result<bool> {
                match this.ignore_exit_code.read() {
                    Ok(ignore_exit_code) => Ok(*ignore_exit_code),
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method_mut(
            "set_ignore_exit_code",
            |_, this, new_ignore_exit_code: bool| -> mlua::Result<()> {
                match this.ignore_exit_code.write() {
                    Ok(mut ignore_exit_code) => {
                        *ignore_exit_code = new_ignore_exit_code;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method("get_elevate", |_, this, ()| -> mlua::Result<bool> {
            match this.elevate.read() {
                Ok(elevate) => Ok(*elevate),
                Err(_) => {
                    return Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                }
            }
        });

        methods.add_method_mut(
            "set_elevate",
            |_, this, new_elevate: bool| -> mlua::Result<()> {
                match this.elevate.write() {
                    Ok(mut elevate) => {
                        *elevate = new_elevate;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method(
            "get_elevation_method",
            |_, this, ()| -> mlua::Result<String> {
                match this.elevation_method.read() {
                    Ok(elevation_method) => Ok(elevation_method.clone()),
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method_mut(
            "set_elevation_method",
            |_, this, new_elevation_method: String| -> mlua::Result<()> {
                match this.elevation_method.write() {
                    Ok(mut elevation_method) => {
                        *elevation_method = new_elevation_method;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method(
            "get_as_user",
            |_, this, ()| -> mlua::Result<Option<String>> {
                match this.as_user.read() {
                    Ok(as_user) => Ok(as_user.clone()),
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method_mut(
            "set_as_user",
            |_, this, new_as_user: Option<String>| -> mlua::Result<()> {
                match this.as_user.write() {
                    Ok(mut as_user) => {
                        *as_user = new_as_user;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method(
            "get_known_hosts_file",
            |_, this, ()| -> mlua::Result<String> {
                match this.known_hosts_file.read() {
                    Ok(known_hosts_file) => Ok(known_hosts_file.clone()),
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method_mut(
            "set_known_hosts_file",
            |_, this, new_known_hosts_file: String| -> mlua::Result<()> {
                match this.known_hosts_file.write() {
                    Ok(mut known_hosts_file) => {
                        *known_hosts_file = new_known_hosts_file;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method("get_host_key_check", |_, this, ()| -> mlua::Result<bool> {
            match this.host_key_check.read() {
                Ok(host_key_check) => Ok(*host_key_check),
                Err(_) => {
                    return Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                }
            }
        });

        methods.add_method_mut(
            "set_host_key_check",
            |_, this, new_host_key_check: bool| -> mlua::Result<()> {
                match this.host_key_check.write() {
                    Ok(mut host_key_check) => {
                        *host_key_check = new_host_key_check;
                        Ok(())
                    }
                    Err(_) => {
                        return Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    }
                }
            },
        );

        methods.add_method("get_all_env", |lua, this, ()| match this.env.read() {
            Ok(map) => {
                let keys: Vec<String> = map.keys().cloned().collect();
                lua.create_table_from(keys.into_iter().enumerate())
            }
            Err(_) => Err(mlua::Error::runtime("Failed to acquire lock")),
        });

        methods.add_method_mut("get_env", |_, this, key: String| -> mlua::Result<String> {
            match this.env.read() {
                Ok(map) => match map.get(&key) {
                    Some(value) => Ok(value.clone()),
                    None => Ok(String::new()),
                },
                Err(_) => Err(mlua::Error::runtime("Failed to acquire lock")),
            }
        });

        methods.add_method_mut(
            "set_env",
            |_, this, (key, value): (String, String)| -> mlua::Result<()> {
                match this.env.write() {
                    Ok(mut map) => {
                        map.insert(key, value);
                        Ok(())
                    }
                    Err(_) => Err(mlua::Error::runtime("Failed to acquire lock")),
                }
            },
        );

        methods.add_method_mut("remove_env", |_, this, key: String| -> mlua::Result<()> {
            match this.env.write() {
                Ok(mut map) => {
                    map.remove(&key);
                    Ok(())
                }
                Err(_) => Err(mlua::Error::runtime("Failed to acquire lock")),
            }
        });
    }
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_new() {
        let defaults = Defaults::new();

        // Test default values
        assert_eq!(*defaults.port.read().unwrap(), 22);
        assert_eq!(*defaults.user.read().unwrap(), None);
        assert_eq!(*defaults.private_key_file.read().unwrap(), None);
        assert_eq!(*defaults.private_key_pass.read().unwrap(), None);
        assert_eq!(*defaults.password.read().unwrap(), None);
        assert_eq!(*defaults.ignore_exit_code.read().unwrap(), false);
        assert_eq!(*defaults.elevate.read().unwrap(), false);
        assert_eq!(*defaults.elevation_method.read().unwrap(), "sudo");
        assert_eq!(*defaults.as_user.read().unwrap(), None);
        assert_eq!(*defaults.host_key_check.read().unwrap(), true);

        // Test default environment variables
        let env = defaults.env.read().unwrap();
        assert_eq!(
            env.get("DEBIAN_FRONTEND"),
            Some(&"noninteractive".to_string())
        );
    }

    #[test]
    fn test_global_singleton() {
        let defaults1 = Defaults::global();
        let defaults2 = Defaults::global();

        // Modify a value using the first instance
        *defaults1.port.write().unwrap() = 2222;

        // Check if the change is reflected in the second instance
        assert_eq!(*defaults2.port.read().unwrap(), 2222);
    }

    #[test]
    fn test_lua_interface() -> mlua::Result<()> {
        let lua = mlua::Lua::new();
        let defaults = Defaults::new();

        // Register the defaults instance with Lua
        lua.globals().set("defaults", defaults.clone())?;

        // Test port
        lua.load("assert(defaults:get_port() == 22)").exec()?;
        lua.load("defaults:set_port(2222)").exec()?;
        lua.load("assert(defaults:get_port() == 2222)").exec()?;

        // Test user
        lua.load("assert(defaults:get_user() == nil)").exec()?;
        lua.load("defaults:set_user('testuser')").exec()?;
        lua.load("assert(defaults:get_user() == 'testuser')")
            .exec()?;
        lua.load("defaults:set_user(nil)").exec()?;
        lua.load("assert(defaults:get_user() == nil)").exec()?;

        // Test private key file
        lua.load("assert(defaults:get_private_key_file() == nil)")
            .exec()?;
        lua.load("defaults:set_private_key_file('/path/to/key')")
            .exec()?;
        lua.load("assert(defaults:get_private_key_file() == '/path/to/key')")
            .exec()?;

        // Test private key password
        lua.load("assert(defaults:get_private_key_pass() == nil)")
            .exec()?;
        lua.load("defaults:set_private_key_pass('password123')")
            .exec()?;
        lua.load("assert(defaults:get_private_key_pass() == 'password123')")
            .exec()?;

        // Test password
        lua.load("assert(defaults:get_password() == nil)").exec()?;
        lua.load("defaults:set_password('secret123')").exec()?;
        lua.load("assert(defaults:get_password() == 'secret123')")
            .exec()?;

        // Test ignore exit code
        lua.load("assert(defaults:get_ignore_exit_code() == false)")
            .exec()?;
        lua.load("defaults:set_ignore_exit_code(true)").exec()?;
        lua.load("assert(defaults:get_ignore_exit_code() == true)")
            .exec()?;

        // Test elevate
        lua.load("assert(defaults:get_elevate() == false)").exec()?;
        lua.load("defaults:set_elevate(true)").exec()?;
        lua.load("assert(defaults:get_elevate() == true)").exec()?;

        // Test elevation method
        lua.load("assert(defaults:get_elevation_method() == 'sudo')")
            .exec()?;
        lua.load("defaults:set_elevation_method('doas')").exec()?;
        lua.load("assert(defaults:get_elevation_method() == 'doas')")
            .exec()?;

        // Test as user
        lua.load("assert(defaults:get_as_user() == nil)").exec()?;
        lua.load("defaults:set_as_user('root')").exec()?;
        lua.load("assert(defaults:get_as_user() == 'root')")
            .exec()?;

        // Test host key check
        lua.load("assert(defaults:get_host_key_check() == true)")
            .exec()?;
        lua.load("defaults:set_host_key_check(false)").exec()?;
        lua.load("assert(defaults:get_host_key_check() == false)")
            .exec()?;

        // Test known hosts file
        lua.load(
            r#"
            local known_hosts = defaults:get_known_hosts_file()
            assert(known_hosts:match('/.ssh/known_hosts$') ~= nil)
            defaults:set_known_hosts_file('/custom/known_hosts')
            assert(defaults:get_known_hosts_file() == '/custom/known_hosts')
        "#,
        )
        .exec()?;

        // Test environment variables
        lua.load("assert(defaults:get_env('TEST_ENV') == '')")
            .exec()?;
        lua.load("defaults:set_env('TEST_ENV', 'test_value')")
            .exec()?;
        lua.load("assert(defaults:get_env('TEST_ENV') == 'test_value')")
            .exec()?;
        lua.load("defaults:remove_env('TEST_ENV')").exec()?;
        lua.load("assert(defaults:get_env('TEST_ENV') == '')")
            .exec()?;

        Ok(())
    }
}
