use crate::connection::ConnectionError;
use crate::connection::{get_auth_config, get_elevation_config, setup_environment_ssh};
use crate::defaults::Defaults;
use crate::ssh::SSHSession;
use crate::util::host_display;
use mlua::{Table, Value};

/// Create and configure an SSH session with host key verification settings
///
/// This function extracts SSH session creation and configuration logic from komando.rs
/// and handles host key verification and known hosts configuration.
///
/// # Arguments
/// * `host` - Host configuration table
///
/// # Returns
/// * `mlua::Result<SSHSession>` - A configured SSH session ready for connection
///
/// # Errors
/// Returns an error if:
/// - SSH session creation fails
/// - Configuration parameters are invalid
pub fn create_ssh_session(host: &Table) -> mlua::Result<SSHSession> {
    let defaults = Defaults::global();
    let mut ssh = SSHSession::new().map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to create SSH session: {e}"),
            context: "SSH session initialization".to_string(),
        }
        .to_runtime_error()
    })?;

    let Ok(default_key_check) = defaults.key_check.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default host key check setting".to_string(),
            context: "defaults access".to_string(),
        }
        .to_runtime_error());
    };

    let host_key_check = match host.get::<Value>("host_key_check")? {
        // Truly absent → fall back to default.
        Value::Nil => *default_key_check,
        // Either boolean value is accepted.
        Value::Boolean(b) => b,
        // Any other value (number, string, table, …) is a type mismatch.
        other => {
            return Err(ConnectionError::Configuration {
                message: format!(
                    "host_key_check must be a boolean, got {}",
                    other.type_name()
                ),
                context: "SSH session configuration".to_string(),
            }
            .to_runtime_error());
        }
    };

    let Ok(default_known_hosts_file) = defaults.known_hosts_file.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default known hosts file setting".to_string(),
            context: "defaults access".to_string(),
        }
        .to_runtime_error());
    };

    if host_key_check {
        // Read as Option<String> so a present-but-wrong-type value surfaces as
        // an error instead of silently falling back to the default.
        ssh.known_hosts_file = host
            .get::<Option<String>>("known_hosts_file")?
            .map_or_else(|| Some(default_known_hosts_file.clone()), Some);
    }

    Ok(ssh)
}

/// Get the port for SSH connection from host or defaults
///
/// # Arguments
/// * `host` - Host configuration table
///
/// # Returns
/// * `mlua::Result<u16>` - The resolved port number
///
/// # Errors
/// Returns an error if:
/// - `port` is present but not a number
/// - The default port lock is poisoned
pub fn get_port_from_host(host: &Table) -> mlua::Result<u16> {
    let defaults = Defaults::global();

    // Read as Option<u16> so a present-but-wrong-type value surfaces as an
    // error instead of silently falling back to the default port.
    if let Some(port) = host.get::<Option<u16>>("port")? {
        Ok(port)
    } else {
        let Ok(default_port) = defaults.port.read() else {
            return Err(ConnectionError::Configuration {
                message: "Failed to read default port setting".to_string(),
                context: "defaults access".to_string(),
            }
            .to_runtime_error());
        };
        Ok(*default_port)
    }
}

/// Create a fully configured SSH session using existing komando.rs logic
///
/// This function combines authentication, session creation, and configuration setup
/// to provide a ready-to-use SSH session.
///
/// # Arguments
/// * `host_table` - Host configuration table
/// * `task` - Task configuration table
///
/// # Returns
/// * `mlua::Result<SSHSession>` - A fully configured SSH session
///
/// # Errors
/// Returns an error if:
/// - Authentication configuration fails
/// - SSH session creation fails
/// - Connection establishment fails
/// - Configuration setup fails
pub fn create_configured_ssh_session(host_table: &Table, task: &Table) -> mlua::Result<SSHSession> {
    let host_display = host_display(host_table);

    // Use existing authentication configuration logic
    let (user, auth_method) = get_auth_config(host_table, task, None)?;

    // Use existing SSH session creation logic
    let mut ssh = create_ssh_session(host_table).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to create SSH session: {e}"),
            context: format!("SSH session creation for host '{host_display}'"),
        }
        .to_runtime_error()
    })?;

    // Extract connection parameters
    let address = host_table.get::<String>("address").map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Missing or invalid address: {e}"),
            context: format!("host '{host_display}'"),
        }
        .to_runtime_error()
    })?;

    let port = get_port_from_host(host_table).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to get port: {e}"),
            context: format!("host '{host_display}'"),
        }
        .to_runtime_error()
    })?;

    // Connect using existing logic with error type classification
    ssh.connect(&address, port, &user, auth_method)
        .map_err(|e| {
            let error_msg = e.to_string();

            // Classify the underlying error by its message content
            let is_auth = error_msg.contains("authentication")
                || error_msg.contains("Authentication")
                || error_msg.contains("auth")
                || error_msg.contains("login")
                || error_msg.contains("password")
                || error_msg.contains("key")
                || error_msg.contains("Permission denied");
            let is_host_key = error_msg.contains("host key")
                || error_msg.contains("Host key")
                || error_msg.contains("known_hosts")
                || error_msg.contains("verification");

            if is_auth {
                ConnectionError::Authentication {
                    message: error_msg,
                    host: format!("{address}:{port}"),
                    user: user.clone(),
                }
                .to_runtime_error()
            } else if is_host_key {
                ConnectionError::HostKeyVerification {
                    message: error_msg,
                    host: address.clone(),
                }
                .to_runtime_error()
            } else {
                ConnectionError::Connection {
                    message: error_msg,
                    host: address.clone(),
                    port,
                }
                .to_runtime_error()
            }
        })?;

    // Apply elevation and environment configuration
    ssh.elevation = get_elevation_config(host_table, task).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to configure elevation: {e}"),
            context: format!("elevation setup for host '{host_display}'"),
        }
        .to_runtime_error()
    })?;

    setup_environment_ssh(&mut ssh, host_table, task).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to setup SSH environment: {e}"),
            context: format!("environment setup for host '{host_display}'"),
        }
        .to_runtime_error()
    })?;

    Ok(ssh)
}
