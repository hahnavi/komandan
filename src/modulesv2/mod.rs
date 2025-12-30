//! # `ModulesV2` System
//!
//! `ModulesV2` introduces a simplified and more intuitive module execution system for Komandan.
//! Instead of the current pattern where users must wrap module calls in task structures and
//! pass them to `komandan.komando(task, host)`, `ModulesV2` allows direct module execution
//! with automatic host detection and connection management.
//!
//! ## Key Features
//!
//! - **Direct Module Execution**: Call modules directly without task wrappers
//! - **Dual-Signature Support**: `k.mod.cmd({params})` for local, `k.mod.cmd({params}, host)` for remote
//! - **Automatic Connection Management**: Handles SSH and local connections transparently
//! - **Backward Compatibility**: Coexists with `ModulesV1` (`k.mods`) without conflicts
//! - **Consistent Result Structure**: Same result format as `ModulesV1`
//!
//! ## Usage Examples
//!
//! ```lua
//! -- Local execution
//! local result = k.mod.cmd({cmd = "echo test"})
//!
//! -- Remote execution
//! local host = {address = "remote.com", user = "deploy"}
//! local result = k.mod.cmd({cmd = "echo test"}, host)
//! ```

mod execution;
mod factory;

// Module implementations
mod apt;
mod cmd;
mod dnf;
mod download;
mod file;
mod systemd_service;
mod template;
mod upload;

pub use execution::*;
pub use factory::*;

use mlua::{Lua, Table};

/// Collect all `ModulesV2` modules and register them under the k.mod namespace
///
/// This function creates the main registry for all `ModulesV2` modules, providing
/// a centralized location for module registration and discovery.
///
/// # Arguments
/// * `lua` - The Lua context for creating functions and tables
///
/// # Returns
/// * `mlua::Result<Table>` - A table containing all registered `ModulesV2` modules
///
/// # Errors
/// Returns an error if:
/// - Module registration fails
/// - Lua table creation fails
/// - Module function creation fails
pub fn collect_modulesv2(lua: &Lua) -> mlua::Result<Table> {
    let modules = lua.create_table()?;

    // Register implemented ModulesV2 modules
    modules.set("cmd", cmd::cmd_v2(lua)?)?;
    modules.set("apt", apt::apt_v2(lua)?)?;
    modules.set("file", file::file_v2(lua)?)?;
    modules.set("dnf", dnf::dnf_v2(lua)?)?;
    modules.set("systemd_service", systemd_service::systemd_service_v2(lua)?)?;
    modules.set("template", template::template_v2(lua)?)?;
    modules.set("upload", upload::upload_v2(lua)?)?;
    modules.set("download", download::download_v2(lua)?)?;

    Ok(modules)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::create_lua;

    #[test]
    fn test_collect_modulesv2() -> mlua::Result<()> {
        let lua = create_lua()?;
        let modules = collect_modulesv2(&lua)?;

        // Check that modules are registered (can't use len() with string keys)
        assert!(modules.contains_key("cmd")?);
        assert!(modules.contains_key("apt")?);
        assert!(modules.contains_key("file")?);
        assert!(modules.contains_key("dnf")?);
        assert!(modules.contains_key("systemd_service")?);
        assert!(modules.contains_key("template")?);
        assert!(modules.contains_key("upload")?);
        assert!(modules.contains_key("download")?);

        // Verify the modules are actually functions
        let _cmd_module: mlua::Function = modules.get("cmd")?;
        let _apt_module: mlua::Function = modules.get("apt")?;
        let _file_module: mlua::Function = modules.get("file")?;
        let _dnf_module: mlua::Function = modules.get("dnf")?;
        let _systemd_service_module: mlua::Function = modules.get("systemd_service")?;
        let _template_module: mlua::Function = modules.get("template")?;
        let _upload_module: mlua::Function = modules.get("upload")?;
        let _download_module: mlua::Function = modules.get("download")?;

        // If we get here without panicking, the modules are functions

        Ok(())
    }
}
