use anyhow::{Error, Result};
use mlua::UserData;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock},
};

static GLOBAL_DEFAULTS: OnceLock<Defaults> = OnceLock::new();

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
    pub key_check: Arc<RwLock<bool>>,
    pub env: Arc<RwLock<HashMap<String, String>>>,
}

impl Defaults {
    pub fn new() -> Result<Self> {
        let env = Arc::new(RwLock::new(HashMap::new()));
        match env.write() {
            Ok(mut env) => {
                env.insert("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string());
            }
            Err(_) => return Err(Error::msg("Failed to acquire write lock".to_string())),
        }

        let port = std::env::var("KOMANDAN_SSH_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(22);

        let user = std::env::var("KOMANDAN_SSH_USER").ok();

        let private_key_file = std::env::var("KOMANDAN_SSH_PRIVATE_KEY_FILE").ok();

        let private_key_pass = std::env::var("KOMANDAN_SSH_PRIVATE_KEY_PASS").ok();

        let password = std::env::var("KOMANDAN_SSH_PASSWORD").ok();

        let known_hosts_file =
            std::env::var("KOMANDAN_SSH_KNOWN_HOSTS_FILE").unwrap_or_else(|_| {
                format!(
                    "{}/.ssh/known_hosts",
                    std::env::var("HOME").unwrap_or_else(|_| "~".to_string())
                )
            });

        let key_check = std::env::var("KOMANDAN_SSH_HOST_KEY_CHECK")
            .ok()
            .and_then(|v| v.parse::<bool>().ok())
            .unwrap_or(true);

        Ok(Self {
            port: Arc::new(RwLock::new(port)),
            user: Arc::new(RwLock::new(user)),
            private_key_file: Arc::new(RwLock::new(private_key_file)),
            private_key_pass: Arc::new(RwLock::new(private_key_pass)),
            password: Arc::new(RwLock::new(password)),
            ignore_exit_code: Arc::new(RwLock::new(false)),
            elevate: Arc::new(RwLock::new(false)),
            elevation_method: Arc::new(RwLock::new("sudo".to_string())),
            as_user: Arc::new(RwLock::new(None)),
            known_hosts_file: Arc::new(RwLock::new(known_hosts_file)),
            key_check: Arc::new(RwLock::new(key_check)),
            env,
        })
    }

    pub fn global() -> Self {
        GLOBAL_DEFAULTS
            .get_or_init(|| {
                Self::new().unwrap_or_else(|e| panic!("Failed to create new defaults: {e}"))
            })
            .clone()
    }
}

impl UserData for Defaults {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get_port", |_, this, ()| -> mlua::Result<u16> {
            this.port.read().map_or_else(
                |_| {
                    Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                },
                |port| Ok(*port),
            )
        });

        methods.add_method_mut("set_port", |_, this, new_port: u16| -> mlua::Result<()> {
            this.port.write().map_or_else(
                |_| {
                    Err(mlua::Error::RuntimeError(
                        "Failed to acquire write lock".to_string(),
                    ))
                },
                |mut port| {
                    *port = new_port;
                    Ok(())
                },
            )
        });

        methods.add_method("get_user", |_, this, ()| -> mlua::Result<Option<String>> {
            this.user.read().map_or_else(
                |_| {
                    Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                },
                |user| Ok(user.clone()),
            )
        });

        methods.add_method_mut(
            "set_user",
            |_, this, new_user: Option<String>| -> mlua::Result<()> {
                this.user.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut user| {
                        *user = new_user;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method(
            "get_private_key_file",
            |_, this, ()| -> mlua::Result<Option<String>> {
                this.private_key_file.read().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    },
                    |private_key_file| Ok(private_key_file.clone()),
                )
            },
        );

        methods.add_method_mut(
            "set_private_key_file",
            |_, this, new_private_key_file: Option<String>| -> mlua::Result<()> {
                this.private_key_file.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut private_key_file| {
                        *private_key_file = new_private_key_file;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method(
            "get_private_key_pass",
            |_, this, ()| -> mlua::Result<Option<String>> {
                this.private_key_pass.read().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    },
                    |private_key_pass| Ok(private_key_pass.clone()),
                )
            },
        );

        methods.add_method_mut(
            "set_private_key_pass",
            |_, this, new_private_key_pass: Option<String>| -> mlua::Result<()> {
                this.private_key_pass.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut private_key_pass| {
                        *private_key_pass = new_private_key_pass;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method(
            "get_password",
            |_, this, ()| -> mlua::Result<Option<String>> {
                this.password.read().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    },
                    |password| Ok(password.clone()),
                )
            },
        );

        methods.add_method_mut(
            "set_password",
            |_, this, new_password: Option<String>| -> mlua::Result<()> {
                this.password.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut password| {
                        *password = new_password;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method(
            "get_ignore_exit_code",
            |_, this, ()| -> mlua::Result<bool> {
                this.ignore_exit_code.read().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    },
                    |ignore_exit_code| Ok(*ignore_exit_code),
                )
            },
        );

        methods.add_method_mut(
            "set_ignore_exit_code",
            |_, this, new_ignore_exit_code: bool| -> mlua::Result<()> {
                this.ignore_exit_code.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut ignore_exit_code| {
                        *ignore_exit_code = new_ignore_exit_code;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method("get_elevate", |_, this, ()| -> mlua::Result<bool> {
            this.elevate.read().map_or_else(
                |_| {
                    Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                },
                |elevate| Ok(*elevate),
            )
        });

        methods.add_method_mut(
            "set_elevate",
            |_, this, new_elevate: bool| -> mlua::Result<()> {
                this.elevate.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut elevate| {
                        *elevate = new_elevate;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method(
            "get_elevation_method",
            |_, this, ()| -> mlua::Result<String> {
                this.elevation_method.read().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    },
                    |elevation_method| Ok(elevation_method.clone()),
                )
            },
        );

        methods.add_method_mut(
            "set_elevation_method",
            |_, this, new_elevation_method: String| -> mlua::Result<()> {
                this.elevation_method.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut elevation_method| {
                        *elevation_method = new_elevation_method;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method(
            "get_as_user",
            |_, this, ()| -> mlua::Result<Option<String>> {
                this.as_user.read().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    },
                    |as_user| Ok(as_user.clone()),
                )
            },
        );

        methods.add_method_mut(
            "set_as_user",
            |_, this, new_as_user: Option<String>| -> mlua::Result<()> {
                this.as_user.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut as_user| {
                        *as_user = new_as_user;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method(
            "get_known_hosts_file",
            |_, this, ()| -> mlua::Result<String> {
                this.known_hosts_file.read().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire read lock".to_string(),
                        ))
                    },
                    |known_hosts_file| Ok(known_hosts_file.clone()),
                )
            },
        );

        methods.add_method_mut(
            "set_known_hosts_file",
            |_, this, new_known_hosts_file: String| -> mlua::Result<()> {
                this.known_hosts_file.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut known_hosts_file| {
                        *known_hosts_file = new_known_hosts_file;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method("get_host_key_check", |_, this, ()| -> mlua::Result<bool> {
            this.key_check.read().map_or_else(
                |_| {
                    Err(mlua::Error::RuntimeError(
                        "Failed to acquire read lock".to_string(),
                    ))
                },
                |key_check| Ok(*key_check),
            )
        });

        methods.add_method_mut(
            "set_host_key_check",
            |_, this, new_key_check: bool| -> mlua::Result<()> {
                this.key_check.write().map_or_else(
                    |_| {
                        Err(mlua::Error::RuntimeError(
                            "Failed to acquire write lock".to_string(),
                        ))
                    },
                    |mut key_check| {
                        *key_check = new_key_check;
                        Ok(())
                    },
                )
            },
        );

        methods.add_method("get_all_env", |lua, this, ()| {
            this.env.read().map_or_else(
                |_| Err(mlua::Error::runtime("Failed to acquire lock")),
                |map| lua.create_table_from(map.keys().cloned().enumerate()),
            )
        });

        methods.add_method_mut("get_env", |_, this, key: String| -> mlua::Result<String> {
            this.env.read().map_or_else(
                |_| Err(mlua::Error::runtime("Failed to acquire lock")),
                |map| {
                    map.get(&key)
                        .map_or_else(|| Ok(String::new()), |value| Ok(value.clone()))
                },
            )
        });

        methods.add_method_mut(
            "set_env",
            |_, this, (key, value): (String, String)| -> mlua::Result<()> {
                this.env.write().map_or_else(
                    |_| Err(mlua::Error::runtime("Failed to acquire lock")),
                    |mut map| {
                        map.insert(key, value);
                        Ok(())
                    },
                )
            },
        );

        methods.add_method_mut("remove_env", |_, this, key: String| -> mlua::Result<()> {
            this.env.write().map_or_else(
                |_| Err(mlua::Error::runtime("Failed to acquire lock")),
                |mut map| {
                    map.remove(&key);
                    Ok(())
                },
            )
        });
    }
}

// Tests
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_defaults_new() -> Result<()> {
        let defaults = Defaults::new()?;

        // Test default values
        assert_eq!(
            *defaults.port.read().map_err(|_| Error::msg("lock error"))?,
            22
        );
        assert_eq!(
            *defaults.user.read().map_err(|_| Error::msg("lock error"))?,
            None
        );
        assert_eq!(
            *defaults
                .private_key_file
                .read()
                .map_err(|_| Error::msg("lock error"))?,
            None
        );
        assert_eq!(
            *defaults
                .private_key_pass
                .read()
                .map_err(|_| Error::msg("lock error"))?,
            None
        );
        assert_eq!(
            *defaults
                .password
                .read()
                .map_err(|_| Error::msg("lock error"))?,
            None
        );
        assert!(
            !(*defaults
                .ignore_exit_code
                .read()
                .map_err(|_| Error::msg("lock error"))?)
        );
        assert!(
            !(*defaults
                .elevate
                .read()
                .map_err(|_| Error::msg("lock error"))?)
        );
        assert_eq!(
            *defaults
                .elevation_method
                .read()
                .map_err(|_| Error::msg("lock error"))?,
            "sudo"
        );
        assert_eq!(
            *defaults
                .as_user
                .read()
                .map_err(|_| Error::msg("lock error"))?,
            None
        );
        assert!(
            *defaults
                .key_check
                .read()
                .map_err(|_| Error::msg("lock error"))?
        );

        // Test default environment variables
        let env = defaults
            .env
            .read()
            .map_err(|_| Error::msg("lock error"))?
            .clone();
        assert_eq!(
            env.get("DEBIAN_FRONTEND"),
            Some(&"noninteractive".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_global_singleton() -> Result<()> {
        let defaults1 = Defaults::global();
        let defaults2 = Defaults::global();

        // Modify a value using the first instance
        *defaults1
            .port
            .write()
            .map_err(|_| Error::msg("lock error"))? = 2222;

        // Check if the change is reflected in the second instance
        assert_eq!(
            *defaults2
                .port
                .read()
                .map_err(|_| Error::msg("lock error"))?,
            2222
        );
        Ok(())
    }

    #[test]
    fn test_lua_interface() -> Result<()> {
        let lua = mlua::Lua::new();
        let defaults = Defaults::new()?;

        // Register the defaults instance with Lua
        lua.globals().set("defaults", defaults)?;

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
            r"
            local known_hosts = defaults:get_known_hosts_file()
            assert(known_hosts:match('/.ssh/known_hosts$') ~= nil)
            defaults:set_known_hosts_file('/custom/known_hosts')
            assert(defaults:get_known_hosts_file() == '/custom/known_hosts')
        ",
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
