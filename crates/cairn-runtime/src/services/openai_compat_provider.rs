//! OpenAI-compatible inference provider adapter.
//!
//! Implements [`GenerationProvider`] and [`EmbeddingProvider`] against any
//! server exposing the OpenAI `/v1/chat/completions` and `/v1/embeddings`
//! endpoints.  Designed for the agntic.garden inference gateway but works
//! with any OpenAI-compatible API (vLLM, LiteLLM, Together, etc.).
//!
//! ## Configuration
//!
//! Set `OPENAI_COMPAT_BASE_URL` and `OPENAI_COMPAT_API_KEY`:
//!
//! ```text
//! OPENAI_COMPAT_BASE_URL=https://agntic.garden/inference/v1 \
//! OPENAI_COMPAT_API_KEY=secret \
//! cargo run -p cairn-app
//! ```

use serde::{Deserialize, Serialize};
use cairn_domain::providers::{
    EmbeddingProvider, EmbeddingResponse, GenerationProvider, GenerationResponse,
    ProviderAdapterError, ProviderBindingSettings,
};

// ── Request / response shapes ────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: &'a [serde_json::Value],
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
    #[serde(default)]
    usage: Option<UsageBlock>,
    model: Option<String>,
}

#[derive(Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Deserialize)]
struct ChatMessage {
    #[serde(default)]
    content: String,
    /// Reasoning/thinking models (e.g. qwen3.5) may place their output here
    /// when `content` is empty (e.g. when max_tokens is too low for the model
    /// to finish its chain-of-thought and produce a final answer).
    #[serde(default)]
    reasoning: Option<String>,
}

#[derive(Deserialize, Default)]
struct UsageBlock {
    prompt_tokens: Option<u32>,
    completion_tokens: Option<u32>,
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbeddingResponseBody {
    data: Vec<EmbeddingData>,
    model: Option<String>,
    #[serde(default)]
    usage: Option<EmbeddingUsage>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[derive(Deserialize, Default)]
struct EmbeddingUsage {
    prompt_tokens: Option<u32>,
}

// ── Provider ─────────────────────────────────────────────────────────────────

/// OpenAI-compatible inference provider.
///
/// Talks to any server that implements the OpenAI `/v1/chat/completions`
/// and `/v1/embeddings` endpoints.  The `base_url` should include the
/// `/v1` prefix (e.g. `https://agntic.garden/inference/v1`).
pub struct OpenAiCompatProvider {
    base_url: String,
    api_key: String,
    client: reqwest::Client,
}

impl OpenAiCompatProvider {
    /// Create a provider with the given base URL and API key.
    ///
    /// `base_url` should end with `/v1` (no trailing slash).
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    /// Create a provider from environment variables.
    ///
    /// Reads `OPENAI_COMPAT_BASE_URL` (legacy) or `CAIRN_WORKER_URL` as a fallback,
    /// plus `OPENAI_COMPAT_API_KEY`.
    /// Returns `None` when no URL is configured.
    pub fn from_env() -> Option<Self> {
        let base_url = std::env::var("OPENAI_COMPAT_BASE_URL")
            .or_else(|_| std::env::var("CAIRN_WORKER_URL"))
            .ok()?;
        let api_key = std::env::var("OPENAI_COMPAT_API_KEY").ok()?;
        Some(Self::new(base_url.trim_end_matches('/'), api_key))
    }

    /// Create a brain-tier provider from `CAIRN_BRAIN_URL` + key.
    ///
    /// Returns `None` when `CAIRN_BRAIN_URL` is unset.
    pub fn from_brain_env() -> Option<Self> {
        let base_url = std::env::var("CAIRN_BRAIN_URL").ok()?;
        let api_key = std::env::var("CAIRN_BRAIN_KEY")
            .or_else(|_| std::env::var("OPENAI_COMPAT_API_KEY"))
            .ok()?;
        Some(Self::new(base_url.trim_end_matches('/'), api_key))
    }

    /// Create a provider pointed at OpenRouter's OpenAI-compatible API.
    ///
    /// Reads `OPENROUTER_API_KEY` (preferred), then falls back to
    /// `CAIRN_BRAIN_KEY` and `OPENAI_COMPAT_API_KEY` so a single key
    /// env var covers multiple providers.
    ///
    /// Base URL is always `https://openrouter.ai/api/v1`.
    /// Returns `None` when no API key is configured.
    pub fn from_openrouter_env() -> Option<Self> {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .or_else(|_| std::env::var("CAIRN_BRAIN_KEY"))
            .or_else(|_| std::env::var("OPENAI_COMPAT_API_KEY"))
            .ok()?;
        Some(Self::new("https://openrouter.ai/api/v1", api_key))
    }

    /// Return the base URL this provider is configured for.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Return the API key this provider is configured with.
    pub fn api_key(&self) -> &str {
        &self.api_key
    }
}

// ── GenerationProvider impl ──────────────────────────────────────────────────

#[async_trait::async_trait]
impl GenerationProvider for OpenAiCompatProvider {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        settings: &ProviderBindingSettings,
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let url = format!("{}/chat/completions", self.base_url);

        let temperature = settings.temperature_milli.map(|m| m as f32 / 1_000.0);

        let body = ChatRequest {
            model: model_id,
            messages: &messages,
            stream: false,
            temperature,
            max_tokens: settings.max_output_tokens,
        };

        let http_resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderAdapterError::TimedOut
                } else {
                    ProviderAdapterError::TransportFailure(e.to_string())
                }
            })?;

        let status = http_resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderAdapterError::RateLimited);
        }
        if !status.is_success() {
            let body_text = http_resp.text().await.unwrap_or_default();
            return Err(ProviderAdapterError::ProviderError(format!(
                "OpenAI-compat returned HTTP {status}: {body_text}"
            )));
        }

        let chat: ChatResponse = http_resp
            .json()
            .await
            .map_err(|e| ProviderAdapterError::ProviderError(e.to_string()))?;

        let text = chat
            .choices
            .into_iter()
            .next()
            .map(|c| {
                if c.message.content.is_empty() {
                    // Reasoning models (e.g. qwen3.5) may leave content empty
                    // and place output in the reasoning field.
                    c.message.reasoning.unwrap_or_default()
                } else {
                    c.message.content
                }
            })
            .unwrap_or_default();

        let usage = chat.usage.unwrap_or_default();

        Ok(GenerationResponse {
            text,
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            model_id: chat.model.unwrap_or_else(|| model_id.to_owned()),
            tool_calls: vec![],
        })
    }
}

// ── EmbeddingProvider impl ───────────────────────────────────────────────────

#[async_trait::async_trait]
impl EmbeddingProvider for OpenAiCompatProvider {
    async fn embed(
        &self,
        model_id: &str,
        texts: Vec<String>,
    ) -> Result<EmbeddingResponse, ProviderAdapterError> {
        if texts.is_empty() {
            return Ok(EmbeddingResponse {
                embeddings: vec![],
                model_id: model_id.to_owned(),
                token_count: 0,
            });
        }

        let url = format!("{}/embeddings", self.base_url);
        let body = EmbeddingRequest {
            model: model_id,
            input: &texts,
        };

        let http_resp = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                if e.is_timeout() {
                    ProviderAdapterError::TimedOut
                } else {
                    ProviderAdapterError::TransportFailure(e.to_string())
                }
            })?;

        let status = http_resp.status();
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(ProviderAdapterError::RateLimited);
        }
        if !status.is_success() {
            let body_text = http_resp.text().await.unwrap_or_default();
            return Err(ProviderAdapterError::ProviderError(format!(
                "OpenAI-compat /embeddings returned HTTP {status}: {body_text}"
            )));
        }

        let resp: EmbeddingResponseBody = http_resp
            .json()
            .await
            .map_err(|e| ProviderAdapterError::ProviderError(e.to_string()))?;

        if resp.data.len() != texts.len() {
            return Err(ProviderAdapterError::ProviderError(format!(
                "OpenAI-compat returned {} embedding(s) for {} input(s)",
                resp.data.len(),
                texts.len(),
            )));
        }

        let embeddings: Vec<Vec<f32>> = resp.data.into_iter().map(|d| d.embedding).collect();
        let token_count = resp.usage.and_then(|u| u.prompt_tokens).unwrap_or(0);

        Ok(EmbeddingResponse {
            embeddings,
            model_id: resp.model.unwrap_or_else(|| model_id.to_owned()),
            token_count,
        })
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_when_unset() {
        std::env::remove_var("OPENAI_COMPAT_BASE_URL");
        std::env::remove_var("OPENAI_COMPAT_API_KEY");
        assert!(OpenAiCompatProvider::from_env().is_none());
    }

    #[test]
    fn from_env_returns_some_when_both_set() {
        std::env::set_var("OPENAI_COMPAT_BASE_URL", "https://example.com/v1");
        std::env::set_var("OPENAI_COMPAT_API_KEY", "test-key");
        let p = OpenAiCompatProvider::from_env().unwrap();
        assert_eq!(p.base_url(), "https://example.com/v1");
        std::env::remove_var("OPENAI_COMPAT_BASE_URL");
        std::env::remove_var("OPENAI_COMPAT_API_KEY");
    }

    #[test]
    fn from_env_returns_none_when_key_missing() {
        std::env::set_var("OPENAI_COMPAT_BASE_URL", "https://example.com/v1");
        std::env::remove_var("OPENAI_COMPAT_API_KEY");
        assert!(OpenAiCompatProvider::from_env().is_none());
        std::env::remove_var("OPENAI_COMPAT_BASE_URL");
    }

    #[test]
    fn trailing_slash_stripped_from_base_url() {
        std::env::set_var("OPENAI_COMPAT_BASE_URL", "https://example.com/v1/");
        std::env::set_var("OPENAI_COMPAT_API_KEY", "key");
        let p = OpenAiCompatProvider::from_env().unwrap();
        assert_eq!(p.base_url(), "https://example.com/v1");
        std::env::remove_var("OPENAI_COMPAT_BASE_URL");
        std::env::remove_var("OPENAI_COMPAT_API_KEY");
    }

    #[tokio::test]
    async fn empty_texts_returns_empty_embeddings() {
        let p = OpenAiCompatProvider::new("https://example.com/v1", "key");
        let resp = p.embed("model", vec![]).await.unwrap();
        assert!(resp.embeddings.is_empty());
        assert_eq!(resp.token_count, 0);
    }

    /// Live integration test against agntic.garden.
    /// Ignored in CI — run manually with:
    /// ```text
    /// cargo test -p cairn-runtime openai_compat_live -- --ignored
    /// ```
    #[tokio::test]
    #[ignore]
    async fn openai_compat_live_chat_completion() {
        let provider = OpenAiCompatProvider::new(
            "https://agntic.garden/inference/v1",
            "Cairn-Inference-2026!",
        );
        let messages = vec![serde_json::json!({
            "role": "user",
            "content": "Reply with exactly one word: hello"
        })];
        let settings = ProviderBindingSettings {
            max_output_tokens: Some(256),
            ..Default::default()
        };
        let resp = provider
            .generate("qwen3.5:9b", messages, &settings)
            .await
            .unwrap();

        assert!(!resp.text.is_empty(), "response text must not be empty");
        assert!(!resp.model_id.is_empty());
    }

    #[tokio::test]
    #[ignore]
    async fn openai_compat_live_embedding() {
        let provider = OpenAiCompatProvider::new(
            "https://agntic.garden/inference/v1",
            "Cairn-Inference-2026!",
        );
        let resp = provider
            .embed(
                "qwen3-embedding:8b",
                vec!["Rust memory safety".to_owned()],
            )
            .await
            .unwrap();

        assert_eq!(resp.embeddings.len(), 1);
        assert!(
            !resp.embeddings[0].is_empty(),
            "embedding vector must not be empty"
        );
    }

}
