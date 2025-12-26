use std::collections::HashMap;

use clap::Parser;
use mlua::{Error::RuntimeError, FromLua, Integer, Lua, Table, Value};
use mlua::{IntoLua, LuaSerdeExt, chunk};
use rayon::prelude::*;

use crate::args::Args;
use crate::connection::{Connection, create_connection};
use crate::create_lua;
use crate::defaults::Defaults;
use crate::local::LocalSession;
use crate::models::{Host, KomandoResult, Task};
use crate::report::{TaskStatus, insert_record};
use crate::ssh::SSHSession;
use crate::util::{host_display, task_display};
use crate::validator::{validate_host, validate_task};

/// Execute a task on a host using the centralized connection factory
///
/// This is the core function for executing automation tasks. It uses the centralized
/// connection factory to establish connections, ensuring consistent authentication,
/// configuration, and error handling across all task executions.
///
/// # Arguments
/// * `lua` - The Lua context for validation and execution
/// * `task` - Task configuration containing the module and parameters to execute
/// * `host` - Host configuration for connection establishment (optional, defaults to localhost)
///
/// # Returns
/// * `mlua::Result<Table>` - Execution results including stdout, stderr, `exit_code`, and metadata
///
/// # Features
/// - Uses centralized connection factory for consistent SSH/local connection handling
/// - Maintains existing task execution flow and behavior
/// - Preserves existing error handling and reporting
/// - Supports both SSH and local execution based on host configuration
pub fn komando(lua: &Lua, (task, host): (Value, Value)) -> mlua::Result<Table> {
    let (task, host) = if host.is_nil() {
        (
            lua.create_function(validate_task)?.call::<Table>(&host)?,
            lua.load(chunk! {
                return { address = "localhost" }
            })
            .eval::<Table>()?,
        )
    } else {
        (
            lua.create_function(validate_task)?.call::<Table>(&task)?,
            lua.create_function(validate_host)?.call::<Table>(&host)?,
        )
    };

    let module = task.get::<Table>(1)?;

    let host_display = host_display(&host);
    let task_display = task_display(&task);

    // Use centralized connection creation
    let connection = create_connection(lua, &Value::Table(host))?;

    let result = match connection {
        Connection::Local(local) => {
            execute_task_local(lua, &module, local, &task_display, &host_display)?
        }
        Connection::SSH(ssh) => execute_task_ssh(lua, &module, ssh, &task_display, &host_display)?,
    };

    let defaults = Defaults::global();
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

pub fn komando_parallel_tasks(lua: &Lua, (tasks, host): (Value, Value)) -> mlua::Result<Table> {
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
            let result = komando(&lua, (task, host)).ok()?;

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

pub fn komando_parallel_hosts(lua: &Lua, (task, hosts): (Value, Value)) -> mlua::Result<Table> {
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
            let result = komando(&lua, (task, host)).ok()?;

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

fn execute_task_ssh(
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
    .set_name("execute_task_ssh")
    .eval::<Table>()
}

fn execute_task_local(
    lua: &Lua,
    module: &Table,
    local_session: LocalSession,
    task_display: &str,
    host_display: &str,
) -> mlua::Result<Table> {
    let dry_run = Args::parse().flags.dry_run;

    lua.load(chunk! {
        print(">> Running task '" .. $task_display .. "' on host '" .. $host_display .."' (local) ...")
        $module.ssh = $local_session

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
    .set_name("execute_task_local")
    .eval::<Table>()
}

// Tests
#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::*;
    use crate::connection::{
        create_ssh_session, get_auth_config, get_elevation_config, setup_environment_ssh,
    };
    use crate::ssh::{Elevation, ElevationMethod, SSHAuthMethod, SSHSession};

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

        let (user, auth) = get_auth_config(&host, &task, None)?;
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
        let (_, auth) = get_auth_config(&host, &task, None)?;
        match auth {
            SSHAuthMethod::Password(pass) => assert_eq!(pass, "testpass"),
            SSHAuthMethod::PublicKey { .. } => panic!("Expected Password authentication"),
        }

        // Test with no authentication method
        host.set("password", Value::Nil)?;
        let temp_dir =
            tempfile::tempdir().map_err(|e| anyhow::anyhow!("failed to create temp dir: {e}"))?;
        let home_path = temp_dir.path().display().to_string();
        let result = get_auth_config(&host, &task, Some(&home_path));
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
        let host = lua.create_table()?;
        let task = lua.create_table()?;

        // Test with environment variables at all levels
        let env_host = lua.create_table()?;
        env_host.set("HOST_VAR", "host_value")?;
        env_host.set("DEFAULT_VAR", "overridden_value")?; // Override default
        host.set("env", env_host)?;

        let env_task = lua.create_table()?;
        env_task.set("TASK_VAR", "task_value")?;
        task.set("env", env_task)?;

        setup_environment_ssh(&mut ssh, &host, &task)?;

        Ok(())
    }
}
