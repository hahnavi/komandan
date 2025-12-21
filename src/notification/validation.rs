//! Input validation utilities for notifications

use crate::notification::errors::{NotificationError, NotificationResult};
use regex::Regex;
use std::sync::LazyLock;
use url::Url;

/// Compiled regex for email validation (RFC 5322 compliant)
static EMAIL_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-zA-Z0-9.!#$%&'*+/=?^_`{|}~-]+@[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(?:\.[a-zA-Z0-9](?:[a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$")
        .unwrap_or_else(|_| panic!("Email regex should be valid"))
});

/// Compiled regex for hostname validation
static HOSTNAME_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9-]{0,61}[a-zA-Z0-9])?)*$")
        .unwrap_or_else(|_| panic!("Hostname regex should be valid"))
});

/// Validate webhook URL format and security
///
/// # Errors
///
/// Returns an error if the URL is invalid, not HTTPS, or contains suspicious patterns.
pub fn validate_webhook_url(url: &str) -> NotificationResult<Url> {
    // Parse URL
    let parsed_url = Url::parse(url).map_err(|_| {
        NotificationError::invalid_webhook_url(format!("Invalid URL format: {url}"))
    })?;

    // Ensure HTTPS for security
    if parsed_url.scheme() != "https" {
        return Err(NotificationError::invalid_webhook_url(
            "Webhook URLs must use HTTPS".to_string(),
        ));
    }

    // Check for valid host
    if parsed_url.host_str().is_none() {
        return Err(NotificationError::invalid_webhook_url(
            "URL must have a valid host".to_string(),
        ));
    }

    // Reject localhost and private IP ranges for security
    if let Some(host) = parsed_url.host_str()
        && (host == "localhost"
            || host == "127.0.0.1"
            || host.starts_with("192.168.")
            || host.starts_with("10."))
    {
        return Err(NotificationError::invalid_webhook_url(
            "Private and localhost URLs are not allowed".to_string(),
        ));
    }

    Ok(parsed_url)
}

/// Validate email address format
///
/// # Errors
///
/// Returns an error if the email address format is invalid.
pub fn validate_email_address(email: &str) -> NotificationResult<()> {
    if email.is_empty() {
        return Err(NotificationError::input_validation_failed(
            "email",
            "Email address cannot be empty",
        ));
    }

    if email.len() > 254 {
        return Err(NotificationError::input_validation_failed(
            "email",
            "Email address too long (max 254 characters)",
        ));
    }

    if !EMAIL_REGEX.is_match(email) {
        return Err(NotificationError::input_validation_failed(
            "email",
            "Invalid email address format",
        ));
    }

    Ok(())
}

/// Validate SMTP server hostname
///
/// # Errors
///
/// Returns an error if the hostname format is invalid.
pub fn validate_hostname(hostname: &str) -> NotificationResult<()> {
    if hostname.is_empty() {
        return Err(NotificationError::input_validation_failed(
            "hostname",
            "Hostname cannot be empty",
        ));
    }

    if hostname.len() > 253 {
        return Err(NotificationError::input_validation_failed(
            "hostname",
            "Hostname too long (max 253 characters)",
        ));
    }

    if !HOSTNAME_REGEX.is_match(hostname) {
        return Err(NotificationError::input_validation_failed(
            "hostname",
            "Invalid hostname format",
        ));
    }

    Ok(())
}

/// Validate SMTP port number
///
/// # Errors
///
/// Returns an error if the port is not in valid range.
pub fn validate_smtp_port(port: u16) -> NotificationResult<()> {
    // Common SMTP ports: 25, 465, 587, 2525
    // Allow any port in valid range but warn about uncommon ones
    if port == 0 {
        return Err(NotificationError::input_validation_failed(
            "port",
            "Port cannot be 0",
        ));
    }

    Ok(())
}

/// Sanitize text content for notifications
///
/// Removes potentially dangerous characters while preserving readability.
#[must_use]
pub fn sanitize_text_content(content: &str) -> String {
    // Remove control characters except newlines and tabs
    content
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect()
}

/// Sanitize parameter names and values using allowlist approach
///
/// # Errors
///
/// Returns an error if the parameter contains disallowed characters.
pub fn sanitize_parameter(param_name: &str, value: &str) -> NotificationResult<String> {
    if value.is_empty() {
        return Err(NotificationError::input_validation_failed(
            param_name,
            "Parameter cannot be empty",
        ));
    }

    // Allowlist: alphanumeric, spaces, common punctuation, but no shell metacharacters
    let sanitized: String = value
        .chars()
        .filter(|c| {
            c.is_alphanumeric()
                || c.is_whitespace()
                || matches!(
                    *c,
                    '.' | '-' | '_' | '@' | ':' | '/' | '=' | '+' | ',' | '(' | ')' | '[' | ']'
                )
        })
        .collect();

    if sanitized.is_empty() {
        return Err(NotificationError::input_validation_failed(
            param_name,
            "Parameter contains only invalid characters",
        ));
    }

    if sanitized != value {
        return Err(NotificationError::input_validation_failed(
            param_name,
            "Parameter contains disallowed characters",
        ));
    }

    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_webhook_url_valid() {
        let result = validate_webhook_url(
            "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_webhook_url_http_rejected() {
        let result = validate_webhook_url("http://example.com/webhook");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_webhook_url_localhost_rejected() {
        let result = validate_webhook_url("https://localhost/webhook");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_email_address_valid() {
        assert!(validate_email_address("user@example.com").is_ok());
        assert!(validate_email_address("test.email+tag@domain.co.uk").is_ok());
    }

    #[test]
    fn test_validate_email_address_invalid() {
        assert!(validate_email_address("").is_err());
        assert!(validate_email_address("invalid-email").is_err());
        assert!(validate_email_address("@domain.com").is_err());
    }

    #[test]
    fn test_validate_hostname_valid() {
        assert!(validate_hostname("smtp.gmail.com").is_ok());
        assert!(validate_hostname("mail.example.org").is_ok());
    }

    #[test]
    fn test_validate_hostname_invalid() {
        assert!(validate_hostname("").is_err());
        assert!(validate_hostname("-invalid.com").is_err());
    }

    #[test]
    fn test_sanitize_text_content() {
        let input = "Hello\x00World\nTest\t";
        let result = sanitize_text_content(input);
        assert_eq!(result, "HelloWorld\nTest\t");
    }

    #[test]
    fn test_sanitize_parameter_valid() {
        let result = sanitize_parameter("test", "valid-parameter_123");
        assert!(result.is_ok());
        if let Ok(val) = result {
            assert_eq!(val, "valid-parameter_123");
        }
    }

    #[test]
    fn test_sanitize_parameter_invalid() {
        let result = sanitize_parameter("test", "invalid;parameter");
        assert!(result.is_err());
    }
}
