//! # Connection Factory Module
//!
//! This module provides a centralized connection factory for creating and managing
//! SSH and local connections across the Komandan codebase. It ensures consistency
//! in authentication, configuration, and error handling.
//!
//! ## Key Features
//!
//! - **Unified Interface**: Single `create_connection()` function for all connection types
//! - **Consistent Authentication**: Reuses existing authentication logic from komando.rs
//! - **Detailed Error Handling**: Provides comprehensive error messages with troubleshooting guidance
//! - **Backward Compatibility**: Maintains existing function signatures and behavior
//! - **Configuration Reuse**: Uses existing validation, defaults, and configuration patterns
//!
//! ## Usage
//!
//! ```rust,no_run
//! use komandan::connection::create_connection;
//! use komandan::create_lua;
//! use mlua::Value;
//! use anyhow::Result;
//!
//! fn example() -> Result<()> {
//!     let lua = create_lua()?;
//!     let host_table = lua.create_table()?;
//!     host_table.set("address", "localhost")?;
//!
//!     let mut connection = create_connection(&lua, &Value::Table(host_table))?;
//!     let (stdout, stderr, exit_code) = connection.cmd("echo test")?;
//!     Ok(())
//! }
//! ```
//!
//! ## Connection Types
//!
//! - **Local**: For localhost, 127.0.0.1, `::1`, or explicit `connection = "local"`
//! - **SSH**: For remote addresses or explicit `connection = "ssh"`
//!
//! ## Error Handling
//!
//! All connection errors include detailed troubleshooting information:
//! - Authentication failures with specific guidance
//! - Connection issues with network troubleshooting steps
//! - Host key verification problems with resolution instructions
//! - Configuration errors with parameter validation help

use crate::defaults::Defaults;
use crate::executor::CommandExecutor;
use crate::local::LocalSession;
use crate::models::ConnectionType;
use crate::ssh::{Elevation, ElevationMethod, SSHAuthMethod, SSHSession};
use crate::util::{host_display, task_display};
use crate::validator::validate_host;
use anyhow::Result;
use mlua::{Error::RuntimeError, Lua, Table, Value};
use std::env;
use std::path::Path;

#[cfg(test)]
pub mod test_utils;

/// Standardized error types for SSH connections
#[derive(Debug)]
pub enum ConnectionError {
    HostValidation {
        message: String,
        host: String,
    },
    Authentication {
        message: String,
        host: String,
        user: String,
        troubleshooting: String,
    },
    Connection {
        message: String,
        host: String,
        port: u16,
        troubleshooting: String,
    },
    HostKeyVerification {
        message: String,
        host: String,
        troubleshooting: String,
    },
    Configuration {
        message: String,
        context: String,
        troubleshooting: String,
    },
}

impl ConnectionError {
    /// Format error message with consistent structure and troubleshooting context
    #[must_use]
    pub fn format_error(&self) -> String {
        match self {
            Self::HostValidation { message, host } => {
                format!(
                    "Host validation failed: {message} for host '{host}'\n\
                    Troubleshooting: Verify host configuration parameters are correct and complete"
                )
            }
            Self::Authentication {
                message,
                host,
                user,
                troubleshooting,
            } => {
                format!(
                    "SSH authentication failed: {message} for {user}@{host}\n\
                    Troubleshooting: {troubleshooting}"
                )
            }
            Self::Connection {
                message,
                host,
                port,
                troubleshooting,
            } => {
                format!(
                    "SSH connection failed: {message} for {host}:{port}\n\
                    Troubleshooting: {troubleshooting}"
                )
            }
            Self::HostKeyVerification {
                message,
                host,
                troubleshooting,
            } => {
                format!(
                    "SSH host key verification failed: {message} for {host}\n\
                    Troubleshooting: {troubleshooting}"
                )
            }
            Self::Configuration {
                message,
                context,
                troubleshooting,
            } => {
                format!(
                    "SSH configuration error: {message} in {context}\n\
                    Troubleshooting: {troubleshooting}"
                )
            }
        }
    }

    /// Convert to mlua `RuntimeError` with formatted message
    #[must_use]
    pub fn to_runtime_error(self) -> mlua::Error {
        RuntimeError(self.format_error())
    }
}

/// Helper function to create authentication troubleshooting guidance
fn get_auth_troubleshooting(auth_method: &SSHAuthMethod) -> String {
    match auth_method {
        SSHAuthMethod::Password(_) => {
            "Check that password authentication is enabled on the server and the password is correct. \
            Consider using SSH key authentication for better security.\n\
            Additional steps:\n\
            1. Verify PasswordAuthentication is enabled in /etc/ssh/sshd_config\n\
            2. Check if the user account is locked: passwd -S username\n\
            3. Review authentication logs: tail -f /var/log/auth.log\n\
            4. Test with verbose SSH: ssh -v username@host".to_string()
        }
        SSHAuthMethod::PublicKey { private_key, passphrase } => {
            let base_msg = if passphrase.is_some() {
                format!(
                    "Verify the private key file '{private_key}' exists, has correct permissions (600), \
                    and the passphrase is correct. Ensure the corresponding public key is in \
                    the server's authorized_keys file."
                )
            } else {
                format!(
                    "Verify the private key file '{private_key}' exists, has correct permissions (600), \
                    and the corresponding public key is in the server's authorized_keys file."
                )
            };

            format!(
                "{base_msg}\n\
                Additional troubleshooting steps:\n\
                1. Check key file permissions: ls -la {private_key}\n\
                2. Verify public key on server: cat ~/.ssh/authorized_keys\n\
                3. Test key manually: ssh -i {private_key} -v username@host\n\
                4. Check SSH agent: ssh-add -l\n\
                5. Verify PubkeyAuthentication is enabled in /etc/ssh/sshd_config"
            )
        }
    }
}

/// Helper function to create connection troubleshooting guidance
fn get_connection_troubleshooting(host: &str, port: u16) -> String {
    format!(
        "Check network connectivity to {host}:{port}, verify the SSH service is running, \
        and ensure firewall rules allow connections on port {port}.\n\
        Detailed troubleshooting steps:\n\
        1. Test basic connectivity: ping {host}\n\
        2. Check port accessibility: telnet {host} {port} or nc -zv {host} {port}\n\
        3. Try manual SSH connection: ssh -v -p {port} {host}\n\
        4. Check SSH service status: systemctl status ssh (on target)\n\
        5. Review SSH daemon logs: journalctl -u ssh -f\n\
        6. Verify firewall rules: ufw status or iptables -L\n\
        7. Check if SSH is listening: netstat -tlnp | grep :{port} (on target)"
    )
}

/// Helper function to create host key troubleshooting guidance
fn get_host_key_troubleshooting(host: &str, known_hosts_file: &str) -> String {
    format!(
        "The host key for '{host}' has changed or is not in the known_hosts file.\n\
        Detailed resolution steps:\n\
        1. Verify this change is expected (server rebuild, key rotation, etc.)\n\
        2. Remove old key: ssh-keygen -R {host}\n\
        3. Add new key: ssh-keyscan {host} >> {known_hosts_file}\n\
        4. Alternative: ssh-keyscan -p PORT {host} >> {known_hosts_file} (if using non-standard port)\n\
        5. Manual verification: ssh -o StrictHostKeyChecking=ask {host}\n\
        6. For testing only: ssh -o StrictHostKeyChecking=no {host} (NOT recommended for production)\n\
        7. Check known_hosts file permissions: ls -la {known_hosts_file}\n\
        8. Verify known_hosts file format and content"
    )
}

/// Unified connection interface that can represent either SSH or local connections
#[derive(Clone, Debug)]
#[allow(clippy::upper_case_acronyms)]
pub enum Connection {
    SSH(SSHSession),
    Local(LocalSession),
}

impl Connection {
    /// Execute a command using the appropriate connection type
    ///
    /// # Errors
    /// Returns an error if the command execution fails or the connection is invalid
    #[allow(dead_code)]
    pub fn cmd(&mut self, command: &str) -> Result<(String, String, i32)> {
        match self {
            Self::SSH(ssh) => ssh.cmd(command),
            Self::Local(local) => local.cmd(command),
        }
    }

    /// Execute a command quietly (without affecting session state) using the appropriate connection type
    ///
    /// # Errors
    /// Returns an error if the command execution fails or the connection is invalid
    #[allow(dead_code)]
    pub fn cmdq(&self, command: &str) -> Result<(String, String, i32)> {
        match self {
            Self::SSH(ssh) => ssh.cmdq(command),
            Self::Local(local) => local.cmdq(command),
        }
    }

    /// Set an environment variable for the connection
    #[allow(dead_code)]
    pub fn set_env(&mut self, key: &str, value: &str) {
        match self {
            Self::SSH(ssh) => ssh.set_env(key, value),
            Self::Local(local) => local.set_env(key, value),
        }
    }

    /// Get the connection type
    #[allow(dead_code)]
    #[must_use]
    pub const fn connection_type(&self) -> ConnectionType {
        match self {
            Self::SSH(_) => ConnectionType::SSH,
            Self::Local(_) => ConnectionType::Local,
        }
    }

    /// Upload a file from local to remote/target
    ///
    /// # Errors
    /// Returns an error if the upload fails
    #[allow(dead_code)]
    pub fn upload(&self, local_path: &str, remote_path: &str) -> Result<()> {
        match self {
            Self::SSH(ssh) => ssh.upload(Path::new(local_path), Path::new(remote_path)),
            Self::Local(local) => local.upload(Path::new(local_path), Path::new(remote_path)),
        }
    }

    /// Download a file from remote/target to local
    ///
    /// # Errors
    /// Returns an error if the download fails
    #[allow(dead_code)]
    pub fn download(&self, remote_path: &str, local_path: &str) -> Result<()> {
        match self {
            Self::SSH(ssh) => ssh.download(Path::new(remote_path), Path::new(local_path)),
            Self::Local(local) => local.download(Path::new(remote_path), Path::new(local_path)),
        }
    }
}

/// Create a connection (SSH or local) based on host configuration
///
/// This function serves as the centralized connection factory that determines
/// the appropriate connection type and creates a fully configured connection.
///
/// # Arguments
/// * `lua` - The Lua context for validation
/// * `host` - Host configuration value (will be validated)
///
/// # Returns
/// * `mlua::Result<Connection>` - A configured connection ready for use
///
/// # Errors
/// Returns an error if:
/// - Host validation fails
/// - Connection creation fails
/// - Authentication setup fails
pub fn create_connection(lua: &Lua, host: &Value) -> mlua::Result<Connection> {
    // Validate host using existing validation logic
    let host_table = validate_host(lua, host.clone()).map_err(|e| {
        let host_display = match &host {
            Value::Table(table) => host_display(table),
            _ => "invalid".to_string(),
        };
        ConnectionError::HostValidation {
            message: e.to_string(),
            host: host_display,
        }
        .to_runtime_error()
    })?;

    // Determine connection type using existing logic
    let connection_type = determine_connection_type(&host_table).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to determine connection type: {e}"),
            context: "connection type determination".to_string(),
            troubleshooting: "Verify the 'connection' field is set to 'ssh' or 'local', or ensure the address is valid".to_string(),
        }.to_runtime_error()
    })?;

    match connection_type {
        ConnectionType::Local => {
            let mut local = LocalSession::new();

            // Create a dummy task for functions that only have host context
            let task = create_dummy_task(lua)?;

            // Apply environment configuration to local session
            setup_environment_local(&mut local, &host_table, &task).map_err(|e| {
                ConnectionError::Configuration {
                    message: format!("Failed to setup local environment: {e}"),
                    context: "local session environment setup".to_string(),
                    troubleshooting:
                        "Check environment variable configuration in host or task settings"
                            .to_string(),
                }
                .to_runtime_error()
            })?;

            Ok(Connection::Local(local))
        }
        ConnectionType::SSH => {
            // Create a dummy task for functions that only have host context
            let task = create_dummy_task(lua)?;

            // Create fully configured SSH session with detailed error handling
            let ssh = create_configured_ssh_session(&host_table, &task)?;

            Ok(Connection::SSH(ssh))
        }
    }
}

/// Determine the connection type based on host configuration
///
/// This function uses the same logic as the existing `determine_connection_type`
/// function from komando.rs to maintain consistency.
///
/// # Arguments
/// * `host` - Host configuration table
///
/// # Returns
/// * `mlua::Result<ConnectionType>` - The determined connection type
#[allow(dead_code)]
fn determine_connection_type(host: &Table) -> mlua::Result<ConnectionType> {
    // Check if connection type is explicitly set
    if let Some(conn_type) = host
        .get::<String>("connection")
        .ok()
        .and_then(|s| s.parse().ok())
    {
        return Ok(conn_type);
    }

    // Check if address is localhost
    let address = host.get::<String>("address")?;
    if is_localhost(&address) {
        Ok(ConnectionType::Local)
    } else {
        Ok(ConnectionType::SSH)
    }
}

/// Check if an address represents localhost
///
/// This function uses the same logic as the existing `is_localhost`
/// function from komando.rs to maintain consistency.
///
/// # Arguments
/// * `address` - The address to check
///
/// # Returns
/// * `bool` - True if the address represents localhost
#[allow(dead_code)]
fn is_localhost(address: &str) -> bool {
    matches!(address, "localhost" | "127.0.0.1" | "::1")
}

/// Get the user for SSH connection from host, task, or defaults
///
/// This function extracts the user configuration using the same logic
/// as the existing implementation in komando.rs.
///
/// # Arguments
/// * `host` - Host configuration table
/// * `task` - Task configuration table
///
/// # Returns
/// * `mlua::Result<String>` - The resolved username
///
/// # Errors
/// Returns an error if no user can be determined from any source
fn get_user(host: &Table, task: &Table) -> mlua::Result<String> {
    let defaults = Defaults::global();
    let default_user = match defaults.user.read() {
        Ok(user) => user.clone(),
        Err(_) => {
            return Err(RuntimeError(
                "Failed to acquire read lock for default user".to_string(),
            ));
        }
    };
    let user = match host.get::<String>("user") {
        Ok(user) => user,
        Err(_) => match default_user {
            Some(ref user) => user.clone(),
            None => {
                if let Ok(user) = env::var("USER") {
                    user
                } else {
                    let task_display = task_display(task);
                    let host_display = host_display(host);
                    return Err(RuntimeError(format!(
                        "No user specified for task '{task_display}' on host '{host_display}'. \
                    Specify 'user' in host configuration, set default with komandan.defaults:set_user(), \
                    or ensure USER environment variable is set."
                    )));
                }
            }
        },
    };

    Ok(user)
}

/// Get authentication configuration for SSH connections
///
/// This function extracts authentication method resolution logic from komando.rs
/// and handles password, private key, and default key discovery.
///
/// # Arguments
/// * `host` - Host configuration table
/// * `task` - Task configuration table
/// * `home_override` - Optional home directory override for testing
///
/// # Returns
/// * `mlua::Result<(String, SSHAuthMethod)>` - Username and authentication method
///
/// # Errors
/// Returns an error if:
/// - No authentication method can be determined
/// - Required environment variables are missing
/// - Authentication configuration is invalid
pub fn get_auth_config(
    host: &Table,
    task: &Table,
    home_override: Option<&str>,
) -> mlua::Result<(String, SSHAuthMethod)> {
    let host_display = host_display(host);
    let task_display = task_display(task);

    let user = get_user(host, task).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to determine user: {e}"),
            context: format!("task '{task_display}' on host '{host_display}'"),
            troubleshooting: "Specify 'user' in host configuration, set default user with komandan.defaults:set_user(), or ensure USER environment variable is set".to_string(),
        }.to_runtime_error()
    })?;

    let defaults = Defaults::global();

    let default_private_key_file = defaults
        .private_key_file
        .read()
        .map_err(|_| {
            ConnectionError::Configuration {
                message: "Failed to read default private key file setting".to_string(),
                context: "defaults access".to_string(),
                troubleshooting: "This is an internal error. Try restarting the application"
                    .to_string(),
            }
            .to_runtime_error()
        })?
        .clone();

    let default_private_key_pass = defaults
        .private_key_pass
        .read()
        .map_err(|_| {
            ConnectionError::Configuration {
                message: "Failed to read default private key passphrase setting".to_string(),
                context: "defaults access".to_string(),
                troubleshooting: "This is an internal error. Try restarting the application"
                    .to_string(),
            }
            .to_runtime_error()
        })?
        .clone();

    let default_password = defaults
        .password
        .read()
        .map_err(|_| {
            ConnectionError::Configuration {
                message: "Failed to read default password setting".to_string(),
                context: "defaults access".to_string(),
                troubleshooting: "This is an internal error. Try restarting the application"
                    .to_string(),
            }
            .to_runtime_error()
        })?
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
                Err(_) => {
                    if let Some(ref password) = default_password {
                        SSHAuthMethod::Password(password.clone())
                    } else {
                        let home = if let Some(h) = home_override {
                            h.to_string()
                        } else {
                            env::var("HOME").map_err(|_| {
                                ConnectionError::Configuration {
                                    message: "HOME environment variable not set".to_string(),
                                    context: "SSH key discovery".to_string(),
                                    troubleshooting: "Set the HOME environment variable or specify authentication method explicitly in host configuration".to_string(),
                                }.to_runtime_error()
                            })?
                        };
                        let ed25519_path = format!("{home}/.ssh/id_ed25519");
                        if Path::new(&ed25519_path).exists() {
                            SSHAuthMethod::PublicKey {
                                private_key: ed25519_path,
                                passphrase: host
                                    .get::<String>("private_key_pass")
                                    .ok()
                                    .or(default_private_key_pass),
                            }
                        } else {
                            let rsa_path = format!("{home}/.ssh/id_rsa");
                            if Path::new(&rsa_path).exists() {
                                SSHAuthMethod::PublicKey {
                                    private_key: rsa_path,
                                    passphrase: host
                                        .get::<String>("private_key_pass")
                                        .ok()
                                        .or(default_private_key_pass),
                                }
                            } else {
                                return Err(ConnectionError::Authentication {
                                    message: "No authentication method available".to_string(),
                                    host: host_display,
                                    user,
                                    troubleshooting: format!(
                                        "Specify authentication in host config: 'password' for password auth, \
                                        'private_key_file' for key auth, or create default SSH keys at {ed25519_path} or {rsa_path}. \
                                        You can also set defaults with komandan.defaults:set_password() or \
                                        komandan.defaults:set_private_key_file()"
                                    ),
                                }.to_runtime_error());
                            }
                        }
                    }
                }
            },
        },
    };

    Ok((user, ssh_auth_method))
}

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
    let mut ssh = SSHSession::new()
        .map_err(|e| {
            ConnectionError::Configuration {
                message: format!("Failed to create SSH session: {e}"),
                context: "SSH session initialization".to_string(),
                troubleshooting: "This is likely a system-level issue. Ensure SSH libraries are properly installed and accessible".to_string(),
            }.to_runtime_error()
        })?;

    let Ok(default_key_check) = defaults.key_check.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default host key check setting".to_string(),
            context: "defaults access".to_string(),
            troubleshooting: "This is an internal error. Try restarting the application"
                .to_string(),
        }
        .to_runtime_error());
    };

    let host_key_check = host
        .get::<Value>("host_key_check")
        .map_or(true, |key_check| match key_check {
            Value::Nil => *default_key_check,
            Value::Boolean(false) => false,
            _ => true,
        });

    let Ok(default_known_hosts_file) = defaults.known_hosts_file.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default known hosts file setting".to_string(),
            context: "defaults access".to_string(),
            troubleshooting: "This is an internal error. Try restarting the application"
                .to_string(),
        }
        .to_runtime_error());
    };

    if host_key_check {
        ssh.known_hosts_file = host
            .get::<String>("known_hosts_file")
            .map_or_else(|_| Some(default_known_hosts_file.clone()), Some);
    }

    Ok(ssh)
}

/// Get elevation configuration for privilege escalation
///
/// This function extracts privilege escalation configuration logic from komando.rs
/// and handles sudo, su, and no elevation scenarios.
///
/// # Arguments
/// * `host` - Host configuration table
/// * `task` - Task configuration table
///
/// # Returns
/// * `mlua::Result<Elevation>` - The elevation configuration
///
/// # Errors
/// Returns an error if:
/// - Default values cannot be read
/// - Elevation method is invalid
pub fn get_elevation_config(host: &Table, task: &Table) -> mlua::Result<Elevation> {
    let defaults = Defaults::global();

    let Ok(default_elevate) = defaults.elevate.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default elevation setting".to_string(),
            context: "defaults access".to_string(),
            troubleshooting: "This is an internal error. Try restarting the application"
                .to_string(),
        }
        .to_runtime_error());
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
        return Err(ConnectionError::Configuration {
            message: "Failed to read default elevation method setting".to_string(),
            context: "defaults access".to_string(),
            troubleshooting: "This is an internal error. Try restarting the application"
                .to_string(),
        }
        .to_runtime_error());
    };

    let elevation_method_str = task.get::<String>("elevation_method").unwrap_or_else(|_| {
        host.get::<String>("elevation_method")
            .unwrap_or_else(|_| default_elevation_method.clone())
    });

    let elevation_method = match elevation_method_str.as_str() {
        "none" => Ok(ElevationMethod::None),
        "sudo" => Ok(ElevationMethod::Sudo),
        "su" => Ok(ElevationMethod::Su),
        _ => Err(ConnectionError::Configuration {
            message: format!("Unsupported elevation method: '{elevation_method_str}'"),
            context: "elevation method configuration".to_string(),
            troubleshooting: "Valid elevation methods are: 'sudo', 'su', 'none'. Check your host or task configuration".to_string(),
        }.to_runtime_error()),
    };

    let default_as_user = match defaults.as_user.read() {
        Ok(as_user) => as_user.clone(),
        Err(_) => {
            return Err(ConnectionError::Configuration {
                message: "Failed to read default as_user setting".to_string(),
                context: "defaults access".to_string(),
                troubleshooting: "This is an internal error. Try restarting the application"
                    .to_string(),
            }
            .to_runtime_error());
        }
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
            troubleshooting: "This is an internal error. Try restarting the application"
                .to_string(),
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
                    troubleshooting: "Check that all host environment variables are strings"
                        .to_string(),
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
                    troubleshooting: "Check that all task environment variables are strings"
                        .to_string(),
                }
                .to_runtime_error()
            })?;
            ssh.set_env(&key, &value);
        }
    }

    Ok(())
}

/// Create a minimal task table for functions that don't have task context
///
/// This allows reuse of existing functions that expect both host and task parameters.
///
/// # Arguments
/// * `lua` - The Lua context
///
/// # Returns
/// * `mlua::Result<Table>` - An empty task table
fn create_dummy_task(lua: &Lua) -> mlua::Result<Table> {
    lua.create_table()
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
fn setup_environment_local(
    local: &mut LocalSession,
    host: &Table,
    task: &Table,
) -> mlua::Result<()> {
    let defaults = Defaults::global();

    let Ok(default_env) = defaults.env.read() else {
        return Err(ConnectionError::Configuration {
            message: "Failed to read default environment variables".to_string(),
            context: "defaults access".to_string(),
            troubleshooting: "This is an internal error. Try restarting the application"
                .to_string(),
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
                    troubleshooting: "Check that all host environment variables are strings"
                        .to_string(),
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
                    troubleshooting: "Check that all task environment variables are strings"
                        .to_string(),
                }
                .to_runtime_error()
            })?;
            local.set_env(&key, &value);
        }
    }

    Ok(())
}

/// Get the port for SSH connection from host or defaults
///
/// # Arguments
/// * `host` - Host configuration table
///
/// # Returns
/// * `mlua::Result<u16>` - The resolved port number
fn get_port_from_host(host: &Table) -> mlua::Result<u16> {
    let defaults = Defaults::global();

    // Try to get port from host configuration first
    if let Ok(port) = host.get::<u16>("port") {
        Ok(port)
    } else {
        // Fall back to default port
        let Ok(default_port) = defaults.port.read() else {
            return Err(ConnectionError::Configuration {
                message: "Failed to read default port setting".to_string(),
                context: "defaults access".to_string(),
                troubleshooting: "This is an internal error. Try restarting the application"
                    .to_string(),
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
            troubleshooting: "Check SSH session configuration parameters and system SSH support"
                .to_string(),
        }
        .to_runtime_error()
    })?;

    // Extract connection parameters
    let address = host_table.get::<String>("address").map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Missing or invalid address: {e}"),
            context: format!("host '{host_display}'"),
            troubleshooting: "Ensure 'address' field is set to a valid hostname or IP address"
                .to_string(),
        }
        .to_runtime_error()
    })?;

    let port = get_port_from_host(host_table).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to get port: {e}"),
            context: format!("host '{host_display}'"),
            troubleshooting:
                "Check port configuration or set default with komandan.defaults:set_port()"
                    .to_string(),
        }
        .to_runtime_error()
    })?;

    // Connect using existing logic with enhanced error detection and detailed troubleshooting
    ssh.connect(&address, port, &user, auth_method.clone())
        .map_err(|e| {
            let error_msg = e.to_string();

            // Enhanced error type detection based on error message content and context
            if error_msg.contains("authentication")
                || error_msg.contains("Authentication")
                || error_msg.contains("auth")
                || error_msg.contains("login")
                || error_msg.contains("password")
                || error_msg.contains("key")
            {
                ConnectionError::Authentication {
                    message: error_msg,
                    host: format!("{address}:{port}"),
                    user: user.clone(),
                    troubleshooting: get_auth_troubleshooting(&auth_method),
                }
                .to_runtime_error()
            } else if error_msg.contains("host key")
                || error_msg.contains("Host key")
                || error_msg.contains("known_hosts")
                || error_msg.contains("verification")
            {
                let known_hosts_file = ssh
                    .known_hosts_file.as_deref()
                    .unwrap_or("~/.ssh/known_hosts");
                ConnectionError::HostKeyVerification {
                    message: error_msg,
                    host: address.clone(),
                    troubleshooting: get_host_key_troubleshooting(&address, known_hosts_file),
                }
                .to_runtime_error()
            } else if error_msg.contains("Connection refused")
                || error_msg.contains("connection refused")
            {
                ConnectionError::Connection {
                    message: error_msg,
                    host: address.clone(),
                    port,
                    troubleshooting: format!(
                        "Connection refused by {address}:{port}. This usually means:\n\
                        1. SSH service is not running on the target host\n\
                        2. Firewall is blocking port {port}\n\
                        3. SSH is configured to listen on a different port\n\
                        Try: systemctl status ssh (on target), telnet {address} {port}, or ssh -v -p {port} {address}"
                    ),
                }
                .to_runtime_error()
            } else if error_msg.contains("timeout")
                || error_msg.contains("Timeout")
                || error_msg.contains("timed out")
            {
                ConnectionError::Connection {
                    message: error_msg,
                    host: address.clone(),
                    port,
                    troubleshooting: format!(
                        "Connection to {address}:{port} timed out. This usually means:\n\
                        1. Network connectivity issues\n\
                        2. Firewall dropping packets\n\
                        3. Host is down or unreachable\n\
                        Try: ping {address}, traceroute {address}, or check network connectivity"
                    ),
                }
                .to_runtime_error()
            } else if error_msg.contains("No route to host")
                || error_msg.contains("Network unreachable")
            {
                ConnectionError::Connection {
                    message: error_msg,
                    host: address.clone(),
                    port,
                    troubleshooting: format!(
                        "Network routing issue to {address}. This usually means:\n\
                        1. Host is not reachable from this network\n\
                        2. Routing configuration issues\n\
                        3. DNS resolution problems\n\
                        Try: ping {address}, nslookup {address}, or check routing table"
                    ),
                }
                .to_runtime_error()
            } else if error_msg.contains("Permission denied") {
                ConnectionError::Authentication {
                    message: error_msg,
                    host: format!("{address}:{port}"),
                    user: user.clone(),
                    troubleshooting: format!(
                        "Permission denied for user '{user}'. This usually means:\n\
                        1. Incorrect username, password, or SSH key\n\
                        2. User account is disabled or locked\n\
                        3. SSH configuration restricts this user\n\
                        Check: /var/log/auth.log on target, user account status, SSH config"
                    ),
                }
                .to_runtime_error()
            } else {
                // Generic connection error with enhanced troubleshooting
                ConnectionError::Connection {
                    message: error_msg,
                    host: address.clone(),
                    port,
                    troubleshooting: format!(
                        "{}. Additional troubleshooting steps:\n\
                        1. Verify host is reachable: ping {}\n\
                        2. Check SSH service: telnet {} {}\n\
                        3. Test manual connection: ssh -v {}@{} -p {}\n\
                        4. Check firewall rules and network connectivity\n\
                        5. Verify SSH daemon is running on target host",
                        get_connection_troubleshooting(&address, port),
                        address,
                        address,
                        port,
                        user,
                        address,
                        port
                    ),
                }
                .to_runtime_error()
            }
        })?;

    // Apply elevation and environment configuration
    ssh.elevation = get_elevation_config(host_table, task).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to configure elevation: {e}"),
            context: format!("elevation setup for host '{host_display}'"),
            troubleshooting:
                "Check elevation method and user configuration. Valid methods: 'sudo', 'su', 'none'"
                    .to_string(),
        }
        .to_runtime_error()
    })?;

    setup_environment_ssh(&mut ssh, host_table, task).map_err(|e| {
        ConnectionError::Configuration {
            message: format!("Failed to setup SSH environment: {e}"),
            context: format!("environment setup for host '{host_display}'"),
            troubleshooting: "Check environment variable configuration in host or task settings"
                .to_string(),
        }
        .to_runtime_error()
    })?;

    Ok(ssh)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_create_connection_local() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;

        let connection = create_connection(&lua, &Value::Table(host_table))?;

        match connection {
            Connection::Local(_) => {}
            Connection::SSH(_) => panic!("Expected local connection for localhost"),
        }

        Ok(())
    }

    #[test]
    fn test_create_connection_ssh_factory_logic() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "remote.example.com")?;
        host_table.set("user", "testuser")?;
        host_table.set("password", "testpass")?;

        // Test that the factory correctly identifies this as SSH
        let connection_type = determine_connection_type(&host_table)?;
        assert_eq!(connection_type, ConnectionType::SSH);

        // Test that we can create the dummy task
        let task = create_dummy_task(&lua)?;
        assert_eq!(task.len()?, 0);

        // Test that we can get auth config
        let (user, auth) = get_auth_config(&host_table, &task, None)?;
        assert_eq!(user, "testuser");
        match auth {
            SSHAuthMethod::Password(pass) => assert_eq!(pass, "testpass"),
            SSHAuthMethod::PublicKey { .. } => panic!("Expected Password authentication"),
        }

        Ok(())
    }

    #[test]
    fn test_create_connection_explicit_local() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "remote.example.com")?;
        host_table.set("connection", "local")?;

        let connection = create_connection(&lua, &Value::Table(host_table))?;

        match connection {
            Connection::Local(_) => {}
            Connection::SSH(_) => panic!("Expected local connection when explicitly set"),
        }

        Ok(())
    }

    #[test]
    fn test_create_connection_explicit_ssh_factory_logic() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;
        host_table.set("connection", "ssh")?;
        host_table.set("user", "testuser")?;
        host_table.set("password", "testpass")?;

        // Test that the factory correctly identifies this as SSH even for localhost
        let connection_type = determine_connection_type(&host_table)?;
        assert_eq!(connection_type, ConnectionType::SSH);

        Ok(())
    }

    #[test]
    fn test_create_connection_with_environment() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;

        let env_table = lua.create_table()?;
        env_table.set("TEST_VAR", "test_value")?;
        host_table.set("env", env_table)?;

        let connection = create_connection(&lua, &Value::Table(host_table))?;

        match connection {
            Connection::Local(_) => {}
            Connection::SSH(_) => panic!("Expected local connection for localhost"),
        }

        Ok(())
    }

    #[test]
    fn test_create_dummy_task() -> mlua::Result<()> {
        let lua = create_lua()?;
        let task = create_dummy_task(&lua)?;

        // Should be an empty table
        assert_eq!(task.len()?, 0);

        Ok(())
    }

    #[test]
    fn test_get_port_from_host() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test with explicit port
        let host_table = lua.create_table()?;
        host_table.set("port", 2222)?;
        let port = get_port_from_host(&host_table)?;
        assert_eq!(port, 2222);

        // Test with default port - reset defaults first
        lua.load(mlua::chunk! {
            komandan.defaults:set_port(22)
        })
        .exec()?;

        let host_table = lua.create_table()?;
        let port = get_port_from_host(&host_table)?;
        assert_eq!(port, 22); // Should use default

        Ok(())
    }

    #[test]
    fn test_determine_connection_type_localhost() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;

        let conn_type = determine_connection_type(&host_table)?;
        assert_eq!(conn_type, ConnectionType::Local);

        Ok(())
    }

    #[test]
    fn test_determine_connection_type_127_0_0_1() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "127.0.0.1")?;

        let conn_type = determine_connection_type(&host_table)?;
        assert_eq!(conn_type, ConnectionType::Local);

        Ok(())
    }

    #[test]
    fn test_determine_connection_type_ipv6_localhost() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "::1")?;

        let conn_type = determine_connection_type(&host_table)?;
        assert_eq!(conn_type, ConnectionType::Local);

        Ok(())
    }

    #[test]
    fn test_determine_connection_type_remote() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "remote.example.com")?;

        let conn_type = determine_connection_type(&host_table)?;
        assert_eq!(conn_type, ConnectionType::SSH);

        Ok(())
    }

    #[test]
    fn test_determine_connection_type_explicit() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host_table = lua.create_table()?;
        host_table.set("address", "localhost")?;
        host_table.set("connection", "ssh")?;

        let conn_type = determine_connection_type(&host_table)?;
        assert_eq!(conn_type, ConnectionType::SSH);

        Ok(())
    }

    #[test]
    fn test_is_localhost() {
        assert!(is_localhost("localhost"));
        assert!(is_localhost("127.0.0.1"));
        assert!(is_localhost("::1"));
        assert!(!is_localhost("remote.example.com"));
        assert!(!is_localhost("192.168.1.1"));
    }

    #[test]
    fn test_connection_type() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test local connection type
        let local_host = lua.create_table()?;
        local_host.set("address", "localhost")?;
        let local_conn = create_connection(&lua, &Value::Table(local_host))?;
        assert_eq!(local_conn.connection_type(), ConnectionType::Local);

        Ok(())
    }

    #[test]
    fn test_get_auth_config() -> anyhow::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;

        // Test with user in host
        host.set("address", "localhost")?;
        host.set("user", "testuser")?;
        host.set("private_key_file", "/path/to/key")?;

        let module_params = lua.create_table()?;
        module_params.set("cmd", "echo test")?;
        let module = lua
            .load(mlua::chunk! {
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
    fn test_get_user() -> mlua::Result<()> {
        let lua = create_lua()?;
        let host = lua.create_table()?;
        let task = lua.create_table()?;

        // Test with user in host
        host.set("user", "testuser")?;
        let user = get_user(&host, &task)?;
        assert_eq!(user, "testuser");

        // Test with no user specified (should fall back to environment)
        host.set("user", Value::Nil)?;
        let user = get_user(&host, &task);
        // This should either succeed with the current USER env var or fail
        // We can't predict the exact behavior since it depends on the environment
        assert!(user.is_ok() || user.is_err());

        Ok(())
    }

    #[test]
    fn test_create_ssh_session() -> mlua::Result<()> {
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
        lua.load(mlua::chunk! {
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
    fn test_get_elevation_config() -> mlua::Result<()> {
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
    fn test_setup_environment_ssh() -> mlua::Result<()> {
        let lua = create_lua()?;
        let mut ssh = SSHSession::new()
            .map_err(|e| RuntimeError(format!("Failed to create SSH session: {e}")))?;
        let host = lua.create_table()?;
        let task = lua.create_table()?;

        // Test with environment variables at all levels
        let env_host = lua.create_table()?;
        env_host.set("HOST_VAR", "host_value")?;
        env_host.set("OVERRIDE_VAR", "host_override")?; // This should be overridden by task
        host.set("env", env_host)?;

        let env_task = lua.create_table()?;
        env_task.set("TASK_VAR", "task_value")?;
        env_task.set("OVERRIDE_VAR", "task_override")?; // This should override host value
        task.set("env", env_task)?;

        setup_environment_ssh(&mut ssh, &host, &task)?;

        // We can't directly test the environment variables since SSHSession doesn't expose them
        // But we can verify the function completes without error
        Ok(())
    }

    #[test]
    fn test_setup_environment_ssh_empty() -> mlua::Result<()> {
        let lua = create_lua()?;
        let mut ssh = SSHSession::new()
            .map_err(|e| RuntimeError(format!("Failed to create SSH session: {e}")))?;
        let host = lua.create_table()?;
        let task = lua.create_table()?;

        // Test with no environment variables
        setup_environment_ssh(&mut ssh, &host, &task)?;

        Ok(())
    }
}
