//! Ollama local embedding provider (RFC 009).
//!
//! Implements [`EmbeddingProvider`] against Ollama's native `/api/embed`
//! endpoint, which returns dense float vectors for a batch of input texts.
//! This enables fully local RAG pipelines without any external API key.
//!
//! ## Endpoint
//!
//! ```text
//! POST http://OLLAMA_HOST/api/embed
//! { "model": "nomic-embed-text", "input": ["text a", "text b"] }
//! ```
//!
//! ## Usage
//!
//! ```rust,ignore
//! let embedder = OllamaEmbeddingProvider::default_local();
//! let response = embedder.embed("nomic-embed-text", vec!["hello world".into()]).await?;
//! ```

use cairn_domain::providers::{EmbeddingProvider, EmbeddingResponse, ProviderAdapterError};
use serde::{Deserialize, Serialize};

// ── Wire types ────────────────────────────────────────────────────────────────

/// Request body for `POST /api/embed`.
#[derive(Serialize)]
struct EmbedRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

/// Response from `POST /api/embed`.
///
/// Ollama returns a 2-D array of float vectors, one per input text.
#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
    /// Total prompt token count across all inputs (may be absent on older
    /// Ollama versions).
    #[serde(default)]
    prompt_eval_count: Option<u32>,
    /// Model name echoed back by the server.
    #[serde(default)]
    model: Option<String>,
}

// ── Provider ──────────────────────────────────────────────────────────────────

/// Ollama-backed embedding provider.
///
/// Uses the `/api/embed` endpoint (added in Ollama 0.1.26).
pub struct OllamaEmbeddingProvider {
    host: String,
    client: reqwest::Client,
}

impl OllamaEmbeddingProvider {
    /// Create a provider pointing at the given Ollama host URL.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120))
                .build()
                .expect("failed to build reqwest client"),
        }
    }

    /// Create a provider using the default `http://localhost:11434` host.
    pub fn default_local() -> Self {
        Self::new("http://localhost:11434")
    }

    /// Create a provider from the `OLLAMA_HOST` environment variable.
    ///
    /// Returns `None` when the variable is unset.
    pub fn from_env() -> Option<Self> {
        let host = std::env::var("OLLAMA_HOST").ok()?;
        Some(Self::new(host.trim_end_matches('/')))
    }

    /// Return the base host URL this provider is configured for.
    pub fn host(&self) -> &str {
        &self.host
    }
}

// ── EmbeddingProvider impl ────────────────────────────────────────────────────

#[async_trait::async_trait]
impl EmbeddingProvider for OllamaEmbeddingProvider {
    /// Embed a batch of `texts` using the specified `model_id`.
    ///
    /// Returns one vector per input text, in the same order as `texts`.
    /// The `token_count` field is the sum of prompt tokens across all inputs
    /// (0 when the server omits usage data).
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

        let url = format!("{}/api/embed", self.host);
        let body = EmbedRequest {
            model: model_id,
            input: &texts,
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
            let body_text = http_resp.text().await.unwrap_or_default();
            return Err(ProviderAdapterError::ProviderError(format!(
                "Ollama /api/embed returned HTTP {status}: {body_text}"
            )));
        }

        let embed: EmbedResponse = http_resp
            .json()
            .await
            .map_err(|e| ProviderAdapterError::ProviderError(e.to_string()))?;

        // Validate the server returned the expected number of vectors.
        if embed.embeddings.len() != texts.len() {
            return Err(ProviderAdapterError::ProviderError(format!(
                "Ollama returned {} embedding(s) for {} input(s)",
                embed.embeddings.len(),
                texts.len(),
            )));
        }

        Ok(EmbeddingResponse {
            embeddings: embed.embeddings,
            model_id: embed.model.unwrap_or_else(|| model_id.to_owned()),
            token_count: embed.prompt_eval_count.unwrap_or(0),
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
        assert!(OllamaEmbeddingProvider::from_env().is_none());
    }

    #[test]
    fn from_env_uses_ollama_host_env_var() {
        std::env::set_var("OLLAMA_HOST", "http://embed-box:11434");
        let p = OllamaEmbeddingProvider::from_env().unwrap();
        assert_eq!(p.host(), "http://embed-box:11434");
        std::env::remove_var("OLLAMA_HOST");
    }

    #[test]
    fn default_local_points_at_localhost() {
        let p = OllamaEmbeddingProvider::default_local();
        assert_eq!(p.host(), "http://localhost:11434");
    }

    #[test]
    fn trailing_slash_stripped_from_env() {
        std::env::set_var("OLLAMA_HOST", "http://localhost:11434/");
        let p = OllamaEmbeddingProvider::from_env().unwrap();
        assert_eq!(p.host(), "http://localhost:11434");
        std::env::remove_var("OLLAMA_HOST");
    }

    /// Empty input returns an empty response without hitting the network.
    #[tokio::test]
    async fn empty_texts_returns_empty_embeddings() {
        let p = OllamaEmbeddingProvider::default_local();
        let resp = p.embed("nomic-embed-text", vec![]).await.unwrap();
        assert!(resp.embeddings.is_empty());
        assert_eq!(resp.token_count, 0);
        assert_eq!(resp.model_id, "nomic-embed-text");
    }
}
