//! Notification system for Komandan
//!
//! This module provides notification functionality through multiple channels:
//! - Google Chat webhooks
//! - Slack webhooks
//! - SMTP email
//!
//! All notification functions are accessible through the `komandan.notification` namespace
//! in Lua scripts and follow consistent interface patterns.

pub mod errors;
pub mod google_chat;
pub mod slack;
pub mod smtp;
pub mod templates;
pub mod utils;
pub mod validation;

pub use errors::NotificationError;
pub use utils::{
    DryRunHandler, HttpClientConfig, HttpResponseProcessor, ParameterExtractor, ResponseFormatter,
    RetryHandler, SessionManager, create_smtp_http_config, create_webhook_http_config,
};

use mlua::{Lua, Table};
use std::collections::HashMap;

/// Register all notification functions in the Lua environment
///
/// This function creates the `komandan.notification` table and registers
/// all notification functions for use in Lua scripts.
///
/// # Errors
///
/// Returns an error if Lua table creation or function registration fails.
pub fn register_notification_functions(lua: &Lua, komandan_table: &Table) -> mlua::Result<()> {
    let notification_table = lua.create_table()?;

    // Register notification functions
    notification_table.set(
        "google_chat_webhook",
        lua.create_function(google_chat::google_chat_webhook)?,
    )?;
    notification_table.set("slack_webhook", lua.create_function(slack::slack_webhook)?)?;
    notification_table.set("smtp", lua.create_function(smtp::smtp)?)?;

    komandan_table.set("notification", notification_table)?;
    Ok(())
}

/// Common notification request structure
#[derive(Debug, Clone)]
pub struct NotificationRequest {
    pub notification_type: NotificationType,
    pub dry_run: bool,
    pub template_vars: HashMap<String, String>,
    pub timeout_seconds: Option<u64>,
    pub retry_attempts: Option<u32>,
}

/// Notification type enumeration
#[derive(Debug, Clone)]
pub enum NotificationType {
    GoogleChat(google_chat::GoogleChatParams),
    Slack(slack::SlackParams),
    Smtp(smtp::SmtpParams),
}

/// Standardized notification response
#[derive(Debug, Clone)]
pub struct NotificationResponse {
    pub success: bool,
    pub message: String,
    pub response_code: Option<u16>,
    pub response_body: Option<String>,
    pub delivery_time_ms: u64,
}

impl NotificationResponse {
    /// Create a success response
    #[must_use]
    pub const fn success(
        message: String,
        response_code: Option<u16>,
        delivery_time_ms: u64,
    ) -> Self {
        Self {
            success: true,
            message,
            response_code,
            response_body: None,
            delivery_time_ms,
        }
    }

    /// Create an error response
    #[must_use]
    pub const fn error(message: String, response_code: Option<u16>, delivery_time_ms: u64) -> Self {
        Self {
            success: false,
            message,
            response_code,
            response_body: None,
            delivery_time_ms,
        }
    }

    /// Convert to Lua table for return to scripts
    ///
    /// # Errors
    ///
    /// Returns an error if Lua table creation fails.
    pub fn to_lua_table(&self, lua: &Lua) -> mlua::Result<Table> {
        let table = lua.create_table()?;
        table.set("success", self.success)?;
        table.set("message", self.message.clone())?;

        if let Some(code) = self.response_code {
            table.set("response_code", code)?;
        }

        if let Some(body) = &self.response_body {
            table.set("response_body", body.clone())?;
        }

        table.set("delivery_time_ms", self.delivery_time_ms)?;
        Ok(table)
    }
}

/// Template context for dynamic content rendering
#[derive(Debug, Clone, serde::Serialize)]
pub struct NotificationContext {
    pub hostname: String,
    pub timestamp: String,
    pub user: String,
    pub task_name: Option<String>,
    pub task_result: Option<TaskResult>,
    pub custom_vars: HashMap<String, String>,
}

/// Task execution result for notifications
#[derive(Debug, Clone, serde::Serialize)]
pub struct TaskResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

impl NotificationContext {
    /// Create a new notification context with system information
    #[must_use]
    pub fn new() -> Self {
        use std::env;

        let hostname = env::var("HOSTNAME")
            .or_else(|_| env::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());

        let user = env::var("USER")
            .or_else(|_| env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());

        let timestamp = chrono::Utc::now().to_rfc3339();

        Self {
            hostname,
            timestamp,
            user,
            task_name: None,
            task_result: None,
            custom_vars: HashMap::new(),
        }
    }

    /// Add custom variables to the context
    #[must_use]
    pub fn with_custom_vars(mut self, vars: HashMap<String, String>) -> Self {
        self.custom_vars = vars;
        self
    }

    /// Add task information to the context
    #[must_use]
    pub fn with_task(mut self, name: String, result: Option<TaskResult>) -> Self {
        self.task_name = Some(name);
        self.task_result = result;
        self
    }
}

impl Default for NotificationContext {
    fn default() -> Self {
        Self::new()
    }
}
