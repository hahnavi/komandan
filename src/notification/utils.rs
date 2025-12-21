//! Shared utilities for notification system

use crate::notification::{
    NotificationContext, NotificationResponse, TaskResult,
    errors::{NotificationError, NotificationResult},
};
use mlua::{Table, Value};
use reqwest::{Client, ClientBuilder, Response, StatusCode};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// HTTP client configuration for webhook requests
pub struct HttpClientConfig {
    pub timeout_seconds: u64,
    pub connect_timeout_seconds: u64,
    pub retry_attempts: u32,
    pub base_retry_delay_ms: u64,
    pub max_retry_delay_ms: u64,
    pub user_agent: String,
    pub pool_idle_timeout_seconds: u64,
    pub pool_max_idle_per_host: usize,
}

impl Default for HttpClientConfig {
    fn default() -> Self {
        Self {
            timeout_seconds: 30,
            connect_timeout_seconds: 10,
            retry_attempts: 3,
            base_retry_delay_ms: 1000,
            max_retry_delay_ms: 30000,
            user_agent: format!("Komandan/{}", env!("CARGO_PKG_VERSION")),
            pool_idle_timeout_seconds: 90,
            pool_max_idle_per_host: 10,
        }
    }
}

impl HttpClientConfig {
    /// Create a new HTTP client configuration
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set request timeout in seconds
    #[must_use]
    pub const fn with_timeout(mut self, timeout_seconds: u64) -> Self {
        self.timeout_seconds = timeout_seconds;
        self
    }

    /// Set connection timeout in seconds
    #[must_use]
    pub const fn with_connect_timeout(mut self, connect_timeout_seconds: u64) -> Self {
        self.connect_timeout_seconds = connect_timeout_seconds;
        self
    }

    /// Set retry attempts
    #[must_use]
    pub const fn with_retry_attempts(mut self, retry_attempts: u32) -> Self {
        self.retry_attempts = retry_attempts;
        self
    }

    /// Set base retry delay in milliseconds
    #[must_use]
    pub const fn with_base_retry_delay(mut self, base_retry_delay_ms: u64) -> Self {
        self.base_retry_delay_ms = base_retry_delay_ms;
        self
    }

    /// Set maximum retry delay in milliseconds
    #[must_use]
    pub const fn with_max_retry_delay(mut self, max_retry_delay_ms: u64) -> Self {
        self.max_retry_delay_ms = max_retry_delay_ms;
        self
    }

    /// Set custom user agent
    #[must_use]
    pub fn with_user_agent(mut self, user_agent: String) -> Self {
        self.user_agent = user_agent;
        self
    }

    /// Set connection pool idle timeout in seconds
    #[must_use]
    pub const fn with_pool_idle_timeout(mut self, pool_idle_timeout_seconds: u64) -> Self {
        self.pool_idle_timeout_seconds = pool_idle_timeout_seconds;
        self
    }

    /// Set maximum idle connections per host
    #[must_use]
    pub const fn with_pool_max_idle_per_host(mut self, pool_max_idle_per_host: usize) -> Self {
        self.pool_max_idle_per_host = pool_max_idle_per_host;
        self
    }

    /// Build reqwest client with comprehensive configuration
    ///
    /// # Errors
    ///
    /// Returns an error if client creation fails.
    pub fn build_client(&self) -> NotificationResult<Client> {
        ClientBuilder::new()
            .timeout(Duration::from_secs(self.timeout_seconds))
            .connect_timeout(Duration::from_secs(self.connect_timeout_seconds))
            .user_agent(&self.user_agent)
            .pool_idle_timeout(Duration::from_secs(self.pool_idle_timeout_seconds))
            .pool_max_idle_per_host(self.pool_max_idle_per_host)
            .tcp_keepalive(Duration::from_secs(60))
            .tcp_nodelay(true)
            .build()
            .map_err(|e| {
                NotificationError::configuration_error(format!("Failed to create HTTP client: {e}"))
            })
    }

    /// Create retry handler from this configuration
    #[must_use]
    pub const fn create_retry_handler(&self) -> RetryHandler {
        RetryHandler::new(self.retry_attempts, self.base_retry_delay_ms)
            .with_max_delay(self.max_retry_delay_ms)
    }
}

/// Dry-run mode handler for notifications
pub struct DryRunHandler {
    enabled: bool,
}

impl DryRunHandler {
    /// Create a new dry-run handler
    #[must_use]
    pub const fn new(enabled: bool) -> Self {
        Self { enabled }
    }

    /// Check if dry-run mode is enabled
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Simulate notification sending in dry-run mode
    ///
    /// Validates parameters and returns a simulated success response
    /// without actually sending the notification.
    #[must_use]
    pub fn simulate_notification(
        &self,
        notification_type: &str,
        params: &HashMap<String, String>,
    ) -> NotificationResponse {
        if !self.enabled {
            return NotificationResponse::error(
                "Dry-run handler called but dry-run mode is disabled".to_string(),
                None,
                0,
            );
        }

        // Simulate processing time
        let simulated_time = 100; // 100ms

        let message = format!(
            "DRY RUN: Would send {notification_type} notification with {} parameters",
            params.len()
        );

        NotificationResponse::success(message, Some(200), simulated_time)
    }

    /// Validate parameters without sending
    ///
    /// # Errors
    ///
    /// Returns an error if required parameters are missing or invalid.
    pub fn validate_parameters(
        &self,
        required_params: &[&str],
        params: &HashMap<String, String>,
    ) -> NotificationResult<()> {
        for param in required_params {
            if !params.contains_key(*param) {
                return Err(NotificationError::missing_parameter(*param));
            }

            let value = &params[*param];
            if value.is_empty() {
                return Err(NotificationError::invalid_parameter(
                    *param,
                    "Parameter cannot be empty",
                ));
            }
        }

        Ok(())
    }
}

/// Parameter extraction utilities for Lua tables
pub struct ParameterExtractor;

impl ParameterExtractor {
    /// Extract string parameter from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if the parameter is missing or not a string.
    pub fn extract_string(table: &Table, key: &str) -> NotificationResult<String> {
        match table.get(key) {
            Ok(Value::String(s)) => Ok(s
                .to_str()
                .map_err(|e| {
                    NotificationError::invalid_parameter(key, format!("Invalid UTF-8: {e}"))
                })?
                .to_string()),
            Ok(Value::Nil) => Err(NotificationError::missing_parameter(key)),
            Ok(_) => Err(NotificationError::invalid_parameter(
                key,
                "Expected string value",
            )),
            Err(e) => Err(NotificationError::invalid_parameter(
                key,
                format!("Failed to extract parameter: {e}"),
            )),
        }
    }

    /// Extract optional string parameter from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if the parameter exists but is not a valid string.
    pub fn extract_optional_string(table: &Table, key: &str) -> NotificationResult<Option<String>> {
        match table.get(key) {
            Ok(Value::String(s)) => Ok(Some(
                s.to_str()
                    .map_err(|e| {
                        NotificationError::invalid_parameter(key, format!("Invalid UTF-8: {e}"))
                    })?
                    .to_string(),
            )),
            Ok(Value::Nil) | Err(_) => Ok(None), // Treat missing keys as None
            Ok(_) => Err(NotificationError::invalid_parameter(
                key,
                "Expected string value or nil",
            )),
        }
    }

    /// Extract integer parameter from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if the parameter is missing or not an integer.
    pub fn extract_integer<T>(table: &Table, key: &str) -> NotificationResult<T>
    where
        T: TryFrom<i64>,
        T::Error: std::fmt::Display,
    {
        match table.get(key) {
            Ok(Value::Integer(i)) => i.try_into().map_err(|e| {
                NotificationError::invalid_parameter(key, format!("Integer conversion failed: {e}"))
            }),
            Ok(Value::Number(n)) => {
                // f64 can only safely represent integers up to 2^53 without precision loss
                // Use this range to ensure no precision loss during conversion
                const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_992.0; // 2^53
                const MIN_SAFE_INTEGER: f64 = -9_007_199_254_740_992.0; // -2^53

                if n.is_finite() && (MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&n) {
                    // Truncate to i64 - safe because we're within the safe integer range
                    // The range check ensures no precision loss, so truncation is intentional
                    #[allow(clippy::cast_possible_truncation)]
                    let i = n.trunc() as i64;
                    i.try_into().map_err(|e| {
                        NotificationError::invalid_parameter(
                            key,
                            format!("Integer conversion failed: {e}"),
                        )
                    })
                } else {
                    Err(NotificationError::invalid_parameter(
                        key,
                        "Number out of range for safe integer conversion",
                    ))
                }
            }
            Ok(Value::Nil) => Err(NotificationError::missing_parameter(key)),
            Ok(_) => Err(NotificationError::invalid_parameter(
                key,
                "Expected integer value",
            )),
            Err(e) => Err(NotificationError::invalid_parameter(
                key,
                format!("Failed to extract parameter: {e}"),
            )),
        }
    }

    /// Extract optional integer parameter from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if the parameter exists but is not a valid integer or cannot be converted to type T.
    pub fn extract_optional_integer<T>(table: &Table, key: &str) -> NotificationResult<Option<T>>
    where
        T: TryFrom<i64>,
        T::Error: std::fmt::Display,
    {
        match table.get(key) {
            Ok(Value::Integer(i)) => Ok(Some(i.try_into().map_err(|e| {
                NotificationError::invalid_parameter(key, format!("Integer conversion failed: {e}"))
            })?)),
            Ok(Value::Number(n)) => {
                // f64 can only safely represent integers up to 2^53 without precision loss
                const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_992.0; // 2^53
                const MIN_SAFE_INTEGER: f64 = -9_007_199_254_740_992.0; // -2^53

                if n.is_finite() && (MIN_SAFE_INTEGER..=MAX_SAFE_INTEGER).contains(&n) {
                    // Round to nearest integer to avoid fractional parts
                    // Safe because we're within the safe integer range
                    #[allow(clippy::cast_possible_truncation)]
                    let i = n.round() as i64;
                    Ok(Some(i.try_into().map_err(|e| {
                        NotificationError::invalid_parameter(
                            key,
                            format!("Integer conversion failed: {e}"),
                        )
                    })?))
                } else {
                    Err(NotificationError::invalid_parameter(
                        key,
                        "Number out of range for safe integer conversion",
                    ))
                }
            }
            Ok(Value::Nil) | Err(_) => Ok(None), // Treat missing keys as None
            Ok(_) => Err(NotificationError::invalid_parameter(
                key,
                "Expected integer value or nil",
            )),
        }
    }

    /// Extract boolean parameter from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if the parameter is not a boolean.
    pub fn extract_boolean(table: &Table, key: &str) -> NotificationResult<bool> {
        match table.get(key) {
            Ok(Value::Boolean(b)) => Ok(b),
            Ok(Value::Nil) | Err(_) => Ok(false), // Default to false for missing keys
            Ok(_) => Err(NotificationError::invalid_parameter(
                key,
                "Expected boolean value",
            )),
        }
    }

    /// Extract string array parameter from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if the parameter is not an array of strings.
    pub fn extract_string_array(table: &Table, key: &str) -> NotificationResult<Vec<String>> {
        match table.get(key) {
            Ok(Value::Table(array_table)) => {
                let mut result = Vec::new();
                for pair in array_table.pairs::<i32, Value>() {
                    let (_, value) = pair.map_err(|e| {
                        NotificationError::invalid_parameter(
                            key,
                            format!("Array iteration failed: {e}"),
                        )
                    })?;

                    match value {
                        Value::String(s) => {
                            result.push(
                                s.to_str()
                                    .map_err(|e| {
                                        NotificationError::invalid_parameter(
                                            key,
                                            format!("Invalid UTF-8: {e}"),
                                        )
                                    })?
                                    .to_string(),
                            );
                        }
                        _ => {
                            return Err(NotificationError::invalid_parameter(
                                key,
                                "Array must contain only strings",
                            ));
                        }
                    }
                }
                Ok(result)
            }
            Ok(Value::Nil) => Err(NotificationError::missing_parameter(key)),
            Ok(_) => Err(NotificationError::invalid_parameter(
                key,
                "Expected array of strings",
            )),
            Err(e) => Err(NotificationError::invalid_parameter(
                key,
                format!("Failed to extract parameter: {e}"),
            )),
        }
    }

    /// Extract optional string array parameter from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if the parameter exists but is not a valid array of strings.
    pub fn extract_optional_string_array(
        table: &Table,
        key: &str,
    ) -> NotificationResult<Option<Vec<String>>> {
        match table.get(key) {
            Ok(Value::Table(array_table)) => {
                let mut result = Vec::new();
                for pair in array_table.pairs::<i32, Value>() {
                    let (_, value) = pair.map_err(|e| {
                        NotificationError::invalid_parameter(
                            key,
                            format!("Array iteration failed: {e}"),
                        )
                    })?;

                    match value {
                        Value::String(s) => {
                            result.push(
                                s.to_str()
                                    .map_err(|e| {
                                        NotificationError::invalid_parameter(
                                            key,
                                            format!("Invalid UTF-8: {e}"),
                                        )
                                    })?
                                    .to_string(),
                            );
                        }
                        _ => {
                            return Err(NotificationError::invalid_parameter(
                                key,
                                "Array must contain only strings",
                            ));
                        }
                    }
                }
                Ok(Some(result))
            }
            Ok(Value::Nil) | Err(_) => Ok(None),
            Ok(_) => Err(NotificationError::invalid_parameter(
                key,
                "Expected array of strings or nil",
            )),
        }
    }

    /// Extract template variables from Lua table
    ///
    /// # Errors
    ///
    /// Returns an error if template variables contain invalid types or UTF-8.
    pub fn extract_template_vars(table: &Table) -> NotificationResult<HashMap<String, String>> {
        let mut vars = HashMap::new();

        // Look for template_vars table
        if let Ok(Value::Table(vars_table)) = table.get("template_vars") {
            for pair in vars_table.pairs::<String, Value>() {
                let (key, value) = pair.map_err(|e| {
                    NotificationError::invalid_parameter(
                        "template_vars",
                        format!("Failed to iterate template variables: {e}"),
                    )
                })?;

                match value {
                    Value::String(s) => {
                        vars.insert(
                            key,
                            s.to_str()
                                .map_err(|e| {
                                    NotificationError::invalid_parameter(
                                        "template_vars",
                                        format!("Invalid UTF-8: {e}"),
                                    )
                                })?
                                .to_string(),
                        );
                    }
                    Value::Integer(i) => {
                        vars.insert(key, i.to_string());
                    }
                    Value::Number(n) => {
                        vars.insert(key, n.to_string());
                    }
                    Value::Boolean(b) => {
                        vars.insert(key, b.to_string());
                    }
                    _ => {
                        return Err(NotificationError::invalid_parameter(
                            "template_vars",
                            "Template variables must be strings, numbers, or booleans",
                        ));
                    }
                }
            }
        }

        Ok(vars)
    }

    /// Create notification context from Lua table parameters
    ///
    /// # Errors
    ///
    /// Returns an error if required parameters are missing or invalid.
    pub fn create_notification_context(table: &Table) -> NotificationResult<NotificationContext> {
        let mut context = NotificationContext::new();

        // Extract custom variables
        context.custom_vars = Self::extract_template_vars(table)?;

        // Extract task information if provided
        if let Ok(task_name) = Self::extract_optional_string(table, "task_name") {
            context.task_name = task_name;
        }

        // Extract task result if provided
        if let Ok(Value::Table(result_table)) = table.get("task_result") {
            let stdout =
                Self::extract_optional_string(&result_table, "stdout")?.unwrap_or_default();
            let stderr =
                Self::extract_optional_string(&result_table, "stderr")?.unwrap_or_default();
            let exit_code =
                Self::extract_optional_integer::<i32>(&result_table, "exit_code")?.unwrap_or(0);
            let duration_ms =
                Self::extract_optional_integer::<u64>(&result_table, "duration_ms")?.unwrap_or(0);

            context.task_result = Some(TaskResult {
                stdout,
                stderr,
                exit_code,
                duration_ms,
            });
        }

        Ok(context)
    }
}

/// Session state management for notifications
pub struct SessionManager {
    changed: bool,
}

impl SessionManager {
    /// Create a new session manager
    #[must_use]
    pub const fn new() -> Self {
        Self { changed: false }
    }

    /// Mark session as changed (notification was sent)
    pub const fn set_changed(&mut self, changed: bool) {
        self.changed = changed;
    }

    /// Check if session has changes
    #[must_use]
    pub const fn is_changed(&self) -> bool {
        self.changed
    }

    /// Reset session state
    pub const fn reset(&mut self) {
        self.changed = false;
    }
}

impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Retry logic for HTTP requests with exponential backoff
pub struct RetryHandler {
    max_attempts: u32,
    base_delay_ms: u64,
    max_delay_ms: u64,
    jitter_factor: f64,
}

impl RetryHandler {
    /// Create a new retry handler
    #[must_use]
    pub const fn new(max_attempts: u32, base_delay_ms: u64) -> Self {
        Self {
            max_attempts,
            base_delay_ms,
            max_delay_ms: 30000, // 30 seconds default max
            jitter_factor: 0.25, // ±25% jitter
        }
    }

    /// Set maximum delay between retries
    #[must_use]
    pub const fn with_max_delay(mut self, max_delay_ms: u64) -> Self {
        self.max_delay_ms = max_delay_ms;
        self
    }

    /// Set jitter factor (0.0 to 1.0)
    #[must_use]
    pub const fn with_jitter_factor(mut self, jitter_factor: f64) -> Self {
        self.jitter_factor = jitter_factor.clamp(0.0, 1.0);
        self
    }

    /// Execute operation with exponential backoff retry
    ///
    /// This method implements a robust retry strategy with:
    /// - Exponential backoff with jitter to prevent thundering herd
    /// - Configurable maximum delay to prevent excessive wait times
    /// - Early exit for non-retryable errors
    /// - Comprehensive error reporting
    ///
    /// # Errors
    ///
    /// Returns the last error if all retry attempts fail, or immediately
    /// returns non-retryable errors.
    pub async fn retry_with_backoff<F, Fut, T>(&self, mut operation: F) -> NotificationResult<T>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = NotificationResult<T>>,
    {
        let mut last_error = None;
        // Track retry attempts for debugging
        #[allow(clippy::collection_is_never_read)]
        let mut retry_delays = Vec::new();

        for attempt in 1..=self.max_attempts {
            let attempt_start = Instant::now();

            match operation().await {
                Ok(result) => {
                    // Request succeeded after retries
                    return Ok(result);
                }
                Err(error) => {
                    let _attempt_duration = attempt_start.elapsed();
                    last_error = Some(error.clone());

                    // Track failed attempt

                    // Don't retry on non-retryable errors
                    if !error.is_retryable() {
                        return Err(error);
                    }

                    // Don't sleep after the last attempt
                    if attempt < self.max_attempts {
                        let delay = self.calculate_delay(attempt);
                        retry_delays.push(delay);

                        // Retrying with exponential backoff

                        tokio::time::sleep(Duration::from_millis(delay)).await;
                    }
                }
            }
        }

        let final_error = last_error.unwrap_or_else(|| {
            NotificationError::internal_error("Retry loop completed without error or result")
        });

        // All retry attempts failed

        Err(final_error)
    }

    /// Calculate exponential backoff delay with jitter
    ///
    /// Uses exponential backoff: delay = `base_delay` * 2^(attempt-1)
    /// Adds jitter to prevent thundering herd: ±`jitter_factor` of the delay
    /// Caps the delay at `max_delay_ms` to prevent excessive wait times
    fn calculate_delay(&self, attempt: u32) -> u64 {
        // Calculate exponential delay: base * 2^(attempt-1)
        let exponential_delay = self
            .base_delay_ms
            .saturating_mul(2_u64.saturating_pow(attempt.saturating_sub(1)));

        // Add jitter to prevent thundering herd
        #[allow(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss
        )]
        let jitter_range = (exponential_delay as f64 * self.jitter_factor) as u64;
        let jitter = if jitter_range > 0 {
            rand::random::<u64>() % (jitter_range * 2)
        } else {
            0
        };
        let jitter_offset = jitter.saturating_sub(jitter_range);

        // Apply jitter and cap at maximum delay
        exponential_delay
            .saturating_add(jitter_offset)
            .min(self.max_delay_ms)
    }

    /// Get retry configuration summary for logging
    #[must_use]
    pub fn config_summary(&self) -> String {
        format!(
            "max_attempts={}, base_delay={}ms, max_delay={}ms, jitter=±{:.0}%",
            self.max_attempts,
            self.base_delay_ms,
            self.max_delay_ms,
            self.jitter_factor * 100.0
        )
    }
}

/// Create optimized HTTP configuration for webhook notifications
///
/// This function provides a pre-configured HTTP client setup optimized
/// for webhook notifications with appropriate timeouts, retry policies,
/// and connection pooling settings.
#[must_use]
pub fn create_webhook_http_config() -> HttpClientConfig {
    HttpClientConfig::new()
        .with_timeout(30) // 30 second request timeout
        .with_connect_timeout(10) // 10 second connection timeout
        .with_retry_attempts(3) // Retry up to 3 times
        .with_base_retry_delay(1000) // Start with 1 second delay
        .with_max_retry_delay(30000) // Cap at 30 seconds
        .with_pool_idle_timeout(90) // Keep connections alive for 90 seconds
        .with_pool_max_idle_per_host(10) // Up to 10 idle connections per host
}

/// Create HTTP configuration optimized for SMTP-related HTTP requests
///
/// SMTP operations may need different timeout characteristics than webhooks.
#[must_use]
pub fn create_smtp_http_config() -> HttpClientConfig {
    HttpClientConfig::new()
        .with_timeout(60) // Longer timeout for SMTP operations
        .with_connect_timeout(15) // Longer connection timeout
        .with_retry_attempts(2) // Fewer retries for SMTP
        .with_base_retry_delay(2000) // Longer base delay
        .with_max_retry_delay(60000) // Longer max delay
}

impl Default for RetryHandler {
    fn default() -> Self {
        Self::new(3, 1000) // 3 attempts, 1 second base delay
    }
}

/// HTTP response processing utilities
pub struct HttpResponseProcessor;

impl HttpResponseProcessor {
    /// Process HTTP response and create notification response
    ///
    /// This function handles all aspects of HTTP response processing including:
    /// - Status code validation
    /// - Response body extraction
    /// - Error categorization
    /// - Success/failure determination
    ///
    /// # Errors
    ///
    /// Returns an error if response processing fails or indicates an error condition.
    pub async fn process_response(
        response: Response,
        start_time: Instant,
    ) -> NotificationResult<NotificationResponse> {
        let delivery_time_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        let status = response.status();
        let status_code = status.as_u16();

        // Extract response body
        let response_body = match response.text().await {
            Ok(body) => Some(body),
            Err(e) => {
                return Err(NotificationError::http_request_failed(
                    status_code,
                    format!("Failed to read response body: {e}"),
                ));
            }
        };

        // Determine success based on status code
        if status.is_success() {
            let message = Self::format_success_message(status_code, response_body.as_ref());
            Ok(NotificationResponse {
                success: true,
                message,
                response_code: Some(status_code),
                response_body,
                delivery_time_ms,
            })
        } else {
            let message = Self::format_error_message(status_code, response_body.as_ref());
            Err(NotificationError::http_request_failed(status_code, message))
        }
    }

    /// Process HTTP response and convert to notification response (non-failing version)
    ///
    /// This version always returns a `NotificationResponse`, even for HTTP errors,
    /// which is useful when you want to return error details to the caller
    /// rather than propagating the error.
    pub async fn process_response_safe(
        response: Response,
        start_time: Instant,
    ) -> NotificationResponse {
        let delivery_time_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        let status = response.status();
        let status_code = status.as_u16();

        // Extract response body
        let response_body = match response.text().await {
            Ok(body) => Some(body),
            Err(e) => {
                return NotificationResponse::error(
                    format!("Failed to read response body: {e}"),
                    Some(status_code),
                    delivery_time_ms,
                );
            }
        };

        // Determine success based on status code
        if status.is_success() {
            let message = Self::format_success_message(status_code, response_body.as_ref());
            NotificationResponse {
                success: true,
                message,
                response_code: Some(status_code),
                response_body,
                delivery_time_ms,
            }
        } else {
            let message = Self::format_error_message(status_code, response_body.as_ref());
            NotificationResponse {
                success: false,
                message,
                response_code: Some(status_code),
                response_body,
                delivery_time_ms,
            }
        }
    }

    /// Format success message based on status code and response
    fn format_success_message(status_code: u16, _response_body: Option<&String>) -> String {
        match status_code {
            200 => "Notification sent successfully".to_string(),
            201 => "Notification created successfully".to_string(),
            202 => "Notification accepted for processing".to_string(),
            204 => "Notification processed successfully (no content)".to_string(),
            _ => format!("Notification completed with status {status_code}"),
        }
    }

    /// Format error message based on status code and response
    fn format_error_message(status_code: u16, response_body: Option<&String>) -> String {
        let base_message = match status_code {
            400 => "Bad request - invalid notification parameters",
            401 => "Unauthorized - check authentication credentials",
            403 => "Forbidden - insufficient permissions",
            404 => "Not found - verify webhook URL",
            408 => "Request timeout - server did not respond in time",
            409 => "Conflict - notification could not be processed",
            413 => "Payload too large - notification content exceeds limits",
            429 => "Rate limited - too many requests",
            500 => "Internal server error - remote service issue",
            502 => "Bad gateway - service temporarily unavailable",
            503 => "Service unavailable - remote service down",
            504 => "Gateway timeout - remote service timeout",
            _ => "HTTP request failed",
        };

        // Include response body if it contains useful error information
        if let Some(body) = response_body
            && !body.is_empty()
            && body.len() < 500
        {
            // Only include short response bodies
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(body) {
                // Try to extract error message from JSON
                if let Some(error_msg) = Self::extract_error_from_json(&json_value) {
                    return format!("{base_message}: {error_msg}");
                }
            }
            // Include raw body if it's not too long and not JSON
            if body.len() < 200 {
                return format!("{base_message}: {body}");
            }
        }

        base_message.to_string()
    }

    /// Extract error message from JSON response
    fn extract_error_from_json(json: &serde_json::Value) -> Option<String> {
        // Common error message fields in webhook APIs
        let error_fields = ["error", "message", "error_description", "detail", "details"];

        for field in &error_fields {
            if let Some(error_value) = json.get(field) {
                match error_value {
                    serde_json::Value::String(s) => return Some(s.clone()),
                    serde_json::Value::Object(obj) => {
                        // Nested error object
                        if let Some(serde_json::Value::String(s)) = obj.get("message") {
                            return Some(s.clone());
                        }
                    }
                    _ => {}
                }
            }
        }

        None
    }

    /// Validate HTTP status code and determine if retryable
    #[must_use]
    pub const fn is_retryable_status(status: StatusCode) -> bool {
        match status.as_u16() {
            // 5xx server errors and some 4xx client errors are retryable
            500..=599 | 408 | 429 => true, // Request timeout, rate limited
            // Other status codes are not retryable
            _ => false,
        }
    }

    /// Create error response from reqwest error
    #[must_use]
    pub fn error_from_reqwest_error(
        error: &reqwest::Error,
        start_time: Instant,
    ) -> NotificationResponse {
        let delivery_time_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        let (message, status_code) = if error.is_timeout() {
            ("Request timed out".to_string(), None)
        } else if error.is_connect() {
            ("Connection failed".to_string(), None)
        } else if let Some(status) = error.status() {
            (format!("HTTP error: {error}"), Some(status.as_u16()))
        } else {
            (format!("Network error: {error}"), None)
        };

        NotificationResponse::error(message, status_code, delivery_time_ms)
    }

    /// Validate response content type for JSON APIs
    ///
    /// # Errors
    ///
    /// Returns an error if the content type is invalid or unexpected.
    pub fn validate_content_type(response: &Response, expected: &str) -> NotificationResult<()> {
        if let Some(content_type) = response.headers().get("content-type") {
            let content_type_str = content_type.to_str().map_err(|e| {
                NotificationError::http_request_failed(
                    response.status().as_u16(),
                    format!("Invalid content-type header: {e}"),
                )
            })?;

            if !content_type_str.contains(expected) {
                return Err(NotificationError::http_request_failed(
                    response.status().as_u16(),
                    format!("Unexpected content-type: expected {expected}, got {content_type_str}"),
                ));
            }
        }
        // If no content-type header, assume it's acceptable
        Ok(())
    }
}

/// Response formatting utilities for consistent error and success responses
pub struct ResponseFormatter;

impl ResponseFormatter {
    /// Create a standardized success response
    #[must_use]
    pub fn success_response(
        message: impl Into<String>,
        delivery_time_ms: u64,
        response_code: Option<u16>,
        response_body: Option<String>,
    ) -> NotificationResponse {
        NotificationResponse {
            success: true,
            message: message.into(),
            response_code,
            response_body,
            delivery_time_ms,
        }
    }

    /// Create a standardized error response
    #[must_use]
    pub fn error_response(
        error: &NotificationError,
        delivery_time_ms: u64,
        response_code: Option<u16>,
    ) -> NotificationResponse {
        let mut message = error.to_string();

        // Add remediation suggestion if available
        if let Some(remediation) = error.remediation() {
            use std::fmt::Write;
            let _ = write!(message, " (Suggestion: {remediation})");
        }

        NotificationResponse {
            success: false,
            message,
            response_code,
            response_body: None,
            delivery_time_ms,
        }
    }

    /// Create error response from notification error with timing
    #[must_use]
    pub fn error_from_notification_error(
        error: &NotificationError,
        start_time: Instant,
    ) -> NotificationResponse {
        let delivery_time_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

        // Extract status code if it's an HTTP error
        let response_code = match &error {
            NotificationError::HttpRequestFailed { status, .. } => Some(*status),
            _ => None,
        };

        Self::error_response(error, delivery_time_ms, response_code)
    }

    /// Create dry-run success response
    #[must_use]
    pub fn dry_run_response(
        notification_type: &str,
        param_count: usize,
        delivery_time_ms: u64,
    ) -> NotificationResponse {
        let message = format!(
            "DRY RUN: Would send {notification_type} notification with {param_count} parameters"
        );

        NotificationResponse {
            success: true,
            message,
            response_code: Some(200), // Simulate success
            response_body: None,
            delivery_time_ms,
        }
    }

    /// Format validation error response
    #[must_use]
    pub fn validation_error_response(
        field: &str,
        reason: &str,
        start_time: Instant,
    ) -> NotificationResponse {
        let delivery_time_ms = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);
        let error = NotificationError::input_validation_failed(field, reason);
        Self::error_response(&error, delivery_time_ms, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_http_client_config() {
        let config = HttpClientConfig::new()
            .with_timeout(60)
            .with_retry_attempts(5)
            .with_user_agent("Test/1.0".to_string());

        assert_eq!(config.timeout_seconds, 60);
        assert_eq!(config.retry_attempts, 5);
        assert_eq!(config.user_agent, "Test/1.0");

        let client = config.build_client();
        assert!(client.is_ok());
    }

    #[test]
    fn test_dry_run_handler() {
        let handler = DryRunHandler::new(true);
        assert!(handler.is_enabled());

        let mut params = HashMap::new();
        params.insert("url".to_string(), "https://example.com".to_string());

        let response = handler.simulate_notification("webhook", &params);
        assert!(response.success);
        assert!(response.message.contains("DRY RUN"));
    }

    #[test]
    fn test_parameter_extractor() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;
        table.set("string_param", "test_value")?;
        table.set("int_param", 42)?;
        table.set("bool_param", true)?;

        let string_val = ParameterExtractor::extract_string(&table, "string_param");
        assert!(string_val.is_ok());
        if let Ok(val) = string_val {
            assert_eq!(val, "test_value");
        }

        let int_val = ParameterExtractor::extract_integer::<i32>(&table, "int_param");
        assert!(int_val.is_ok());
        if let Ok(val) = int_val {
            assert_eq!(val, 42);
        }

        let bool_val = ParameterExtractor::extract_boolean(&table, "bool_param");
        assert!(bool_val.is_ok());
        if let Ok(val) = bool_val {
            assert!(val);
        }

        Ok(())
    }

    #[test]
    fn test_session_manager() {
        let mut manager = SessionManager::new();
        assert!(!manager.is_changed());

        manager.set_changed(true);
        assert!(manager.is_changed());

        manager.reset();
        assert!(!manager.is_changed());
    }

    #[test]
    fn test_retry_handler_delay_calculation() {
        let handler = RetryHandler::new(3, 1000);

        let delay1 = handler.calculate_delay(1);
        let delay2 = handler.calculate_delay(2);
        let delay3 = handler.calculate_delay(3);

        // Delays should increase exponentially (with jitter)
        assert!((750..=1250).contains(&delay1)); // ~1000ms ±25%
        assert!((1500..=2500).contains(&delay2)); // ~2000ms ±25%
        assert!((3000..=5000).contains(&delay3)); // ~4000ms ±25%
    }

    #[test]
    fn test_http_response_processor_success_message() {
        let message = HttpResponseProcessor::format_success_message(200, None);
        assert_eq!(message, "Notification sent successfully");

        let message = HttpResponseProcessor::format_success_message(201, None);
        assert_eq!(message, "Notification created successfully");

        let message = HttpResponseProcessor::format_success_message(202, None);
        assert_eq!(message, "Notification accepted for processing");
    }

    #[test]
    fn test_http_response_processor_error_message() {
        let message = HttpResponseProcessor::format_error_message(400, None);
        assert_eq!(message, "Bad request - invalid notification parameters");

        let message = HttpResponseProcessor::format_error_message(401, None);
        assert_eq!(message, "Unauthorized - check authentication credentials");

        let message = HttpResponseProcessor::format_error_message(404, None);
        assert_eq!(message, "Not found - verify webhook URL");

        let body = Some("{\"error\": \"Invalid webhook URL\"}".to_string());
        let message = HttpResponseProcessor::format_error_message(400, body.as_ref());
        assert!(message.contains("Invalid webhook URL"));
    }

    #[test]
    fn test_http_response_processor_retryable_status() {
        use reqwest::StatusCode;

        // 5xx errors should be retryable
        assert!(HttpResponseProcessor::is_retryable_status(
            StatusCode::INTERNAL_SERVER_ERROR
        ));
        assert!(HttpResponseProcessor::is_retryable_status(
            StatusCode::BAD_GATEWAY
        ));
        assert!(HttpResponseProcessor::is_retryable_status(
            StatusCode::SERVICE_UNAVAILABLE
        ));

        // Some 4xx errors should be retryable
        assert!(HttpResponseProcessor::is_retryable_status(
            StatusCode::REQUEST_TIMEOUT
        ));
        assert!(HttpResponseProcessor::is_retryable_status(
            StatusCode::TOO_MANY_REQUESTS
        ));

        // Most 4xx errors should not be retryable
        assert!(!HttpResponseProcessor::is_retryable_status(
            StatusCode::BAD_REQUEST
        ));
        assert!(!HttpResponseProcessor::is_retryable_status(
            StatusCode::UNAUTHORIZED
        ));
        assert!(!HttpResponseProcessor::is_retryable_status(
            StatusCode::FORBIDDEN
        ));
        assert!(!HttpResponseProcessor::is_retryable_status(
            StatusCode::NOT_FOUND
        ));

        // 2xx and 3xx should not be retryable
        assert!(!HttpResponseProcessor::is_retryable_status(StatusCode::OK));
        assert!(!HttpResponseProcessor::is_retryable_status(
            StatusCode::CREATED
        ));
        assert!(!HttpResponseProcessor::is_retryable_status(
            StatusCode::MOVED_PERMANENTLY
        ));
    }

    #[test]
    fn test_response_formatter() {
        use std::time::Instant;

        let _start_time = Instant::now();

        // Test success response
        let response = ResponseFormatter::success_response(
            "Test success",
            100,
            Some(200),
            Some("OK".to_string()),
        );
        assert!(response.success);
        assert_eq!(response.message, "Test success");
        assert_eq!(response.response_code, Some(200));
        assert_eq!(response.delivery_time_ms, 100);

        // Test error response
        let error = NotificationError::invalid_webhook_url("bad-url");
        let response = ResponseFormatter::error_response(&error, 150, Some(400));
        assert!(!response.success);
        assert!(response.message.contains("Invalid webhook URL"));
        assert_eq!(response.response_code, Some(400));
        assert_eq!(response.delivery_time_ms, 150);

        // Test dry-run response
        let response = ResponseFormatter::dry_run_response("webhook", 5, 50);
        assert!(response.success);
        assert!(response.message.contains("DRY RUN"));
        assert!(response.message.contains("webhook"));
        assert!(response.message.contains("5 parameters"));
        assert_eq!(response.delivery_time_ms, 50);
    }

    #[test]
    fn test_extract_error_from_json() {
        // Test simple error message
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(r#"{"error": "Test error"}"#) {
            let error_msg = HttpResponseProcessor::extract_error_from_json(&json);
            assert_eq!(error_msg, Some("Test error".to_string()));
        }

        // Test nested error message
        if let Ok(json) =
            serde_json::from_str::<serde_json::Value>(r#"{"error": {"message": "Nested error"}}"#)
        {
            let error_msg = HttpResponseProcessor::extract_error_from_json(&json);
            assert_eq!(error_msg, Some("Nested error".to_string()));
        }

        // Test message field
        if let Ok(json) =
            serde_json::from_str::<serde_json::Value>(r#"{"message": "Direct message"}"#)
        {
            let error_msg = HttpResponseProcessor::extract_error_from_json(&json);
            assert_eq!(error_msg, Some("Direct message".to_string()));
        }

        // Test no error message
        if let Ok(json) = serde_json::from_str::<serde_json::Value>(r#"{"status": "ok"}"#) {
            let error_msg = HttpResponseProcessor::extract_error_from_json(&json);
            assert_eq!(error_msg, None);
        }
    }
}
