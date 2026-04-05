//! Integration tests for the Ollama provider and embedding provider.
//!
//! These tests do NOT require Ollama to be running — they validate:
//!   (1) OllamaProvider::from_env returns None when OLLAMA_HOST unset
//!   (2) OllamaProvider::new creates provider with correct host
//!   (3) OllamaEmbeddingProvider::from_env same pattern
//!   (4) health_check returns a transport error when no server reachable
//!   (5) OllamaEmbeddingProvider::embed returns transport error when no server
//!   (6) Empty text input returns immediately without network call
//!   (7) from_env strips trailing slash

use cairn_domain::providers::{EmbeddingProvider, ProviderAdapterError};
use cairn_runtime::services::{OllamaEmbeddingProvider, OllamaProvider};

// ── (1) OllamaProvider::from_env — None when unset ───────────────────────────

#[tokio::test]
async fn ollama_provider_from_env_returns_none_when_unset() {
    std::env::remove_var("OLLAMA_HOST");
    assert!(
        OllamaProvider::from_env().is_none(),
        "from_env must return None when OLLAMA_HOST is not set"
    );
}

// ── (2) OllamaProvider::new — correct host ────────────────────────────────────

#[tokio::test]
async fn ollama_provider_new_stores_host() {
    let p = OllamaProvider::new("http://gpu-box:11434");
    assert_eq!(p.host(), "http://gpu-box:11434");
}

#[tokio::test]
async fn ollama_provider_default_local_uses_localhost() {
    let p = OllamaProvider::default_local();
    assert_eq!(p.host(), "http://localhost:11434");
}

#[tokio::test]
async fn ollama_provider_from_env_uses_env_var() {
    std::env::set_var("OLLAMA_HOST", "http://remote-ollama:11434");
    let p = OllamaProvider::from_env().expect("must be Some when OLLAMA_HOST is set");
    assert_eq!(p.host(), "http://remote-ollama:11434");
    std::env::remove_var("OLLAMA_HOST");
}

#[tokio::test]
async fn ollama_provider_from_env_strips_trailing_slash() {
    std::env::set_var("OLLAMA_HOST", "http://localhost:11434/");
    let p = OllamaProvider::from_env().unwrap();
    assert_eq!(p.host(), "http://localhost:11434", "trailing slash must be stripped");
    std::env::remove_var("OLLAMA_HOST");
}

// ── (3) OllamaEmbeddingProvider::from_env — same pattern ─────────────────────

#[tokio::test]
async fn ollama_embedding_from_env_returns_none_when_unset() {
    std::env::remove_var("OLLAMA_HOST");
    assert!(
        OllamaEmbeddingProvider::from_env().is_none(),
        "from_env must return None when OLLAMA_HOST is not set"
    );
}

#[tokio::test]
async fn ollama_embedding_new_stores_host() {
    let p = OllamaEmbeddingProvider::new("http://embed-server:11434");
    assert_eq!(p.host(), "http://embed-server:11434");
}

#[tokio::test]
async fn ollama_embedding_default_local_uses_localhost() {
    let p = OllamaEmbeddingProvider::default_local();
    assert_eq!(p.host(), "http://localhost:11434");
}

#[tokio::test]
async fn ollama_embedding_from_env_strips_trailing_slash() {
    std::env::set_var("OLLAMA_HOST", "http://localhost:11434/");
    let p = OllamaEmbeddingProvider::from_env().unwrap();
    assert_eq!(p.host(), "http://localhost:11434");
    std::env::remove_var("OLLAMA_HOST");
}

// ── (4) health_check — error when no server reachable ────────────────────────

#[tokio::test]
async fn ollama_health_check_fails_when_no_server() {
    // Point at a port that is definitely not listening.
    let p = OllamaProvider::new("http://127.0.0.1:19434");
    let result = p.health_check().await;
    assert!(
        result.is_err(),
        "health_check must fail when no Ollama daemon is running"
    );
    // Must be a transport failure, not a logic error.
    assert!(
        matches!(result.unwrap_err(), ProviderAdapterError::TransportFailure(_)),
        "error must be TransportFailure when connection is refused"
    );
}

#[tokio::test]
async fn ollama_is_healthy_returns_false_when_no_server() {
    let p = OllamaProvider::new("http://127.0.0.1:19434");
    assert!(
        !p.is_healthy().await,
        "is_healthy must return false when server is unreachable"
    );
}

// ── (5) OllamaEmbeddingProvider::embed — transport error when no server ───────

#[tokio::test]
async fn ollama_embedding_fails_when_no_server() {
    let p = OllamaEmbeddingProvider::new("http://127.0.0.1:19434");
    let result = p.embed("nomic-embed-text", vec!["hello world".to_owned()]).await;
    assert!(
        result.is_err(),
        "embed must fail when no Ollama daemon is running"
    );
    assert!(
        matches!(result.unwrap_err(), ProviderAdapterError::TransportFailure(_)),
        "error must be TransportFailure when connection is refused"
    );
}

// ── (6) Empty input — immediate return, no network call ───────────────────────

#[tokio::test]
async fn ollama_embedding_empty_texts_returns_immediately() {
    // Even with an invalid host, empty input must not reach the network.
    let p = OllamaEmbeddingProvider::new("http://127.0.0.1:19434");
    let result = p.embed("nomic-embed-text", vec![]).await;
    assert!(result.is_ok(), "empty input must succeed without network call");
    let resp = result.unwrap();
    assert!(resp.embeddings.is_empty());
    assert_eq!(resp.token_count, 0);
    assert_eq!(resp.model_id, "nomic-embed-text");
}

// ── (7) list_models error path ────────────────────────────────────────────────

#[tokio::test]
async fn ollama_list_models_fails_when_no_server() {
    let p = OllamaProvider::new("http://127.0.0.1:19434");
    let result = p.list_models().await;
    assert!(
        result.is_err(),
        "list_models must fail when no Ollama daemon is running"
    );
}
