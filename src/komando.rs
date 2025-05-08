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

    let defaults = Defaults::global()?;

    let (user, ssh_auth_method) = get_auth_config(&host, &task)?;
    let elevation = get_elevation_config(&host, &task)?;

    let default_port = match defaults.port.read() {
        Ok(port) => port,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let port = host.get::<Integer>("port").unwrap_or(*default_port as i64) as u16;

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
        Ok(ignore_exit_code) => ignore_exit_code,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let ignore_exit_code = task
        .get::<bool>("ignore_exit_code")
        .unwrap_or(*default_ignore_exit_code);

    let exit_code = result.get::<Integer>("exit_code")?;

    if exit_code != 0 && !ignore_exit_code {
        return Err(RuntimeError("Failed to run task.".to_string()));
    }

    let task_status = if exit_code != 0 {
        TaskStatus::Failed
    } else {
        match result.get::<bool>("changed")? {
            true => TaskStatus::Changed,
            false => TaskStatus::OK,
        }
    };

    if !Args::parse().no_report {
        insert_record(task_display, host_display, task_status);
    }

    Ok(result)
}

pub fn komando_parallel_tasks(lua: &Lua, (host, tasks): (Value, Value)) -> mlua::Result<Table> {
    let host = Host::from_lua(host, lua)?;
    let mut tasks_hm = HashMap::<u32, Task>::new();
    for pair in tasks.as_table().unwrap().pairs::<u32, Value>() {
        let (key, value): (u32, Value) = pair?;
        let task = Task::from_lua(value, lua)?;
        tasks_hm.insert(key, task);
    }

    let results: HashMap<u32, KomandoResult> = tasks_hm
        .par_iter()
        .map(|(i, task)| {
            let lua = create_lua().unwrap();
            let host = host.clone().into_lua(&lua).unwrap();
            let task = task.clone().into_lua(&lua).unwrap();
            let result = komando(&lua, (host, task)).unwrap();

            (
                *i,
                lua.from_value::<KomandoResult>(Value::Table(result))
                    .unwrap(),
            )
        })
        .collect::<HashMap<u32, KomandoResult>>();

    let results_table = lua.create_table()?;
    results.iter().for_each(|(i, result)| {
        results_table
            .set(*i, lua.to_value(result).unwrap())
            .unwrap();
    });

    Ok(results_table)
}

pub fn komando_parallel_hosts(lua: &Lua, (hosts, task): (Value, Value)) -> mlua::Result<Table> {
    let task = Task::from_lua(task, lua)?;
    let mut hosts_hm = HashMap::<u32, Host>::new();
    for pair in hosts.as_table().unwrap().pairs::<u32, Value>() {
        let (key, value): (u32, Value) = pair?;
        let host = Host::from_lua(value, lua)?;
        hosts_hm.insert(key, host);
    }

    let results: HashMap<u32, KomandoResult> = hosts_hm
        .par_iter()
        .map(|(i, host)| {
            let lua = create_lua().unwrap();
            let host = host.clone().into_lua(&lua).unwrap();
            let task = task.clone().into_lua(&lua).unwrap();
            let result = komando(&lua, (host, task)).unwrap();

            (
                *i,
                lua.from_value::<KomandoResult>(Value::Table(result))
                    .unwrap(),
            )
        })
        .collect::<HashMap<u32, KomandoResult>>();

    let results_table = lua.create_table()?;
    results.iter().for_each(|(i, result)| {
        results_table
            .set(*i, lua.to_value(result).unwrap())
            .unwrap();
    });

    Ok(results_table)
}

fn get_user(host: &Table, task: &Table) -> mlua::Result<String> {
    let defaults = Defaults::global()?;
    let default_user = match defaults.user.read() {
        Ok(user) => user,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };
    let user = match host.get::<String>("user") {
        Ok(user) => user,
        Err(_) => match *default_user {
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

    let defaults = Defaults::global()?;

    let default_private_key_file = match defaults.private_key_file.read() {
        Ok(private_key_file) => private_key_file,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let default_private_key_pass = match defaults.private_key_pass.read() {
        Ok(private_key_pass) => private_key_pass,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let default_password = match defaults.password.read() {
        Ok(password) => password,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let ssh_auth_method = match host.get::<String>("private_key_file") {
        Ok(private_key_file) => SSHAuthMethod::PublicKey {
            private_key: private_key_file,
            passphrase: match host.get::<String>("private_key_pass") {
                Ok(passphrase) => Some(passphrase),
                Err(_) => (*default_private_key_pass).clone(),
            },
        },
        Err(_) => match *default_private_key_file {
            Some(ref private_key_file) => SSHAuthMethod::PublicKey {
                private_key: private_key_file.clone(),
                passphrase: match host.get::<String>("private_key_pass") {
                    Ok(passphrase) => Some(passphrase),
                    Err(_) => (*default_private_key_pass).clone(),
                },
            },
            None => match host.get::<String>("password") {
                Ok(password) => SSHAuthMethod::Password(password),
                Err(_) => match *default_password {
                    Some(ref password) => SSHAuthMethod::Password(password.clone()),
                    None => {
                        return Err(RuntimeError(format!(
                            "No authentication method specified for task '{}' on host '{}'.",
                            task_display, host_display
                        )));
                    }
                },
            },
        },
    };

    Ok((user, ssh_auth_method))
}

fn get_elevation_config(host: &Table, task: &Table) -> mlua::Result<Elevation> {
    let defaults = Defaults::global()?;

    let default_elevate = match defaults.elevate.read() {
        Ok(elevate) => elevate,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let task_elevate = task.get::<Value>("elevate")?;
    let host_elevate = host.get::<Value>("elevate")?;

    let elevate = if !task_elevate.is_nil() {
        task_elevate.as_boolean().unwrap()
    } else if !host_elevate.is_nil() {
        host_elevate.as_boolean().unwrap()
    } else {
        *default_elevate
    };

    if !elevate {
        return Ok(Elevation {
            method: ElevationMethod::None,
            as_user: None,
        });
    }

    let default_elevation_method = match defaults.elevation_method.read() {
        Ok(elevation_method) => elevation_method,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let elevation_method_str = task.get::<String>("elevation_method").unwrap_or(
        host.get::<String>("elevation_method")
            .unwrap_or(default_elevation_method.clone()),
    );

    let elevation_method = match elevation_method_str.as_str() {
        "none" => Ok(ElevationMethod::None),
        "sudo" => Ok(ElevationMethod::Sudo),
        "su" => Ok(ElevationMethod::Su),
        _ => Err(RuntimeError(format!(
            "Unsupported elevation method: {}",
            elevation_method_str
        ))),
    };

    let default_as_user = match defaults.as_user.read() {
        Ok(as_user) => as_user,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let as_user = task.get::<Option<String>>("as_user").unwrap_or(
        host.get::<Option<String>>("as_user")
            .unwrap_or(default_as_user.clone()),
    );

    Ok(Elevation {
        method: elevation_method?,
        as_user,
    })
}

fn create_ssh_session(host: &Table) -> mlua::Result<SSHSession> {
    let defaults = Defaults::global()?;
    let mut ssh = SSHSession::new()?;

    let default_host_key_check = match defaults.host_key_check.read() {
        Ok(host_key_check) => host_key_check,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let host_key_check = match host.get::<Value>("host_key_check") {
        Ok(host_key_check) => match host_key_check {
            Value::Nil => *default_host_key_check,
            Value::Boolean(false) => false,
            _ => true,
        },
        Err(_) => true,
    };

    let default_known_hosts_file = match defaults.known_hosts_file.read() {
        Ok(known_hosts_file) => known_hosts_file,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    if host_key_check {
        ssh.known_hosts_file = match host.get::<String>("known_hosts_file") {
            Ok(known_hosts_file) => Some(known_hosts_file),
            Err(_) => Some(default_known_hosts_file.clone()),
        };
    }

    Ok(ssh)
}

fn setup_environment(ssh: &mut SSHSession, host: &Table, task: &Table) -> mlua::Result<()> {
    let defaults = Defaults::global()?;

    let default_env = match defaults.env.read() {
        Ok(env) => env,
        Err(_) => return Err(RuntimeError("Failed to acquire read lock".to_string())),
    };

    let env_host = host.get::<Option<Table>>("env")?;
    let env_task = task.get::<Option<Table>>("env")?;

    for (key, value) in default_env.clone() {
        ssh.set_env(&key, &value);
    }

    if env_host.is_some() {
        for pair in env_host.unwrap().pairs() {
            let (key, value): (String, String) = pair?;
            ssh.set_env(&key, &value);
        }
    }

    if env_task.is_some() {
        for pair in env_task.unwrap().pairs() {
            let (key, value): (String, String) = pair?;
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
    let dry_run = Args::parse().dry_run;

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
            _ => panic!("Expected PublicKey authentication"),
        }

        // Test with password auth
        host.set("private_key_file", Value::Nil)?;
        host.set("password", "testpass")?;
        let (_, auth) = get_auth_config(&host, &task)?;
        match auth {
            SSHAuthMethod::Password(pass) => assert_eq!(pass, "testpass"),
            _ => panic!("Expected Password authentication"),
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
        assert_eq!(ssh.known_hosts_file.unwrap(), "/path/to/known_hosts");

        // Test with known_hosts from defaults
        host.set("known_hosts_file", Value::Nil)?;
        lua.load(chunk! {
            komandan.defaults:set_known_hosts_file("/default/known_hosts")
        })
        .exec()?;
        let ssh = create_ssh_session(&host)?;
        assert_eq!(ssh.known_hosts_file.unwrap(), "/default/known_hosts");

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
