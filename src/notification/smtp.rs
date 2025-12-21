//! SMTP email notification handler

use crate::{
    args::Args,
    notification::{
        NotificationResponse,
        errors::{NotificationError, NotificationResult},
        utils::{ParameterExtractor, ResponseFormatter},
    },
};
use clap::Parser;
use lettre::{
    Address, Message, Transport,
    message::{Mailbox, MultiPart, SinglePart, header::ContentType},
    transport::smtp::{
        SmtpTransport,
        authentication::{Credentials, Mechanism},
        client::{Tls, TlsParameters},
    },
};
use mlua::{Lua, Table};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// SMTP notification parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmtpParams {
    pub smtp_server: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
    pub from: String,
    pub to: Vec<String>,
    pub cc: Option<Vec<String>>,
    pub bcc: Option<Vec<String>>,
    pub subject: String,
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    pub attachments: Option<Vec<EmailAttachment>>,
}

/// Email attachment structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAttachment {
    pub filename: String,
    pub content_type: String,
    pub content: Vec<u8>,
}

impl SmtpParams {
    /// Validate SMTP parameters
    ///
    /// # Errors
    ///
    /// Returns an error if required parameters are missing or invalid.
    pub fn validate(&self) -> NotificationResult<()> {
        // Validate SMTP server hostname
        crate::notification::validation::validate_hostname(&self.smtp_server)?;

        // Validate SMTP port
        if self.smtp_port == 0 {
            return Err(NotificationError::invalid_parameter(
                "smtp_port",
                "Port must be greater than 0",
            ));
        }

        // Validate username and password
        if self.username.trim().is_empty() {
            return Err(NotificationError::missing_parameter("username"));
        }

        if self.password.is_empty() {
            return Err(NotificationError::missing_parameter("password"));
        }

        // Validate from address
        self.validate_email_address(&self.from, "from")?;

        // Validate to addresses
        if self.to.is_empty() {
            return Err(NotificationError::missing_parameter(
                "to - at least one recipient is required",
            ));
        }

        for (i, email) in self.to.iter().enumerate() {
            self.validate_email_address(email, &format!("to[{i}]"))?;
        }

        // Validate CC addresses if provided
        if let Some(cc_addresses) = &self.cc {
            for (i, email) in cc_addresses.iter().enumerate() {
                self.validate_email_address(email, &format!("cc[{i}]"))?;
            }
        }

        // Validate BCC addresses if provided
        if let Some(bcc_addresses) = &self.bcc {
            for (i, email) in bcc_addresses.iter().enumerate() {
                self.validate_email_address(email, &format!("bcc[{i}]"))?;
            }
        }

        // Validate subject
        if self.subject.trim().is_empty() {
            return Err(NotificationError::missing_parameter("subject"));
        }

        // Ensure at least one body format is provided
        if self.body_text.is_none() && self.body_html.is_none() {
            return Err(NotificationError::missing_parameter(
                "body_text or body_html - at least one must be provided",
            ));
        }

        // Validate body content if provided
        if let Some(text) = &self.body_text {
            if text.trim().is_empty() {
                return Err(NotificationError::invalid_parameter(
                    "body_text",
                    "Text body cannot be empty",
                ));
            }
        }

        if let Some(html) = &self.body_html {
            if html.trim().is_empty() {
                return Err(NotificationError::invalid_parameter(
                    "body_html",
                    "HTML body cannot be empty",
                ));
            }
        }

        Ok(())
    }

    /// Validate email address format
    ///
    /// # Errors
    ///
    /// Returns an error if the email address is invalid.
    fn validate_email_address(&self, email: &str, field_name: &str) -> NotificationResult<()> {
        crate::notification::validation::validate_email_address(email)
            .map_err(|e| NotificationError::invalid_parameter(field_name, e.to_string()))?;
        Ok(())
    }

    /// Parse email address to lettre Address
    ///
    /// # Errors
    ///
    /// Returns an error if the email address cannot be parsed.
    fn parse_email_address(&self, email: &str) -> NotificationResult<Address> {
        email.parse::<Address>().map_err(|e| {
            NotificationError::email_format_error(format!("Invalid email '{email}': {e}"))
        })
    }

    /// Parse email address to lettre Mailbox
    ///
    /// # Errors
    ///
    /// Returns an error if the email address cannot be parsed.
    fn parse_mailbox(&self, email: &str) -> NotificationResult<Mailbox> {
        let address = self.parse_email_address(email)?;
        Ok(Mailbox::new(None, address))
    }

    /// Build email message using lettre
    ///
    /// # Errors
    ///
    /// Returns an error if message construction fails.
    fn build_message(&self) -> NotificationResult<Message> {
        let mut message_builder = Message::builder()
            .from(self.parse_mailbox(&self.from)?)
            .subject(&self.subject);

        // Add To recipients
        for to_addr in &self.to {
            message_builder = message_builder.to(self.parse_mailbox(to_addr)?);
        }

        // Add CC recipients if provided
        if let Some(cc_addresses) = &self.cc {
            for cc_addr in cc_addresses {
                message_builder = message_builder.cc(self.parse_mailbox(cc_addr)?);
            }
        }

        // Add BCC recipients if provided
        if let Some(bcc_addresses) = &self.bcc {
            for bcc_addr in bcc_addresses {
                message_builder = message_builder.bcc(self.parse_mailbox(bcc_addr)?);
            }
        }

        // Build message body based on available content
        let message = match (&self.body_text, &self.body_html) {
            (Some(text), Some(html)) => {
                // Both text and HTML - create multipart message
                let text_part = SinglePart::builder()
                    .header(ContentType::TEXT_PLAIN)
                    .body(text.clone());

                let html_part = SinglePart::builder()
                    .header(ContentType::TEXT_HTML)
                    .body(html.clone());

                let multipart = MultiPart::alternative()
                    .singlepart(text_part)
                    .singlepart(html_part);

                message_builder.multipart(multipart).map_err(|e| {
                    NotificationError::email_format_error(format!(
                        "Failed to build multipart message: {e}"
                    ))
                })?
            }
            (Some(text), None) => {
                // Text only
                message_builder
                    .header(ContentType::TEXT_PLAIN)
                    .body(text.clone())
                    .map_err(|e| {
                        NotificationError::email_format_error(format!(
                            "Failed to build text message: {e}"
                        ))
                    })?
            }
            (None, Some(html)) => {
                // HTML only
                message_builder
                    .header(ContentType::TEXT_HTML)
                    .body(html.clone())
                    .map_err(|e| {
                        NotificationError::email_format_error(format!(
                            "Failed to build HTML message: {e}"
                        ))
                    })?
            }
            (None, None) => {
                // This should be caught by validation, but handle it anyway
                return Err(NotificationError::missing_parameter(
                    "body_text or body_html - at least one must be provided",
                ));
            }
        };

        Ok(message)
    }
}

/// Send SMTP email notification
///
/// # Errors
///
/// Returns an error if SMTP connection or email sending fails.
fn send_smtp_notification(params: &SmtpParams) -> NotificationResult<NotificationResponse> {
    let start_time = Instant::now();

    // Validate parameters
    params.validate()?;

    // Build email message
    let message = params.build_message()?;

    // Create SMTP transport with authentication
    let credentials = Credentials::new(params.username.clone(), params.password.clone());

    // Configure TLS parameters
    let tls_parameters = TlsParameters::new(params.smtp_server.clone()).map_err(|e| {
        NotificationError::smtp_connection_failed(format!("TLS configuration failed: {e}"))
    })?;

    // Build SMTP transport
    let transport = SmtpTransport::relay(&params.smtp_server)
        .map_err(|e| {
            NotificationError::smtp_connection_failed(format!("Failed to create SMTP relay: {e}"))
        })?
        .port(params.smtp_port)
        .credentials(credentials)
        .authentication(vec![Mechanism::Plain, Mechanism::Login, Mechanism::Xoauth2])
        .tls(Tls::Required(tls_parameters))
        .build();

    // Send email
    let result = transport.send(&message);

    let delivery_time = u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX);

    match result {
        Ok(response) => {
            let total_recipients = params.to.len()
                + params.cc.as_ref().map_or(0, |cc| cc.len())
                + params.bcc.as_ref().map_or(0, |bcc| bcc.len());

            let response_message = response.message().next().unwrap_or_default();

            let message = format!(
                "Email sent successfully to {total_recipients} recipients. SMTP response: {response_message}"
            );

            Ok(NotificationResponse::success(
                message,
                Some(250), // SMTP success code
                delivery_time,
            ))
        }
        Err(error) => Err(NotificationError::smtp_connection_failed(format!(
            "Failed to send email: {error}"
        ))),
    }
}

/// SMTP email notification function for Lua
///
/// This function is called from Lua scripts to send email notifications.
/// It extracts parameters from the Lua table, validates them, and sends the email.
///
/// # Errors
///
/// Returns an error if parameter extraction or email sending fails.
pub fn smtp(lua: &Lua, params: Table) -> mlua::Result<Table> {
    // Extract parameters from Lua table
    let smtp_server = ParameterExtractor::extract_string(&params, "smtp_server")
        .map_err(|e| mlua::Error::external(e))?;

    let smtp_port = ParameterExtractor::extract_integer::<u16>(&params, "smtp_port")
        .map_err(|e| mlua::Error::external(e))?;

    let username = ParameterExtractor::extract_string(&params, "username")
        .map_err(|e| mlua::Error::external(e))?;

    let password = ParameterExtractor::extract_string(&params, "password")
        .map_err(|e| mlua::Error::external(e))?;

    let from = ParameterExtractor::extract_string(&params, "from")
        .map_err(|e| mlua::Error::external(e))?;

    let to = ParameterExtractor::extract_string_array(&params, "to")
        .map_err(|e| mlua::Error::external(e))?;

    let cc = ParameterExtractor::extract_optional_string_array(&params, "cc")
        .map_err(|e| mlua::Error::external(e))?;

    let bcc = ParameterExtractor::extract_optional_string_array(&params, "bcc")
        .map_err(|e| mlua::Error::external(e))?;

    let subject = ParameterExtractor::extract_string(&params, "subject")
        .map_err(|e| mlua::Error::external(e))?;

    let body_text = ParameterExtractor::extract_optional_string(&params, "body_text")
        .map_err(|e| mlua::Error::external(e))?;

    let body_html = ParameterExtractor::extract_optional_string(&params, "body_html")
        .map_err(|e| mlua::Error::external(e))?;

    // Note: Attachments are not implemented in this task
    let attachments = None;

    let smtp_params = SmtpParams {
        smtp_server,
        smtp_port,
        username,
        password,
        from,
        to,
        cc,
        bcc,
        subject,
        body_text,
        body_html,
        attachments,
    };

    // Check for dry-run mode from global Args
    let dry_run = Args::parse().flags.dry_run;

    // Handle dry-run mode
    if dry_run {
        let start_time = Instant::now();

        // Validate parameters even in dry-run mode
        if let Err(error) = smtp_params.validate() {
            let response = ResponseFormatter::error_from_notification_error(&error, start_time);
            return response.to_lua_table(lua);
        }

        let param_count = 5 + // smtp_server, smtp_port, username, from, subject
            smtp_params.to.len() +
            smtp_params.cc.as_ref().map_or(0, |cc| cc.len()) +
            smtp_params.bcc.as_ref().map_or(0, |bcc| bcc.len()) +
            if smtp_params.body_text.is_some() { 1 } else { 0 } +
            if smtp_params.body_html.is_some() { 1 } else { 0 };

        let response = ResponseFormatter::dry_run_response(
            "SMTP",
            param_count,
            u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        return response.to_lua_table(lua);
    }

    // Send email synchronously
    let result = send_smtp_notification(&smtp_params);

    match result {
        Ok(response) => response.to_lua_table(lua),
        Err(error) => {
            let start_time = Instant::now();
            let error_response =
                ResponseFormatter::error_from_notification_error(&error, start_time);
            error_response.to_lua_table(lua)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mlua::Lua;

    #[test]
    fn test_smtp_params_validation_valid() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: None,
            bcc: None,
            subject: "Test Subject".to_string(),
            body_text: Some("Test body".to_string()),
            body_html: None,
            attachments: None,
        };

        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_smtp_params_validation_invalid_email() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "invalid-email".to_string(), // Invalid email
            to: vec!["recipient@example.com".to_string()],
            cc: None,
            bcc: None,
            subject: "Test Subject".to_string(),
            body_text: Some("Test body".to_string()),
            body_html: None,
            attachments: None,
        };

        assert!(params.validate().is_err());
    }

    #[test]
    fn test_smtp_params_validation_missing_body() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: None,
            bcc: None,
            subject: "Test Subject".to_string(),
            body_text: None, // Missing body
            body_html: None, // Missing body
            attachments: None,
        };

        assert!(params.validate().is_err());
    }

    #[test]
    fn test_smtp_params_validation_empty_recipients() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "sender@example.com".to_string(),
            to: vec![], // Empty recipients
            cc: None,
            bcc: None,
            subject: "Test Subject".to_string(),
            body_text: Some("Test body".to_string()),
            body_html: None,
            attachments: None,
        };

        assert!(params.validate().is_err());
    }

    #[test]
    fn test_smtp_params_validation_invalid_port() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 0, // Invalid port
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: None,
            bcc: None,
            subject: "Test Subject".to_string(),
            body_text: Some("Test body".to_string()),
            body_html: None,
            attachments: None,
        };

        assert!(params.validate().is_err());
    }

    #[test]
    fn test_smtp_params_build_message_text_only() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: None,
            bcc: None,
            subject: "Test Subject".to_string(),
            body_text: Some("Test body content".to_string()),
            body_html: None,
            attachments: None,
        };

        let message = params.build_message();
        assert!(message.is_ok());
    }

    #[test]
    fn test_smtp_params_build_message_html_only() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: None,
            bcc: None,
            subject: "Test Subject".to_string(),
            body_text: None,
            body_html: Some("<h1>Test HTML</h1>".to_string()),
            attachments: None,
        };

        let message = params.build_message();
        assert!(message.is_ok());
    }

    #[test]
    fn test_smtp_params_build_message_multipart() {
        let params = SmtpParams {
            smtp_server: "smtp.gmail.com".to_string(),
            smtp_port: 587,
            username: "test@example.com".to_string(),
            password: "password123".to_string(),
            from: "sender@example.com".to_string(),
            to: vec!["recipient@example.com".to_string()],
            cc: Some(vec!["cc@example.com".to_string()]),
            bcc: Some(vec!["bcc@example.com".to_string()]),
            subject: "Test Subject".to_string(),
            body_text: Some("Test body content".to_string()),
            body_html: Some("<h1>Test HTML</h1>".to_string()),
            attachments: None,
        };

        let message = params.build_message();
        assert!(message.is_ok());
    }

    #[test]
    fn test_smtp_lua_parameter_extraction() -> mlua::Result<()> {
        let lua = Lua::new();
        let table = lua.create_table()?;

        // Set up valid SMTP parameters
        table.set("smtp_server", "smtp.gmail.com")?;
        table.set("smtp_port", 587)?;
        table.set("username", "test@example.com")?;
        table.set("password", "password123")?;
        table.set("from", "sender@example.com")?;

        let to_array = lua.create_table()?;
        to_array.set(1, "recipient@example.com")?;
        table.set("to", to_array)?;

        table.set("subject", "Test Subject")?;
        table.set("body_text", "Test body content")?;

        // Test parameter extraction (this would normally be done inside the smtp function)
        let smtp_server = ParameterExtractor::extract_string(&table, "smtp_server");
        assert!(smtp_server.is_ok());
        if let Ok(server) = smtp_server {
            assert_eq!(server, "smtp.gmail.com");
        }

        let smtp_port = ParameterExtractor::extract_integer::<u16>(&table, "smtp_port");
        assert!(smtp_port.is_ok());
        if let Ok(port) = smtp_port {
            assert_eq!(port, 587);
        }

        let to_addresses = ParameterExtractor::extract_string_array(&table, "to");
        assert!(to_addresses.is_ok());
        if let Ok(addresses) = to_addresses {
            assert_eq!(addresses, vec!["recipient@example.com"]);
        }

        Ok(())
    }
}
