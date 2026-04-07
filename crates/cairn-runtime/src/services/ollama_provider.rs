//! Ollama local LLM provider adapter (RFC 009).
//!
//! Implements [`GenerationProvider`] against Ollama's OpenAI-compatible
//! `/v1/chat/completions` endpoint, plus a health probe via `/api/tags`.
//!
//! ## Configuration
//!
//! Set `OLLAMA_HOST` to use a non-default address:
//!
//! ```text
//! OLLAMA_HOST=http://gpu-box:11434 cargo run -p cairn-app
//! ```
//!
//! When `OLLAMA_HOST` is unset the default `http://localhost:11434` is used.
//!
//! ## Wire-up
//!
//! ```rust,ignore
//! if let Some(provider) = OllamaProvider::from_env() {
//!     // register as the default generation provider
//! }
//! ```

use serde::{Deserialize, Serialize};
use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
};

// ── Thinking-mode detection ───────────────────────────────────────────────────

/// Return the list of model-name prefixes that require `think: false` to
/// suppress chain-of-thought reasoning.
///
/// Reads `CAIRN_THINKING_MODELS` (comma-separated). Default: `"qwen3"`.
fn thinking_model_prefixes() -> Vec<String> {
    std::env::var("CAIRN_THINKING_MODELS")
        .unwrap_or_else(|_| "qwen3".to_owned())
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .collect()
}

// ── Request / response shapes (OpenAI-compatible) ────────────────────────────

/// Ollama-specific options block passed alongside the OpenAI-compat request.
///
/// Ollama forwards these to the underlying model runner.  `think: false`
/// disables Qwen3's chain-of-thought reasoning pass when set.
#[derive(Serialize)]
struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    think: Option<bool>,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    model:    &'a str,
    messages: &'a [serde_json::Value],
    stream:   bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    /// Ollama-specific options forwarded to the model runner.
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<OllamaOptions>,
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
    content: String,
}

#[derive(Deserialize, Default)]
struct UsageBlock {
    prompt_tokens:     Option<u32>,
    completion_tokens: Option<u32>,
}

/// Response from `GET /api/tags` — list of locally available models.
#[derive(Deserialize, Debug)]
pub struct OllamaTagsResponse {
    pub models: Vec<OllamaModel>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct OllamaModel {
    pub name: String,
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Ollama local LLM provider.
///
/// Uses Ollama's OpenAI-compatible `/v1/chat/completions` endpoint.
pub struct OllamaProvider {
    host:   String,
    client: reqwest::Client,
}

impl OllamaProvider {
    /// Create a provider pointing at the given Ollama host URL.
    ///
    /// `host` should be a base URL such as `"http://localhost:11434"`.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host:   host.into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(300))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    /// Create a provider from the `OLLAMA_HOST` environment variable.
    ///
    /// Returns `None` when the variable is unset, allowing callers to skip
    /// Ollama registration on deployments where it is not available.
    pub fn from_env() -> Option<Self> {
        let host = std::env::var("OLLAMA_HOST").ok()?;
        Some(Self::new(host.trim_end_matches('/')))
    }

    /// Create a provider using the default `http://localhost:11434` host.
    pub fn default_local() -> Self {
        Self::new("http://localhost:11434")
    }

    /// Return the base host URL this provider is configured for.
    pub fn host(&self) -> &str {
        &self.host
    }

    /// Probe Ollama's `/api/tags` endpoint.
    ///
    /// Returns `Ok(models)` when Ollama is reachable, `Err` otherwise.
    /// This is the canonical health check — a non-empty model list means
    /// the daemon is up and at least one model is loaded.
    pub async fn health_check(&self) -> Result<OllamaTagsResponse, ProviderAdapterError> {
        let url = format!("{}/api/tags", self.host);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| ProviderAdapterError::TransportFailure(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(ProviderAdapterError::ProviderError(format!(
                "Ollama /api/tags returned HTTP {}",
                resp.status()
            )));
        }

        resp.json::<OllamaTagsResponse>()
            .await
            .map_err(|e| ProviderAdapterError::ProviderError(e.to_string()))
    }

    /// Return `true` when the Ollama daemon is reachable.
    pub async fn is_healthy(&self) -> bool {
        self.health_check().await.is_ok()
    }

    /// List models currently available in the local Ollama registry.
    pub async fn list_models(&self) -> Result<Vec<OllamaModel>, ProviderAdapterError> {
        self.health_check().await.map(|r| r.models)
    }
}

// ── GenerationProvider impl ───────────────────────────────────────────────────

#[async_trait::async_trait]
impl GenerationProvider for OllamaProvider {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        settings: &ProviderBindingSettings,
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let url = format!("{}/v1/chat/completions", self.host);

        // Convert temperature: domain stores it in milli-degrees (700 = 0.7).
        let temperature = settings
            .temperature_milli
            .map(|m| m as f32 / 1_000.0);

        // Some models default to chain-of-thought reasoning, which adds several
        // seconds of latency and hundreds of extra output tokens.  Passing
        // `options: { think: false }` via Ollama's extension field disables the
        // thinking pass and returns direct answers.  This is the documented API
        // approach; the `/no_think` suffix only works on the native `/api/chat`
        // endpoint, not on the OpenAI-compat `/v1/chat/completions` path.
        //
        // CAIRN_THINKING_MODELS controls which model-name prefixes receive this
        // option. Default: "qwen3". Comma-separate for multiple families.
        let options = if thinking_model_prefixes()
            .iter()
            .any(|prefix| model_id.contains(prefix.as_str()))
        {
            Some(OllamaOptions { think: Some(false) })
        } else {
            None
        };

        let body = ChatRequest {
            model:       model_id,
            messages:    &messages,
            stream:      false,
            temperature,
            max_tokens:  settings.max_output_tokens,
            options,
        };

        let http_resp = self
            .client
            .post(&url)
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
            let body = http_resp.text().await.unwrap_or_default();
            return Err(ProviderAdapterError::ProviderError(format!(
                "Ollama returned HTTP {status}: {body}"
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
            .map(|c| c.message.content)
            .unwrap_or_default();

        let usage = chat.usage.unwrap_or_default();

        Ok(GenerationResponse {
            text,
            input_tokens:  usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
            model_id:      chat.model.unwrap_or_else(|| model_id.to_owned()),
            tool_calls:    vec![],
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_returns_none_when_unset() {
        std::env::remove_var("OLLAMA_HOST");
        assert!(OllamaProvider::from_env().is_none());
    }

    #[test]
    fn from_env_returns_some_when_set() {
        std::env::set_var("OLLAMA_HOST", "http://gpu-box:11434");
        let p = OllamaProvider::from_env().unwrap();
        assert_eq!(p.host(), "http://gpu-box:11434");
        std::env::remove_var("OLLAMA_HOST");
    }

    #[test]
    fn default_local_uses_expected_host() {
        let p = OllamaProvider::default_local();
        assert_eq!(p.host(), "http://localhost:11434");
    }

    #[test]
    fn trailing_slash_stripped_from_env() {
        std::env::set_var("OLLAMA_HOST", "http://localhost:11434/");
        let p = OllamaProvider::from_env().unwrap();
        assert_eq!(p.host(), "http://localhost:11434");
        std::env::remove_var("OLLAMA_HOST");
    }
}
