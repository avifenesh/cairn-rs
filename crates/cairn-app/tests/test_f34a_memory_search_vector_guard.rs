//! F34a: `memory_search` must not crash the orchestrator when the LLM calls
//! it with `mode=vector` on a retrieval backend that has no embedding
//! provider configured.
//!
//! Before this fix, `InMemoryRetrieval` returned
//! `RetrievalError::Internal("VectorOnly mode requires an embedding
//! provider ...")`, the tool propagated that as `ToolError::Transient`, and
//! the orchestrator terminated the run (`Gather(...)` failure).
//!
//! After the fix, the tool detects that specific error, falls back to
//! `LexicalOnly`, and attaches a `mode_clamped` diagnostic so the LLM can
//! adapt on the next step.

use std::sync::Arc;

use cairn_app::tool_impls::ConcreteMemorySearchTool;
use cairn_domain::ProjectKey;
use cairn_memory::{
    in_memory::{InMemoryDocumentStore, InMemoryRetrieval},
    retrieval::RetrievalService,
};
use cairn_tools::builtins::{ToolHandler, ToolResult};

fn project() -> ProjectKey {
    ProjectKey::new("t-f34a", "w-f34a", "p-f34a")
}

/// Retrieval with no embedder configured — exactly the production default
/// today, where the real embedding stack is planned to ship with the
/// external memory crate.
fn retrieval_without_embedder() -> Arc<dyn RetrievalService> {
    let store = Arc::new(InMemoryDocumentStore::new());
    Arc::new(InMemoryRetrieval::new(store))
}

fn extract_ok(result: Result<ToolResult, cairn_tools::builtins::ToolError>) -> serde_json::Value {
    match result {
        Ok(tr) => {
            assert!(!tr.truncated, "unexpected truncation for a tiny payload");
            tr.output
        }
        Err(e) => panic!("tool returned an error that would terminate the run: {e}"),
    }
}

#[tokio::test]
async fn mode_vector_clamps_to_lexical_instead_of_terminating_run() {
    let tool = ConcreteMemorySearchTool::new(retrieval_without_embedder());

    let result = tool
        .execute(
            &project(),
            serde_json::json!({ "query": "anything", "mode": "vector" }),
        )
        .await;

    let value = extract_ok(result);
    // Result payload shape is preserved.
    assert!(value.get("results").is_some(), "missing results");
    assert!(value.get("total").is_some(), "missing total");

    // Diagnostic tells the LLM what happened and why.
    let clamp = value
        .get("mode_clamped")
        .expect("clamp diagnostic should be present when vector was requested");
    assert_eq!(clamp["from"], "vector");
    assert_eq!(clamp["to"], "lexical");
    assert!(
        clamp["reason"]
            .as_str()
            .map(|s| s.contains("embedding"))
            .unwrap_or(false),
        "reason should mention embedding provider: {clamp:?}"
    );
}

#[tokio::test]
async fn mode_lexical_succeeds_without_diagnostic() {
    let tool = ConcreteMemorySearchTool::new(retrieval_without_embedder());

    let result = tool
        .execute(
            &project(),
            serde_json::json!({ "query": "anything", "mode": "lexical" }),
        )
        .await;

    let value = extract_ok(result);
    assert!(value.get("results").is_some());
    assert!(
        value.get("mode_clamped").is_none(),
        "lexical mode must not emit a clamp diagnostic: {value}"
    );
}

/// Hybrid without an embedder already degrades to lexical inside
/// `InMemoryRetrieval` itself, so the tool sees a successful response and
/// does not need to clamp. We assert the run-survival contract (no error)
/// and document the current behavior: no `mode_clamped` diagnostic is
/// surfaced today because the backend silently falls back.
#[tokio::test]
async fn mode_hybrid_succeeds_via_backend_fallback() {
    let tool = ConcreteMemorySearchTool::new(retrieval_without_embedder());

    let result = tool
        .execute(
            &project(),
            serde_json::json!({ "query": "anything", "mode": "hybrid" }),
        )
        .await;

    let value = extract_ok(result);
    assert!(value.get("results").is_some());
    // Current behavior: backend-level fallback is silent, so no tool-level
    // clamp diagnostic. Documented here so a future change to backend
    // behavior (e.g. strict hybrid) trips this assertion deliberately.
    assert!(
        value.get("mode_clamped").is_none(),
        "hybrid currently falls back inside the backend; update this test \
         if the backend starts erroring on hybrid-without-embedder: {value}"
    );
}

#[tokio::test]
async fn empty_query_still_rejected_as_invalid_args() {
    // The vector-guard fix must not weaken the existing empty-query check.
    let tool = ConcreteMemorySearchTool::new(retrieval_without_embedder());

    let result = tool
        .execute(
            &project(),
            serde_json::json!({ "query": "   ", "mode": "vector" }),
        )
        .await;

    match result {
        Err(cairn_tools::builtins::ToolError::InvalidArgs { field, .. }) => {
            assert_eq!(field, "query");
        }
        other => panic!("expected InvalidArgs for empty query, got {other:?}"),
    }
}
