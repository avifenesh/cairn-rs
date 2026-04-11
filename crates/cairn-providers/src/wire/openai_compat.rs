//! OpenAI-compatible wire format — single provider implementation for all
//! backends that speak `/chat/completions`.
//!
//! No generics.  One struct.  Backend differences are runtime config.

use std::collections::HashMap;
use std::pin::Pin;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use either::*;
use futures::{Stream, StreamExt};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};

use crate::chat::{
    ChatMessage, ChatProvider, ChatResponse, ChatRole, MessageContent, StreamChoice, StreamChunk,
    StreamDelta, StreamResponse, StructuredOutput, Tool, ToolChoice,
};
use crate::error::{ProviderError, truncate_raw_response};
use crate::{FunctionCall, ToolCall, Usage};

// ── Provider config (runtime, not generic) ───────────────────────────────────

/// Runtime configuration for an OpenAI-compatible backend.
/// Each backend is just a different set of defaults — no generics needed.
#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub name: &'static str,
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    pub chat_endpoint: &'static str,
    pub supports_reasoning_effort: bool,
    pub supports_structured_output: bool,
    pub supports_parallel_tool_calls: bool,
    pub supports_stream_options: bool,
    pub custom_headers: Vec<(String, String)>,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            name: "OpenAI-Compatible",
            default_base_url: "http://localhost:8080/v1/",
            default_model: "default",
            chat_endpoint: "chat/completions",
            supports_reasoning_effort: false,
            supports_structured_output: false,
            supports_parallel_tool_calls: false,
            supports_stream_options: false,
            custom_headers: Vec::new(),
        }
    }
}

// ── Preset configs for known backends ────────────────────────────────────────

impl ProviderConfig {
    pub const OPENAI: Self = Self {
        name: "OpenAI",
        default_base_url: "https://api.openai.com/v1/",
        default_model: "gpt-4.1-nano",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: true,
        supports_structured_output: true,
        supports_parallel_tool_calls: false,
        supports_stream_options: true,
        custom_headers: Vec::new(),
    };

    pub const ANTHROPIC: Self = Self {
        name: "Anthropic",
        default_base_url: "https://api.anthropic.com/v1/",
        default_model: "claude-sonnet-4-6",
        chat_endpoint: "messages",
        supports_reasoning_effort: false,
        supports_structured_output: true,
        supports_parallel_tool_calls: false,
        supports_stream_options: false,
        custom_headers: Vec::new(),
    };

    pub const OLLAMA: Self = Self {
        name: "Ollama",
        default_base_url: "http://localhost:11434/v1/",
        default_model: "llama3.2:3b",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: false,
        supports_parallel_tool_calls: false,
        supports_stream_options: false,
        custom_headers: Vec::new(),
    };

    pub const OPENROUTER: Self = Self {
        name: "OpenRouter",
        default_base_url: "https://openrouter.ai/api/v1/",
        default_model: "openrouter/auto",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: true,
        supports_parallel_tool_calls: false,
        supports_stream_options: true,
        custom_headers: Vec::new(),
    };

    pub const GROQ: Self = Self {
        name: "Groq",
        default_base_url: "https://api.groq.com/openai/v1/",
        default_model: "llama-3.3-70b-versatile",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: false,
        supports_parallel_tool_calls: false,
        supports_stream_options: true,
        custom_headers: Vec::new(),
    };

    pub const DEEPSEEK: Self = Self {
        name: "DeepSeek",
        default_base_url: "https://api.deepseek.com/v1/",
        default_model: "deepseek-chat",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: false,
        supports_parallel_tool_calls: false,
        supports_stream_options: true,
        custom_headers: Vec::new(),
    };

    pub const XAI: Self = Self {
        name: "xAI",
        default_base_url: "https://api.x.ai/v1/",
        default_model: "grok-3-mini",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: true,
        supports_structured_output: false,
        supports_parallel_tool_calls: false,
        supports_stream_options: true,
        custom_headers: Vec::new(),
    };

    pub const GOOGLE: Self = Self {
        name: "Google",
        default_base_url: "https://generativelanguage.googleapis.com/v1beta/openai/",
        default_model: "gemini-2.5-flash",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: true,
        supports_parallel_tool_calls: false,
        supports_stream_options: false,
        custom_headers: Vec::new(),
    };

    pub const AZURE_OPENAI: Self = Self {
        name: "Azure OpenAI",
        default_base_url: "https://YOUR_RESOURCE.openai.azure.com/openai/deployments/YOUR_DEPLOYMENT/",
        default_model: "gpt-4.1",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: true,
        supports_parallel_tool_calls: false,
        supports_stream_options: true,
        custom_headers: Vec::new(),
    };

    pub const MINIMAX: Self = Self {
        name: "MiniMax",
        default_base_url: "https://api.minimaxi.chat/v1/",
        default_model: "MiniMax-M1",
        chat_endpoint: "chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: false,
        supports_parallel_tool_calls: false,
        supports_stream_options: true,
        custom_headers: Vec::new(),
    };

    /// Bedrock OpenAI-compatible gateway.  Simpler than Converse but fewer
    /// features (no guardrails, no document blocks).  URL is region-dependent —
    /// operator must set base_url to `https://bedrock-runtime.{region}.amazonaws.com/`.
    pub const BEDROCK_COMPAT: Self = Self {
        name: "Bedrock (OpenAI-compat)",
        default_base_url: "https://bedrock-runtime.us-west-2.amazonaws.com/",
        default_model: "us.anthropic.claude-sonnet-4-6-v1",
        chat_endpoint: "v1/chat/completions",
        supports_reasoning_effort: false,
        supports_structured_output: true,
        supports_parallel_tool_calls: false,
        supports_stream_options: false,
        custom_headers: Vec::new(),
    };

    /// Resolve a config from backend name.  Returns the generic default for
    /// unknown names so operators can use any OpenAI-compatible endpoint.
    pub fn from_name(name: &str) -> Self {
        match name.to_lowercase().as_str() {
            "openai" => Self::OPENAI,
            "anthropic" => Self::ANTHROPIC,
            "ollama" => Self::OLLAMA,
            "openrouter" => Self::OPENROUTER,
            "groq" => Self::GROQ,
            "deepseek" => Self::DEEPSEEK,
            "xai" => Self::XAI,
            "google" | "gemini" => Self::GOOGLE,
            "azure-openai" | "azure_openai" | "azureopenai" => Self::AZURE_OPENAI,
            "minimax" => Self::MINIMAX,
            "bedrock-compat" | "bedrock_compat" => Self::BEDROCK_COMPAT,
            _ => Self::default(),
        }
    }
}

// ── The one and only provider struct ─────────────────────────────────────────

/// OpenAI-compatible provider.  **One struct for all backends.**
/// Differences between OpenAI, Groq, DeepSeek, etc. are captured in
/// [`ProviderConfig`] fields — no generics, no monomorphization.
pub struct OpenAiCompat {
    config: ProviderConfig,
    pub api_key: String,
    pub base_url: Url,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub timeout_secs: Option<u64>,
    pub top_p: Option<f32>,
    pub top_k: Option<u32>,
    pub tool_choice: Option<ToolChoice>,
    pub reasoning_effort: Option<String>,
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    pub parallel_tool_calls: bool,
    pub embedding_encoding_format: Option<String>,
    pub embedding_dimensions: Option<u32>,
    pub normalize_response: bool,
    client: Client,
}

impl OpenAiCompat {
    /// Create from a preset config with overrides.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: ProviderConfig,
        api_key: impl Into<String>,
        base_url: Option<String>,
        model: Option<String>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
        timeout_secs: Option<u64>,
    ) -> Result<Self, ProviderError> {
        let mut builder = Client::builder();
        if let Some(sec) = timeout_secs {
            builder = builder.timeout(std::time::Duration::from_secs(sec));
        }
        let raw_url = base_url.unwrap_or_else(|| config.default_base_url.to_owned());
        let normalized = format!("{}/", raw_url.trim_end_matches('/'));
        let base_url = Url::parse(&normalized).map_err(|err| {
            ProviderError::InvalidRequest(format!("invalid base URL for {}: {err}", config.name))
        })?;
        let client = builder.build().map_err(|err| {
            ProviderError::InvalidRequest(format!(
                "failed to build HTTP client for {}: {err}",
                config.name
            ))
        })?;
        Ok(Self {
            api_key: api_key.into(),
            base_url,
            model: model.unwrap_or_else(|| config.default_model.to_owned()),
            max_tokens,
            temperature,
            timeout_secs,
            top_p: None,
            top_k: None,
            tool_choice: None,
            reasoning_effort: None,
            extra_body: serde_json::Map::new(),
            parallel_tool_calls: false,
            normalize_response: true,
            embedding_encoding_format: None,
            embedding_dimensions: None,
            client,
            config,
        })
    }

    pub fn config(&self) -> &ProviderConfig {
        &self.config
    }

    pub(crate) fn client(&self) -> &Client {
        &self.client
    }

    fn prepare_messages<'a>(&self, messages: &'a [ChatMessage]) -> Vec<WireMessage<'a>> {
        messages
            .iter()
            .flat_map(|msg| {
                if let MessageContent::ToolResult(ref results) = msg.content_type {
                    results
                        .iter()
                        .map(|r| WireMessage {
                            role: "tool",
                            tool_call_id: Some(r.id.clone()),
                            tool_calls: None,
                            content: Some(Right(r.function.arguments.clone())),
                        })
                        .collect::<Vec<_>>()
                } else {
                    vec![to_wire_message(msg)]
                }
            })
            .collect()
    }

    fn build_request<'a>(
        &'a self,
        model: &'a str,
        messages: Vec<WireMessage<'a>>,
        tools: Option<Vec<Tool>>,
        schema: Option<StructuredOutput>,
        stream: bool,
    ) -> WireRequest<'a> {
        let response_format = if self.config.supports_structured_output {
            schema.map(WireResponseFormat::from)
        } else {
            None
        };
        let reasoning = if self.config.supports_reasoning_effort {
            self.reasoning_effort.clone()
        } else {
            None
        };
        let parallel = if self.config.supports_parallel_tool_calls {
            Some(self.parallel_tool_calls)
        } else {
            None
        };
        let tool_choice = if tools.is_some() {
            self.tool_choice.clone()
        } else {
            None
        };
        let stream_options = if stream && self.config.supports_stream_options {
            Some(WireStreamOptions {
                include_usage: true,
            })
        } else {
            None
        };
        WireRequest {
            model,
            messages,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            stream,
            top_p: self.top_p,
            top_k: self.top_k,
            tools,
            tool_choice,
            reasoning_effort: reasoning,
            response_format,
            stream_options,
            parallel_tool_calls: parallel,
            extra_body: self.extra_body.clone(),
        }
    }

    pub(crate) async fn chat_with_tools_for_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        let wire_msgs = self.prepare_messages(messages);
        let effective_model = model
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .unwrap_or(&self.model);
        let body = self.build_request(
            effective_model,
            wire_msgs,
            tools.map(|t| t.to_vec()),
            schema,
            false,
        );
        let resp = self.send_request(&body).await?;
        let text = resp.text().await?;
        let parsed: WireChatResponse =
            serde_json::from_str(&text).map_err(|e| ProviderError::ResponseFormat {
                message: format!("failed to decode {} response: {e}", self.config.name),
                raw_response: truncate_raw_response(&text),
            })?;
        Ok(Box::new(parsed))
    }

    async fn send_request(
        &self,
        body: &WireRequest<'_>,
    ) -> Result<reqwest::Response, ProviderError> {
        if self.api_key.is_empty() {
            return Err(ProviderError::Auth(format!(
                "missing {} API key",
                self.config.name
            )));
        }
        let url = self
            .base_url
            .join(self.config.chat_endpoint)
            .map_err(|e| ProviderError::Http(e.to_string()))?;
        let mut req = self.client.post(url).bearer_auth(&self.api_key).json(body);
        for (k, v) in &self.config.custom_headers {
            req = req.header(k, v);
        }
        if let Some(timeout) = self.timeout_secs {
            req = req.timeout(std::time::Duration::from_secs(timeout));
        }
        let resp = req.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            if status.as_u16() == 429 {
                return Err(ProviderError::RateLimited);
            }
            let body = truncate_raw_response(&resp.text().await.unwrap_or_default());
            return Err(ProviderError::ResponseFormat {
                message: format!("{} returned HTTP {status}", self.config.name),
                raw_response: body,
            });
        }
        Ok(resp)
    }
}

// ── ChatProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl ChatProvider for OpenAiCompat {
    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        self.chat_with_tools_for_model(None, messages, tools, schema)
            .await
    }

    async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        schema: Option<StructuredOutput>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<String, ProviderError>> + Send>>, ProviderError>
    {
        let struct_stream = self.chat_stream_structured(messages, None, schema).await?;
        let content_stream = struct_stream.filter_map(|result| async move {
            match result {
                Ok(sr) => sr
                    .choices
                    .first()
                    .and_then(|c| c.delta.content.as_ref())
                    .filter(|s| !s.is_empty())
                    .map(|s| Ok(s.clone())),
                Err(e) => Some(Err(e)),
            }
        });
        Ok(Box::pin(content_stream))
    }

    async fn chat_stream_structured(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        schema: Option<StructuredOutput>,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<StreamResponse, ProviderError>> + Send>>,
        ProviderError,
    > {
        let wire_msgs = self.prepare_messages(messages);
        let body = self.build_request(
            &self.model,
            wire_msgs,
            tools.map(|t| t.to_vec()),
            schema,
            true,
        );
        let resp = self.send_request(&body).await?;
        Ok(create_sse_stream(resp, self.normalize_response))
    }

    async fn chat_stream_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        schema: Option<StructuredOutput>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>, ProviderError>
    {
        let wire_msgs = self.prepare_messages(messages);
        let body = self.build_request(
            &self.model,
            wire_msgs,
            tools.map(|t| t.to_vec()),
            schema,
            true,
        );
        let resp = self.send_request(&body).await?;
        Ok(create_tool_sse_stream(resp))
    }
}

// ── Wire types (OpenAI JSON shapes) ─────────────────────────────────────────

#[derive(Serialize, Debug)]
pub struct WireMessage<'a> {
    pub role: &'a str,
    #[serde(
        skip_serializing_if = "Option::is_none",
        with = "either::serde_untagged_optional"
    )]
    pub content: Option<Either<Vec<WireContentPart>, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct WireContentPart {
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub part_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_url: Option<WireImageUrl>,
}

#[derive(Serialize, Debug)]
pub struct WireImageUrl {
    pub url: String,
}

#[derive(Serialize, Debug)]
pub struct WireRequest<'a> {
    pub model: &'a str,
    pub messages: Vec<WireMessage<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_format: Option<WireResponseFormat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<WireStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
    #[serde(flatten)]
    pub extra_body: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct WireChatResponse {
    pub choices: Vec<WireChatChoice>,
    pub usage: Option<Usage>,
}

#[derive(Deserialize, Debug)]
pub struct WireChatChoice {
    pub message: WireChatMsg,
}

#[derive(Deserialize, Debug)]
pub struct WireChatMsg {
    #[allow(dead_code)]
    pub role: String,
    pub content: Option<String>,
    #[serde(default, alias = "reasoning")]
    pub reasoning_content: Option<String>,
    pub tool_calls: Option<Vec<ToolCall>>,
}

impl ChatResponse for WireChatResponse {
    fn text(&self) -> Option<String> {
        self.choices.first().and_then(|c| c.message.content.clone())
    }
    fn tool_calls(&self) -> Option<Vec<ToolCall>> {
        self.choices
            .first()
            .and_then(|c| c.message.tool_calls.clone())
    }
    fn thinking(&self) -> Option<String> {
        self.choices
            .first()
            .and_then(|c| c.message.reasoning_content.clone())
    }
    fn usage(&self) -> Option<Usage> {
        self.usage.clone()
    }
}

impl std::fmt::Display for WireChatResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(c) = self.choices.first() {
            if let Some(ref text) = c.message.content {
                write!(f, "{text}")?;
            }
            if let Some(ref calls) = c.message.tool_calls {
                for tc in calls {
                    write!(f, "{tc}")?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WireResponseFormat {
    #[serde(rename = "type")]
    pub response_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<StructuredOutput>,
}

impl From<StructuredOutput> for WireResponseFormat {
    fn from(s: StructuredOutput) -> Self {
        let schema = s.schema.map(|mut v| {
            if v.get("additionalProperties").is_none() {
                v["additionalProperties"] = serde_json::json!(false);
            }
            v
        });
        Self {
            response_type: "json_schema".to_owned(),
            json_schema: Some(StructuredOutput {
                name: s.name,
                description: s.description,
                schema,
                strict: s.strict,
            }),
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WireStreamOptions {
    pub include_usage: bool,
}

// ── Message conversion ───────────────────────────────────────────────────────

fn to_wire_message(msg: &ChatMessage) -> WireMessage<'_> {
    let role = match msg.role {
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::System => "system",
        ChatRole::Tool => "user",
    };
    let content = match &msg.content_type {
        MessageContent::Text => Some(Right(msg.content.clone())),
        MessageContent::Image(mime, bytes) => {
            let url = format!("data:{};base64,{}", mime.as_str(), BASE64.encode(bytes));
            Some(Left(vec![WireContentPart {
                part_type: Some("image_url"),
                text: None,
                image_url: Some(WireImageUrl { url }),
            }]))
        }
        MessageContent::ImageUrl(url) => Some(Left(vec![WireContentPart {
            part_type: Some("image_url"),
            text: None,
            image_url: Some(WireImageUrl { url: url.clone() }),
        }])),
        MessageContent::Pdf(_) | MessageContent::ToolUse(_) | MessageContent::ToolResult(_) => None,
    };
    let tool_calls = if let MessageContent::ToolUse(calls) = &msg.content_type {
        Some(
            calls
                .iter()
                .map(|c| ToolCall {
                    id: c.id.clone(),
                    call_type: "function".to_owned(),
                    function: FunctionCall {
                        name: c.function.name.clone(),
                        arguments: c.function.arguments.clone(),
                    },
                })
                .collect(),
        )
    } else {
        None
    };
    WireMessage {
        role,
        content,
        tool_calls,
        tool_call_id: None,
    }
}

// ── SSE streaming ────────────────────────────────────────────────────────────

fn find_sse_boundary(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|p| (p, 2));
    let crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|p| (p, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    }
}

struct SseParser {
    buf: Vec<u8>,
    tool_buf: ToolCall,
    usage: Option<Usage>,
    results: Vec<Result<StreamResponse, ProviderError>>,
    normalize: bool,
}

impl SseParser {
    fn new(normalize: bool) -> Self {
        Self {
            buf: Vec::new(),
            usage: None,
            results: Vec::new(),
            tool_buf: ToolCall {
                id: String::new(),
                call_type: "function".to_owned(),
                function: FunctionCall {
                    name: String::new(),
                    arguments: String::new(),
                },
            },
            normalize,
        }
    }

    fn flush_tool(&mut self) {
        if self.normalize && !self.tool_buf.function.name.is_empty() {
            self.results.push(Ok(StreamResponse {
                choices: vec![StreamChoice {
                    delta: StreamDelta {
                        content: None,
                        reasoning_content: None,
                        tool_calls: Some(vec![self.tool_buf.clone()]),
                    },
                }],
                usage: None,
            }));
        }
        self.tool_buf = ToolCall {
            id: String::new(),
            call_type: "function".to_owned(),
            function: FunctionCall {
                name: String::new(),
                arguments: String::new(),
            },
        };
    }

    fn parse_event(&mut self, event_bytes: &[u8]) {
        let event = String::from_utf8_lossy(event_bytes);
        let mut data = String::new();
        for line in event.lines() {
            if let Some(d) = line
                .strip_prefix("data: ")
                .or_else(|| line.strip_prefix("data:").map(str::trim_start))
            {
                if d == "[DONE]" {
                    self.flush_tool();
                    if let Some(u) = self.usage.take() {
                        self.results.push(Ok(StreamResponse {
                            choices: vec![StreamChoice {
                                delta: StreamDelta {
                                    content: None,
                                    reasoning_content: None,
                                    tool_calls: None,
                                },
                            }],
                            usage: Some(u),
                        }));
                    }
                    return;
                }
                data.push_str(d);
            } else {
                data.push_str(line);
            }
        }
        if data.is_empty() {
            return;
        }

        #[derive(Deserialize)]
        struct Chunk {
            choices: Vec<CC>,
            usage: Option<Usage>,
        }
        #[derive(Deserialize)]
        struct CC {
            delta: CD,
        }
        #[derive(Deserialize)]
        struct CD {
            content: Option<String>,
            #[serde(default, alias = "reasoning")]
            reasoning_content: Option<String>,
            tool_calls: Option<Vec<CT>>,
        }
        #[derive(Deserialize)]
        struct CT {
            id: Option<String>,
            #[serde(rename = "type", default)]
            _call_type: Option<String>,
            function: CF,
        }
        #[derive(Deserialize)]
        struct CF {
            name: Option<String>,
            #[serde(default)]
            arguments: String,
        }

        if let Ok(chunk) = serde_json::from_str::<Chunk>(&data) {
            if let Some(u) = chunk.usage {
                self.usage = Some(u);
            }
            for choice in &chunk.choices {
                let content = choice.delta.content.clone();
                let reasoning = choice.delta.reasoning_content.clone();
                let calls: Option<Vec<ToolCall>> = choice.delta.tool_calls.as_ref().map(|tcs| {
                    tcs.iter()
                        .map(|c| ToolCall {
                            id: c.id.clone().unwrap_or_default(),
                            call_type: "function".to_owned(),
                            function: FunctionCall {
                                name: c.function.name.clone().unwrap_or_default(),
                                arguments: c.function.arguments.clone(),
                            },
                        })
                        .collect()
                });
                if content.is_some() || reasoning.is_some() || calls.is_some() {
                    if self.normalize
                        && let Some(ref call_list) = calls
                    {
                        for tc in call_list {
                            if !tc.function.name.is_empty() {
                                self.flush_tool();
                                self.tool_buf.function.name.clone_from(&tc.function.name);
                            }
                            if !tc.function.arguments.is_empty() {
                                self.tool_buf
                                    .function
                                    .arguments
                                    .push_str(&tc.function.arguments);
                            }
                            if !tc.id.is_empty() {
                                self.tool_buf.id.clone_from(&tc.id);
                            }
                        }
                    } else {
                        self.flush_tool();
                        self.results.push(Ok(StreamResponse {
                            choices: vec![StreamChoice {
                                delta: StreamDelta {
                                    content,
                                    reasoning_content: reasoning,
                                    tool_calls: calls,
                                },
                            }],
                            usage: None,
                        }));
                    }
                }
            }
        }
    }

    fn consume(&mut self, bytes: &[u8]) -> Vec<Result<StreamResponse, ProviderError>> {
        self.buf.extend_from_slice(bytes);
        while let Some((pos, len)) = find_sse_boundary(&self.buf) {
            let event = self.buf[..pos].to_vec();
            self.buf.drain(..pos + len);
            self.parse_event(&event);
        }
        self.results.drain(..).collect()
    }
}

fn create_sse_stream(
    response: reqwest::Response,
    normalize: bool,
) -> Pin<Box<dyn Stream<Item = Result<StreamResponse, ProviderError>> + Send>> {
    let stream = response
        .bytes_stream()
        .scan(SseParser::new(normalize), |parser, chunk| {
            let results = match chunk {
                Ok(bytes) => parser.consume(&bytes),
                Err(e) => vec![Err(ProviderError::Http(e.to_string()))],
            };
            futures::future::ready(Some(results))
        })
        .flat_map(futures::stream::iter);
    Box::pin(stream)
}

// ── Tool streaming ───────────────────────────────────────────────────────────

#[derive(Debug, Default)]
struct ToolUseState {
    id: String,
    name: String,
    args: String,
    started: bool,
}

fn create_tool_sse_stream(
    response: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>> {
    let stream = response
        .bytes_stream()
        .scan(
            (Vec::<u8>::new(), HashMap::<usize, ToolUseState>::new()),
            move |(buf, states), chunk| {
                let results = match chunk {
                    Ok(bytes) => {
                        let mut out = Vec::new();
                        buf.extend_from_slice(&bytes);
                        while let Some((pos, len)) = find_sse_boundary(buf) {
                            let event = buf[..pos].to_vec();
                            buf.drain(..pos + len);
                            let text = String::from_utf8_lossy(&event);
                            match parse_tool_chunk(text.trim(), states) {
                                Ok(chunks) => out.extend(chunks.into_iter().map(Ok)),
                                Err(e) => out.push(Err(e)),
                            }
                        }
                        out
                    }
                    Err(e) => vec![Err(ProviderError::Http(e.to_string()))],
                };
                async move { Some(results) }
            },
        )
        .flat_map(futures::stream::iter);
    Box::pin(stream)
}

fn parse_tool_chunk(
    event: &str,
    states: &mut HashMap<usize, ToolUseState>,
) -> Result<Vec<StreamChunk>, ProviderError> {
    let mut results = Vec::new();
    for line in event.lines() {
        let data = match line
            .strip_prefix("data: ")
            .or_else(|| line.strip_prefix("data:").map(str::trim_start))
        {
            Some(d) => d.trim(),
            None => continue,
        };
        if data == "[DONE]" {
            for (idx, state) in states.drain() {
                if state.started {
                    results.push(StreamChunk::ToolUseComplete {
                        index: idx,
                        tool_call: ToolCall {
                            id: state.id,
                            call_type: "function".to_owned(),
                            function: FunctionCall {
                                name: state.name,
                                arguments: state.args,
                            },
                        },
                    });
                }
            }
            results.push(StreamChunk::Done {
                stop_reason: "end_turn".to_owned(),
            });
            return Ok(results);
        }

        #[derive(Deserialize)]
        struct C {
            choices: Vec<CC>,
            #[serde(default)]
            usage: Option<Usage>,
        }
        #[derive(Deserialize)]
        struct CC {
            delta: CD,
            finish_reason: Option<String>,
        }
        #[derive(Deserialize)]
        struct CD {
            content: Option<String>,
            #[serde(default, alias = "reasoning")]
            reasoning_content: Option<String>,
            tool_calls: Option<Vec<CT>>,
        }
        #[derive(Deserialize)]
        struct CT {
            index: Option<usize>,
            id: Option<String>,
            function: CF,
        }
        #[derive(Deserialize)]
        struct CF {
            name: Option<String>,
            #[serde(default)]
            arguments: String,
        }

        let chunk: C =
            serde_json::from_str(data).map_err(|e| ProviderError::Json(e.to_string()))?;
        let mut usage_opt = chunk.usage;
        for choice in &chunk.choices {
            if let Some(ref text) = choice.delta.content
                && !text.is_empty()
            {
                results.push(StreamChunk::Text(text.clone()));
            }
            if let Some(ref r) = choice.delta.reasoning_content
                && !r.is_empty()
            {
                results.push(StreamChunk::Reasoning(r.clone()));
            }
            if let Some(ref tcs) = choice.delta.tool_calls {
                for tc in tcs {
                    let idx = tc.index.unwrap_or(0);
                    let state = states.entry(idx).or_default();
                    if let Some(ref id) = tc.id {
                        state.id = id.clone();
                    }
                    if let Some(ref name) = tc.function.name {
                        state.name = name.clone();
                        if !state.started {
                            state.started = true;
                            results.push(StreamChunk::ToolUseStart {
                                index: idx,
                                id: state.id.clone(),
                                name: state.name.clone(),
                            });
                        }
                    }
                    if !tc.function.arguments.is_empty() {
                        state.args.push_str(&tc.function.arguments);
                        results.push(StreamChunk::ToolUseDelta {
                            index: idx,
                            partial_json: tc.function.arguments.clone(),
                        });
                    }
                }
            }
            if let Some(ref reason) = choice.finish_reason {
                for (idx, state) in states.drain() {
                    if state.started {
                        results.push(StreamChunk::ToolUseComplete {
                            index: idx,
                            tool_call: ToolCall {
                                id: state.id,
                                call_type: "function".to_owned(),
                                function: FunctionCall {
                                    name: state.name,
                                    arguments: state.args,
                                },
                            },
                        });
                    }
                }
                if let Some(u) = usage_opt.take() {
                    results.push(StreamChunk::Usage(u));
                }
                let stop = match reason.as_str() {
                    "tool_calls" => "tool_use",
                    "stop" => "end_turn",
                    other => other,
                };
                results.push(StreamChunk::Done {
                    stop_reason: stop.to_owned(),
                });
            }
        }
        if let Some(u) = usage_opt.take() {
            results.push(StreamChunk::Usage(u));
        }
    }
    Ok(results)
}
