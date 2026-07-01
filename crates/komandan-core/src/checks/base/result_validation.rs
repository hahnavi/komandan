//! Result structure validation functions

use super::CheckResult;
use anyhow::Result;
use std::collections::HashMap;
use std::collections::HashSet;

/// Standard field names that should be used consistently across all check modules
pub struct StandardFields;

impl StandardFields {
    /// Fields that indicate existence or presence
    pub const EXISTS: &'static str = "exists";
    pub const INSTALLED: &'static str = "installed";

    /// Fields for file properties
    pub const MODE: &'static str = "mode";
    pub const OWNER: &'static str = "owner";
    pub const GROUP: &'static str = "group";

    /// Fields for service properties
    pub const STATE: &'static str = "state";
    pub const ENABLED: &'static str = "enabled";

    /// Fields for package properties
    pub const VERSION: &'static str = "version";

    /// Fields for command output
    pub const STDOUT: &'static str = "stdout";
    pub const STDERR: &'static str = "stderr";
    pub const EXIT_CODE: &'static str = "exit_code";
}

/// Validate that a `CheckResult` has the required structure
pub fn validate_check_result_structure(result: &CheckResult) -> Result<()> {
    // Validate that ok field is present (it's always present in our struct)
    // This is guaranteed by the type system

    // Validate that actual field is present (it's always present in our struct)
    // This is guaranteed by the type system

    // Validate that if ok is false and there's no actual data, there should be an error
    if !result.ok && result.actual.is_empty() && result.error.is_none() {
        anyhow::bail!("CheckResult with ok=false must have either actual data or an error message");
    }

    Ok(())
}

/// Validate that field names in actual data follow standard conventions
pub fn validate_field_naming(actual: &HashMap<String, String>, module_type: &str) -> Result<()> {
    let valid_fields = get_valid_fields_for_module(module_type);

    for field_name in actual.keys() {
        if !valid_fields.contains(field_name.as_str()) {
            anyhow::bail!(
                "Invalid field name '{field_name}' for {module_type} module. Valid fields: {valid_fields:?}"
            );
        }
    }

    Ok(())
}

/// Get valid field names for a specific module type
fn get_valid_fields_for_module(module_type: &str) -> HashSet<&'static str> {
    let mut fields = HashSet::new();

    // Common fields for all modules
    fields.insert(StandardFields::STDOUT);
    fields.insert(StandardFields::STDERR);
    fields.insert(StandardFields::EXIT_CODE);

    match module_type {
        "file" => {
            fields.insert(StandardFields::EXISTS);
            fields.insert(StandardFields::MODE);
            fields.insert(StandardFields::OWNER);
            fields.insert(StandardFields::GROUP);
        }
        "service" => {
            fields.insert(StandardFields::EXISTS);
            fields.insert(StandardFields::STATE);
            fields.insert(StandardFields::ENABLED);
        }
        "package" => {
            fields.insert(StandardFields::INSTALLED);
            fields.insert(StandardFields::VERSION);
        }
        _ => {
            // For unknown modules, allow any field names
            // This provides flexibility for future modules
            return HashSet::new();
        }
    }

    fields
}

/// Create a validated `CheckResult` with consistent field naming
pub fn create_validated_result(
    ok: bool,
    actual: &HashMap<String, String>,
    error: Option<String>,
    module_type: &str,
) -> Result<CheckResult> {
    let result = error.map_or_else(
        || {
            if ok {
                CheckResult::success(actual.clone())
            } else {
                CheckResult::failure(actual.clone())
            }
        },
        CheckResult::error,
    );
    // Validate the result structure
    validate_check_result_structure(&result)?;

    // Validate field naming if we have actual data
    if !actual.is_empty() {
        validate_field_naming(actual, module_type)?;
    }

    Ok(result)
}

/// Helper to ensure consistent boolean field representation
pub fn format_boolean_field(value: bool) -> String {
    if value {
        "true".to_string()
    } else {
        "false".to_string()
    }
}

/// Helper to ensure consistent existence field representation
pub fn set_exists_field(actual: &mut HashMap<String, String>, exists: bool) {
    actual.insert(
        StandardFields::EXISTS.to_string(),
        format_boolean_field(exists),
    );
}

/// Helper to ensure consistent installed field representation
pub fn set_installed_field(actual: &mut HashMap<String, String>, installed: bool) {
    actual.insert(
        StandardFields::INSTALLED.to_string(),
        format_boolean_field(installed),
    );
}

/// Helper to ensure consistent enabled field representation
pub fn set_enabled_field(actual: &mut HashMap<String, String>, enabled: Option<bool>) {
    let value = match enabled {
        Some(true) => "true".to_string(),
        Some(false) => "false".to_string(),
        None => "unknown".to_string(),
    };
    actual.insert(StandardFields::ENABLED.to_string(), value);
}

/// Helper to set version field with proper handling of None values
pub fn set_version_field(actual: &mut HashMap<String, String>, version: Option<String>) {
    let value = version.unwrap_or_else(|| "unknown".to_string());
    actual.insert(StandardFields::VERSION.to_string(), value);
}
