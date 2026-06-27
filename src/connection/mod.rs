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
//! - **Structured Errors**: Errors carry semantic fields (`host`, `port`, `user`, `context`)
//!   rendered via `Display`; verbose troubleshooting strings were removed
//!   (see `REFACTOR_PLAN.md` §2.2)
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
//! Connection errors are typed via [`ConnectionError`] (a `thiserror` enum)
//! whose `Display` output carries structured fields (`host`, `port`, `user`,
//! `context`). Verbose troubleshooting text was removed in favor of
//! documentation; see `REFACTOR_PLAN.md` §2.2.

mod auth;
mod elevation;
mod env;
mod error;
mod session;

#[cfg(test)]
mod tests;

pub use auth::get_auth_config;
pub use elevation::get_elevation_config;
pub(crate) use env::setup_environment_local;
pub use env::setup_environment_ssh;
pub use error::ConnectionError;
pub use session::{create_configured_ssh_session, create_ssh_session};

use crate::executor::CommandExecutor;
use crate::local::LocalSession;
use crate::models::ConnectionType;
use crate::ssh::SSHSession;
use crate::util::host_display;
use crate::validator::validate_host;
use anyhow::Result;
use mlua::{Lua, Table, Value};

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
        }
        .to_runtime_error()
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
