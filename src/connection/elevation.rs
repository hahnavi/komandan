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

    let as_user = task.get::<Option<String>>("as_user").unwrap_or_else(|_| {
        host.get::<Option<String>>("as_user")
            .unwrap_or(default_as_user)
    });

    Ok(Elevation {
        method: elevation_method?,
        as_user,
    })
}
