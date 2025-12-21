//! Template processing utilities for notifications
//!
//! This module provides comprehensive template processing capabilities for the notification system,
//! including dynamic content rendering, task result formatting, system context inclusion,
//! and robust error handling with graceful degradation.

use crate::notification::{
    NotificationContext, TaskResult,
    errors::{NotificationError, NotificationResult},
    utils::ParameterExtractor,
};
use minijinja::{Environment, Value, context};
use mlua::Table;
use regex;
use std::collections::HashMap;

/// Template processor for notification content
pub struct TemplateProcessor {
    env: Environment<'static>,
}

impl TemplateProcessor {
    /// Create a new template processor with notification-specific filters and settings
    pub fn new() -> Self {
        let mut env = Environment::new();

        // Configure template environment for robust error handling
        env.set_undefined_behavior(minijinja::UndefinedBehavior::Lenient);

        // Add custom filters for notification formatting
        env.add_filter("truncate", truncate_filter);
        env.add_filter("escape_markdown", escape_markdown_filter);
        env.add_filter("format_duration", format_duration_filter);
        env.add_filter("format_exit_code", format_exit_code_filter);
        env.add_filter("format_output", format_output_filter);
        env.add_filter("default", default_value_filter);

        Self { env }
    }

    /// Render template with notification context
    ///
    /// # Errors
    ///
    /// Returns an error if template compilation or rendering fails.
    pub fn render_template(
        &self,
        template_str: &str,
        context: &NotificationContext,
    ) -> NotificationResult<String> {
        let template = self
            .env
            .template_from_str(template_str)
            .map_err(NotificationError::from)?;

        let rendered = template.render(context).map_err(NotificationError::from)?;

        Ok(rendered)
    }

    /// Render template with custom variables
    ///
    /// # Errors
    ///
    /// Returns an error if template compilation or rendering fails.
    pub fn render_with_vars(
        &self,
        template_str: &str,
        vars: &HashMap<String, String>,
    ) -> NotificationResult<String> {
        let template = self
            .env
            .template_from_str(template_str)
            .map_err(NotificationError::from)?;

        let rendered = template.render(vars).map_err(NotificationError::from)?;

        Ok(rendered)
    }

    /// Render template with enhanced context including system information and task results
    ///
    /// This method provides the most comprehensive template rendering with full context,
    /// including system information, task results, and custom variables.
    ///
    /// # Errors
    ///
    /// Returns an error if template compilation or rendering fails.
    pub fn render_with_full_context(
        &self,
        template_str: &str,
        context: &NotificationContext,
        additional_vars: Option<&HashMap<String, String>>,
    ) -> NotificationResult<String> {
        let template = self
            .env
            .template_from_str(template_str)
            .map_err(NotificationError::from)?;

        // Create comprehensive context for template rendering
        // Start with the base context
        let mut all_vars = context.custom_vars.clone();

        // Add additional variables if provided
        if let Some(vars) = additional_vars {
            all_vars.extend(vars.clone());
        }

        // Create the template context
        let template_context = context! {
            hostname => context.hostname,
            timestamp => context.timestamp,
            user => context.user,
            task_name => context.task_name,
            task_result => context.task_result,
            custom_vars => all_vars,
        };

        let rendered = template
            .render(&template_context)
            .map_err(NotificationError::from)?;

        Ok(rendered)
    }

    /// Render template with graceful error handling and fallback
    ///
    /// This method attempts to render the template and falls back to a default message
    /// if rendering fails, ensuring notifications are always sent even with template errors.
    ///
    /// # Errors
    ///
    /// Only returns an error if both template rendering and fallback fail.
    pub fn render_with_fallback(
        &self,
        template_str: &str,
        context: &NotificationContext,
        fallback_message: Option<&str>,
    ) -> NotificationResult<String> {
        match self.render_template(template_str, context) {
            Ok(rendered) => Ok(rendered),
            Err(template_error) => {
                // Log the template error for debugging
                eprintln!(
                    "Template rendering failed: {}. Using fallback message.",
                    template_error
                );

                if let Some(fallback) = fallback_message {
                    Ok(fallback.to_string())
                } else {
                    // Generate a basic fallback message with available context
                    let basic_message = format!(
                        "Notification from {} at {} (template rendering failed)",
                        context.hostname, context.timestamp
                    );
                    Ok(basic_message)
                }
            }
        }
    }
}

impl Default for TemplateProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// Truncate text to specified length with ellipsis
fn truncate_filter(value: Value, length: Value) -> Result<Value, minijinja::Error> {
    let text = value.as_str().unwrap_or("");
    let max_len = length.as_usize().unwrap_or(100);

    if text.len() <= max_len {
        Ok(Value::from(text))
    } else {
        let truncated = format!("{}...", &text[..max_len.saturating_sub(3)]);
        Ok(Value::from(truncated))
    }
}

/// Escape markdown special characters
fn escape_markdown_filter(value: Value) -> Result<Value, minijinja::Error> {
    let text = value.as_str().unwrap_or("");
    let escaped = text
        .replace('\\', "\\\\")
        .replace('*', "\\*")
        .replace('_', "\\_")
        .replace('`', "\\`")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('(', "\\(")
        .replace(')', "\\)")
        .replace('#', "\\#")
        .replace('+', "\\+")
        .replace('-', "\\-")
        .replace('.', "\\.")
        .replace('!', "\\!");

    Ok(Value::from(escaped))
}

/// Format duration in milliseconds to human-readable format
fn format_duration_filter(value: Value) -> Result<Value, minijinja::Error> {
    let duration_ms = value.as_i64().unwrap_or(0).max(0) as u64;

    let formatted = if duration_ms < 1000 {
        format!("{}ms", duration_ms)
    } else if duration_ms < 60_000 {
        format!("{:.1}s", duration_ms as f64 / 1000.0)
    } else if duration_ms < 3_600_000 {
        format!("{:.1}m", duration_ms as f64 / 60_000.0)
    } else {
        format!("{:.1}h", duration_ms as f64 / 3_600_000.0)
    };

    Ok(Value::from(formatted))
}

/// Format exit code with status indicator
fn format_exit_code_filter(value: &Value) -> minijinja::Value {
    let exit_code = i32::try_from(value.as_i64().unwrap_or(0)).unwrap_or(0);

    let formatted = if exit_code == 0 {
        "✅ SUCCESS (0)".to_string()
    } else {
        format!("❌ FAILED ({exit_code})")
    };

    Value::from(formatted)
}

/// Format command output with proper truncation and escaping
fn format_output_filter(value: &Value, max_lines: Option<Value>) -> minijinja::Value {
    let output = value.as_str().unwrap_or("");
    let max_lines = max_lines.and_then(|v| v.as_usize()).unwrap_or(20);

    if output.is_empty() {
        return Value::from("(no output)");
    }

    let lines: Vec<&str> = output.lines().collect();

    let formatted = if lines.len() <= max_lines {
        format!("```\n{}\n```", output.trim())
    } else {
        let truncated_lines = &lines[..max_lines];
        let remaining = lines.len() - max_lines;
        format!(
            "```\n{}\n... ({remaining} more lines)\n```",
            truncated_lines.join("\n")
        )
    };

    Value::from(formatted)
}

/// Provide default value for undefined variables
fn default_value_filter(value: Value, default: Value) -> minijinja::Value {
    if value.is_undefined() || value.is_none() {
        default
    } else {
        value
    }
}

/// Format task result for notification display with comprehensive information
///
/// This function creates a well-formatted, readable representation of task execution results
/// including status indicators, timing information, and properly formatted output.
#[must_use]
pub fn format_task_result(
    task_name: &str,
    stdout: &str,
    stderr: &str,
    exit_code: i32,
    duration_ms: u64,
) -> String {
    let status = if exit_code == 0 {
        "✅ SUCCESS"
    } else {
        "❌ FAILED"
    };
    let duration = format_duration_ms(duration_ms);

    use std::fmt::Write;

    let mut result = format!("**Task: {task_name}**\n");
    write!(result, "Status: {status} (exit code: {exit_code})\n").ok();
    write!(result, "Duration: {duration}\n").ok();

    // Format stdout with proper handling of empty/large output
    if !stdout.is_empty() {
        let formatted_stdout = format_command_output(stdout, "Output", 20);
        result.push_str(&formatted_stdout);
    }

    // Format stderr with proper handling of empty/large output
    if !stderr.is_empty() {
        let formatted_stderr = format_command_output(stderr, "Errors", 10);
        result.push_str(&formatted_stderr);
    }

    // Add summary if both stdout and stderr are empty
    if stdout.is_empty() && stderr.is_empty() {
        result.push_str("\n*No output produced*\n");
    }

    result
}

/// Format command output with truncation and proper escaping
fn format_command_output(output: &str, label: &str, max_lines: usize) -> String {
    let trimmed = output.trim();
    if trimmed.is_empty() {
        return format!("\n**{label}:** *(empty)*\n");
    }

    let lines: Vec<&str> = trimmed.lines().collect();

    if lines.len() <= max_lines {
        format!("\n**{label}:**\n```\n{trimmed}\n```\n")
    } else {
        let truncated_lines = &lines[..max_lines];
        let remaining = lines.len() - max_lines;
        let joined = truncated_lines.join("\n");
        format!("\n**{label}:**\n```\n{joined}\n... ({remaining} more lines)\n```\n")
    }
}

/// Create a comprehensive task result from individual components
#[must_use]
pub const fn create_task_result(
    stdout: String,
    stderr: String,
    exit_code: i32,
    duration_ms: u64,
) -> TaskResult {
    TaskResult {
        stdout,
        stderr,
        exit_code,
        duration_ms,
    }
}

/// Format duration in milliseconds to human-readable string
fn format_duration_ms(duration_ms: u64) -> String {
    if duration_ms < 1000 {
        format!("{duration_ms}ms")
    } else if duration_ms < 60_000 {
        // Safe conversion for durations less than 60 seconds
        let seconds = f64::from(u32::try_from(duration_ms).unwrap_or(u32::MAX)) / 1000.0;
        format!("{seconds:.1}s")
    } else if duration_ms < 3_600_000 {
        // Safe conversion for durations less than 1 hour
        let minutes = f64::from(u32::try_from(duration_ms).unwrap_or(u32::MAX)) / 60_000.0;
        format!("{minutes:.1}m")
    } else {
        // For very long durations, precision loss is acceptable
        let hours = f64::from(u32::try_from(duration_ms.min(u32::MAX as u64)).unwrap_or(u32::MAX))
            / 3_600_000.0;
        format!("{hours:.1}h")
    }
}

/// Create system context with current environment information
///
/// This function gathers comprehensive system information including hostname,
/// current user, and timestamp for use in notification templates.
#[must_use]
pub fn create_system_context() -> NotificationContext {
    NotificationContext::new()
}

/// Create enhanced system context with additional system information
///
/// This function creates a more comprehensive system context that includes
/// additional environment details useful for notifications.
#[must_use]
pub fn create_enhanced_system_context() -> NotificationContext {
    let mut context = NotificationContext::new();

    // Add additional system information to custom_vars
    if let Ok(pwd) = std::env::var("PWD") {
        context
            .custom_vars
            .insert("working_directory".to_string(), pwd);
    }

    if let Ok(shell) = std::env::var("SHELL") {
        context.custom_vars.insert("shell".to_string(), shell);
    }

    if let Ok(term) = std::env::var("TERM") {
        context.custom_vars.insert("terminal".to_string(), term);
    }

    // Add process ID for uniqueness
    context
        .custom_vars
        .insert("pid".to_string(), std::process::id().to_string());

    context
}

/// Create notification context from Lua table parameters
///
/// # Errors
///
/// Returns an error if parameter extraction fails.
pub fn create_context_from_lua_table(table: &Table) -> NotificationResult<NotificationContext> {
    ParameterExtractor::create_notification_context(table)
}

/// Create notification context with task information
///
/// This function creates a context specifically for task-related notifications,
/// including task execution results and timing information.
#[must_use]
pub fn create_task_context(
    task_name: String,
    task_result: Option<TaskResult>,
    custom_vars: Option<impl IntoIterator<Item = (String, String)>>,
) -> NotificationContext {
    let mut context = NotificationContext::new();

    context.task_name = Some(task_name);
    context.task_result = task_result;

    if let Some(vars) = custom_vars {
        context.custom_vars.extend(vars);
    }

    context
}

/// Process template content with error handling and fallback
///
/// This function provides robust template processing with graceful degradation.
/// If template rendering fails, it attempts to use a fallback message or
/// generates a basic notification message from available context.
///
/// # Errors
///
/// Returns an error if template processing fails and no fallback is possible.
pub fn process_template_with_fallback(
    template_str: &str,
    context: &NotificationContext,
    fallback_text: Option<&str>,
) -> NotificationResult<String> {
    let processor = TemplateProcessor::new();

    match processor.render_template(template_str, context) {
        Ok(rendered) => Ok(rendered),
        Err(template_error) => {
            if let Some(fallback) = fallback_text {
                // Log the template error but use fallback
                eprintln!("Template processing failed, using fallback: {template_error}");
                Ok(fallback.to_string())
            } else {
                Err(template_error)
            }
        }
    }
}

/// Validate template syntax without rendering
///
/// This function checks if a template string has valid syntax without
/// actually rendering it, useful for validation during configuration.
///
/// # Errors
///
/// Returns an error if the template syntax is invalid.
pub fn validate_template_syntax(template_str: &str) -> NotificationResult<()> {
    let processor = TemplateProcessor::new();

    // Try to compile the template to check syntax
    processor
        .env
        .template_from_str(template_str)
        .map_err(NotificationError::from)?;

    Ok(())
}

/// Extract template variables from template string
///
/// This function analyzes a template string and returns a list of
/// variable names that are referenced in the template.
///
/// # Panics
///
/// Panics if the internal regex pattern is invalid (should never happen).
#[must_use]
pub fn extract_template_variables(template_str: &str) -> Vec<String> {
    let mut variables = Vec::new();

    // Simple regex-based extraction of {{ variable }} patterns
    // This regex is hardcoded and known to be valid
    let re =
        regex::Regex::new(r"\{\{\s*([a-zA-Z_][a-zA-Z0-9_]*(?:\.[a-zA-Z_][a-zA-Z0-9_]*)*)\s*\}\}")
            .unwrap_or_else(|e| panic!("Template variable regex should be valid: {e}"));

    for cap in re.captures_iter(template_str) {
        if let Some(var_name) = cap.get(1) {
            let var_str = var_name.as_str().to_string();
            if !variables.contains(&var_str) {
                variables.push(var_str);
            }
        }
    }

    variables
}

/// Create default notification message when templates fail
///
/// This function generates a basic but informative notification message
/// using available context when template processing fails completely.
#[must_use]
pub fn create_default_notification_message(context: &NotificationContext) -> String {
    use std::fmt::Write;

    let mut message = format!("Notification from {}", context.hostname);

    if let Some(task_name) = &context.task_name {
        write!(message, " - Task: {task_name}").ok();

        if let Some(task_result) = &context.task_result {
            let status = if task_result.exit_code == 0 {
                "SUCCESS"
            } else {
                "FAILED"
            };
            write!(message, " ({status})").ok();
        }
    }

    write!(message, " at {}", context.timestamp).ok();

    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_template_processor_basic() {
        let processor = TemplateProcessor::new();
        let context = NotificationContext::new();

        let template = "Hello {{ user }} from {{ hostname }}";
        let result = processor.render_template(template, &context);

        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert!(rendered.contains(&context.user));
            assert!(rendered.contains(&context.hostname));
        }
    }

    #[test]
    fn test_template_processor_with_vars() {
        let processor = TemplateProcessor::new();
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Test".to_string());
        vars.insert("value".to_string(), "123".to_string());

        let template = "Name: {{ name }}, Value: {{ value }}";
        let result = processor.render_with_vars(template, &vars);

        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert_eq!(rendered, "Name: Test, Value: 123");
        }
    }

    #[test]
    fn test_template_processor_with_full_context() {
        let processor = TemplateProcessor::new();
        let mut context = NotificationContext::new();
        context.task_name = Some("Test Task".to_string());

        let mut additional_vars = HashMap::new();
        additional_vars.insert("custom_var".to_string(), "custom_value".to_string());

        // Access additional variables through custom_vars in the template
        let template =
            "Task: {{ task_name }}, Custom: {{ custom_vars.custom_var }}, Host: {{ hostname }}";
        let result = processor.render_with_full_context(template, &context, Some(&additional_vars));

        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert!(rendered.contains("Test Task"));
            assert!(rendered.contains("custom_value"));
            assert!(rendered.contains(&context.hostname));
        }
    }

    #[test]
    fn test_template_processor_with_fallback() {
        let processor = TemplateProcessor::new();
        let context = NotificationContext::new();

        // Test with invalid template syntax
        let invalid_template = "Hello {{ invalid.syntax.here";
        let result =
            processor.render_with_fallback(invalid_template, &context, Some("Fallback message"));

        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert_eq!(rendered, "Fallback message");
        }
    }

    #[test]
    fn test_template_processor_undefined_variables() {
        let processor = TemplateProcessor::new();
        let context = NotificationContext::new();

        // Template with undefined variable should still render due to lenient mode
        let template = "Hello {{ undefined_var | default('default_value') }}";
        let result = processor.render_template(template, &context);

        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert!(rendered.contains("default_value"));
        }
    }

    #[test]
    fn test_format_task_result() {
        let result = format_task_result("Test Task", "Hello World", "", 0, 1500);

        assert!(result.contains("Test Task"));
        assert!(result.contains("✅ SUCCESS"));
        assert!(result.contains("1.5s"));
        assert!(result.contains("Hello World"));
    }

    #[test]
    fn test_format_task_result_with_error() {
        let result = format_task_result("Failed Task", "", "Error occurred", 1, 2500);

        assert!(result.contains("Failed Task"));
        assert!(result.contains("❌ FAILED"));
        assert!(result.contains("2.5s"));
        assert!(result.contains("Error occurred"));
    }

    #[test]
    fn test_format_task_result_empty_output() {
        let result = format_task_result("Empty Task", "", "", 0, 100);

        assert!(result.contains("Empty Task"));
        assert!(result.contains("✅ SUCCESS"));
        assert!(result.contains("No output produced"));
    }

    #[test]
    fn test_format_duration_ms() {
        assert_eq!(format_duration_ms(500), "500ms");
        assert_eq!(format_duration_ms(1500), "1.5s");
        assert_eq!(format_duration_ms(90000), "1.5m");
        assert_eq!(format_duration_ms(7_200_000), "2.0h");
    }

    #[test]
    fn test_create_enhanced_system_context() {
        let context = create_enhanced_system_context();

        assert!(!context.hostname.is_empty());
        assert!(!context.user.is_empty());
        assert!(!context.timestamp.is_empty());
        assert!(context.custom_vars.contains_key("pid"));
    }

    #[test]
    fn test_create_task_context() {
        let task_result = create_task_result("output".to_string(), String::new(), 0, 1000);

        let mut custom_vars = HashMap::new();
        custom_vars.insert("env".to_string(), "test".to_string());

        let context = create_task_context(
            "Test Task".to_string(),
            Some(task_result),
            Some(custom_vars),
        );

        assert_eq!(context.task_name, Some("Test Task".to_string()));
        assert!(context.task_result.is_some());
        assert_eq!(context.custom_vars.get("env"), Some(&"test".to_string()));
    }

    #[test]
    fn test_validate_template_syntax() {
        // Valid template
        let valid_template = "Hello {{ name }}";
        assert!(validate_template_syntax(valid_template).is_ok());

        // Invalid template
        let invalid_template = "Hello {{ unclosed";
        assert!(validate_template_syntax(invalid_template).is_err());
    }

    #[test]
    fn test_extract_template_variables() {
        let template = "Hello {{ name }}, your task {{ task.name }} completed at {{ timestamp }}";
        let variables = extract_template_variables(template);

        assert!(variables.contains(&"name".to_string()));
        assert!(variables.contains(&"task.name".to_string()));
        assert!(variables.contains(&"timestamp".to_string()));
        assert_eq!(variables.len(), 3);
    }

    #[test]
    fn test_create_default_notification_message() {
        let mut context = NotificationContext::new();
        context.task_name = Some("Test Task".to_string());
        context.task_result = Some(TaskResult {
            stdout: "output".to_string(),
            stderr: String::new(),
            exit_code: 0,
            duration_ms: 1000,
        });

        let message = create_default_notification_message(&context);

        assert!(message.contains(&context.hostname));
        assert!(message.contains("Test Task"));
        assert!(message.contains("SUCCESS"));
        assert!(message.contains(&context.timestamp));
    }

    #[test]
    fn test_format_command_output_truncation() {
        let long_output = (0..30)
            .map(|i| format!("Line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let formatted = format_command_output(&long_output, "Output", 10);

        assert!(formatted.contains("Output"));
        assert!(formatted.contains("Line 0"));
        assert!(formatted.contains("Line 9"));
        assert!(formatted.contains("20 more lines"));
        assert!(!formatted.contains("Line 29"));
    }

    #[test]
    fn test_template_filters() {
        let processor = TemplateProcessor::new();
        let context = NotificationContext::new();

        // Test truncate filter
        let template = "{{ 'very long text that should be truncated' | truncate(10) }}";
        let result = processor.render_template(template, &context);
        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert!(rendered.contains("..."));
        }

        // Test format_duration filter
        let template = "Duration: {{ 1500 | format_duration }}";
        let result = processor.render_template(template, &context);
        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert!(rendered.contains("1.5s"));
        }

        // Test format_exit_code filter
        let template = "Status: {{ 0 | format_exit_code }}";
        let result = processor.render_template(template, &context);
        assert!(result.is_ok());
        if let Ok(rendered) = result {
            assert!(rendered.contains("✅ SUCCESS"));
        }
    }
}
