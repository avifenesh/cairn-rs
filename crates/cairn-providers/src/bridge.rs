//! Bridge between cairn-providers and cairn-domain's `GenerationProvider` trait.
//!
//! This lets cairn-app use `OpenAiCompat` and `Bedrock` everywhere the
//! existing `GenerationProvider` trait is expected — zero changes needed
//! in the orchestrate/generate handlers.

use async_trait::async_trait;
use cairn_domain::providers::{
    EmbeddingProvider as DomainEmbeddingProvider, EmbeddingResponse, GenerationProvider,
    GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};
use serde_json::Value;

use crate::backends::bedrock::Bedrock;
use crate::chat::{ChatMessage, ChatRole, FunctionDef, MessageContent, Tool};
use crate::error::safe_raw_response;
use crate::redact::redact_secrets;
use crate::wire::openai_compat::OpenAiCompat;
use crate::{FunctionCall, ToolCall};

fn json_content_to_string(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn json_tool_calls(value: &Value) -> Option<Vec<ToolCall>> {
    value.as_array().map(|calls| {
        calls
            .iter()
            .map(|call| ToolCall {
                id: call
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                call_type: call
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("function")
                    .to_owned(),
                function: FunctionCall {
                    name: call
                        .get("function")
                        .and_then(|function| function.get("name"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    arguments: call
                        .get("function")
                        .and_then(|function| function.get("arguments"))
                        .map(json_content_to_string)
                        .unwrap_or_default(),
                },
            })
            .collect()
    })
}

/// Convert OpenAI-format tool definitions (`[{type: "function", function: {name, description, parameters}}]`)
/// into the cairn-providers `Tool` type for passing to `chat_with_tools_for_model`.
fn json_tools_to_tools(tools: &[Value]) -> Option<Vec<Tool>> {
    if tools.is_empty() {
        return None;
    }
    let converted: Vec<Tool> = tools
        .iter()
        .filter_map(|t| {
            let func = t.get("function")?;
            Some(Tool {
                tool_type: "function".to_owned(),
                function: FunctionDef {
                    name: func.get("name")?.as_str()?.to_owned(),
                    description: func
                        .get("description")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_owned(),
                    parameters: func
                        .get("parameters")
                        .cloned()
                        .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
                },
            })
        })
        .collect();
    if converted.is_empty() {
        None
    } else {
        Some(converted)
    }
}

fn json_messages_to_chat_messages(messages: &[Value]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|message| {
            let role = message
                .get("role")
                .and_then(Value::as_str)
                .unwrap_or("user");
            let content = message
                .get("content")
                .map(json_content_to_string)
                .unwrap_or_default();
            match role {
                "system" => ChatMessage::system(content),
                "assistant" => {
                    if let Some(tool_calls) = message.get("tool_calls").and_then(json_tool_calls) {
                        ChatMessage {
                            role: ChatRole::Assistant,
                            content_type: MessageContent::ToolUse(tool_calls),
                            content,
                        }
                    } else {
                        ChatMessage::assistant(content)
                    }
                }
                "tool" => ChatMessage {
                    role: ChatRole::Tool,
                    content_type: MessageContent::ToolResult(vec![ToolCall {
                        id: message
                            .get("tool_call_id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        call_type: "function".to_owned(),
                        function: FunctionCall {
                            name: message
                                .get("name")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            arguments: content,
                        },
                    }]),
                    content: String::new(),
                },
                _ => ChatMessage::user(content),
            }
        })
        .collect()
}

// ── OpenAiCompat → GenerationProvider ────────────────────────────────────────

#[async_trait]
impl GenerationProvider for OpenAiCompat {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
        tools: &[serde_json::Value],
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let chat_messages = json_messages_to_chat_messages(&messages);
        let effective_model = model_id.trim();

        let native_tools = json_tools_to_tools(tools);
        let response = self
            .chat_with_tools_for_model(
                (!effective_model.is_empty()).then_some(effective_model),
                &chat_messages,
                native_tools.as_deref(),
                None,
            )
            .await
            .map_err(provider_error_to_adapter_error)?;

        let text = response.text().unwrap_or_default();
        let usage = response.usage();
        let finish_reason = response.finish_reason();
        let tool_calls: Vec<serde_json::Value> = response
            .tool_calls()
            .unwrap_or_default()
            .into_iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "type": tc.call_type,
                    "function": {
                        "name": tc.function.name,
                        "arguments": tc.function.arguments,
                    }
                })
            })
            .collect();

        let resolved_model = if effective_model.is_empty() {
            self.model.clone()
        } else {
            effective_model.to_owned()
        };

        // Detect empty-completion case: successful HTTP, but the model
        // returned zero usable output (no text after trim, no tool_calls).
        // This is the MiniMax-minimax-m2.5:free failure pattern observed in
        // dogfood run 2 — 17 tokens of whitespace, no tool_calls. Surface
        // as a distinct error so the orchestrator fallback loop can retry
        // on a different model.
        if text.trim().is_empty() && tool_calls.is_empty() {
            return Err(ProviderAdapterError::EmptyResponse {
                model_id: resolved_model,
                prompt_tokens: usage.as_ref().map(|u| u.prompt_tokens),
                completion_tokens: usage.as_ref().map(|u| u.completion_tokens),
            });
        }

        Ok(GenerationResponse {
            text,
            input_tokens: usage.as_ref().map(|u| u.prompt_tokens),
            output_tokens: usage.as_ref().map(|u| u.completion_tokens),
            model_id: resolved_model,
            tool_calls,
            finish_reason,
        })
    }
}

/// Map the richer cairn-providers `ProviderError` into the `ProviderAdapterError`
/// the `GenerationProvider` trait exposes.
///
/// Each `ProviderError` variant maps to the closest `ProviderAdapterError` so
/// the orchestrator fallback loop can classify attempts correctly (rate-limit
/// vs 5xx vs auth vs bad-request vs response-format).
fn provider_error_to_adapter_error(e: crate::error::ProviderError) -> ProviderAdapterError {
    use crate::error::ProviderError;
    match e {
        ProviderError::RateLimited => ProviderAdapterError::RateLimited,
        ProviderError::TimedOut => ProviderAdapterError::TimedOut,
        ProviderError::Auth(msg) => ProviderAdapterError::Auth(msg),
        ProviderError::InvalidRequest(msg) => ProviderAdapterError::InvalidRequest(msg),
        ProviderError::ServerError { status, message } => {
            ProviderAdapterError::ServerError { status, message }
        }
        ProviderError::EmptyResponse {
            model_id,
            prompt_tokens,
            completion_tokens,
        } => ProviderAdapterError::EmptyResponse {
            model_id,
            prompt_tokens,
            completion_tokens,
        },
        ProviderError::ResponseFormat {
            message,
            raw_response,
        } => ProviderAdapterError::StructuredOutputInvalid(format!(
            "{message} (raw: {raw_response})"
        )),
        ProviderError::Http(msg) => ProviderAdapterError::TransportFailure(msg),
        ProviderError::Provider(msg) => ProviderAdapterError::ProviderError(msg),
        ProviderError::Json(msg) => ProviderAdapterError::StructuredOutputInvalid(msg),
        ProviderError::ToolConfig(msg) | ProviderError::Unsupported(msg) => {
            ProviderAdapterError::InvalidRequest(msg)
        }
    }
}

// ── Bedrock → GenerationProvider ─────────────────────────────────────────────
// Bedrock already implements ChatProvider, so the bridge is the same pattern.

#[async_trait]
impl GenerationProvider for Bedrock {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
        tools: &[serde_json::Value],
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let chat_messages = json_messages_to_chat_messages(&messages);
        let effective_model = if model_id.trim().is_empty() {
            self.model_id()
        } else {
            model_id.trim()
        };

        let native_tools = json_tools_to_tools(tools);
        let response = self
            .chat_with_tools_for_model(
                Some(effective_model),
                &chat_messages,
                native_tools.as_deref(),
                None,
            )
            .await
            .map_err(provider_error_to_adapter_error)?;

        let text = response.text().unwrap_or_default();
        let usage = response.usage();
        let finish_reason = response.finish_reason();
        let tool_calls: Vec<serde_json::Value> = response
            .tool_calls()
            .unwrap_or_default()
            .into_iter()
            .map(|tc| {
                serde_json::json!({
                    "id": tc.id,
                    "type": tc.call_type,
                    "function": {
                        "name": tc.function.name,
                        "arguments": tc.function.arguments,
                    }
                })
            })
            .collect();

        if text.trim().is_empty() && tool_calls.is_empty() {
            return Err(ProviderAdapterError::EmptyResponse {
                model_id: effective_model.to_owned(),
                prompt_tokens: usage.as_ref().map(|u| u.prompt_tokens),
                completion_tokens: usage.as_ref().map(|u| u.completion_tokens),
            });
        }

        Ok(GenerationResponse {
            text,
            input_tokens: usage.as_ref().map(|u| u.prompt_tokens),
            output_tokens: usage.as_ref().map(|u| u.completion_tokens),
            model_id: effective_model.to_owned(),
            tool_calls,
            finish_reason,
        })
    }
}

// ── EmbeddingProvider bridges ────────────────────────────────────────────────
// cairn-domain's EmbeddingProvider uses (model_id, texts) → EmbeddingResponse.
// We bridge through to the OpenAI /embeddings endpoint.

#[async_trait]
impl DomainEmbeddingProvider for OpenAiCompat {
    async fn embed(
        &self,
        _model_id: &str,
        texts: Vec<String>,
    ) -> Result<EmbeddingResponse, ProviderAdapterError> {
        // Use the OpenAI /embeddings endpoint via reqwest directly.
        let url = self
            .base_url
            .join("embeddings")
            .map_err(|e| ProviderAdapterError::TransportFailure(redact_secrets(&e.to_string())))?;
        let model = if _model_id.is_empty() {
            &self.model
        } else {
            _model_id
        };
        let body = serde_json::json!({
            "model": model,
            "input": texts,
        });
        let resp = self
            .client()
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(redact_secrets(&e.to_string())))?;
        if !resp.status().is_success() {
            let status = resp.status();
            if status.as_u16() == 429 {
                return Err(ProviderAdapterError::RateLimited);
            }
            let text = safe_raw_response(&resp.text().await.unwrap_or_default());
            return Err(ProviderAdapterError::ProviderError(format!(
                "embedding {status}: {text}"
            )));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(redact_secrets(&e.to_string())))?;
        let embeddings: Vec<Vec<f32>> = json["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item["embedding"].as_array().map(|v| {
                            v.iter()
                                .filter_map(|n| n.as_f64().map(|f| f as f32))
                                .collect()
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let token_count = json["usage"]["total_tokens"].as_u64().unwrap_or(0) as u32;
        Ok(EmbeddingResponse {
            embeddings,
            model_id: model.to_owned(),
            token_count,
        })
    }
}
