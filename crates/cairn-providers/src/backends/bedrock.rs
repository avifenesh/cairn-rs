//! AWS Bedrock backend — calls the Converse API via HTTP Bearer auth.
//!
//! Native cairn backend (not based on OpenAI wire format).  Supports any
//! Bedrock model accessible via the Converse API (Anthropic Claude, MiniMax,
//! Meta Llama, Mistral, etc.).

use async_trait::async_trait;
use serde_json::Value;

use crate::chat::{ChatMessage, ChatProvider, ChatResponse, ChatRole, StructuredOutput, Tool};
use crate::completion::{CompletionProvider, CompletionRequest, CompletionResponse};
use crate::embedding::EmbeddingProvider;
use crate::error::{ProviderError, safe_raw_response};
use crate::models::ModelsProvider;
use crate::redact::redact_secrets;
use crate::{CairnProvider, ToolCall, Usage};

pub struct Bedrock {
    model_id: String,
    region: String,
    api_key: String,
    client: reqwest::Client,
}

impl Bedrock {
    pub fn new(
        model_id: impl Into<String>,
        region: impl Into<String>,
        api_key: impl Into<String>,
    ) -> Self {
        Self {
            model_id: model_id.into(),
            region: region.into(),
            api_key: api_key.into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
        }
    }

    /// Construct from environment variables.
    ///
    /// - `BEDROCK_API_KEY` or `AWS_BEARER_TOKEN_BEDROCK`
    /// - `BEDROCK_MODEL_ID` (default: `minimax.minimax-m2.5`)
    /// - `AWS_REGION` (default: `us-west-2`)
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("BEDROCK_API_KEY")
            .or_else(|_| std::env::var("AWS_BEARER_TOKEN_BEDROCK"))
            .ok()
            .filter(|k| !k.is_empty())?;
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-west-2".to_owned());
        let model_id =
            std::env::var("BEDROCK_MODEL_ID").unwrap_or_else(|_| "minimax.minimax-m2.5".to_owned());
        Some(Self::new(model_id, region, api_key))
    }

    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    pub fn region(&self) -> &str {
        &self.region
    }

    pub(crate) async fn chat_with_tools_for_model(
        &self,
        model: Option<&str>,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        if tools.is_some_and(|tools| !tools.is_empty()) || schema.is_some() {
            return Err(ProviderError::Unsupported(
                "Bedrock chat_with_tools does not support tools or structured output yet"
                    .to_owned(),
            ));
        }

        let model = model
            .filter(|model| !model.trim().is_empty())
            .unwrap_or(&self.model_id);
        let wire_msgs: Vec<Value> = messages
            .iter()
            .map(|m| {
                serde_json::json!({
                    "role": m.role.to_string(),
                    "content": m.content,
                })
            })
            .collect();
        let system = messages
            .iter()
            .find(|m| m.role == ChatRole::System)
            .map(|m| m.content.clone());
        let (text, input_tokens, output_tokens) = self.converse(model, wire_msgs, system).await?;
        let usage = match (input_tokens, output_tokens) {
            (Some(i), Some(o)) => Some(Usage {
                prompt_tokens: i,
                completion_tokens: o,
                total_tokens: i + o,
            }),
            _ => None,
        };
        Ok(Box::new(BedrockChatResponse { text, usage }))
    }

    async fn converse(
        &self,
        model: &str,
        messages: Vec<Value>,
        system: Option<String>,
    ) -> Result<(String, Option<u32>, Option<u32>), ProviderError> {
        let url = format!(
            "https://bedrock-runtime.{}.amazonaws.com/model/{}/converse",
            self.region, model
        );
        let bedrock_msgs: Vec<Value> = messages
            .iter()
            .filter(|m| m["role"].as_str() != Some("system"))
            .map(|m| {
                let role = m["role"].as_str().unwrap_or("user");
                let content = m["content"].as_str().unwrap_or("");
                serde_json::json!({
                    "role": role,
                    "content": [{"text": content}]
                })
            })
            .collect();
        let mut body = serde_json::json!({ "messages": bedrock_msgs });
        if let Some(sys) = &system {
            body["system"] = serde_json::json!([{"text": sys}]);
        }
        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Http(redact_secrets(&format!("Bedrock: {e}"))))?;
        if !resp.status().is_success() {
            let status = resp.status();
            if status.as_u16() == 429 {
                return Err(ProviderError::RateLimited);
            }
            let body = safe_raw_response(&resp.text().await.unwrap_or_default());
            return Err(ProviderError::Provider(format!("Bedrock {status}: {body}")));
        }
        let resp_body: Value = resp
            .json()
            .await
            .map_err(|e| ProviderError::Http(redact_secrets(&format!("parse: {e}"))))?;
        let text = resp_body["output"]["message"]["content"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("")
            })
            .unwrap_or_default();
        let input_tokens = resp_body["usage"]["inputTokens"].as_u64().map(|n| n as u32);
        let output_tokens = resp_body["usage"]["outputTokens"]
            .as_u64()
            .map(|n| n as u32);
        Ok((text, input_tokens, output_tokens))
    }
}

// ── ChatProvider ─────────────────────────────────────────────────────────────

struct BedrockChatResponse {
    text: String,
    usage: Option<Usage>,
}

impl ChatResponse for BedrockChatResponse {
    fn text(&self) -> Option<String> {
        Some(self.text.clone())
    }
    fn tool_calls(&self) -> Option<Vec<ToolCall>> {
        None
    }
    fn usage(&self) -> Option<Usage> {
        self.usage.clone()
    }
}

impl std::fmt::Debug for BedrockChatResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BedrockChatResponse")
            .field("text", &self.text)
            .finish()
    }
}

impl std::fmt::Display for BedrockChatResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.text)
    }
}

#[async_trait]
impl ChatProvider for Bedrock {
    async fn chat_with_tools(
        &self,
        messages: &[ChatMessage],
        tools: Option<&[Tool]>,
        schema: Option<StructuredOutput>,
    ) -> Result<Box<dyn ChatResponse>, ProviderError> {
        self.chat_with_tools_for_model(None, messages, tools, schema)
            .await
    }
}

// ── CompletionProvider ───────────────────────────────────────────────────────

#[async_trait]
impl CompletionProvider for Bedrock {
    async fn complete(&self, req: &CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": req.prompt,
        })];
        let (text, _, _) = self.converse(&self.model_id, messages, None).await?;
        Ok(CompletionResponse { text })
    }
}

// ── EmbeddingProvider ────────────────────────────────────────────────────────

#[async_trait]
impl EmbeddingProvider for Bedrock {
    async fn embed(&self, _input: Vec<String>) -> Result<Vec<Vec<f32>>, ProviderError> {
        Err(ProviderError::Unsupported(
            "Bedrock embedding not yet implemented".into(),
        ))
    }
}

#[async_trait]
impl ModelsProvider for Bedrock {}

impl CairnProvider for Bedrock {}
