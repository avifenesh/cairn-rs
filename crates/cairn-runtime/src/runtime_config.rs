//! Hot-reloadable runtime configuration via DefaultsService.
//!
//! `RuntimeConfig` wraps [`DefaultsService`] and provides typed accessors for
//! model settings and operational knobs.  Every accessor follows this priority
//! chain:
//!
//! 1. **DefaultsService** (store-backed, changeable via `PUT /v1/settings/defaults/…`
//!    without a server restart)
//! 2. **Environment variable** (set at process startup)
//! 3. **Hardcoded default** (compile-time constant)
//!
//! The key names used for DefaultsService are intentionally short
//! (e.g. `"generate_model"`) so they can be set via the API:
//!
//! ```text
//! PUT /v1/settings/defaults/system/generate_model
//! { "value": "llama3.2:3b" }
//! ```

use std::sync::Arc;

use cairn_domain::Scope;
use cairn_store::projections::DefaultsReadModel;

// ── Setting keys ──────────────────────────────────────────────────────────────

/// DefaultsService key for the primary generation model (worker/everyday path).
pub const KEY_GENERATE_MODEL: &str = "generate_model";
/// DefaultsService key for the brain model (compute-heavy / reasoning path).
pub const KEY_BRAIN_MODEL: &str = "brain_model";
/// DefaultsService key for the SSE streaming model.
pub const KEY_STREAM_MODEL: &str = "stream_model";
/// DefaultsService key for the embedding model (OpenAI-compat path).
pub const KEY_EMBED_MODEL: &str = "embed_model";
/// DefaultsService key for the embedding model when Ollama is active.
pub const KEY_OLLAMA_EMBED_MODEL: &str = "ollama_embed_model";
/// DefaultsService key for max output tokens.
pub const KEY_MAX_TOKENS: &str = "max_tokens";
/// DefaultsService key for comma-separated thinking-mode model prefixes.
pub const KEY_THINKING_MODEL_PREFIXES: &str = "thinking_model_prefixes";
/// DefaultsService key for the brain inference endpoint URL.
pub const KEY_BRAIN_URL: &str = "brain_url";
/// DefaultsService key for the worker inference endpoint URL.
pub const KEY_WORKER_URL: &str = "worker_url";

// ── RuntimeConfig ─────────────────────────────────────────────────────────────

/// Hot-reloadable runtime configuration.
///
/// Wraps any type that implements [`DefaultsReadModel`] (e.g. `InMemoryStore`)
/// to read system-scoped settings. The `Arc<dyn …>` is type-erased so
/// `RuntimeConfig` can be stored in `AppState` and `InMemoryServices` without
/// generic parameters.
pub struct RuntimeConfig {
    store: Arc<dyn DefaultsReadModel + Send + Sync>,
}

impl RuntimeConfig {
    /// Create a config backed by the given store.
    ///
    /// Pass `runtime.store.clone()` from `InMemoryServices`.
    pub fn new(store: Arc<dyn DefaultsReadModel + Send + Sync>) -> Self {
        Self { store }
    }

    /// Read a string setting using the three-layer fallback.
    async fn get_string(&self, key: &str, env_var: &str, fallback: &str) -> String {
        // 1. Store (hot-reloadable via DefaultsService::set)
        if let Ok(Some(setting)) = self.store.get(Scope::System, "system", key).await {
            if let Some(s) = setting.value.as_str() {
                return s.to_owned();
            }
        }
        // 2. Environment variable
        if let Ok(s) = std::env::var(env_var) {
            if !s.is_empty() {
                return s;
            }
        }
        // 3. Hardcoded default
        fallback.to_owned()
    }

    // ── Typed accessors ───────────────────────────────────────────────────────

    /// Default model for generation requests (worker/everyday path, openai-compat).
    ///
    /// Key: `generate_model` · Env: `CAIRN_DEFAULT_GENERATE_MODEL` · Default: `qwen3.5:9b`
    pub async fn default_generate_model(&self) -> String {
        self.get_string(KEY_GENERATE_MODEL, "CAIRN_DEFAULT_GENERATE_MODEL", "qwen3.5:9b")
            .await
    }

    /// Default model for the brain (compute-heavy / reasoning) path.
    ///
    /// Key: `brain_model` · Env: `CAIRN_BRAIN_MODEL` · Default: `cyankiwi/gemma-4-31B-it-AWQ-4bit`
    pub async fn default_brain_model(&self) -> String {
        self.get_string(
            KEY_BRAIN_MODEL,
            "CAIRN_BRAIN_MODEL",
            "cyankiwi/gemma-4-31B-it-AWQ-4bit",
        )
        .await
    }

    /// Base URL for the brain inference endpoint.
    ///
    /// Key: `brain_url` · Env: `CAIRN_BRAIN_URL`
    /// Default: `https://agntic.garden/inference/brain/v1`
    pub async fn brain_url(&self) -> String {
        self.get_string(
            KEY_BRAIN_URL,
            "CAIRN_BRAIN_URL",
            "https://agntic.garden/inference/brain/v1",
        )
        .await
    }

    /// Base URL for the worker inference endpoint (everyday generation + embeddings).
    ///
    /// Key: `worker_url` · Env: `CAIRN_WORKER_URL`
    /// Default: `https://agntic.garden/inference/worker/v1`
    pub async fn worker_url(&self) -> String {
        self.get_string(
            KEY_WORKER_URL,
            "CAIRN_WORKER_URL",
            "https://agntic.garden/inference/worker/v1",
        )
        .await
    }

    /// Default model for SSE token-streaming.
    ///
    /// Key: `stream_model` · Env: `CAIRN_DEFAULT_STREAM_MODEL` · Default: `qwen3.5:9b`
    pub async fn default_stream_model(&self) -> String {
        self.get_string(KEY_STREAM_MODEL, "CAIRN_DEFAULT_STREAM_MODEL", "qwen3.5:9b")
            .await
    }

    /// Default embedding model (OpenAI-compat provider path).
    ///
    /// Key: `embed_model` · Env: `CAIRN_DEFAULT_EMBED_MODEL` · Default: `qwen3-embedding:8b`
    pub async fn default_embed_model(&self) -> String {
        self.get_string(KEY_EMBED_MODEL, "CAIRN_DEFAULT_EMBED_MODEL", "qwen3-embedding:8b")
            .await
    }

    /// Default embedding model when the Ollama provider is active.
    ///
    /// Key: `ollama_embed_model` · Env: `CAIRN_DEFAULT_OLLAMA_EMBED` · Default: `nomic-embed-text`
    pub async fn default_ollama_embed_model(&self) -> String {
        self.get_string(
            KEY_OLLAMA_EMBED_MODEL,
            "CAIRN_DEFAULT_OLLAMA_EMBED",
            "nomic-embed-text",
        )
        .await
    }

    /// Default max output tokens for generation calls.
    ///
    /// Key: `max_tokens` · Env: `CAIRN_DEFAULT_MAX_TOKENS` · Default: `4096`
    pub async fn default_max_tokens(&self) -> u32 {
        let s = self
            .get_string(KEY_MAX_TOKENS, "CAIRN_DEFAULT_MAX_TOKENS", "4096")
            .await;
        s.parse().unwrap_or(4096)
    }

    /// Comma-separated model-name prefixes that require `think: false` to
    /// suppress chain-of-thought reasoning (e.g. Qwen3 models).
    ///
    /// Key: `thinking_model_prefixes` · Env: `CAIRN_THINKING_MODELS` · Default: `qwen3`
    pub async fn thinking_model_prefixes(&self) -> Vec<String> {
        let s = self
            .get_string(
                KEY_THINKING_MODEL_PREFIXES,
                "CAIRN_THINKING_MODELS",
                "qwen3",
            )
            .await;
        s.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Return `true` when `model_id` starts with any thinking-mode prefix.
    pub async fn supports_thinking_mode(&self, model_id: &str) -> bool {
        self.thinking_model_prefixes()
            .await
            .iter()
            .any(|prefix| model_id.contains(prefix.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_store::InMemoryStore;

    fn make_config() -> RuntimeConfig {
        let store = Arc::new(InMemoryStore::new());
        RuntimeConfig::new(store)
    }

    /// Without any store value or env var, returns the hardcoded default.
    #[tokio::test]
    async fn hardcoded_fallback_when_no_store_or_env() {
        let cfg = make_config();
        assert_eq!(cfg.default_generate_model().await, "qwen3.5:9b");
        assert_eq!(cfg.default_stream_model().await, "qwen3.5:9b");
        assert_eq!(cfg.default_embed_model().await, "qwen3-embedding:8b");
        assert_eq!(cfg.default_ollama_embed_model().await, "nomic-embed-text");
        assert_eq!(cfg.default_max_tokens().await, 4096);
        assert_eq!(cfg.thinking_model_prefixes().await, vec!["qwen3"]);
    }

    /// Store-backed value takes precedence over env and hardcoded default.
    #[tokio::test]
    async fn store_value_wins_over_env_and_default() {
        use cairn_domain::{DefaultSettingSet, RuntimeEvent, Scope};
        use cairn_store::EventLog;

        let store = Arc::new(InMemoryStore::new());
        // Write a system-scoped default setting directly to the store.
        store
            .append(&[cairn_domain::EventEnvelope::for_runtime_event(
                cairn_domain::EventId::new("evt_cfg_test"),
                cairn_domain::EventSource::System,
                RuntimeEvent::DefaultSettingSet(DefaultSettingSet {
                    scope: Scope::System,
                    scope_id: "system".to_owned(),
                    key: KEY_GENERATE_MODEL.to_owned(),
                    value: serde_json::json!("llama3.2:3b"),
                }),
            )])
            .await
            .unwrap();

        let cfg = RuntimeConfig::new(store);
        assert_eq!(
            cfg.default_generate_model().await,
            "llama3.2:3b",
            "store value must override hardcoded default"
        );
    }

    /// supports_thinking_mode uses the thinking_model_prefixes list.
    #[tokio::test]
    async fn supports_thinking_mode_matches_prefix() {
        let cfg = make_config();
        assert!(cfg.supports_thinking_mode("qwen3.5:9b").await);
        assert!(cfg.supports_thinking_mode("qwen3:8b").await);
        assert!(!cfg.supports_thinking_mode("llama3.2:3b").await);
        assert!(!cfg.supports_thinking_mode("nomic-embed-text").await);
    }
}
