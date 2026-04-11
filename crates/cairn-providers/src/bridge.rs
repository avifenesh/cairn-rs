//! Bridge between cairn-providers and cairn-domain's `GenerationProvider` trait.
//!
//! This lets cairn-app use `OpenAiCompat` and `Bedrock` everywhere the
//! existing `GenerationProvider` trait is expected — zero changes needed
//! in the orchestrate/generate handlers.

use async_trait::async_trait;
use cairn_domain::providers::{
    EmbeddingProvider as DomainEmbeddingProvider, EmbeddingResponse,
    GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};

use crate::chat::{ChatMessage, ChatProvider};
use crate::wire::openai_compat::OpenAiCompat;
use crate::backends::bedrock::Bedrock;

// ── OpenAiCompat → GenerationProvider ────────────────────────────────────────

#[async_trait]
impl GenerationProvider for OpenAiCompat {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|m| {
                let role = m["role"].as_str().unwrap_or("user");
                let content = m["content"].as_str().unwrap_or("").to_owned();
                match role {
                    "system" => ChatMessage::system(content),
                    "assistant" => ChatMessage::assistant(content),
                    _ => ChatMessage::user(content),
                }
            })
            .collect();

        // Temporarily override model if caller specifies one.
        // The OpenAiCompat struct stores model as a field — for now we use
        // the configured model. A future version will allow per-call override.
        let _ = model_id; // TODO: per-call model override

        let response = self
            .chat_with_tools(&chat_messages, None, None)
            .await
            .map_err(|e| match e {
                crate::error::ProviderError::RateLimited => ProviderAdapterError::RateLimited,
                crate::error::ProviderError::Http(msg) => ProviderAdapterError::TransportFailure(msg),
                crate::error::ProviderError::Auth(msg) => ProviderAdapterError::TransportFailure(msg),
                other => ProviderAdapterError::ProviderError(other.to_string()),
            })?;

        let text = response.text().unwrap_or_default();
        let usage = response.usage();
        let tool_calls: Vec<serde_json::Value> = response
            .tool_calls()
            .unwrap_or_default()
            .into_iter()
            .map(|tc| serde_json::json!({
                "id": tc.id,
                "type": tc.call_type,
                "function": {
                    "name": tc.function.name,
                    "arguments": tc.function.arguments,
                }
            }))
            .collect();

        Ok(GenerationResponse {
            text,
            input_tokens: usage.as_ref().map(|u| u.prompt_tokens),
            output_tokens: usage.as_ref().map(|u| u.completion_tokens),
            model_id: self.model.clone(),
            tool_calls,
        })
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
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let chat_messages: Vec<ChatMessage> = messages
            .iter()
            .map(|m| {
                let role = m["role"].as_str().unwrap_or("user");
                let content = m["content"].as_str().unwrap_or("").to_owned();
                match role {
                    "system" => ChatMessage::system(content),
                    "assistant" => ChatMessage::assistant(content),
                    _ => ChatMessage::user(content),
                }
            })
            .collect();

        let response = self
            .chat_with_tools(&chat_messages, None, None)
            .await
            .map_err(|e| match e {
                crate::error::ProviderError::RateLimited => ProviderAdapterError::RateLimited,
                crate::error::ProviderError::Http(msg) => ProviderAdapterError::TransportFailure(msg),
                other => ProviderAdapterError::ProviderError(other.to_string()),
            })?;

        let text = response.text().unwrap_or_default();
        let usage = response.usage();
        let effective_model = if model_id.is_empty() {
            self.model_id().to_owned()
        } else {
            model_id.to_owned()
        };

        Ok(GenerationResponse {
            text,
            input_tokens: usage.as_ref().map(|u| u.prompt_tokens),
            output_tokens: usage.as_ref().map(|u| u.completion_tokens),
            model_id: effective_model,
            tool_calls: vec![],
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
            .map_err(|e| ProviderAdapterError::TransportFailure(e.to_string()))?;
        let model = if _model_id.is_empty() {
            &self.model
        } else {
            _model_id
        };
        let body = serde_json::json!({
            "model": model,
            "input": texts,
        });
        let resp = reqwest::Client::new()
            .post(url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(e.to_string()))?;
        if !resp.status().is_success() {
            let status = resp.status();
            if status.as_u16() == 429 {
                return Err(ProviderAdapterError::RateLimited);
            }
            let text = resp.text().await.unwrap_or_default();
            return Err(ProviderAdapterError::ProviderError(format!(
                "embedding {status}: {text}"
            )));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(e.to_string()))?;
        let embeddings: Vec<Vec<f32>> = json["data"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        item["embedding"]
                            .as_array()
                            .map(|v| v.iter().filter_map(|n| n.as_f64().map(|f| f as f32)).collect())
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
