//! # `ModulesV2` Factory System
//!
//! The factory system creates Lua functions that support both single-parameter (local)
//! and dual-parameter (remote) calling patterns. This enables the simplified `ModulesV2`
//! syntax while maintaining full backward compatibility.
//!
//! ## Dual-Signature Pattern
//!
//! `ModulesV2` functions support two calling patterns:
//! - `k.mod.cmd({cmd = "echo test"})` - Local execution (1 parameter)
//! - `k.mod.cmd({cmd = "echo test"}, host)` - Remote execution (2 parameters)
//!
//! ## Implementation
//!
//! The factory creates Lua functions that inspect the number of arguments and
//! route execution to the appropriate handler based on whether a host parameter
//! is provided.

use mlua::{Lua, Table, Value, Variadic};

/// Create a `ModulesV2` function with dual-signature support
///
/// This factory function creates Lua functions that can be called with either
/// one parameter (for local execution) or two parameters (for remote execution).
/// The function automatically determines the execution context and routes to
/// the appropriate handler.
///
/// # Arguments
/// * `lua` - The Lua context for creating the function
/// * `module_name` - Name of the module (for error messages)
/// * `executor` - The execution function that handles the actual module logic
///
/// # Returns
/// * `mlua::Result<mlua::Function>` - A Lua function with dual-signature support
///
/// # Errors
/// Returns an error if:
/// - Function creation fails
/// - Parameter validation fails
/// - Executor function fails
///
/// # Example
/// ```rust,no_run
/// use mlua::{Lua, Table};
/// use komandan::modulesv2::create_modulev2_function;
///
/// fn cmd_v2(lua: &Lua) -> mlua::Result<mlua::Function> {
///     create_modulev2_function(lua, "cmd", |lua: &Lua, params: Table, host: Option<Table>| {
///         // Module implementation here
///         let result_table = lua.create_table()?;
///         result_table.set("stdout", "test output")?;
///         result_table.set("stderr", "")?;
///         result_table.set("exit_code", 0)?;
///         Ok(result_table)
///     })
/// }
/// ```
pub fn create_modulev2_function<F>(
    lua: &Lua,
    module_name: &str,
    executor: F,
) -> mlua::Result<mlua::Function>
where
    F: Fn(&Lua, Table, Option<Table>) -> mlua::Result<Table> + 'static,
{
    let module_name = module_name.to_string();
    lua.create_function(move |lua, args: Variadic<Value>| {
        match args.len() {
            1 => {
                // Local execution: k.mod.cmd({params})
                let params = args[0].as_table().ok_or_else(|| {
                    mlua::Error::RuntimeError(format!(
                        "First parameter to {module_name} must be a table"
                    ))
                })?;
                executor(lua, params.clone(), None)
            }
            2 => {
                // Remote execution: k.mod.cmd({params}, host)
                let params = args[0].as_table().ok_or_else(|| {
                    mlua::Error::RuntimeError(format!(
                        "First parameter to {module_name} must be a table"
                    ))
                })?;
                let host = args[1].as_table().ok_or_else(|| {
                    mlua::Error::RuntimeError(format!(
                        "Second parameter to {module_name} must be a host table"
                    ))
                })?;
                executor(lua, params.clone(), Some(host.clone()))
            }
            _ => Err(mlua::Error::RuntimeError(format!(
                "{module_name} expects 1 or 2 parameters"
            ))),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_create_modulev2_function_local_execution() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Create a test module function
        let test_module = create_modulev2_function(&lua, "test", |lua, params, host| {
            let result = lua.create_table()?;
            result.set("params_received", true)?;
            result.set("host_provided", host.is_some())?;

            // Echo back a parameter to verify it was received
            if let Ok(test_param) = params.get::<String>("test_param") {
                result.set("test_param", test_param)?;
            }

            Ok(result)
        })?;

        // Test local execution (1 parameter)
        let params = lua.create_table()?;
        params.set("test_param", "local_test")?;

        let result: Table = test_module.call(params)?;

        assert!(result.get::<bool>("params_received")?);
        assert!(!result.get::<bool>("host_provided")?);
        assert_eq!(result.get::<String>("test_param")?, "local_test");

        Ok(())
    }

    #[test]
    fn test_create_modulev2_function_remote_execution() -> mlua::Result<()> {
        let lua = create_lua()?;

        // Create a test module function
        let test_module = create_modulev2_function(&lua, "test", |lua, params, host| {
            let result = lua.create_table()?;
            result.set("params_received", true)?;
            result.set("host_provided", host.is_some())?;

            // Echo back parameters to verify they were received
            if let Ok(test_param) = params.get::<String>("test_param") {
                result.set("test_param", test_param)?;
            }

            if let Some(host_table) = host
                && let Ok(address) = host_table.get::<String>("address")
            {
                result.set("host_address", address)?;
            }

            Ok(result)
        })?;

        // Test remote execution (2 parameters)
        let params = lua.create_table()?;
        params.set("test_param", "remote_test")?;

        let host = lua.create_table()?;
        host.set("address", "remote.example.com")?;

        let result: Table = test_module.call((params, host))?;

        assert!(result.get::<bool>("params_received")?);
        assert!(result.get::<bool>("host_provided")?);
        assert_eq!(result.get::<String>("test_param")?, "remote_test");
        assert_eq!(result.get::<String>("host_address")?, "remote.example.com");

        Ok(())
    }

    #[test]
    fn test_create_modulev2_function_invalid_parameter_count() -> mlua::Result<()> {
        let lua = create_lua()?;

        let test_module =
            create_modulev2_function(&lua, "test", |lua, _params, _host| lua.create_table())?;

        // Test with no parameters (should fail)
        let result: mlua::Result<Table> = test_module.call(());
        assert!(result.is_err());

        // Test with too many parameters (should fail)
        let params1 = lua.create_table()?;
        let params2 = lua.create_table()?;
        let params3 = lua.create_table()?;

        let result: mlua::Result<Table> = test_module.call((params1, params2, params3));
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_create_modulev2_function_invalid_parameter_types() -> mlua::Result<()> {
        let lua = create_lua()?;

        let test_module =
            create_modulev2_function(&lua, "test", |lua, _params, _host| lua.create_table())?;

        // Test with non-table first parameter (should fail)
        let result: mlua::Result<Table> = test_module.call("not_a_table".to_string());
        assert!(result.is_err());

        // Test with non-table second parameter (should fail)
        let params = lua.create_table()?;
        let result: mlua::Result<Table> = test_module.call((params, "not_a_table".to_string()));
        assert!(result.is_err());

        Ok(())
    }

    #[test]
    fn test_create_modulev2_function_error_messages() -> mlua::Result<()> {
        let lua = create_lua()?;

        let test_module = create_modulev2_function(&lua, "test_module", |lua, _params, _host| {
            lua.create_table()
        })?;

        // Test error message for wrong parameter count
        let result: mlua::Result<Table> = test_module.call(());
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("test_module expects 1 or 2 parameters"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("test_module expects 1 or 2 parameters"));
                }
                _ => panic!("Expected RuntimeError in callback, got: {cause:?}"),
            },
            Err(e) => panic!("Expected RuntimeError, got: {e:?}"),
            Ok(_) => panic!("Expected error, got success"),
        }

        // Test error message for wrong parameter type
        let result: mlua::Result<Table> = test_module.call("not_a_table".to_string());
        match result {
            Err(mlua::Error::RuntimeError(msg)) => {
                assert!(msg.contains("First parameter to test_module must be a table"));
            }
            Err(mlua::Error::CallbackError { cause, .. }) => match cause.as_ref() {
                mlua::Error::RuntimeError(msg) => {
                    assert!(msg.contains("First parameter to test_module must be a table"));
                }
                _ => panic!("Expected RuntimeError in callback, got: {cause:?}"),
            },
            Err(e) => panic!("Expected RuntimeError, got: {e:?}"),
            Ok(_) => panic!("Expected error, got success"),
        }

        Ok(())
    }
}
