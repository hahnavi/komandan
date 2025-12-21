//! Slack webhook notification handler

use crate::{
    args::Args,
    notification::{
        NotificationResponse,
        errors::{NotificationError, NotificationResult},
        utils::{ParameterExtractor, ResponseFormatter},
    },
};
use clap::Parser;
use mlua::{Lua, Table};
use serde::{Deserialize, Serialize};
use std::time::Instant;

/// Slack notification parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackParams {
    pub webhook_url: String,
    pub text: Option<String>,
    pub blocks: Option<Vec<SlackBlock>>,
    pub channel: Option<String>,
    pub username: Option<String>,
    pub icon_emoji: Option<String>,
    pub icon_url: Option<String>,
    pub unfurl_links: Option<bool>,
    pub unfurl_media: Option<bool>,
}

/// Slack Block Kit structure following Slack Block Kit specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SlackBlock {
    #[serde(rename = "section")]
    Section {
        #[serde(skip_serializing_if = "Option::is_none")]
        text: Option<SlackText>,
        #[serde(skip_serializing_if = "Option::is_none")]
        fields: Option<Vec<SlackText>>,
        #[serde(skip_serializing_if = "Option::is_none")]
        accessory: Option<SlackElement>,
    },
    #[serde(rename = "divider")]
    Divider,
    #[serde(rename = "header")]
    Header { text: SlackText },
    #[serde(rename = "context")]
    Context { elements: Vec<SlackContextElement> },
    #[serde(rename = "actions")]
    Actions { elements: Vec<SlackElement> },
}

/// Slack text object
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SlackText {
    #[serde(rename = "type")]
    pub text_type: SlackTextType,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emoji: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbatim: Option<bool>,
}

/// Slack text types
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SlackTextType {
    PlainText,
    Mrkdwn,
}

/// Slack context elements (simplified)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SlackContextElement {
    #[serde(rename = "plain_text")]
    PlainText { text: String },
    #[serde(rename = "mrkdwn")]
    Mrkdwn { text: String },
    #[serde(rename = "image")]
    Image { image_url: String, alt_text: String },
}

/// Slack interactive elements (simplified)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SlackElement {
    #[serde(rename = "button")]
    Button {
        text: SlackText,
        #[serde(skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        action_id: Option<String>,
    },
    #[serde(rename = "image")]
    Image { image_url: String, alt_text: String },
}

/// Slack message payload structure
#[derive(Debug, Clone, Serialize)]
struct SlackMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    blocks: Option<Vec<SlackBlock>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_emoji: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    icon_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unfurl_links: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    unfurl_media: Option<bool>,
}

impl SlackParams {
    /// Validate Slack parameters
    ///
    /// # Errors
    ///
    /// Returns an error if required parameters are missing or invalid.
    pub fn validate(&self) -> NotificationResult<()> {
        // Validate webhook URL
        crate::notification::validation::validate_webhook_url(&self.webhook_url)?;

        // Ensure at least text or blocks are provided
        if self.text.is_none() && self.blocks.is_none() {
            return Err(NotificationError::missing_parameter(
                "text or blocks - at least one must be provided",
            ));
        }

        // Validate text content if provided
        if let Some(text) = &self.text {
            let sanitized_text = crate::notification::validation::sanitize_text_content(text);
            if sanitized_text.trim().is_empty() {
                return Err(NotificationError::invalid_parameter(
                    "text",
                    "Text content cannot be empty after sanitization",
                ));
            }
        }

        // Validate blocks if provided
        if let Some(blocks) = &self.blocks {
            if blocks.is_empty() {
                return Err(NotificationError::invalid_parameter(
                    "blocks",
                    "Blocks array cannot be empty",
                ));
            }

            for (i, block) in blocks.iter().enumerate() {
                block
                    .validate()
                    .map_err(|e| NotificationError::invalid_parameter(format!("blocks[{i}]"), e))?;
            }
        }

        // Validate channel format if provided (must start with # or @)
        if let Some(channel) = &self.channel
            && !channel.is_empty()
            && !channel.starts_with('#')
            && !channel.starts_with('@')
        {
            return Err(NotificationError::invalid_parameter(
                "channel",
                "Channel must start with # for channels or @ for users",
            ));
        }

        // Validate username if provided (no special characters)
        if let Some(username) = &self.username {
            if username.trim().is_empty() {
                return Err(NotificationError::invalid_parameter(
                    "username",
                    "Username cannot be empty",
                ));
            }

            // Sanitize username to ensure it contains only allowed characters
            crate::notification::validation::sanitize_parameter("username", username)?;
        }

        // Validate icon_emoji format if provided (must be :emoji:)
        if let Some(icon_emoji) = &self.icon_emoji
            && (!icon_emoji.starts_with(':') || !icon_emoji.ends_with(':'))
        {
            return Err(NotificationError::invalid_parameter(
                "icon_emoji",
                "Icon emoji must be in format :emoji_name:",
            ));
        }

        // Validate icon_url if provided
        if let Some(icon_url) = &self.icon_url {
            crate::notification::validation::validate_webhook_url(icon_url)?;
        }

        Ok(())
    }

    /// Convert to Slack message payload with proper escaping
    fn to_message(&self) -> SlackMessage {
        SlackMessage {
            text: self.text.as_ref().map(|t| escape_slack_text(t)),
            blocks: self.blocks.clone(),
            channel: self.channel.clone(),
            username: self.username.clone(),
            icon_emoji: self.icon_emoji.clone(),
            icon_url: self.icon_url.clone(),
            unfurl_links: self.unfurl_links,
            unfurl_media: self.unfurl_media,
        }
    }
}

impl SlackBlock {
    /// Validate Slack block structure
    ///
    /// # Errors
    ///
    /// Returns an error if the block structure is invalid.
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Section { text, fields, .. } => {
                // Section must have either text or fields
                if text.is_none() && fields.is_none() {
                    return Err("Section block must have either text or fields".to_string());
                }

                if let Some(text_obj) = text {
                    text_obj.validate()?;
                }

                if let Some(fields_vec) = fields {
                    if fields_vec.is_empty() {
                        return Err("Section fields array cannot be empty".to_string());
                    }
                    if fields_vec.len() > 10 {
                        return Err("Section can have maximum 10 fields".to_string());
                    }
                    for (i, field) in fields_vec.iter().enumerate() {
                        field.validate().map_err(|e| format!("Field {i}: {e}"))?;
                    }
                }
            }
            Self::Divider => {
                // Divider blocks have no content to validate
            }
            Self::Header { text } => {
                if !matches!(text.text_type, SlackTextType::PlainText) {
                    return Err("Header text must be plain_text type".to_string());
                }
                text.validate()?;
            }
            Self::Context { elements } => {
                if elements.is_empty() {
                    return Err("Context block must have at least one element".to_string());
                }
                if elements.len() > 10 {
                    return Err("Context block can have maximum 10 elements".to_string());
                }
                for (i, element) in elements.iter().enumerate() {
                    element
                        .validate()
                        .map_err(|e| format!("Context element {i}: {e}"))?;
                }
            }
            Self::Actions { elements } => {
                if elements.is_empty() {
                    return Err("Actions block must have at least one element".to_string());
                }
                if elements.len() > 5 {
                    return Err("Actions block can have maximum 5 elements".to_string());
                }
                for (i, element) in elements.iter().enumerate() {
                    element
                        .validate()
                        .map_err(|e| format!("Action element {i}: {e}"))?;
                }
            }
        }
        Ok(())
    }
}

impl SlackText {
    /// Validate Slack text object
    ///
    /// # Errors
    ///
    /// Returns an error if the text object is invalid.
    fn validate(&self) -> Result<(), String> {
        if self.text.trim().is_empty() {
            return Err("Text content cannot be empty".to_string());
        }

        // Check text length limits
        match self.text_type {
            SlackTextType::PlainText => {
                if self.text.len() > 3000 {
                    return Err("Plain text cannot exceed 3000 characters".to_string());
                }
            }
            SlackTextType::Mrkdwn => {
                if self.text.len() > 3000 {
                    return Err("Markdown text cannot exceed 3000 characters".to_string());
                }
            }
        }

        Ok(())
    }
}

impl SlackContextElement {
    /// Validate Slack context element
    ///
    /// # Errors
    ///
    /// Returns an error if the context element is invalid.
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::PlainText { text } | Self::Mrkdwn { text } => {
                if text.trim().is_empty() {
                    return Err("Context text cannot be empty".to_string());
                }
                if text.len() > 75 {
                    return Err("Context text cannot exceed 75 characters".to_string());
                }
            }
            Self::Image {
                image_url,
                alt_text,
            } => {
                if image_url.trim().is_empty() {
                    return Err("Image URL cannot be empty".to_string());
                }
                if alt_text.trim().is_empty() {
                    return Err("Image alt text cannot be empty".to_string());
                }
                if alt_text.len() > 75 {
                    return Err("Image alt text cannot exceed 75 characters".to_string());
                }
            }
        }
        Ok(())
    }
}

impl SlackElement {
    /// Validate Slack element
    ///
    /// # Errors
    ///
    /// Returns an error if the element is invalid.
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::Button { text, .. } => {
                if !matches!(text.text_type, SlackTextType::PlainText) {
                    return Err("Button text must be plain_text type".to_string());
                }
                text.validate()?;
                if text.text.len() > 75 {
                    return Err("Button text cannot exceed 75 characters".to_string());
                }
            }
            Self::Image {
                image_url,
                alt_text,
            } => {
                if image_url.trim().is_empty() {
                    return Err("Image URL cannot be empty".to_string());
                }
                if alt_text.trim().is_empty() {
                    return Err("Image alt text cannot be empty".to_string());
                }
                if alt_text.len() > 2000 {
                    return Err("Image alt text cannot exceed 2000 characters".to_string());
                }
            }
        }
        Ok(())
    }
}

/// Escape special characters in Slack text
///
/// Slack requires escaping of certain characters to prevent formatting issues
/// and security vulnerabilities.
fn escape_slack_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Send Slack webhook notification with retry logic
///
/// # Errors
///
/// Returns an error if the HTTP request fails or parameters are invalid.
async fn send_slack_notification(params: &SlackParams) -> NotificationResult<NotificationResponse> {
    // Validate parameters
    params.validate()?;

    // Create HTTP client with optimized webhook configuration
    let http_config = crate::notification::utils::create_webhook_http_config();

    let client = http_config.build_client()?;
    let retry_handler = http_config.create_retry_handler();

    // Create message payload with proper escaping
    let message = params.to_message();
    let webhook_url = params.webhook_url.clone();

    // Send HTTP POST request with retry logic
    let start_time = Instant::now();

    let result = retry_handler
        .retry_with_backoff(|| async {
            let request_start = Instant::now();

            match client.post(&webhook_url).json(&message).send().await {
                Ok(response) => {
                    // Process successful HTTP response
                    crate::notification::utils::HttpResponseProcessor::process_response(
                        response,
                        request_start,
                    )
                    .await
                }
                Err(reqwest_error) => {
                    // Convert reqwest error to notification error
                    Err(NotificationError::from(reqwest_error))
                }
            }
        })
        .await;

    match result {
        Ok(response) => Ok(response),
        Err(error) => {
            // Convert final error to response
            Ok(ResponseFormatter::error_from_notification_error(
                &error, start_time,
            ))
        }
    }
}

/// Slack webhook notification function for Lua
///
/// This function is called from Lua scripts to send Slack notifications.
/// It extracts parameters from the Lua table, validates them, and sends the notification.
///
/// # Errors
///
/// Returns an error if parameter extraction or notification sending fails.
pub fn slack_webhook(lua: &Lua, params: Table) -> mlua::Result<Table> {
    // Extract parameters from Lua table
    let webhook_url = ParameterExtractor::extract_string(&params, "webhook_url")
        .map_err(mlua::Error::external)?;

    let text = ParameterExtractor::extract_optional_string(&params, "text")
        .map_err(mlua::Error::external)?;

    let channel = ParameterExtractor::extract_optional_string(&params, "channel")
        .map_err(mlua::Error::external)?;

    let username = ParameterExtractor::extract_optional_string(&params, "username")
        .map_err(mlua::Error::external)?;

    let icon_emoji = ParameterExtractor::extract_optional_string(&params, "icon_emoji")
        .map_err(mlua::Error::external)?;

    let icon_url = ParameterExtractor::extract_optional_string(&params, "icon_url")
        .map_err(mlua::Error::external)?;

    let unfurl_links = ParameterExtractor::extract_boolean(&params, "unfurl_links").ok();

    let unfurl_media = ParameterExtractor::extract_boolean(&params, "unfurl_media").ok();

    // Extract blocks if provided (simplified for now - full block parsing would be more complex)
    let blocks = if let Ok(mlua::Value::Table(_blocks_table)) = params.get("blocks") {
        // For now, we'll accept blocks as None and focus on text messages
        // Full block parsing implementation would require recursive table parsing
        None
    } else {
        None
    };

    let slack_params = SlackParams {
        webhook_url,
        text,
        blocks,
        channel,
        username,
        icon_emoji,
        icon_url,
        unfurl_links,
        unfurl_media,
    };

    // Check for dry-run mode from global Args
    let dry_run = Args::parse().flags.dry_run;

    // Handle dry-run mode
    if dry_run {
        let start_time = Instant::now();

        // Validate parameters even in dry-run mode
        if let Err(error) = slack_params.validate() {
            let response = ResponseFormatter::error_from_notification_error(&error, start_time);
            return response.to_lua_table(lua);
        }

        let param_count = 1 + // webhook_url
            usize::from(slack_params.text.is_some()) +
            usize::from(slack_params.blocks.is_some()) +
            usize::from(slack_params.channel.is_some()) +
            usize::from(slack_params.username.is_some()) +
            usize::from(slack_params.icon_emoji.is_some()) +
            usize::from(slack_params.icon_url.is_some());

        let response = ResponseFormatter::dry_run_response(
            "Slack",
            param_count,
            u64::try_from(start_time.elapsed().as_millis()).unwrap_or(u64::MAX),
        );
        return response.to_lua_table(lua);
    }

    // Send notification using tokio runtime
    let runtime = tokio::runtime::Runtime::new().map_err(|e| {
        mlua::Error::external(NotificationError::internal_error(format!(
            "Failed to create async runtime: {e}"
        )))
    })?;

    let result = runtime.block_on(send_slack_notification(&slack_params));

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

    #[test]
    fn test_slack_params_validation_valid() {
        let params = SlackParams {
            webhook_url:
                "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX"
                    .to_string(),
            text: Some("Test message".to_string()),
            blocks: None,
            channel: Some("#general".to_string()),
            username: Some("bot".to_string()),
            icon_emoji: Some(":robot_face:".to_string()),
            icon_url: None,
            unfurl_links: Some(false),
            unfurl_media: Some(false),
        };

        assert!(params.validate().is_ok());
    }

    #[test]
    fn test_slack_params_validation_missing_content() {
        let params = SlackParams {
            webhook_url:
                "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX"
                    .to_string(),
            text: None,
            blocks: None,
            channel: None,
            username: None,
            icon_emoji: None,
            icon_url: None,
            unfurl_links: None,
            unfurl_media: None,
        };

        assert!(params.validate().is_err());
    }

    #[test]
    fn test_slack_params_validation_invalid_channel() {
        let params = SlackParams {
            webhook_url:
                "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX"
                    .to_string(),
            text: Some("Test message".to_string()),
            blocks: None,
            channel: Some("invalid-channel".to_string()),
            username: None,
            icon_emoji: None,
            icon_url: None,
            unfurl_links: None,
            unfurl_media: None,
        };

        assert!(params.validate().is_err());
    }

    #[test]
    fn test_slack_params_validation_invalid_emoji() {
        let params = SlackParams {
            webhook_url:
                "https://hooks.slack.com/services/T00000000/B00000000/XXXXXXXXXXXXXXXXXXXXXXXX"
                    .to_string(),
            text: Some("Test message".to_string()),
            blocks: None,
            channel: None,
            username: None,
            icon_emoji: Some("invalid_emoji".to_string()),
            icon_url: None,
            unfurl_links: None,
            unfurl_media: None,
        };

        assert!(params.validate().is_err());
    }

    #[test]
    fn test_escape_slack_text() {
        let input = "Hello <world> & everyone!";
        let expected = "Hello &lt;world&gt; &amp; everyone!";
        assert_eq!(escape_slack_text(input), expected);
    }

    #[test]
    fn test_slack_text_validation() {
        let valid_text = SlackText {
            text_type: SlackTextType::PlainText,
            text: "Valid text".to_string(),
            emoji: Some(true),
            verbatim: None,
        };
        assert!(valid_text.validate().is_ok());

        let empty_text = SlackText {
            text_type: SlackTextType::PlainText,
            text: String::new(),
            emoji: None,
            verbatim: None,
        };
        assert!(empty_text.validate().is_err());

        let long_text = SlackText {
            text_type: SlackTextType::PlainText,
            text: "a".repeat(3001),
            emoji: None,
            verbatim: None,
        };
        assert!(long_text.validate().is_err());
    }

    #[test]
    fn test_slack_block_validation() {
        let valid_section = SlackBlock::Section {
            text: Some(SlackText {
                text_type: SlackTextType::PlainText,
                text: "Valid section".to_string(),
                emoji: None,
                verbatim: None,
            }),
            fields: None,
            accessory: None,
        };
        assert!(valid_section.validate().is_ok());

        let invalid_section = SlackBlock::Section {
            text: None,
            fields: None,
            accessory: None,
        };
        assert!(invalid_section.validate().is_err());

        let divider = SlackBlock::Divider;
        assert!(divider.validate().is_ok());
    }
}
