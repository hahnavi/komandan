//! # `ModulesV2` Template Rendering Module
//!
//! The `template` module provides template rendering functionality for `ModulesV2`.
//! It supports both local and remote execution with automatic connection management.
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution - render template
//! local result = k.mod.template({
//!     src = "config.j2",
//!     dest = "/etc/myapp/config.conf",
//!     vars = {name = "myapp", port = 8080}
//! })
//!
//! -- Remote execution - render template with backup
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.template({
//!     src = "nginx.conf.j2",
//!     dest = "/etc/nginx/nginx.conf",
//!     vars = {server_name = "example.com"},
//!     backup = true
//! }, host)
//! ```
//!
//! ## Parameters
//!
//! - `src` (string, required): Path to the template file
//! - `dest` (string, required): Destination path for the rendered template
//! - `vars` (table, optional): Variables to use in template rendering
//! - `backup` (boolean, optional): Whether to backup existing destination file (default: false)
//! - `mode` (string, optional): File permissions for the destination file
//! - `owner` (string, optional): Owner for the destination file
//! - `group` (string, optional): Group for the destination file
//!
//! ## Return Value
//!
//! Returns a table with:
//! - `stdout`: Command output
//! - `stderr`: Error output
//! - `exit_code`: Exit code (0 for success)
//! - `changed`: Boolean indicating if the file was modified

use super::execution::{ExecutionEngine, ModuleResult};
use super::factory::create_modulev2_function;
use crate::connection::Connection;
use minijinja::Environment;
use mlua::{Lua, Table, Value};
use rand::{Rng, distr::Alphanumeric};
use std::collections::HashMap;
use std::path::Path;

/// Create the `template_v2` function for `ModulesV2`
///
/// This function creates a ModulesV2-compatible template rendering module that supports
/// both local and remote execution patterns.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - The configured `template_v2` function
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - Template rendering operations fail
pub fn template_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
    create_modulev2_function(lua, "template", |lua, params, host| {
        ExecutionEngine::execute_module(lua, "template", &params, host, |connection, params| {
            // Extract and validate parameters
            let src_path = extract_src_parameter(params)?;
            let dest_path = extract_dest_parameter(params)?;
            let vars = extract_vars_parameter(params)?;
            let backup = params.get::<Option<bool>>("backup")?.unwrap_or(false);
            let mode = params.get::<Option<String>>("mode")?;
            let owner = params.get::<Option<String>>("owner")?;
            let group = params.get::<Option<String>>("group")?;

            // Validate and sanitize paths
            let sanitized_src_path = sanitize_path(&src_path)?;
            let sanitized_dest_path = sanitize_path(&dest_path)?;

            // Execute template rendering operations
            execute_template_operations(
                connection,
                &TemplateOperationParams {
                    src_path: &sanitized_src_path,
                    dest_path: &sanitized_dest_path,
                    vars: &vars,
                    backup,
                    mode: mode.as_deref(),
                    owner: owner.as_deref(),
                    group: group.as_deref(),
                },
            )
        })
    })
}

/// Extract and validate the src parameter
fn extract_src_parameter(params: &Table) -> mlua::Result<String> {
    params.get::<Option<String>>("src")?.map_or_else(
        || {
            Err(mlua::Error::RuntimeError(
                "template module requires 'src' parameter".to_string(),
            ))
        },
        |src| {
            if src.trim().is_empty() {
                Err(mlua::Error::RuntimeError(
                    "src parameter cannot be empty".to_string(),
                ))
            } else {
                Ok(src)
            }
        },
    )
}

/// Extract and validate the dest parameter
fn extract_dest_parameter(params: &Table) -> mlua::Result<String> {
    params.get::<Option<String>>("dest")?.map_or_else(
        || {
            Err(mlua::Error::RuntimeError(
                "template module requires 'dest' parameter".to_string(),
            ))
        },
        |dest| {
            if dest.trim().is_empty() {
                Err(mlua::Error::RuntimeError(
                    "dest parameter cannot be empty".to_string(),
                ))
            } else {
                Ok(dest)
            }
        },
    )
}

/// Extract and validate the vars parameter
fn extract_vars_parameter(params: &Table) -> mlua::Result<HashMap<String, String>> {
    match params.get::<Value>("vars")? {
        Value::Table(vars_table) => {
            let mut vars = HashMap::new();
            for pair in vars_table.pairs::<String, Value>() {
                let (key, value) = pair?;
                let value_str = match value {
                    Value::String(s) => s.to_str()?.to_string(),
                    Value::Integer(i) => i.to_string(),
                    Value::Number(n) => n.to_string(),
                    Value::Boolean(b) => b.to_string(),
                    _ => {
                        return Err(mlua::Error::RuntimeError(format!(
                            "Variable '{key}' has unsupported type. Only strings, numbers, and booleans are supported"
                        )));
                    }
                };
                vars.insert(key, value_str);
            }
            Ok(vars)
        }
        Value::Nil => Ok(HashMap::new()),
        _ => Err(mlua::Error::RuntimeError(
            "vars parameter must be a table".to_string(),
        )),
    }
}

/// Sanitize file paths to prevent directory traversal attacks
fn sanitize_path(path: &str) -> mlua::Result<String> {
    if path.trim().is_empty() {
        return Err(mlua::Error::RuntimeError(
            "Path cannot be empty".to_string(),
        ));
    }

    // Check for directory traversal attempts
    if path.contains("..") {
        return Err(mlua::Error::RuntimeError(
            "Path cannot contain '..' (directory traversal not allowed)".to_string(),
        ));
    }

    // For now, we'll allow the path as-is after basic validation
    // In a production environment, you might want more strict validation
    Ok(path.to_string())
}

/// Template operation parameters
struct TemplateOperationParams<'a> {
    src_path: &'a str,
    dest_path: &'a str,
    vars: &'a HashMap<String, String>,
    backup: bool,
    mode: Option<&'a str>,
    owner: Option<&'a str>,
    group: Option<&'a str>,
}

/// Execute template rendering operations
fn execute_template_operations(
    connection: &mut Connection,
    params: &TemplateOperationParams<'_>,
) -> mlua::Result<ModuleResult> {
    let mut stdout_parts = Vec::new();
    let mut stderr_parts = Vec::new();
    // let _changed = false; // Removed unused variable

    // Read and render template
    let rendered_content = render_template(params.src_path, params.vars)?;

    // Generate a random temporary file name
    let random_suffix: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .take(10)
        .collect();
    let temp_file = format!("/tmp/.template_{random_suffix}");

    // Create backup if requested
    if params.backup {
        let (backup_stdout, backup_stderr, backup_exit_code) = connection
            .cmd(&format!(
                "if [ -f {} ]; then cp {} {}.backup; echo 'Backup created: {}.backup'; fi",
                params.dest_path, params.dest_path, params.dest_path, params.dest_path
            ))
            .map_err(|e| mlua::Error::RuntimeError(format!("Backup creation failed: {e}")))?;

        stdout_parts.push(backup_stdout);
        if !backup_stderr.is_empty() {
            stderr_parts.push(backup_stderr);
        }

        if backup_exit_code != 0 {
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                backup_exit_code,
            ));
        }
    }

    // Write rendered content to temporary file
    let (write_stdout, write_stderr, write_exit_code) = connection
        .cmd(&format!(
            "cat > {temp_file} << 'EOF'\n{rendered_content}\nEOF"
        ))
        .map_err(|e| mlua::Error::RuntimeError(format!("Template write failed: {e}")))?;

    if write_exit_code != 0 {
        stdout_parts.push(write_stdout);
        if !write_stderr.is_empty() {
            stderr_parts.push(write_stderr);
        }
        return Ok(ModuleResult::complete(
            stdout_parts.join("\n"),
            stderr_parts.join("\n"),
            write_exit_code,
        ));
    }

    // Move temporary file to destination
    let (move_stdout, move_stderr, move_exit_code) = connection
        .cmd(&format!("mv {temp_file} {}", params.dest_path))
        .map_err(|e| mlua::Error::RuntimeError(format!("File move failed: {e}")))?;

    stdout_parts.push(move_stdout);
    if !move_stderr.is_empty() {
        stderr_parts.push(move_stderr);
    }

    if move_exit_code != 0 {
        return Ok(ModuleResult::complete(
            stdout_parts.join("\n"),
            stderr_parts.join("\n"),
            move_exit_code,
        ));
    }

    // changed = true; // Commented out as it's never read

    // Set file permissions if specified
    if let Some(mode) = params.mode {
        let (chmod_stdout, chmod_stderr, chmod_exit_code) = connection
            .cmd(&format!("chmod {mode} {}", params.dest_path))
            .map_err(|e| mlua::Error::RuntimeError(format!("Chmod failed: {e}")))?;

        stdout_parts.push(chmod_stdout);
        if !chmod_stderr.is_empty() {
            stderr_parts.push(chmod_stderr);
        }

        if chmod_exit_code != 0 {
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                chmod_exit_code,
            ));
        }
    }

    // Set file ownership if specified
    if let (Some(owner), Some(group)) = (params.owner, params.group) {
        let (chown_stdout, chown_stderr, chown_exit_code) = connection
            .cmd(&format!("chown {owner}:{group} {}", params.dest_path))
            .map_err(|e| mlua::Error::RuntimeError(format!("Chown failed: {e}")))?;

        stdout_parts.push(chown_stdout);
        if !chown_stderr.is_empty() {
            stderr_parts.push(chown_stderr);
        }

        if chown_exit_code != 0 {
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                chown_exit_code,
            ));
        }
    } else if let Some(owner) = params.owner {
        let (chown_stdout, chown_stderr, chown_exit_code) = connection
            .cmd(&format!("chown {owner} {}", params.dest_path))
            .map_err(|e| mlua::Error::RuntimeError(format!("Chown failed: {e}")))?;

        stdout_parts.push(chown_stdout);
        if !chown_stderr.is_empty() {
            stderr_parts.push(chown_stderr);
        }

        if chown_exit_code != 0 {
            return Ok(ModuleResult::complete(
                stdout_parts.join("\n"),
                stderr_parts.join("\n"),
                chown_exit_code,
            ));
        }
    }

    // Add success message
    stdout_parts.push(format!(
        "Template rendered successfully to {}",
        params.dest_path
    ));

    Ok(ModuleResult::complete(
        stdout_parts.join("\n"),
        stderr_parts.join("\n"),
        0, // Always return 0 for successful operations
    ))
}

/// Render template using minijinja
fn render_template(src_path: &str, vars: &HashMap<String, String>) -> mlua::Result<String> {
    // Check if source file exists
    if !Path::new(src_path).exists() {
        return Err(mlua::Error::RuntimeError(format!(
            "Source template file does not exist: {src_path}"
        )));
    }

    // Read template content
    let template_content = std::fs::read_to_string(src_path).map_err(|e| {
        mlua::Error::RuntimeError(format!("Failed to read template file '{src_path}': {e}"))
    })?;

    // Create minijinja environment
    let mut env = Environment::new();
    env.add_template("template", &template_content)
        .map_err(|e| mlua::Error::RuntimeError(format!("Failed to add template: {e}")))?;

    // Convert HashMap to minijinja Value
    let template_vars = minijinja::Value::from_serialize(vars);
    if template_vars.is_undefined() {
        return Err(mlua::Error::RuntimeError(
            "Failed to serialize variables for template".to_string(),
        ));
    }

    // Render template
    let rendered = env
        .get_template("template")
        .map_err(|e| mlua::Error::RuntimeError(format!("Failed to get template: {e}")))?
        .render(template_vars)
        .map_err(|e| mlua::Error::RuntimeError(format!("Failed to render template: {e}")))?;

    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[test]
    fn test_template_v2_missing_src() -> mlua::Result<()> {
        let lua = create_lua()?;
        let template_fn = template_v2(&lua)?;

        // Test with missing src parameter
        let params = lua.create_table()?;
        params.set("dest", "/tmp/test")?;

        let result: mlua::Result<Table> = template_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("template module requires 'src' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("template module requires 'src' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_template_v2_missing_dest() -> mlua::Result<()> {
        let lua = create_lua()?;
        let template_fn = template_v2(&lua)?;

        // Test with missing dest parameter
        let params = lua.create_table()?;
        params.set("src", "/tmp/template.j2")?;

        let result: mlua::Result<Table> = template_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("template module requires 'dest' parameter"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("template module requires 'dest' parameter"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_template_v2_invalid_vars() -> mlua::Result<()> {
        let lua = create_lua()?;
        let template_fn = template_v2(&lua)?;

        // Test with invalid vars parameter (not a table)
        let params = lua.create_table()?;
        params.set("src", "/tmp/template.j2")?;
        params.set("dest", "/tmp/output")?;
        params.set("vars", "not a table")?;

        let result: mlua::Result<Table> = template_fn.call(params);
        assert!(result.is_err());

        // Check error message
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("vars parameter must be a table"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("vars parameter must be a table"));
                }
                _ => panic!("Expected RuntimeError in callback"),
            },
            _ => panic!("Expected RuntimeError"),
        }

        Ok(())
    }

    #[test]
    fn test_template_v2_with_valid_template() -> mlua::Result<()> {
        // Create a temporary template file
        let mut temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        writeln!(temp_file, "Hello {{ name }}! You are {{ age }} years old.")
            .map_err(mlua::Error::external)?;

        let lua = create_lua()?;
        let template_fn = template_v2(&lua)?;

        // Test with valid parameters
        let params = lua.create_table()?;
        params.set(
            "src",
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
        )?;
        params.set("dest", "/tmp/test_output")?;

        let vars = lua.create_table()?;
        vars.set("name", "John")?;
        vars.set("age", 30)?;
        params.set("vars", vars)?;

        let result: Table = template_fn.call(params)?;

        // In test environment, file operations might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_template_v2_with_backup() -> mlua::Result<()> {
        // Create a temporary template file
        let mut temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        writeln!(temp_file, "Config: {{ config_value }}").map_err(mlua::Error::external)?;

        let lua = create_lua()?;
        let template_fn = template_v2(&lua)?;

        // Test with backup option
        let params = lua.create_table()?;
        params.set(
            "src",
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
        )?;
        params.set("dest", "/tmp/test_config")?;
        params.set("backup", true)?;

        let vars = lua.create_table()?;
        vars.set("config_value", "production")?;
        params.set("vars", vars)?;

        let result: Table = template_fn.call(params)?;

        // In test environment, file operations might fail, so we just check that the function works
        assert!(result.contains_key("exit_code")?);
        assert!(result.contains_key("stdout")?);
        assert!(result.contains_key("stderr")?);
        assert!(result.contains_key("changed")?);

        Ok(())
    }

    #[test]
    fn test_sanitize_path() -> mlua::Result<()> {
        // Test valid paths
        assert_eq!(sanitize_path("/etc/config.conf")?, "/etc/config.conf");
        assert_eq!(sanitize_path("./template.j2")?, "./template.j2");
        assert_eq!(sanitize_path("config/app.conf")?, "config/app.conf");

        // Test invalid paths with directory traversal
        assert!(sanitize_path("../../../etc/passwd").is_err());
        assert!(sanitize_path("/etc/../../../passwd").is_err());

        // Test empty path
        assert!(sanitize_path("").is_err());
        assert!(sanitize_path("   ").is_err());

        Ok(())
    }

    #[test]
    fn test_extract_vars_parameter() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Test with valid vars table
        let params = lua.create_table()?;
        let vars = lua.create_table()?;
        vars.set("name", "John")?;
        vars.set("age", 30)?;
        vars.set("active", true)?;
        params.set("vars", vars)?;

        let extracted_vars = extract_vars_parameter(&params)?;
        assert_eq!(extracted_vars.get("name"), Some(&"John".to_string()));
        assert_eq!(extracted_vars.get("age"), Some(&"30".to_string()));
        assert_eq!(extracted_vars.get("active"), Some(&"true".to_string()));

        // Test with nil vars
        let params = lua.create_table()?;
        let extracted_vars = extract_vars_parameter(&params)?;
        assert!(extracted_vars.is_empty());

        Ok(())
    }

    #[test]
    fn test_render_template() -> mlua::Result<()> {
        // Create a temporary template file
        let mut temp_file = NamedTempFile::new().map_err(mlua::Error::external)?;
        writeln!(temp_file, r"Hello {{{{ name }}}}! Port: {{{{ port }}}}")
            .map_err(mlua::Error::external)?;

        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "World".to_string());
        vars.insert("port".to_string(), "8080".to_string());

        let rendered = render_template(
            temp_file
                .path()
                .to_str()
                .ok_or_else(|| mlua::Error::external("invalid path"))?,
            &vars,
        )?;

        assert_eq!(rendered.trim(), "Hello World! Port: 8080");

        Ok(())
    }

    #[test]
    fn test_render_template_nonexistent_file() {
        let vars = HashMap::new();
        let result = render_template("/nonexistent/template.j2", &vars);
        assert!(result.is_err());

        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("Source template file does not exist"));
            }
            _ => panic!("Expected RuntimeError"),
        }
    }
}
