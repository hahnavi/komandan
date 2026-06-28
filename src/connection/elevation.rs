use crate::connection::ConnectionError;
use crate::defaults::Defaults;
use crate::ssh::{Elevation, ElevationMethod};
use mlua::{Table, Value};

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
        }
        .to_runtime_error());
    };

    let task_elevate = task.get::<Value>("elevate")?;
    let host_elevate = host.get::<Value>("elevate")?;

    // Resolve elevate: only Value::Nil falls back to the next source. Any
    // other non-boolean value is a configuration error.
    let elevate = if !task_elevate.is_nil() {
        match task_elevate {
            Value::Boolean(b) => b,
            other => {
                return Err(ConnectionError::Configuration {
                    message: format!(
                        "task 'elevate' must be a boolean, got {}",
                        other.type_name()
                    ),
                    context: "elevation configuration".to_string(),
                }
                .to_runtime_error());
            }
        }
    } else if !host_elevate.is_nil() {
        match host_elevate {
            Value::Boolean(b) => b,
            other => {
                return Err(ConnectionError::Configuration {
                    message: format!(
                        "host 'elevate' must be a boolean, got {}",
                        other.type_name()
                    ),
                    context: "elevation configuration".to_string(),
                }
                .to_runtime_error());
            }
        }
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
        }
        .to_runtime_error());
    };

    // Read elevation_method as Option<String> so present-but-wrong-type values
    // surface as errors instead of silently falling back to host/default.
    let elevation_method_str = match task.get::<Option<String>>("elevation_method")? {
        Some(s) => s,
        None => host
            .get::<Option<String>>("elevation_method")?
            .unwrap_or_else(|| default_elevation_method.clone()),
    };

    let elevation_method = match elevation_method_str.as_str() {
        "none" => Ok(ElevationMethod::None),
        "sudo" => Ok(ElevationMethod::Sudo),
        "su" => Ok(ElevationMethod::Su),
        _ => Err(ConnectionError::Configuration {
            message: format!("Unsupported elevation method: '{elevation_method_str}'"),
            context: "elevation method configuration".to_string(),
        }
        .to_runtime_error()),
    };

    let default_as_user = match defaults.as_user.read() {
        Ok(as_user) => as_user.clone(),
        Err(_) => {
            return Err(ConnectionError::Configuration {
                message: "Failed to read default as_user setting".to_string(),
                context: "defaults access".to_string(),
            }
            .to_runtime_error());
        }
    };

    // Read as_user as Option<String> from each layer in turn; wrong types error.
    let as_user = match task.get::<Option<String>>("as_user")? {
        Some(user) => Some(user),
        None => host
            .get::<Option<String>>("as_user")?
            .map_or(default_as_user, Some),
    };

    Ok(Elevation {
        method: elevation_method?,
        as_user,
    })
}
