//! Native Z.ai wire adapter (GLM coding-plan + general paas endpoints).
//!
//! # Why its own file
//!
//! Z.ai is OpenAI-ish but has quirks that historically caused silent
//! regressions when patched into the generic `openai_compat` adapter:
//!
//! * **Coding endpoint**: operators on the GLM Coding Plan use
//!   `https://api.z.ai/api/coding/paas/v4/`, not the general paas endpoint.
//!   Same auth (Bearer), same JSON shape, different URL and model catalogue.
//! * **`reasoning_content` on message**: OpenAI-compat puts CoT in
//!   `reasoning_content`; Z.ai does too but *always emits it* unless
//!   `thinking: {type: "disabled"}` is set on the request. Generic adapter
//!   has no knob for this.
//! * **`prompt_tokens_details.cached_tokens`**: Z.ai reports server-side
//!   cache hits nested inside usage. Generic adapter drops this field.
//!   Part of the coding plan's pricing model, so we parse it.
//! * **`completion_tokens_details.reasoning_tokens`**: similarly nested.
//!   Parsed for observability.
//! * **Tool call shape**: top-level `index` on tool_calls even in
//!   non-streaming responses (harmless, but worth documenting).
//! * **Error envelope**: Z.ai returns `{"error": {"code": "1305", "message": "..."}}`
//!   for rate/overload conditions with HTTP 200 bodies sometimes (!), so we
//!   inspect the body even on success codes.
//!
//! Structurally this mirrors `wire::openai_compat` — same wire types, same
//! SSE parser — because a full re-implementation would be a regression risk
//! with no benefit. Isolation means Z.ai-specific behaviour changes can't
//! break OpenAI / DeepSeek / Groq.
//!
//! See `docs/providers/zai.md` for the API contract probed against the live
//! coding endpoint.

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
use crate::error::{ProviderError, safe_raw_response};
use crate::redact::redact_secrets;
use crate::{FunctionCall, ToolCall, Usage};

// ── Z.ai tier presets ────────────────────────────────────────────────────────

/// Default client timeout for Z.ai requests (seconds).
///
/// GLM reasoning with `thinking: enabled` routinely takes 30–90s; 120s leaves
/// headroom without letting a hung socket block the orchestrator forever. See
/// F27 dogfood blocker.
pub const DEFAULT_TIMEOUT_SECS: u64 = 120;

/// Z.ai endpoint tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZaiTier {
    /// GLM Coding Plan subscribers: `https://api.z.ai/api/coding/paas/v4/`.
    /// Different quota/pricing, same wire shape.
    Coding,
    /// General pay-as-you-go: `https://api.z.ai/api/paas/v4/`.
    General,
}

impl ZaiTier {
    pub fn base_url(self) -> &'static str {
        match self {
            Self::Coding => "https://api.z.ai/api/coding/paas/v4/",
            Self::General => "https://api.z.ai/api/paas/v4/",
        }
    }
}

/// Runtime configuration for a Z.ai backend.  Mirrors `ProviderConfig` in
/// shape so the call sites look familiar, but dedicated to avoid cross-family
/// regressions.
#[derive(Debug, Clone)]
pub struct ZaiConfig {
    pub default_base_url: &'static str,
    pub default_model: &'static str,
    pub chat_endpoint: &'static str,
    /// Whether to enable the GLM reasoning chain.  `false` sends
    /// `"thinking": {"type": "disabled"}` on every request and suppresses
    /// `reasoning_tokens` in usage.
    pub enable_thinking: bool,
    /// Default HTTP client timeout in seconds when the caller does NOT
    /// supply an explicit `timeout_secs`. GLM reasoning can be slow
    /// (multi-minute completions with long `thinking` chains), so we bias
    /// generous but finite. Never `None`: a hung TCP connect from our
    /// datacenter to `api.z.ai` was the F27 dogfood blocker that motivated
    /// making this non-optional.
    pub default_timeout_secs: u64,
}

impl Default for ZaiConfig {
    fn default() -> Self {
        Self::CODING
    }
}

impl ZaiConfig {
    /// GLM Coding Plan tier — the endpoint Avi's account uses.  Default model
    /// `glm-4.7` because it's the most widely-available coding-plan model at
    /// time of writing.
    pub const CODING: Self = Self {
        default_base_url: "https://api.z.ai/api/coding/paas/v4/",
        default_model: "glm-4.7",
        chat_endpoint: "chat/completions",
        enable_thinking: true,
        default_timeout_secs: DEFAULT_TIMEOUT_SECS,
    };

    /// General pay-as-you-go tier.
    pub const GENERAL: Self = Self {
        default_base_url: "https://api.z.ai/api/paas/v4/",
        default_model: "glm-4.7",
        chat_endpoint: "chat/completions",
        enable_thinking: true,
        default_timeout_secs: DEFAULT_TIMEOUT_SECS,
    };

    pub fn from_tier(tier: ZaiTier) -> Self {
        match tier {
            ZaiTier::Coding => Self::CODING,
            ZaiTier::General => Self::GENERAL,
        }
    }
}

// ── Provider struct ──────────────────────────────────────────────────────────

/// Native Z.ai adapter. Use [`crate::Backend::Zai`] via [`crate::ProviderBuilder`].
pub struct ZaiProvider {
    config: ZaiConfig,
    pub api_key: String,
    pub base_url: Url,
    pub model: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub timeout_secs: Option<u64>,
    pub top_p: Option<f32>,
    pub tool_choice: Option<ToolChoice>,
    /// Optional flag to override `enable_thinking` per-instance.  `None`
    /// inherits from config.
    pub enable_thinking_override: Option<bool>,
    pub extra_body: serde_json::Map<String, serde_json::Value>,
    pub normalize_response: bool,
    client: Client,
}

impl ZaiProvider {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        config: ZaiConfig,
        api_key: impl Into<String>,
        base_url: Option<String>,
        model: Option<String>,
        max_tokens: Option<u32>,
        temperature: Option<f32>,
        timeout_secs: Option<u64>,
    ) -> Result<Self, ProviderError> {
        // ALWAYS install a client-level timeout. `None` resolves to the
        // per-tier default (120s today). Leaving reqwest's default (which
        // is effectively unbounded for connect-idle) caused F27: an
        // unresponsive Z.ai upstream hung the orchestrator forever
        // because no socket-level deadline ever fired. See
        // `DEFAULT_TIMEOUT_SECS` and the module-level doc.
        let effective_timeout = timeout_secs.unwrap_or(config.default_timeout_secs);
        let builder = Client::builder().timeout(std::time::Duration::from_secs(effective_timeout));
        let raw_url = base_url.unwrap_or_else(|| config.default_base_url.to_owned());
        let normalized = format!("{}/", raw_url.trim_end_matches('/'));
        let base_url = Url::parse(&normalized).map_err(|err| {
            ProviderError::InvalidRequest(format!("invalid Z.ai base URL: {err}"))
        })?;
        let client = builder.build().map_err(|err| {
            ProviderError::InvalidRequest(format!("failed to build Z.ai HTTP client: {err}"))
        })?;
        Ok(Self {
            api_key: api_key.into(),
            base_url,
            model: model.unwrap_or_else(|| config.default_model.to_owned()),
            max_tokens,
            temperature,
            timeout_secs,
            top_p: None,
            tool_choice: None,
            enable_thinking_override: None,
            extra_body: serde_json::Map::new(),
            normalize_response: true,
            client,
            config,
        })
    }

    pub fn config(&self) -> &ZaiConfig {
        &self.config
    }

    fn thinking_enabled(&self) -> bool {
        self.enable_thinking_override
            .unwrap_or(self.config.enable_thinking)
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
        stream: bool,
    ) -> WireRequest<'a> {
        let tool_choice = if tools.is_some() {
            self.tool_choice.clone()
        } else {
            None
        };
        let stream_options = if stream {
            Some(WireStreamOptions {
                include_usage: true,
            })
        } else {
            None
        };
        let thinking = Some(WireThinking {
            // Z.ai docs: "enabled" | "disabled"
            type_: if self.thinking_enabled() {
                "enabled"
            } else {
                "disabled"
            },
        });
        WireRequest {
            model,
            messages,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            stream,
            top_p: self.top_p,
            tools,
            tool_choice,
            stream_options,
            thinking,
            extra_body: self.extra_body.clone(),
        }
    }

    pub(crate) async fn chat_with_tools_for_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        let wire_msgs = self.prepare_messages(messages);
        let effective_model = model
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .unwrap_or(&self.model);
        let body = self.build_request(effective_model, wire_msgs, tools.map(|t| t.to_vec()), false);
        let resp = self.send_request(&body).await?;
        let text = resp.text().await?;
        // Z.ai occasionally returns `{"error":{"code":"1305","message":"..."}}`
        // with HTTP 200. Detect and surface.
        if let Ok(envelope) = serde_json::from_str::<ZaiErrorEnvelope>(&text)
            && envelope.error.is_some()
        {
            let err = envelope.error.unwrap();
            let msg = format!("Z.ai error code {}: {}", err.code, err.message);
            if err.code == "1305" || err.code == "429" {
                return Err(ProviderError::RateLimited);
            }
            return Err(ProviderError::InvalidRequest(redact_secrets(&msg)));
        }
        let parsed: WireChatResponse =
            serde_json::from_str(&text).map_err(|e| ProviderError::ResponseFormat {
                message: redact_secrets(&format!("failed to decode Z.ai response: {e}")),
                raw_response: safe_raw_response(&text),
            })?;
        Ok(Box::new(parsed))
    }

    async fn send_request(
        &self,
        body: &WireRequest<'_>,
    ) -> Result<reqwest::Response, ProviderError> {
        if self.api_key.is_empty() {
            return Err(ProviderError::Auth("missing Z.ai API key".to_owned()));
        }
        let url = self
            .base_url
            .join(self.config.chat_endpoint)
            .map_err(|e| ProviderError::Http(redact_secrets(&e.to_string())))?;
        // Per-request timeout: explicit override if set, otherwise the
        // per-tier default. Redundant with the client-level timeout but
        // belt-and-suspenders — reqwest applies the lower of the two and
        // we want a hard ceiling either way.
        let per_request_timeout = self
            .timeout_secs
            .unwrap_or(self.config.default_timeout_secs);
        let req = self
            .client
            .post(url)
            .bearer_auth(&self.api_key)
            .json(body)
            .timeout(std::time::Duration::from_secs(per_request_timeout));
        let resp = req.send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let code = status.as_u16();
            let body = safe_raw_response(&resp.text().await.unwrap_or_default());
            if code == 429 {
                return Err(ProviderError::RateLimited);
            }
            if code == 401 || code == 403 {
                return Err(ProviderError::Auth(format!(
                    "Z.ai returned HTTP {status}: {body}"
                )));
            }
            if (400..500).contains(&code) {
                return Err(ProviderError::InvalidRequest(format!(
                    "Z.ai returned HTTP {status}: {body}"
                )));
            }
            if (500..600).contains(&code) {
                return Err(ProviderError::ServerError {
                    status: code,
                    message: format!("Z.ai returned HTTP {status}: {body}"),
                });
            }
            return Err(ProviderError::ResponseFormat {
                message: format!("Z.ai returned HTTP {status}"),
                raw_response: body,
            });
        }
        Ok(resp)
    }
}

// ── ChatProvider impl ────────────────────────────────────────────────────────

#[async_trait]
impl ChatProvider for ZaiProvider {
    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        _schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        // Z.ai coding endpoint doesn't document response_format; omit to
        // avoid silent fallback to freeform.
        self.chat_with_tools_for_model(None, messages, tools).await
    }

    async fn chat_with_tools_for_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        _schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        ZaiProvider::chat_with_tools_for_model(self, model, messages, tools).await
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
        _schema: Option<StructuredOutput>,
    ) -> Result<
        Pin<Box<dyn Stream<Item = Result<StreamResponse, ProviderError>> + Send>>,
        ProviderError,
    > {
        let wire_msgs = self.prepare_messages(messages);
        let body = self.build_request(&self.model, wire_msgs, tools.map(|t| t.to_vec()), true);
        let resp = self.send_request(&body).await?;
        Ok(create_sse_stream(resp, self.normalize_response))
    }

    async fn chat_stream_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        _schema: Option<StructuredOutput>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk, ProviderError>> + Send>>, ProviderError>
    {
        let wire_msgs = self.prepare_messages(messages);
        let body = self.build_request(&self.model, wire_msgs, tools.map(|t| t.to_vec()), true);
        let resp = self.send_request(&body).await?;
        Ok(create_tool_sse_stream(resp))
    }
}

// ── Wire types ───────────────────────────────────────────────────────────────

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
pub struct WireThinking {
    #[serde(rename = "type")]
    pub type_: &'static str,
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
    pub tools: Option<Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<WireStreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<WireThinking>,
    #[serde(flatten)]
    pub extra_body: serde_json::Map<String, serde_json::Value>,
}

#[derive(Deserialize, Debug)]
pub struct WireChatResponse {
    pub choices: Vec<WireChatChoice>,
    #[serde(default)]
    pub usage: Option<ZaiUsage>,
}

#[derive(Deserialize, Debug)]
pub struct WireChatChoice {
    pub message: WireChatMsg,
    pub finish_reason: Option<String>,
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

/// Z.ai extended usage with nested `prompt_tokens_details.cached_tokens` and
/// `completion_tokens_details.reasoning_tokens`.
#[derive(Deserialize, Debug, Default)]
pub struct ZaiUsage {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
    #[serde(default)]
    pub prompt_tokens_details: Option<ZaiPromptDetails>,
    #[serde(default)]
    #[allow(dead_code)]
    pub completion_tokens_details: Option<ZaiCompletionDetails>,
}

#[derive(Deserialize, Debug, Default)]
pub struct ZaiPromptDetails {
    #[serde(default)]
    pub cached_tokens: u32,
}

#[derive(Deserialize, Debug, Default)]
pub struct ZaiCompletionDetails {
    #[serde(default)]
    #[allow(dead_code)]
    pub reasoning_tokens: u32,
}

impl From<ZaiUsage> for Usage {
    fn from(z: ZaiUsage) -> Self {
        let cached = z
            .prompt_tokens_details
            .as_ref()
            .map(|d| d.cached_tokens)
            .filter(|n| *n > 0);
        Usage {
            prompt_tokens: z.prompt_tokens,
            completion_tokens: z.completion_tokens,
            total_tokens: z.total_tokens,
            cached_tokens: cached,
        }
    }
}

#[derive(Deserialize, Debug)]
struct ZaiErrorEnvelope {
    #[serde(default)]
    error: Option<ZaiErrorBody>,
}

#[derive(Deserialize, Debug)]
struct ZaiErrorBody {
    #[serde(default)]
    code: String,
    #[serde(default)]
    message: String,
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
        self.usage.as_ref().map(|u| Usage {
            prompt_tokens: u.prompt_tokens,
            completion_tokens: u.completion_tokens,
            total_tokens: u.total_tokens,
            cached_tokens: u
                .prompt_tokens_details
                .as_ref()
                .map(|d| d.cached_tokens)
                .filter(|n| *n > 0),
        })
    }
    fn finish_reason(&self) -> Option<String> {
        self.choices.first().and_then(|c| c.finish_reason.clone())
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

/// Streaming parser for the structured-response variant of Z.ai's SSE.
///
/// Holds a single `tool_buf` because Z.ai's coding endpoint emits one
/// tool call at a time (observed 2026-04-24 — no parallel tool deltas
/// in practice). If that changes upstream, the tool-call variant
/// `parse_tool_chunk` already maintains a `HashMap<index, state>` and
/// should be used instead via `chat_stream_with_tools`. Gemini review
/// on #280 flagged the single-buf design; we keep it for now because
/// (a) it mirrors `wire::openai_compat`'s shape for minimum surface
/// area, and (b) the parallel-tool path is untested against a live
/// GLM response. Revisit when Z.ai documents parallel-call support.
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
            }
            // Per the SSE spec, lines without a recognised field name (e.g.
            // `event:`, `id:`, `:` comments) are ignored. Previously we
            // concatenated them into the data buffer, which broke JSON
            // parsing when a server emitted a keep-alive comment mid-frame.
            // Gemini review on #280.
        }
        if data.is_empty() {
            return;
        }

        #[derive(Deserialize)]
        struct Chunk {
            choices: Vec<CC>,
            usage: Option<ZaiUsage>,
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
                self.usage = Some(u.into());
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
                        // Accumulate tool_call deltas; flush on name change.
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
                        // BUGBOT#280: Z.ai deltas can carry BOTH content/reasoning
                        // AND tool_calls in the same frame (the coding endpoint
                        // always emits `reasoning_content` alongside whatever
                        // else is in the delta). Previously the normalize branch
                        // silently dropped text/reasoning when tool_calls were
                        // present. Emit them as a separate StreamResponse so
                        // callers that tee reasoning→UI don't lose frames.
                        if content.is_some() || reasoning.is_some() {
                            self.results.push(Ok(StreamResponse {
                                choices: vec![StreamChoice {
                                    delta: StreamDelta {
                                        content,
                                        reasoning_content: reasoning,
                                        tool_calls: None,
                                    },
                                }],
                                usage: None,
                            }));
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
        // BUGBOT#280: Z.ai occasionally returns `{"error":{"code":"1305",...}}`
        // as a plain JSON body with HTTP 200 on the streaming endpoint
        // (observed 2026-04-24). The SSE parser would otherwise never find a
        // `data:` frame and close the stream silently — defeating the
        // fallback chain. Detect the envelope as soon as we have enough bytes
        // and surface it as RateLimited/InvalidRequest.
        if let Some(err) = detect_error_envelope(&self.buf) {
            self.buf.clear();
            self.results.push(Err(err));
            return self.results.drain(..).collect();
        }
        while let Some((pos, len)) = find_sse_boundary(&self.buf) {
            let event = self.buf[..pos].to_vec();
            self.buf.drain(..pos + len);
            self.parse_event(&event);
        }
        self.results.drain(..).collect()
    }
}

/// Checks whether a streaming response body starts with the Z.ai error
/// envelope (`{"error":{"code":"...","message":"..."}}`) and returns the
/// mapped `ProviderError`.  Returns `None` if the body does not parse as a
/// complete envelope yet (so the caller can keep accumulating bytes).
fn detect_error_envelope(buf: &[u8]) -> Option<ProviderError> {
    // Quick guard: SSE frames start with `data:` or `event:` or `id:` or `:`.
    // The error envelope is raw JSON. Cheap check before parsing.
    let trimmed_start = buf
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &buf[i..])
        .unwrap_or(buf);
    if !trimmed_start.starts_with(b"{") {
        return None;
    }
    // Try a full JSON parse; only act if it deserialises to the error shape.
    let envelope: ZaiErrorEnvelope = serde_json::from_slice(trimmed_start).ok()?;
    let err = envelope.error?;
    if err.code == "1305" || err.code == "429" {
        Some(ProviderError::RateLimited)
    } else {
        Some(ProviderError::InvalidRequest(redact_secrets(&format!(
            "Z.ai stream error code {}: {}",
            err.code, err.message
        ))))
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
                Err(e) => vec![Err(e.into())],
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
                        // BUGBOT#280: surface HTTP-200 error envelopes on
                        // the tool-streaming path too. Same rationale as
                        // `SseParser::consume`. When the envelope parses, we
                        // drain the buffer and skip SSE framing entirely.
                        let mut aborted = false;
                        if let Some(err) = detect_error_envelope(buf) {
                            buf.clear();
                            out.push(Err(err));
                            aborted = true;
                        }
                        while !aborted && let Some((pos, len)) = find_sse_boundary(buf) {
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
                    Err(e) => vec![Err(e.into())],
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
            usage: Option<ZaiUsage>,
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

        let chunk: C = serde_json::from_str(data)?;
        let mut usage_opt: Option<Usage> = chunk.usage.map(Into::into);
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tier_base_urls() {
        assert_eq!(
            ZaiTier::Coding.base_url(),
            "https://api.z.ai/api/coding/paas/v4/"
        );
        assert_eq!(ZaiTier::General.base_url(), "https://api.z.ai/api/paas/v4/");
    }

    #[test]
    fn usage_parses_cached_tokens() {
        let raw = r#"{
            "prompt_tokens": 100,
            "completion_tokens": 50,
            "total_tokens": 150,
            "prompt_tokens_details": {"cached_tokens": 42},
            "completion_tokens_details": {"reasoning_tokens": 10}
        }"#;
        let zu: ZaiUsage = serde_json::from_str(raw).unwrap();
        let u: Usage = zu.into();
        assert_eq!(u.prompt_tokens, 100);
        assert_eq!(u.completion_tokens, 50);
        assert_eq!(u.total_tokens, 150);
        assert_eq!(u.cached_tokens, Some(42));
    }

    #[test]
    fn usage_zero_cached_is_none() {
        let raw = r#"{
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15,
            "prompt_tokens_details": {"cached_tokens": 0}
        }"#;
        let zu: ZaiUsage = serde_json::from_str(raw).unwrap();
        let u: Usage = zu.into();
        assert_eq!(u.cached_tokens, None);
    }

    #[test]
    fn usage_missing_details_is_none() {
        let raw = r#"{
            "prompt_tokens": 10,
            "completion_tokens": 5,
            "total_tokens": 15
        }"#;
        let zu: ZaiUsage = serde_json::from_str(raw).unwrap();
        let u: Usage = zu.into();
        assert_eq!(u.cached_tokens, None);
    }

    #[test]
    fn error_envelope_detection() {
        let body =
            r#"{"error":{"code":"1305","message":"The service may be temporarily overloaded"}}"#;
        let env: ZaiErrorEnvelope = serde_json::from_str(body).unwrap();
        let err = env.error.unwrap();
        assert_eq!(err.code, "1305");
        assert!(err.message.contains("overloaded"));
    }

    #[test]
    fn full_chat_response_parses() {
        // Captured from live coding endpoint 2026-04-24.
        let raw = r#"{
            "choices":[{"finish_reason":"tool_calls","index":0,"message":{
                "content":"I'll check the weather.",
                "reasoning_content":"User wants weather.",
                "role":"assistant",
                "tool_calls":[{"function":{"arguments":"{\"city\":\"Paris\"}","name":"get_weather"},"id":"call_-7682507267639338722","index":0,"type":"function"}]
            }}],
            "created":1777040104,
            "id":"x",
            "model":"glm-4.7",
            "object":"chat.completion",
            "usage":{"completion_tokens":78,"completion_tokens_details":{"reasoning_tokens":56},"prompt_tokens":160,"prompt_tokens_details":{"cached_tokens":0},"total_tokens":238}
        }"#;
        let parsed: WireChatResponse = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.text().as_deref(), Some("I'll check the weather."));
        assert_eq!(parsed.thinking().as_deref(), Some("User wants weather."));
        let calls = parsed.tool_calls().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "get_weather");
        assert_eq!(calls[0].id, "call_-7682507267639338722");
        let usage = parsed.usage().unwrap();
        assert_eq!(usage.prompt_tokens, 160);
        assert_eq!(usage.completion_tokens, 78);
        assert_eq!(usage.cached_tokens, None); // zero → None
    }

    #[test]
    fn thinking_flag_serializes() {
        let config = ZaiConfig::CODING;
        let provider = ZaiProvider::new(
            config,
            "test-key",
            None,
            Some("glm-4.7".to_owned()),
            None,
            None,
            None,
        )
        .unwrap();
        let req = provider.build_request("glm-4.7", vec![], None, false);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["thinking"]["type"], "enabled");
    }

    #[test]
    fn thinking_disabled_when_override_set() {
        let mut provider =
            ZaiProvider::new(ZaiConfig::CODING, "test-key", None, None, None, None, None).unwrap();
        provider.enable_thinking_override = Some(false);
        let req = provider.build_request("glm-4.7", vec![], None, false);
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["thinking"]["type"], "disabled");
    }
}
