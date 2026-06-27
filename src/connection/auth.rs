use crate::connection::ConnectionError;
use crate::defaults::Defaults;
use crate::ssh::SSHAuthMethod;
use crate::util::{host_display, task_display};
use mlua::{Error::RuntimeError, Table};
use secrecy::{ExposeSecret, SecretString};
use std::env;
use std::path::Path;

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
pub fn get_user(host: &Table, task: &Table) -> mlua::Result<String> {
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
        }
        .to_runtime_error()
    })?;

    let defaults = Defaults::global();

    let default_private_key_file = defaults
        .private_key_file
        .read()
        .map_err(|_| {
            ConnectionError::Configuration {
                message: "Failed to read default private key file setting".to_string(),
                context: "defaults access".to_string(),
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
            }
            .to_runtime_error()
        })?
        .as_ref()
        .map(|s: &SecretString| s.expose_secret().to_string());

    let default_password = defaults
        .password
        .read()
        .map_err(|_| {
            ConnectionError::Configuration {
                message: "Failed to read default password setting".to_string(),
                context: "defaults access".to_string(),
            }
            .to_runtime_error()
        })?
        .as_ref()
        .map(|s: &SecretString| s.expose_secret().to_string());

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
                        // Check if SSH key auto-discovery is enabled
                        if !*defaults.ssh_auto_discover_keys.read().map_err(|_| {
                            ConnectionError::Configuration {
                                message: "Failed to read ssh_auto_discover_keys setting"
                                    .to_string(),
                                context: "defaults access".to_string(),
                            }
                            .to_runtime_error()
                        })? {
                            return Err(ConnectionError::Authentication {
                                message: "No authentication method available and SSH key auto-discovery is disabled".to_string(),
                                host: host_display,
                                user,
                            }.to_runtime_error());
                        }

                        let home = if let Some(h) = home_override {
                            h.to_string()
                        } else {
                            env::var("HOME").map_err(|_| {
                                ConnectionError::Configuration {
                                    message: "HOME environment variable not set".to_string(),
                                    context: "SSH key discovery".to_string(),
                                }
                                .to_runtime_error()
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
                                }
                                .to_runtime_error());
                            }
                        }
                    }
                }
            },
        },
    };

    Ok((user, ssh_auth_method))
}
