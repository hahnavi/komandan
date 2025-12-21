//! Google Chat webhook notification handler

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

/// Google Chat notification parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleChatParams {
    pub webhook_url: String,
    pub text: Option<String>,
    pub cards: Option<Vec<GoogleChatCard>>,
    pub thread_key: Option<String>,
}

/// Google Chat Card structure following Google Chat Card API v1
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleChatCard {
    pub header: Option<GoogleChatCardHeader>,
    pub sections: Option<Vec<GoogleChatCardSection>>,
}

/// Google Chat Card Header
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleChatCardHeader {
    pub title: String,
    pub subtitle: Option<String>,
    #[serde(rename = "imageUrl")]
    pub image_url: Option<String>,
    #[serde(rename = "imageStyle")]
    pub image_style: Option<String>,
}

/// Google Chat Card Section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleChatCardSection {
    pub header: Option<String>,
    pub widgets: Option<Vec<GoogleChatWidget>>,
}

/// Google Chat Widget
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum GoogleChatWidget {
    #[serde(rename = "textParagraph")]
    TextParagraph { text: String },
    #[serde(rename = "keyValue")]
    KeyValue {
        #[serde(rename = "topLabel")]
        top_label: Option<String>,
        content: String,
        #[serde(rename = "contentMultiline")]
        content_multiline: Option<bool>,
        #[serde(rename = "bottomLabel")]
        bottom_label: Option<String>,
    },
}

/// Google Chat message payload structure
#[derive(Debug, Clone, Serialize)]
struct GoogleChatMessage {
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cards: Option<Vec<GoogleChatCard>>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "thread")]
    thread_key: Option<GoogleChatThread>,
}

/// Google Chat thread structure
#[derive(Debug, Clone, Serialize)]
struct GoogleChatThread {
    name: String,
}

impl GoogleChatParams {
    /// Validate Google Chat parameters
    ///
    /// # Errors
    ///
    /// Returns an error if required parameters are missing or invalid.
    pub fn validate(&self) -> NotificationResult<()> {
        // Validate webhook URL
        crate::notification::validation::validate_webhook_url(&self.webhook_url)?;

        // Ensure at least text or cards are provided
        if self.text.is_none() && self.cards.is_none() {
            return Err(NotificationError::missing_parameter(
                "text or cards - at least one must be provided",
            ));
        }

        // Validate text content if provided
        if let Some(text) = &self.text
            && text.trim().is_empty()
        {
            return Err(NotificationError::invalid_parameter(
                "text",
                "Text content cannot be empty",
            ));
        }

        // Validate cards if provided
        if let Some(cards) = &self.cards {
            if cards.is_empty() {
                return Err(NotificationError::invalid_parameter(
                    "cards",
                    "Cards array cannot be empty",
                ));
            }

            for (i, card) in cards.iter().enumerate() {
                card.validate()
                    .map_err(|e| NotificationError::invalid_parameter(format!("cards[{i}]"), e))?;
            }
        }

        Ok(())
    }

    /// Convert to Google Chat message payload
    fn to_message(&self) -> GoogleChatMessage {
        GoogleChatMessage {
            text: self.text.clone(),
            cards: self.cards.clone(),
            thread_key: self
                .thread_key
                .as_ref()
                .map(|key| GoogleChatThread { name: key.clone() }),
        }
    }
}

impl GoogleChatCard {
    /// Validate Google Chat card structure
    ///
    /// # Errors
    ///
    /// Returns an error if the card structure is invalid.
    fn validate(&self) -> Result<(), String> {
        // At least header or sections must be present
        if self.header.is_none() && self.sections.is_none() {
            return Err("Card must have either header or sections".to_string());
        }

        // Validate header if present
        if let Some(header) = &self.header
            && header.title.trim().is_empty()
        {
            return Err("Card header title cannot be empty".to_string());
        }

        // Validate sections if present
        if let Some(sections) = &self.sections {
            if sections.is_empty() {
                return Err("Card sections array cannot be empty".to_string());
            }

            for (i, section) in sections.iter().enumerate() {
                section
                    .validate()
                    .map_err(|e| format!("Section {i}: {e}"))?;
            }
        }

        Ok(())
    }
}

impl GoogleChatCardSection {
    /// Validate Google Chat card section
    ///
    /// # Errors
    ///
    /// Returns an error if the section structure is invalid.
    fn validate(&self) -> Result<(), String> {
        // Validate widgets if present
        if let Some(widgets) = &self.widgets {
            if widgets.is_empty() {
                return Err("Section widgets array cannot be empty".to_string());
            }

            for (i, widget) in widgets.iter().enumerate() {
                widget.validate().map_err(|e| format!("Widget {i}: {e}"))?;
            }
        }

        Ok(())
    }
}

impl GoogleChatWidget {
    /// Validate Google Chat widget
    ///
    /// # Errors
    ///
    /// Returns an error if the widget structure is invalid.
    fn validate(&self) -> Result<(), String> {
        match self {
            Self::TextParagraph { text } => {
                if text.trim().is_empty() {
                    return Err("TextParagraph text cannot be empty".to_string());
                }
            }
            Self::KeyValue { content, .. } => {
                if content.trim().is_empty() {
                    return Err("KeyValue content cannot be empty".to_string());
                }
            }
        }
        Ok(())
    }
}

/// Send Google Chat webhook notification with retry logic
///
/// # Errors
///
/// Returns an error if the HTTP request fails or parameters are invalid.
async fn send_google_chat_notification(
    params: &GoogleChatParams,
) -> NotificationResult<NotificationResponse> {
    // Validate parameters
    params.validate()?;

    // Create HTTP client with optimized webhook configuration
    let http_config = crate::notification::utils::create_webhook_http_config();

    let client = http_config.build_client()?;
    let retry_handler = http_config.create_retry_handler();

    // Create message payload
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

/// Google Chat webhook notification function for Lua
///
/// This function is called from Lua scripts to send Google Chat notifications.
/// It extracts parameters from the Lua table, validates them, and sends the notification.
///
/// # Errors
///
/// Returns an error if parameter extraction or notification sending fails.
pub fn google_chat_webhook(lua: &Lua, params: Table) -> mlua::Result<Table> {
    // Extract parameters from Lua table
    let webhook_url = ParameterExtractor::extract_string(&params, "webhook_url")
        .map_err(mlua::Error::external)?;

    let text = ParameterExtractor::extract_optional_string(&params, "text")
        .map_err(mlua::Error::external)?;

    let thread_key = ParameterExtractor::extract_optional_string(&params, "thread_key")
        .map_err(mlua::Error::external)?;

    // Extract cards if provided (simplified for now - full card parsing would be more complex)
    let cards = if let Ok(mlua::Value::Table(_cards_table)) = params.get("cards") {
        // For now, we'll accept cards as None and focus on text messages
        // Full card parsing implementation would require recursive table parsing
        None
    } else {
        None
    };

    let google_chat_params = GoogleChatParams {
        webhook_url,
        text,
        cards,
        thread_key,
    };

    // Check for dry-run mode from global Args
    let dry_run = Args::parse().flags.dry_run;

    // Handle dry-run mode
    if dry_run {
        let start_time = Instant::now();

        // Validate parameters even in dry-run mode
        if let Err(error) = google_chat_params.validate() {
            let response = ResponseFormatter::error_from_notification_error(&error, start_time);
            return response.to_lua_table(lua);
        }

        let param_count = 1 + // webhook_url
            usize::from(google_chat_params.text.is_some()) +
            usize::from(google_chat_params.cards.is_some()) +
            usize::from(google_chat_params.thread_key.is_some());

        let response = ResponseFormatter::dry_run_response(
            "Google Chat",
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

    let result = runtime.block_on(send_google_chat_notification(&google_chat_params));

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
