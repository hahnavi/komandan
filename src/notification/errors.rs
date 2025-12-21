//! Error types for the notification system

use thiserror::Error;

/// Notification system error types
#[derive(Debug, Clone, Error)]
pub enum NotificationError {
    #[error("Invalid webhook URL: {url}")]
    InvalidWebhookUrl { url: String },

    #[error("HTTP request failed: {status} - {message}")]
    HttpRequestFailed { status: u16, message: String },

    #[error("SMTP connection failed: {details}")]
    SmtpConnectionFailed { details: String },

    #[error("Template rendering failed: {template_error}")]
    TemplateRenderingFailed { template_error: String },

    #[error("Input validation failed: {field} - {reason}")]
    InputValidationFailed { field: String, reason: String },

    #[error("Authentication failed: {provider}")]
    AuthenticationFailed { provider: String },

    #[error("Timeout exceeded: {timeout_ms}ms")]
    TimeoutExceeded { timeout_ms: u64 },

    #[error("Network error: {details}")]
    NetworkError { details: String },

    #[error("JSON serialization failed: {details}")]
    JsonSerializationFailed { details: String },

    #[error("Email format error: {details}")]
    EmailFormatError { details: String },

    #[error("Missing required parameter: {parameter}")]
    MissingParameter { parameter: String },

    #[error("Invalid parameter value: {parameter} - {reason}")]
    InvalidParameter { parameter: String, reason: String },

    #[error("Configuration error: {details}")]
    ConfigurationError { details: String },

    #[error("Internal error: {details}")]
    InternalError { details: String },
}

impl NotificationError {
    /// Create an invalid webhook URL error
    pub fn invalid_webhook_url(url: impl Into<String>) -> Self {
        Self::InvalidWebhookUrl { url: url.into() }
    }

    /// Create an HTTP request failed error
    pub fn http_request_failed(status: u16, message: impl Into<String>) -> Self {
        Self::HttpRequestFailed {
            status,
            message: message.into(),
        }
    }

    /// Create an SMTP connection failed error
    pub fn smtp_connection_failed(details: impl Into<String>) -> Self {
        Self::SmtpConnectionFailed {
            details: details.into(),
        }
    }

    /// Create a template rendering failed error
    pub fn template_rendering_failed(template_error: impl Into<String>) -> Self {
        Self::TemplateRenderingFailed {
            template_error: template_error.into(),
        }
    }

    /// Create an input validation failed error
    pub fn input_validation_failed(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InputValidationFailed {
            field: field.into(),
            reason: reason.into(),
        }
    }

    /// Create an authentication failed error
    pub fn authentication_failed(provider: impl Into<String>) -> Self {
        Self::AuthenticationFailed {
            provider: provider.into(),
        }
    }

    /// Create a timeout exceeded error
    #[must_use]
    pub const fn timeout_exceeded(timeout_ms: u64) -> Self {
        Self::TimeoutExceeded { timeout_ms }
    }

    /// Create a network error
    pub fn network_error(details: impl Into<String>) -> Self {
        Self::NetworkError {
            details: details.into(),
        }
    }

    /// Create a JSON serialization failed error
    pub fn json_serialization_failed(details: impl Into<String>) -> Self {
        Self::JsonSerializationFailed {
            details: details.into(),
        }
    }

    /// Create an email format error
    pub fn email_format_error(details: impl Into<String>) -> Self {
        Self::EmailFormatError {
            details: details.into(),
        }
    }

    /// Create a missing parameter error
    pub fn missing_parameter(parameter: impl Into<String>) -> Self {
        Self::MissingParameter {
            parameter: parameter.into(),
        }
    }

    /// Create an invalid parameter error
    pub fn invalid_parameter(parameter: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidParameter {
            parameter: parameter.into(),
            reason: reason.into(),
        }
    }

    /// Create a configuration error
    pub fn configuration_error(details: impl Into<String>) -> Self {
        Self::ConfigurationError {
            details: details.into(),
        }
    }

    /// Create an internal error
    pub fn internal_error(details: impl Into<String>) -> Self {
        Self::InternalError {
            details: details.into(),
        }
    }

    /// Get the error category for logging and metrics
    #[must_use]
    pub const fn category(&self) -> &'static str {
        match self {
            Self::InvalidWebhookUrl { .. }
            | Self::InputValidationFailed { .. }
            | Self::MissingParameter { .. }
            | Self::InvalidParameter { .. } => "validation",
            Self::HttpRequestFailed { .. } => "http",
            Self::SmtpConnectionFailed { .. } => "smtp",
            Self::TemplateRenderingFailed { .. } => "template",
            Self::AuthenticationFailed { .. } => "auth",
            Self::TimeoutExceeded { .. } => "timeout",
            Self::NetworkError { .. } => "network",
            Self::JsonSerializationFailed { .. } => "serialization",
            Self::EmailFormatError { .. } => "email",
            Self::ConfigurationError { .. } => "config",
            Self::InternalError { .. } => "internal",
        }
    }

    /// Check if the error is retryable
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        match self {
            Self::HttpRequestFailed { status, .. } => {
                // Retry on 5xx server errors and some 4xx client errors
                *status >= 500 || *status == 408 || *status == 429
            }
            Self::NetworkError { .. }
            | Self::TimeoutExceeded { .. }
            | Self::SmtpConnectionFailed { .. } => true,
            _ => false,
        }
    }

    /// Get suggested remediation for the error
    #[must_use]
    pub const fn remediation(&self) -> Option<&'static str> {
        match self {
            Self::InvalidWebhookUrl { .. } => {
                Some("Verify the webhook URL format and ensure it starts with https://")
            }
            Self::HttpRequestFailed { status, .. } => match *status {
                401 => Some("Check authentication credentials"),
                403 => Some("Verify permissions and API access"),
                404 => Some("Confirm the webhook URL is correct"),
                429 => Some("Reduce request rate or implement backoff"),
                _ => None,
            },
            Self::SmtpConnectionFailed { .. } => {
                Some("Verify SMTP server settings, credentials, and network connectivity")
            }
            Self::AuthenticationFailed { .. } => {
                Some("Check username, password, and authentication method")
            }
            Self::InputValidationFailed { .. } => {
                Some("Review parameter format and allowed characters")
            }
            Self::MissingParameter { .. } => {
                Some("Provide all required parameters for the notification type")
            }
            _ => None,
        }
    }
}

/// Convert reqwest errors to notification errors
impl From<reqwest::Error> for NotificationError {
    fn from(error: reqwest::Error) -> Self {
        if error.is_timeout() {
            Self::timeout_exceeded(30000) // Default timeout assumption
        } else if error.is_connect() {
            Self::network_error(format!("Connection failed: {error}"))
        } else if let Some(status) = error.status() {
            Self::http_request_failed(status.as_u16(), error.to_string())
        } else {
            Self::network_error(error.to_string())
        }
    }
}

/// Convert lettre errors to notification errors
impl From<lettre::transport::smtp::Error> for NotificationError {
    fn from(error: lettre::transport::smtp::Error) -> Self {
        Self::smtp_connection_failed(error.to_string())
    }
}

/// Convert lettre address errors to notification errors
impl From<lettre::address::AddressError> for NotificationError {
    fn from(error: lettre::address::AddressError) -> Self {
        Self::email_format_error(error.to_string())
    }
}

/// Convert serde JSON errors to notification errors
impl From<serde_json::Error> for NotificationError {
    fn from(error: serde_json::Error) -> Self {
        Self::json_serialization_failed(error.to_string())
    }
}

/// Convert URL parsing errors to notification errors
impl From<url::ParseError> for NotificationError {
    fn from(error: url::ParseError) -> Self {
        Self::invalid_webhook_url(error.to_string())
    }
}

/// Convert minijinja errors to notification errors
impl From<minijinja::Error> for NotificationError {
    fn from(error: minijinja::Error) -> Self {
        Self::template_rendering_failed(error.to_string())
    }
}

/// Result type alias for notification operations
pub type NotificationResult<T> = Result<T, NotificationError>;
