use crate::connection::ConnectionError;
use crate::defaults::Defaults;
use crate::executor::CommandExecutor;
use crate::local::LocalSession;
use crate::ssh::SSHSession;
use mlua::Table;

/// Set up environment variables for SSH sessions
///
/// This function extracts environment variable setup logic from komando.rs
/// and handles defaults, host-level, and task-level environment variables.
///
/// # Arguments
/// * `ssh` - Mutable reference to SSH session
/// * `host` - Host configuration table
/// * `task` - Task configuration table
///
/// # Returns
/// * `mlua::Result<()>` - Success or error
///
/// # Errors
/// Returns an error if:
/// - Default values cannot be read
/// - Environment variable tables cannot be processed
pub fn setup_environment_ssh(ssh: &mut SSHSession, host: &Table, task: &Table) -> mlua::Result<()> {
    let defaults = Defaults::global();

    let Ok(default_env) = defaults.env.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default environment variables".to_string(),
            context: "defaults access".to_string(),
        }
        .to_runtime_error());
    };

    let env_host = host.get::<Option<Table>>("env")?;
    let env_task = task.get::<Option<Table>>("env")?;

    for (key, value) in default_env.iter() {
        ssh.set_env(key, value);
    }

    if let Some(env_host) = env_host {
        for pair in env_host.pairs::<String, String>() {
            let (key, value) = pair.map_err(|e| {
                ConnectionError::Configuration {
                    message: format!("Invalid host environment variable: {e}"),
                    context: "host environment variable processing".to_string(),
                }
                .to_runtime_error()
            })?;
            ssh.set_env(&key, &value);
        }
    }

    if let Some(env_task) = env_task {
        for pair in env_task.pairs::<String, String>() {
            let (key, value) = pair.map_err(|e| {
                ConnectionError::Configuration {
                    message: format!("Invalid task environment variable: {e}"),
                    context: "task environment variable processing".to_string(),
                }
                .to_runtime_error()
            })?;
            ssh.set_env(&key, &value);
        }
    }

    Ok(())
}

/// Set up environment variables for local sessions
///
/// This function applies environment variables to local sessions using the same
/// logic as SSH sessions for consistency.
///
/// # Arguments
/// * `local` - Mutable reference to local session
/// * `host` - Host configuration table
/// * `task` - Task configuration table
///
/// # Returns
/// * `mlua::Result<()>` - Success or error
pub fn setup_environment_local(
    local: &mut LocalSession,
    host: &Table,
    task: &Table,
) -> mlua::Result<()> {
    let defaults = Defaults::global();

    let Ok(default_env) = defaults.env.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default environment variables".to_string(),
            context: "defaults access".to_string(),
        }
        .to_runtime_error());
    };

    let env_host = host.get::<Option<Table>>("env")?;
    let env_task = task.get::<Option<Table>>("env")?;

    for (key, value) in default_env.iter() {
        local.set_env(key, value);
    }

    if let Some(env_host) = env_host {
        for pair in env_host.pairs::<String, String>() {
            let (key, value) = pair.map_err(|e| {
                ConnectionError::Configuration {
                    message: format!("Invalid host environment variable: {e}"),
                    context: "host environment variable processing".to_string(),
                }
                .to_runtime_error()
            })?;
            local.set_env(&key, &value);
        }
    }

    if let Some(env_task) = env_task {
        for pair in env_task.pairs::<String, String>() {
            let (key, value) = pair.map_err(|e| {
                ConnectionError::Configuration {
                    message: format!("Invalid task environment variable: {e}"),
                    context: "task environment variable processing".to_string(),
                }
                .to_runtime_error()
            })?;
            local.set_env(&key, &value);
        }
    }

    Ok(())
}
