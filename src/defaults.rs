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
        env.write()
            .map_err(|_| Error::msg("Failed to acquire write lock"))?
            .insert("DEBIAN_FRONTEND".to_string(), "noninteractive".to_string());

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

fn handle_lock_error<T>(lock_name: &str, is_write: bool) -> mlua::Result<T> {
    let action = if is_write { "write" } else { "read" };
    Err(mlua::Error::RuntimeError(format!(
        "Failed to acquire {} lock on {}",
        action, lock_name
    )))
}

impl UserData for Defaults {
    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("get_port", |_, this, ()| -> mlua::Result<u16> {
            this.port.read().map(|port| *port).map_err(|_| {
                mlua::Error::RuntimeError("Failed to acquire read lock on port".to_string())
            })
        });

        methods.add_method_mut("set_port", |_, this, new_port: u16| -> mlua::Result<()> {
            this.port
                .write()
                .map(|mut port| *port = new_port)
                .map_err(|_| {
                    mlua::Error::RuntimeError("Failed to acquire write lock on port".to_string())
                })
        });

        methods.add_method("get_user", |_, this, ()| {
            this.user
                .read()
                .map(|user| user.clone())
                .map_err(|_| handle_lock_error::<Option<String>>("user", false).unwrap_err())
        });

        methods.add_method_mut("set_user", |_, this, new_user: Option<String>| {
            this.user
                .write()
                .map(|mut user| *user = new_user)
                .map_err(|_| handle_lock_error::<()>("user", true).unwrap_err())
        });

        methods.add_method("get_private_key_file", |_, this, ()| {
            this.private_key_file
                .read()
                .map(|private_key_file| private_key_file.clone())
                .map_err(|_| {
                    handle_lock_error::<Option<String>>("private_key_file", false).unwrap_err()
                })
        });

        methods.add_method_mut(
            "set_private_key_file",
            |_, this, new_private_key_file: Option<String>| {
                this.private_key_file
                    .write()
                    .map(|mut private_key_file| *private_key_file = new_private_key_file)
                    .map_err(|_| handle_lock_error::<()>("private_key_file", true).unwrap_err())
            },
        );

        methods.add_method("get_private_key_pass", |_, this, ()| {
            this.private_key_pass
                .read()
                .map(|private_key_pass| private_key_pass.clone())
                .map_err(|_| {
                    handle_lock_error::<Option<String>>("private_key_pass", false).unwrap_err()
                })
        });

        methods.add_method_mut(
            "set_private_key_pass",
            |_, this, new_private_key_pass: Option<String>| {
                this.private_key_pass
                    .write()
                    .map(|mut private_key_pass| *private_key_pass = new_private_key_pass)
                    .map_err(|_| handle_lock_error::<()>("private_key_pass", true).unwrap_err())
            },
        );

        methods.add_method("get_password", |_, this, ()| {
            this.password
                .read()
                .map(|password| password.clone())
                .map_err(|_| handle_lock_error::<Option<String>>("password", false).unwrap_err())
        });

        methods.add_method_mut("set_password", |_, this, new_password: Option<String>| {
            this.password
                .write()
                .map(|mut password| *password = new_password)
                .map_err(|_| handle_lock_error::<()>("password", true).unwrap_err())
        });

        methods.add_method("get_ignore_exit_code", |_, this, ()| {
            this.ignore_exit_code
                .read()
                .map(|ignore_exit_code| *ignore_exit_code)
                .map_err(|_| handle_lock_error::<bool>("ignore_exit_code", false).unwrap_err())
        });

        methods.add_method_mut(
            "set_ignore_exit_code",
            |_, this, new_ignore_exit_code: bool| {
                this.ignore_exit_code
                    .write()
                    .map(|mut ignore_exit_code| *ignore_exit_code = new_ignore_exit_code)
                    .map_err(|_| handle_lock_error::<()>("ignore_exit_code", true).unwrap_err())
            },
        );

        methods.add_method("get_elevate", |_, this, ()| {
            this.elevate
                .read()
                .map(|elevate| *elevate)
                .map_err(|_| handle_lock_error::<bool>("elevate", false).unwrap_err())
        });

        methods.add_method_mut("set_elevate", |_, this, new_elevate: bool| {
            this.elevate
                .write()
                .map(|mut elevate| *elevate = new_elevate)
                .map_err(|_| handle_lock_error::<()>("elevate", true).unwrap_err())
        });

        methods.add_method("get_elevation_method", |_, this, ()| {
            this.elevation_method
                .read()
                .map(|elevation_method| elevation_method.clone())
                .map_err(|_| handle_lock_error::<String>("elevation_method", false).unwrap_err())
        });

        methods.add_method_mut(
            "set_elevation_method",
            |_, this, new_elevation_method: String| {
                this.elevation_method
                    .write()
                    .map(|mut elevation_method| *elevation_method = new_elevation_method)
                    .map_err(|_| handle_lock_error::<()>("elevation_method", true).unwrap_err())
            },
        );

        methods.add_method("get_as_user", |_, this, ()| {
            this.as_user
                .read()
                .map(|as_user| as_user.clone())
                .map_err(|_| handle_lock_error::<Option<String>>("as_user", false).unwrap_err())
        });

        methods.add_method_mut("set_as_user", |_, this, new_as_user: Option<String>| {
            this.as_user
                .write()
                .map(|mut as_user| *as_user = new_as_user)
                .map_err(|_| handle_lock_error::<()>("as_user", true).unwrap_err())
        });

        methods.add_method("get_known_hosts_file", |_, this, ()| {
            this.known_hosts_file
                .read()
                .map(|known_hosts_file| known_hosts_file.clone())
                .map_err(|_| handle_lock_error::<String>("known_hosts_file", false).unwrap_err())
        });

        methods.add_method_mut(
            "set_known_hosts_file",
            |_, this, new_known_hosts_file: String| {
                this.known_hosts_file
                    .write()
                    .map(|mut known_hosts_file| *known_hosts_file = new_known_hosts_file)
                    .map_err(|_| handle_lock_error::<()>("known_hosts_file", true).unwrap_err())
            },
        );

        methods.add_method("get_host_key_check", |_, this, ()| {
            this.key_check
                .read()
                .map(|key_check| *key_check)
                .map_err(|_| handle_lock_error::<bool>("key_check", false).unwrap_err())
        });

        methods.add_method_mut("set_host_key_check", |_, this, new_key_check: bool| {
            this.key_check
                .write()
                .map(|mut key_check| *key_check = new_key_check)
                .map_err(|_| handle_lock_error::<()>("key_check", true).unwrap_err())
        });

        methods.add_method("get_all_env", |lua, this, ()| {
            this.env
                .read()
                .map_err(|_| handle_lock_error::<()>("env", false).unwrap_err())
                .and_then(|map| lua.create_table_from(map.keys().cloned().enumerate()))
        });

        methods.add_method_mut("get_env", |_, this, key: String| {
            this.env
                .read()
                .map(|map| {
                    map.get(&key)
                        .map_or_else(|| String::new(), |value| value.clone())
                })
                .map_err(|_| handle_lock_error::<String>("env", false).unwrap_err())
        });

        methods.add_method_mut("set_env", |_, this, (key, value): (String, String)| {
            this.env
                .write()
                .map(|mut map| {
                    map.insert(key, value);
                })
                .map_err(|_| handle_lock_error::<()>("env", true).unwrap_err())
        });

        methods.add_method_mut("remove_env", |_, this, key: String| {
            this.env
                .write()
                .map(|mut map| {
                    map.remove(&key);
                })
                .map_err(|_| handle_lock_error::<()>("env", true).unwrap_err())
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
