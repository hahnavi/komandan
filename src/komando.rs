use std::cell::OnceCell;
use std::collections::HashMap;

use mlua::{Error::RuntimeError, FromLua, Integer, Lua, Table, Value};
use mlua::{IntoLua, LuaSerdeExt, chunk};
use rayon::prelude::*;

use crate::connection::{Connection, create_connection};
use crate::create_lua;
use crate::defaults::Defaults;
use crate::models::{Host, KomandoResult, Task};
use crate::report::{TaskStatus, insert_record};
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
        Connection::Local(local) => execute_task(
            lua,
            &module,
            local,
            &task_display,
            &host_display,
            " (local)",
        )?,
        Connection::SSH(ssh) => execute_task(lua, &module, ssh, &task_display, &host_display, "")?,
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

    if !crate::args::global_flags().no_report {
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
    let tasks_table = tasks
        .as_table()
        .ok_or_else(|| RuntimeError("Tasks must be a table".to_string()))?;
    let items = collect_keyed_values::<Task>(lua, tasks_table)?;
    parallel_komando(
        lua,
        items,
        |inner, task| {
            let host_v = host.clone().into_lua(inner)?;
            let task_v = task.clone().into_lua(inner)?;
            Ok((task_v, host_v))
        },
        "Failed to execute parallel tasks",
    )
}

pub fn komando_parallel_hosts(lua: &Lua, (task, hosts): (Value, Value)) -> mlua::Result<Table> {
    let task = Task::from_lua(task, lua)?;
    let hosts_table = hosts
        .as_table()
        .ok_or_else(|| RuntimeError("Hosts must be a table".to_string()))?;
    let items = collect_keyed_values::<Host>(lua, hosts_table)?;
    parallel_komando(
        lua,
        items,
        |inner, host| {
            let task_v = task.clone().into_lua(inner)?;
            let host_v = host.clone().into_lua(inner)?;
            Ok((task_v, host_v))
        },
        "Failed to execute parallel hosts",
    )
}

/// Walk a Lua table of `(key, value)` pairs into a `Vec` keyed by
/// `ParallelHashMapKey`, parsing each value into `T` via `FromLua`.
///
/// Number keys become `Number(u32)`; all other keys are stringified to `Text`.
/// Later duplicates overwrite earlier ones (matching the prior
/// `HashMap::insert` last-write-wins semantics).
///
/// # Errors
///
/// Returns `mlua::Error::RuntimeError` when a number key cannot be read as an
/// `i64`, when an integer key is negative or otherwise out of `u32` range, or
/// when `T::from_lua` fails for any value.
fn collect_keyed_values<T: FromLua>(
    lua: &Lua,
    table: &Table,
) -> mlua::Result<Vec<(ParallelHashMapKey, T)>> {
    let mut map: HashMap<ParallelHashMapKey, T> = HashMap::new();
    for pair in table.pairs::<Value, Value>() {
        let (key, value): (Value, Value) = pair?;
        let parsed = T::from_lua(value, lua)?;
        let phk = if key.is_number() {
            let key_int = key
                .as_integer()
                .ok_or_else(|| RuntimeError("Key must be an integer".to_string()))?;
            let key_u32 = u32::try_from(key_int)
                .map_err(|_| RuntimeError("Key out of range for u32".to_string()))?;
            ParallelHashMapKey::Number(key_u32)
        } else {
            ParallelHashMapKey::Text(key.to_string()?)
        };
        map.insert(phk, parsed);
    }
    Ok(map.into_iter().collect())
}

thread_local! {
    /// Per-worker-thread pooled Lua VM for parallel task execution.
    ///
    /// Built lazily on first use in a given rayon worker thread, then reused
    /// for every subsequent task that thread processes. This replaces the
    /// prior one-`create_lua()`-per-task model (the #1 perf hot spot, per
    /// `REFACTOR_PLAN.md` §1.2).
    ///
    /// # Why thread-local
    ///
    /// `mlua::Lua` is `!Send`/`!Sync`; it cannot live in a `Mutex`/channel
    /// shared across threads. A thread-local sidesteps that because each rayon
    /// closure body runs to completion on a single worker thread.
    ///
    /// # Safety invariants (must remain true)
    ///
    /// - The `komandan` global table is read-only after `setup_komandan_table`.
    /// - Built-in modules mutate only `self`/locals, never `_G`.
    /// - User Lua never executes inside a worker VM — only `komando`'s
    ///   internal `execute_task` chunk does.
    ///
    /// A future module that writes `_G` would violate this and corrupt pooled
    /// VMs; review new modules against this invariant.
    static WORKER_LUA: OnceCell<Lua> = const { OnceCell::new() };
}

/// Run `f` against this worker thread's pooled Lua VM, initializing it on first
/// use.
///
/// The VM is constructed once per rayon worker thread via `create_lua()` and
/// reused for the thread's lifetime. Uses `OnceCell` shared borrows so that a
/// (theoretical) re-entrant call within `f` does not panic on double-borrow —
/// it would simply observe the same VM.
///
/// # Errors
///
/// Returns the inner `mlua::Error` if VM construction or `f` fails. Returns
/// `mlua::Error::RuntimeError` if the thread-local is inaccessible (e.g. the
/// worker thread is tearing down), which should not occur for rayon pool
/// threads.
fn with_worker_lua<R>(f: impl FnOnce(&Lua) -> mlua::Result<R>) -> mlua::Result<R> {
    WORKER_LUA
        .try_with(|cell| {
            if cell.get().is_none() {
                let lua = create_lua().map_err(mlua::Error::external)?;
                cell.set(lua)
                    .map_err(|_| RuntimeError("worker Lua already set".to_string()))?;
            }
            let inner = cell
                .get()
                .ok_or_else(|| RuntimeError("worker Lua missing after set".to_string()))?;
            f(inner)
        })
        .map_err(|_| RuntimeError("worker thread-local Lua access failed".to_string()))
        .flatten()
}

/// Run `komando` in parallel over `items`, collecting the per-item results into
/// a Lua table keyed by the original `ParallelHashMapKey`.
///
/// Each item is processed on the calling rayon worker thread's pooled Lua VM
/// (see `WORKER_LUA`), which is built once per worker and reused across tasks
/// — see `REFACTOR_PLAN.md` §1.2. `build_args` is invoked per item to convert
/// the item plus the fixed operand — host for tasks-mode, task for hosts-mode
/// — into the `(task, host)` pair `komando` expects, expressed in the inner
/// VM's value space.
///
/// # Errors
///
/// Returns `mlua::Error::RuntimeError` carrying `error_msg` if any per-item
/// step fails: inner VM construction, argument conversion, `komando` execution,
/// or `KomandoResult` parsing. The final result-table build may surface its
/// own `mlua::Error` variants (e.g. string allocation failures).
fn parallel_komando<T, F>(
    lua: &Lua,
    items: Vec<(ParallelHashMapKey, T)>,
    build_args: F,
    error_msg: &str,
) -> mlua::Result<Table>
where
    T: Clone + Send + Sync,
    F: Fn(&Lua, &T) -> mlua::Result<(Value, Value)> + Send + Sync,
{
    let results: Option<Vec<(ParallelHashMapKey, KomandoResult)>> = items
        .into_par_iter()
        .map(|(key, item)| {
            let result: mlua::Result<(ParallelHashMapKey, KomandoResult)> =
                with_worker_lua(|inner| {
                    let (task_v, host_v) = build_args(inner, &item)?;
                    let result = komando(inner, (task_v, host_v))?;
                    let parsed = inner.from_value::<KomandoResult>(Value::Table(result))?;
                    Ok((key, parsed))
                });
            result.ok()
        })
        .collect();

    let results = results.ok_or_else(|| RuntimeError(error_msg.to_string()))?;

    let results_table = lua.create_table()?;
    for (key, result) in results {
        let key_v: Value = match key {
            ParallelHashMapKey::Number(n) => Value::Number(f64::from(n)),
            ParallelHashMapKey::Text(s) => Value::String(lua.create_string(&s)?),
        };
        results_table.set(key_v, lua.to_value(&result)?)?;
    }
    Ok(results_table)
}

/// Run a single task's Lua-side execution flow against `module` on a connected
/// session.
///
/// Unified over SSH and local transports: the session is exposed to Lua as
/// `$module.ssh` regardless of transport (the field name is an internal
/// Komandan convention referenced by the README, not a user-facing knob).
/// `connection_label` is appended to the initial "Running task ... on host
/// ..." status line so local runs are distinguishable in stdout — pass `""`
/// for SSH and `" (local)"` for local execution; all other status lines are
/// transport-agnostic by design.
///
/// # Errors
///
/// Propagates any `mlua::Error` raised while loading or evaluating the
/// per-task Lua chunk: module field access, `dry_run` / `run` / `cleanup`
/// invocations, result extraction, or status printing.
fn execute_task<S>(
    lua: &Lua,
    module: &Table,
    session: S,
    task_display: &str,
    host_display: &str,
    connection_label: &str,
) -> mlua::Result<Table>
where
    S: IntoLua + Clone,
{
    let dry_run = crate::args::global_flags().dry_run;

    lua.load(chunk! {
        print(">> Running task '" .. $task_display .. "' on host '" .. $host_display .. "'" .. $connection_label .. " ...")
        $module.ssh = $session

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
