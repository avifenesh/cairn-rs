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

// ── (1) OllamaProvider::new — correct host (no env, no race) ─────────────────

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

// ── (2) OllamaEmbeddingProvider::new — same pattern ──────────────────────────

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

// ── (3) from_env tests — run sequentially to avoid env-var races ──────────────
//
// tokio tests run in parallel by default. Env var mutation is not thread-safe,
// so all OLLAMA_HOST set/remove operations are in a single serialised test.

#[test]
fn ollama_from_env_behaviour() {
    // Ensure unset → None for both providers.
    std::env::remove_var("OLLAMA_HOST");
    assert!(
        OllamaProvider::from_env().is_none(),
        "OllamaProvider::from_env must be None when OLLAMA_HOST unset"
    );
    assert!(
        OllamaEmbeddingProvider::from_env().is_none(),
        "OllamaEmbeddingProvider::from_env must be None when OLLAMA_HOST unset"
    );

    // Set and read back.
    std::env::set_var("OLLAMA_HOST", "http://remote-ollama:11434");
    let p = OllamaProvider::from_env().expect("must be Some when OLLAMA_HOST is set");
    assert_eq!(p.host(), "http://remote-ollama:11434");
    let e = OllamaEmbeddingProvider::from_env().expect("must be Some when OLLAMA_HOST is set");
    assert_eq!(e.host(), "http://remote-ollama:11434");

    // Trailing slash stripped.
    std::env::set_var("OLLAMA_HOST", "http://localhost:11434/");
    let p2 = OllamaProvider::from_env().unwrap();
    assert_eq!(
        p2.host(),
        "http://localhost:11434",
        "trailing slash must be stripped"
    );
    let e2 = OllamaEmbeddingProvider::from_env().unwrap();
    assert_eq!(
        e2.host(),
        "http://localhost:11434",
        "trailing slash must be stripped"
    );

    // Clean up.
    std::env::remove_var("OLLAMA_HOST");
    assert!(OllamaProvider::from_env().is_none());
    assert!(OllamaEmbeddingProvider::from_env().is_none());
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
        matches!(
            result.unwrap_err(),
            ProviderAdapterError::TransportFailure(_)
        ),
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
    let result = p
        .embed("nomic-embed-text", vec!["hello world".to_owned()])
        .await;
    assert!(
        result.is_err(),
        "embed must fail when no Ollama daemon is running"
    );
    assert!(
        matches!(
            result.unwrap_err(),
            ProviderAdapterError::TransportFailure(_)
        ),
        "error must be TransportFailure when connection is refused"
    );
}

// ── (6) Empty input — immediate return, no network call ───────────────────────

#[tokio::test]
async fn ollama_embedding_empty_texts_returns_immediately() {
    // Even with an invalid host, empty input must not reach the network.
    let p = OllamaEmbeddingProvider::new("http://127.0.0.1:19434");
    let result = p.embed("nomic-embed-text", vec![]).await;
    assert!(
        result.is_ok(),
        "empty input must succeed without network call"
    );
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
