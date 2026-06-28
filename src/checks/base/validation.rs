//! Common parameter validation functions

use anyhow::{Context, Result};
use mlua::Table;

/// Validate that a required string parameter exists and is not empty
pub fn validate_required_string(params: &Table, param_name: &str) -> Result<String> {
    let value = params
        .get::<String>(param_name)
        .with_context(|| format!("Parameter '{param_name}' is required"))?;

    if value.trim().is_empty() {
        anyhow::bail!("Parameter '{param_name}' cannot be empty");
    }

    Ok(value)
}

/// Validate an optional string parameter
pub fn validate_optional_string(params: &Table, param_name: &str) -> Result<Option<String>> {
    match params.get::<Option<String>>(param_name)? {
        Some(value) if value.trim().is_empty() => Ok(None),
        other => Ok(other),
    }
}

/// Validate an optional boolean parameter
pub fn validate_optional_bool(params: &Table, param_name: &str) -> Result<Option<bool>> {
    Ok(params.get::<Option<bool>>(param_name)?)
}

/// Validate file mode format (4-digit octal)
pub fn validate_file_mode(mode: &str) -> Result<()> {
    if mode.len() != 4 || !mode.starts_with('0') {
        anyhow::bail!("File mode must be a 4-digit octal number (e.g., '0644')");
    }

    for c in mode.chars().skip(1) {
        if !('0'..='7').contains(&c) {
            anyhow::bail!("File mode must contain only octal digits (0-7)");
        }
    }

    Ok(())
}
