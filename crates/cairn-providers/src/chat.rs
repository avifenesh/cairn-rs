//! Chat provider trait and message types.

use std::collections::HashMap;
use std::fmt;
use std::pin::Pin;

use async_trait::async_trait;
use futures::stream::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ToolCall, Usage, error::ProviderError};

// ── Roles & messages ─────────────────────────────────────────────────────────

/// Participant role in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChatRole {
    System,
    User,
    Assistant,
    Tool,
}

impl fmt::Display for ChatRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::System => "system",
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::Tool => "tool",
        })
    }
}

/// Content type within a message.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum MessageContent {
    #[default]
    Text,
    Image(ImageMime, Vec<u8>),
    ImageUrl(String),
    Pdf(Vec<u8>),
    ToolUse(Vec<ToolCall>),
    ToolResult(Vec<ToolCall>),
}

/// Supported image MIME types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageMime {
    Jpeg,
    Png,
    Gif,
    WebP,
}

impl ImageMime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Gif => "image/gif",
            Self::WebP => "image/webp",
        }
    }
}

/// A single message in a conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: ChatRole,
    pub content_type: MessageContent,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::User,
            content_type: MessageContent::Text,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::Assistant,
            content_type: MessageContent::Text,
            content: content.into(),
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: ChatRole::System,
            content_type: MessageContent::Text,
            content: content.into(),
        }
    }

    pub fn tool_result(id: String, name: String, output: String) -> Self {
        Self {
            role: ChatRole::Tool,
            content_type: MessageContent::ToolResult(vec![ToolCall {
                id,
                call_type: "function".to_owned(),
                function: crate::FunctionCall {
                    name,
                    arguments: output,
                },
            }]),
            content: String::new(),
        }
    }
}

// ── Tools ────────────────────────────────────────────────────────────────────

/// A tool the model can invoke.
#[derive(Debug, Clone, Serialize)]
pub struct Tool {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDef,
}

/// Function definition within a [`Tool`].
#[derive(Debug, Clone, Serialize)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// How the model should choose among available tools.
#[derive(Debug, Clone, Default)]
pub enum ToolChoice {
    Any,
    #[default]
    Auto,
    Specific(String),
    None,
}

impl Serialize for ToolChoice {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Any => serializer.serialize_str("required"),
            Self::Auto => serializer.serialize_str("auto"),
            Self::None => serializer.serialize_str("none"),
            Self::Specific(name) => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("type", "function")?;
                map.serialize_entry("function", &HashMap::from([("name", name.as_str())]))?;
                map.end()
            }
        }
    }
}

/// Structured output schema for JSON-mode responses.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StructuredOutput {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

// ── Response trait ───────────────────────────────────────────────────────────

/// Provider-agnostic chat response.
pub trait ChatResponse: fmt::Debug + fmt::Display + Send + Sync {
    fn text(&self) -> Option<String>;
    fn tool_calls(&self) -> Option<Vec<ToolCall>>;
    fn thinking(&self) -> Option<String> {
        None
    }
    fn usage(&self) -> Option<Usage> {
        None
    }
}

// ── Streaming types ──────────────────────────────────────────────────────────

/// Structured streaming response (OpenAI wire-format compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamResponse {
    pub choices: Vec<StreamChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamChoice {
    pub delta: StreamDelta,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamDelta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
}

/// Unified streaming chunk for agentic workflows.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamChunk {
    Text(String),
    Reasoning(String),
    ToolUseStart {
        index: usize,
        id: String,
        name: String,
    },
    ToolUseDelta {
        index: usize,
        partial_json: String,
    },
    ToolUseComplete {
        index: usize,
        tool_call: ToolCall,
    },
    Usage(Usage),
    Done {
        stop_reason: String,
    },
}

/// Reasoning effort hint for models that support it.
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        })
    }
}

// ── Provider trait ───────────────────────────────────────────────────────────

/// Trait for providers that support chat-style interactions.
#[async_trait]
pub trait ChatProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        self.chat_with_tools(messages, None, schema).await
    }

    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError>;

    async fn chat_stream(
        &self,
        _messages: &[ChatMessage],
        _schema: Option<StructuredOutput>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>>, ProviderError>
    {
        Err(ProviderError::Unsupported("streaming not supported".into()))
    }

    async fn chat_stream_structured(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[Tool]>,
        _schema: Option<StructuredOutput>,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<StreamResponse, ProviderError>> + Send>>,
        ProviderError,
    > {
        Err(ProviderError::Unsupported(
            "structured streaming not supported".into(),
        ))
    }

    async fn chat_stream_with_tools(
        &self,
        _messages: &[ChatMessage],
        _tools: Option<&[Tool]>,
        _schema: Option<StructuredOutput>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>, ProviderError>
    {
        Err(ProviderError::Unsupported(
            "tool streaming not supported".into(),
        ))
    }
}
