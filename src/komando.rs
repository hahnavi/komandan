use std::collections::HashMap;
use std::env;

use clap::Parser;
use mlua::{Error::RuntimeError, FromLua, Integer, Lua, Table, Value};
use mlua::{IntoLua, LuaSerdeExt, chunk};
use rayon::prelude::*;

use crate::args::Args;
use crate::create_lua;
use crate::defaults::Defaults;
use crate::models::{Host, KomandoResult, Task};
use crate::report::{TaskStatus, insert_record};
use crate::ssh::{Elevation, ElevationMethod, SSHAuthMethod, SSHSession};
use crate::util::{host_display, task_display};
use crate::validator::{validate_host, validate_task};

pub fn komando(lua: &Lua, (host, task): (Value, Value)) -> mlua::Result<Table> {
    let host = lua.create_function(validate_host)?.call::<Table>(&host)?;
    let task = lua.create_function(validate_task)?.call::<Table>(&task)?;
    let module = task.get::<Table>(1)?;

    let host_display = host_display(&host);
    let task_display = task_display(&task);

    let defaults = Defaults::global();

    let (user, ssh_auth_method) = get_auth_config(&host, &task)?;
    let elevation = get_elevation_config(&host, &task)?;

    let default_port = match defaults.port.read() {
        Ok(port) => *port,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let port = host
        .get::<Integer>("port")
        .unwrap_or_else(|_| i64::from(default_port));
    let port = u16::try_from(port).unwrap_or(default_port);

    let mut ssh = create_ssh_session(&host)?;
    ssh.elevation = elevation;

    ssh.connect(
        host.get::<String>("address")?.as_str(),
        port,
        &user,
        ssh_auth_method,
    )?;

    setup_environment(&mut ssh, &host, &task)?;

    let result = execute_task(lua, &module, ssh, &task_display, &host_display)?;

    let default_ignore_exit_code = match defaults.ignore_exit_code.read() {
        Ok(ignore_exit_code) => *ignore_exit_code,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let ignore_exit_code = task
        .get::<bool>("ignore_exit_code")
        .unwrap_or(default_ignore_exit_code);

    let exit_code = result.get::<Integer>("exit_code")?;

    if exit_code != 0 && !ignore_exit_code {
        return Err(RuntimeError("Failed to run task.".to_string()));
    }

    let task_status = if exit_code != 0 {
        TaskStatus::Failed
    } else if result.get::<bool>("changed")? {
        TaskStatus::Changed
    } else {
        TaskStatus::OK
    };

    if !Args::parse().flags.no_report {
        insert_record(task_display, host_display, task_status);
    }

    Ok(result)
}

#[derive(Clone, Debug, Hash, Eq, PartialEq)]
enum ParallelHashMapKey {
    Number(u32),
    Text(String),
}

pub fn komando_parallel_tasks(lua: &Lua, (host, tasks): (Value, Value)) -> mlua::Result<Table> {
    let host = Host::from_lua(host, lua)?;
    let mut tasks_hm = HashMap::<ParallelHashMapKey, Task>::new();
    let tasks_table = tasks
        .as_table()
        .ok_or_else(|| RuntimeError("Tasks must be a table".to_string()))?;

    for pair in tasks_table.pairs::<Value, Value>() {
        let (key, value): (Value, Value) = pair?;
        let task = Task::from_lua(value, lua)?;
        if key.is_number() {
            let key_int = key
                .as_integer()
                .ok_or_else(|| RuntimeError("Key must be an integer".to_string()))?;
            let key_u32 = u32::try_from(key_int)
                .map_err(|_| RuntimeError("Key out of range for u32".to_string()))?;
            tasks_hm.insert(ParallelHashMapKey::Number(key_u32), task);
        } else {
            tasks_hm.insert(ParallelHashMapKey::Text(key.to_string()?), task);
        }
    }

    let results: HashMap<ParallelHashMapKey, KomandoResult> = tasks_hm
        .par_iter()
        .map(|(i, task)| {
            let lua = create_lua().ok()?;
            let host = host.clone().into_lua(&lua).ok()?;
            let task = task.clone().into_lua(&lua).ok()?;
            let result = komando(&lua, (host, task)).ok()?;

            Some((
                i.clone(),
                lua.from_value::<KomandoResult>(Value::Table(result)).ok()?,
            ))
        })
        .collect::<Option<HashMap<ParallelHashMapKey, KomandoResult>>>()
        .ok_or_else(|| RuntimeError("Failed to execute parallel tasks".to_string()))?;

    let results_table = lua.create_table()?;
    for (i, result) in &results {
        let key: Value = match i {
            ParallelHashMapKey::Number(i) => Value::Number(f64::from(*i)),
            ParallelHashMapKey::Text(i) => Value::String(lua.create_string(i)?),
        };
        results_table.set(key, lua.to_value(result)?)?;
    }

    Ok(results_table)
}

pub fn komando_parallel_hosts(lua: &Lua, (hosts, task): (Value, Value)) -> mlua::Result<Table> {
    let task = Task::from_lua(task, lua)?;
    let mut hosts_hm = HashMap::<ParallelHashMapKey, Host>::new();
    let hosts_table = hosts
        .as_table()
        .ok_or_else(|| RuntimeError("Hosts must be a table".to_string()))?;

    for pair in hosts_table.pairs::<Value, Value>() {
        let (key, value): (Value, Value) = pair?;
        let host = Host::from_lua(value, lua)?;
        if key.is_number() {
            let key_int = key
                .as_integer()
                .ok_or_else(|| RuntimeError("Key must be an integer".to_string()))?;
            let key_u32 = u32::try_from(key_int)
                .map_err(|_| RuntimeError("Key out of range for u32".to_string()))?;
            hosts_hm.insert(ParallelHashMapKey::Number(key_u32), host);
        } else {
            hosts_hm.insert(ParallelHashMapKey::Text(key.to_string()?), host);
        }
    }

    let results: HashMap<ParallelHashMapKey, KomandoResult> = hosts_hm
        .par_iter()
        .map(|(i, host)| {
            let lua = create_lua().ok()?;
            let host = host.clone().into_lua(&lua).ok()?;
            let task = task.clone().into_lua(&lua).ok()?;
            let result = komando(&lua, (host, task)).ok()?;

            Some((
                i.clone(),
                lua.from_value::<KomandoResult>(Value::Table(result)).ok()?,
            ))
        })
        .collect::<Option<HashMap<ParallelHashMapKey, KomandoResult>>>()
        .ok_or_else(|| RuntimeError("Failed to execute parallel hosts".to_string()))?;

    let results_table = lua.create_table()?;
    for (i, result) in &results {
        let key = match i {
            ParallelHashMapKey::Number(i) => Value::Number(f64::from(*i)),
            ParallelHashMapKey::Text(i) => Value::String(lua.create_string(i)?),
        };
        results_table.set(key, lua.to_value(result)?)?;
    }

    Ok(results_table)
}

fn get_user(host: &Table, task: &Table) -> mlua::Result<String> {
    let defaults = Defaults::global();
    let default_user = match defaults.user.read() {
        Ok(user) => user.clone(),
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };
    let user = match host.get::<String>("user") {
        Ok(user) => user,
        Err(_) => match default_user {
            Some(ref user) => user.clone(),
            None => match env::var("USER") {
                Ok(user) => user,
                Err(_) => {
                    return Err(RuntimeError(format!(
                        "No user specified for task '{}'.",
                        task_display(task)
                    )));
                }
            },
        },
    };

    Ok(user)
}

fn get_auth_config(host: &Table, task: &Table) -> mlua::Result<(String, SSHAuthMethod)> {
    let host_display = host_display(host);
    let task_display = task_display(task);

    let user = get_user(host, task)?;

    let defaults = Defaults::global();

    let default_private_key_file = defaults
        .private_key_file
        .read()
        .map_err(|_| RuntimeError("Failed to acquire read lock".to_string()))?
        .clone();

    let default_private_key_pass = defaults
        .private_key_pass
        .read()
        .map_err(|_| RuntimeError("Failed to acquire read lock".to_string()))?
        .clone();

    let default_password = defaults
        .password
        .read()
        .map_err(|_| RuntimeError("Failed to acquire read lock".to_string()))?
        .clone();

    let ssh_auth_method = match host.get::<String>("private_key_file") {
        Ok(private_key_file) => SSHAuthMethod::PublicKey {
            private_key: private_key_file,
            passphrase: host
                .get::<String>("private_key_pass")
                .ok()
                .or(default_private_key_pass),
        },
        Err(_) => match default_private_key_file {
            Some(ref private_key_file) => SSHAuthMethod::PublicKey {
                private_key: private_key_file.clone(),
                passphrase: host
                    .get::<String>("private_key_pass")
                    .ok()
                    .or(default_private_key_pass),
            },
            None => match host.get::<String>("password") {
                Ok(password) => SSHAuthMethod::Password(password),
                Err(_) => match default_password {
                    Some(ref password) => SSHAuthMethod::Password(password.clone()),
                    None => {
                        return Err(RuntimeError(format!(
                            "No authentication method specified for task '{task_display}' on host '{host_display}'."
                        )));
                    }
                },
            },
        },
    };

    Ok((user, ssh_auth_method))
}

fn get_elevation_config(host: &Table, task: &Table) -> mlua::Result<Elevation> {
    let defaults = Defaults::global();

    let Ok(default_elevate) = defaults.elevate.read() else {
        return Err(RuntimeError("Failed to acquire read lock".to_string()));
    };

    let task_elevate = task.get::<Value>("elevate")?;
    let host_elevate = host.get::<Value>("elevate")?;

    let elevate = if !task_elevate.is_nil() {
        task_elevate.as_boolean().unwrap_or(false)
    } else if !host_elevate.is_nil() {
        host_elevate.as_boolean().unwrap_or(false)
    } else {
        *default_elevate
    };

    if !elevate {
        return Ok(Elevation {
            method: ElevationMethod::None,
            as_user: None,
        });
    }

    let Ok(default_elevation_method) = defaults.elevation_method.read() else {
        return Err(RuntimeError("Failed to acquire read lock".to_string()));
    };

    let elevation_method_str = task.get::<String>("elevation_method").unwrap_or_else(|_| {
        host.get::<String>("elevation_method")
            .unwrap_or_else(|_| default_elevation_method.clone())
    });

    let elevation_method = match elevation_method_str.as_str() {
        "none" => Ok(ElevationMethod::None),
        "sudo" => Ok(ElevationMethod::Sudo),
        "su" => Ok(ElevationMethod::Su),
        _ => Err(RuntimeError(format!(
            "Unsupported elevation method: {elevation_method_str}"
        ))),
    };

    let default_as_user = match defaults.as_user.read() {
        Ok(as_user) => as_user.clone(),
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let as_user = task.get::<Option<String>>("as_user").unwrap_or_else(|_| {
        host.get::<Option<String>>("as_user")
            .unwrap_or(default_as_user)
    });

    Ok(Elevation {
        method: elevation_method?,
        as_user,
    })
}

fn create_ssh_session(host: &Table) -> mlua::Result<SSHSession> {
    let defaults = Defaults::global();
    let mut ssh = SSHSession::new()?;

    let Ok(default_key_check) = defaults.key_check.read() else {
        return Err(RuntimeError("Failed to acquire read lock".to_string()));
    };

    let host_key_check = host
        .get::<Value>("host_key_check")
        .map_or(true, |key_check| match key_check {
            Value::Nil => *default_key_check,
            Value::Boolean(false) => false,
            _ => true,
        });

    let Ok(default_known_hosts_file) = defaults.known_hosts_file.read() else {
        return Err(RuntimeError("Failed to acquire read lock".to_string()));
    };

    if host_key_check {
        ssh.known_hosts_file = host
            .get::<String>("known_hosts_file")
            .map_or_else(|_| Some(default_known_hosts_file.clone()), Some);
    }

    Ok(ssh)
}

fn setup_environment(ssh: &mut SSHSession, host: &Table, task: &Table) -> mlua::Result<()> {
    let defaults = Defaults::global();

    let Ok(default_env) = defaults.env.read() else {
        return Err(RuntimeError("Failed to acquire read lock".to_string()));
    };

    let env_host = host.get::<Option<Table>>("env")?;
    let env_task = task.get::<Option<Table>>("env")?;

    for (key, value) in default_env.iter() {
        ssh.set_env(key, value);
    }

    if let Some(env_host) = env_host {
        for pair in env_host.pairs::<String, String>() {
            let (key, value) = pair?;
            ssh.set_env(&key, &value);
        }
    }

    if let Some(env_task) = env_task {
        for pair in env_task.pairs::<String, String>() {
            let (key, value) = pair?;
            ssh.set_env(&key, &value);
        }
    }

    Ok(())
}

fn execute_task(
    lua: &Lua,
    module: &Table,
    ssh: SSHSession,
    task_display: &str,
    host_display: &str,
) -> mlua::Result<Table> {
    let dry_run = Args::parse().flags.dry_run;

    lua.load(chunk! {
        print(">> Running task '" .. $task_display .. "' on host '" .. $host_display .."' ...")
        $module.ssh = $ssh

        if $dry_run then
            if $module.dry_run ~= nil then
                $module:dry_run()
            else
                print("[[ Task '" .. $task_display .. "' on host '" .. $host_display .."' does not support dry-run. Assuming 'changed' is true. ]]")
                $module.ssh:set_changed(true)
            end
        else
            $module:run()
        end

        local result = $module.ssh:get_session_result()
        komandan.dprint(result.stdout)
        if result.exit_code ~= 0 then
            print(">> Task '" .. $task_display .. "' on host '" .. $host_display .."' failed with exit code " .. result.exit_code .. ": " .. result.stderr)
        else
            local state = "[OK]"
            if result.changed then
                state = "[Changed]"
            end
            print(">> Task '" .. $task_display .. "' on host '" .. $host_display .."' succeeded. " .. state)
        end

        if $module.cleanup ~= nil then
            $module:cleanup()
        end

        return result
    })
    .set_name("execute_task")
    .eval::<Table>()
}

// Tests
#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;

    #[test]
    fn test_get_auth_config() -> Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;

        // Test with user in host
        host.set("address", "localhost")?;
        host.set("user", "testuser")?;
        host.set("private_key_file", "/path/to/key")?;

        let module_params = lua.create_table()?;
        module_params.set("cmd", "echo test")?;
        let module = lua
            .load(chunk! {
                return komandan.modules.cmd($module_params)
            })
            .eval::<Table>()?;
        let task = lua.create_table()?;
        task.set(1, module)?;

        let (user, auth) = get_auth_config(&host, &task)?;
        assert_eq!(user, "testuser");
        match auth {
            SSHAuthMethod::PublicKey {
                private_key,
                passphrase,
            } => {
                assert_eq!(private_key, "/path/to/key");
                assert!(passphrase.is_none());
            }
            SSHAuthMethod::Password(_) => panic!("Expected PublicKey authentication"),
        }

        // Test with password auth
        host.set("private_key_file", Value::Nil)?;
        host.set("password", "testpass")?;
        let (_, auth) = get_auth_config(&host, &task)?;
        match auth {
            SSHAuthMethod::Password(pass) => assert_eq!(pass, "testpass"),
            SSHAuthMethod::PublicKey { .. } => panic!("Expected Password authentication"),
        }

        // Test with no authentication method
        host.set("password", Value::Nil)?;
        let result = get_auth_config(&host, &task);
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_get_elevation_config() -> Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        let task = lua.create_table()?;

        // Test with no elevation
        let elevation = get_elevation_config(&host, &task)?;
        assert!(matches!(
            elevation,
            Elevation {
                method: ElevationMethod::None,
                as_user: None
            }
        ));

        // Test with elevation from task
        task.set("elevate", true)?;
        let elevation = get_elevation_config(&host, &task)?;
        assert!(matches!(
            elevation,
            Elevation {
                method: ElevationMethod::Sudo,
                as_user: None
            }
        ));

        // Test with custom elevation method
        task.set("elevation_method", "su")?;
        let elevation = get_elevation_config(&host, &task)?;
        assert!(matches!(
            elevation,
            Elevation {
                method: ElevationMethod::Su,
                as_user: None
            }
        ));

        // Test invalid elevation method
        task.set("elevation_method", "invalid")?;
        assert!(get_elevation_config(&host, &task).is_err());

        Ok(())
    }

    #[test]
    fn test_setup_ssh_session() -> Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        host.set("address", "localhost")?;

        // Test with default settings
        let ssh = create_ssh_session(&host)?;
        assert!(ssh.known_hosts_file.is_some());

        // Test with host key check disabled
        host.set("host_key_check", false)?;
        let ssh = create_ssh_session(&host)?;
        assert!(ssh.known_hosts_file.is_none());

        // Test with custom known_hosts file
        host.set("known_hosts_file", "/path/to/known_hosts")?;
        host.set("host_key_check", true)?;
        let ssh = create_ssh_session(&host)?;
        assert_eq!(
            ssh.known_hosts_file,
            Some("/path/to/known_hosts".to_string())
        );

        // Test with known_hosts from defaults
        host.set("known_hosts_file", Value::Nil)?;
        lua.load(chunk! {
            komandan.defaults:set_known_hosts_file("/default/known_hosts")
        })
        .exec()?;
        let ssh = create_ssh_session(&host)?;
        assert_eq!(
            ssh.known_hosts_file,
            Some("/default/known_hosts".to_string())
        );

        Ok(())
    }

    #[test]
    fn test_setup_environment() -> Result<()> {
        let lua = create_lua()?;
        let mut ssh = SSHSession::new()?;
        let defaults = lua.create_table()?;
        let host = lua.create_table()?;
        let task = lua.create_table()?;

        // Test with environment variables at all levels
        let env_defaults = lua.create_table()?;
        env_defaults.set("DEFAULT_VAR", "default_value")?;
        defaults.set("env", env_defaults)?;

        let env_host = lua.create_table()?;
        env_host.set("HOST_VAR", "host_value")?;
        env_host.set("DEFAULT_VAR", "overridden_value")?; // Override default
        host.set("env", env_host)?;

        let env_task = lua.create_table()?;
        env_task.set("TASK_VAR", "task_value")?;
        task.set("env", env_task)?;

        setup_environment(&mut ssh, &host, &task)?;

        Ok(())
    }
}
