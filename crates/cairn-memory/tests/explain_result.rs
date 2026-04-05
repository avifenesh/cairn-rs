//! RFC 003 Gap 1: explain_result — "why-this-result" explanations for operators.
//!
//! Tests that InMemoryRetrieval::explain_result() returns a populated
//! ResultExplanation with all scoring dimensions and a human-readable summary.

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use std::sync::Arc;

fn make_request(doc_id: &str, src_id: &str, content: &str, project: ProjectKey) -> IngestRequest {
    IngestRequest {
        document_id: KnowledgeDocumentId::new(doc_id),
        source_id: SourceId::new(src_id),
        source_type: SourceType::PlainText,
        project,
        content: content.to_owned(),
        tags: vec![],
        corpus_id: None,
        bundle_source_id: None,
        import_id: None,
    }
}

/// RFC 003: explain_result must be populated for a known chunk and query.
#[tokio::test]
async fn explain_result_is_populated_for_known_query() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let chunker = ParagraphChunker {
        max_chunk_size: 500,
    };
    let pipeline = IngestPipeline::new(store.clone(), chunker);
    let project = ProjectKey::new("t", "w", "p");

    pipeline
        .submit(make_request(
            "doc_explain",
            "src_explain",
            "The Rust borrow checker enforces memory safety at compile time.",
            project.clone(),
        ))
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store.clone());
    let all_chunks = store.all_chunks();
    assert!(!all_chunks.is_empty(), "must have ingested at least one chunk");

    let chunk_id = all_chunks[0].chunk_id.as_str().to_owned();

    let explanation = retrieval
        .explain_result(&chunk_id, "borrow checker memory safety", &project)
        .expect("explain_result must return Some for a known chunk");

    assert_eq!(explanation.chunk_id, chunk_id);
    assert_eq!(explanation.query_text, "borrow checker memory safety");

    assert!(
        explanation.lexical_relevance > 0.0,
        "lexical_relevance must be > 0 for a matching query"
    );

    assert!(
        explanation.freshness > 0.0,
        "freshness must be > 0 for a recently ingested chunk"
    );

    assert!(
        explanation.quality_score >= 0.0 && explanation.quality_score <= 1.0,
        "quality_score must be in [0, 1]"
    );

    assert!(
        !explanation.summary.is_empty(),
        "summary must be non-empty"
    );
}

/// RFC 003: explain_result returns None for a chunk that does not exist.
#[test]
fn explain_result_returns_none_for_unknown_chunk() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let retrieval = InMemoryRetrieval::new(store);
    let project = ProjectKey::new("t", "w", "p");

    let result = retrieval.explain_result("nonexistent_chunk", "any query", &project);
    assert!(result.is_none(), "explain_result must return None for unknown chunk_id");
}

/// RFC 003: explain_result returns None for a chunk in a different project.
#[tokio::test]
async fn explain_result_returns_none_for_different_project() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let chunker = ParagraphChunker::default();
    let pipeline = IngestPipeline::new(store.clone(), chunker);
    let project_a = ProjectKey::new("t", "w", "p_a");
    let project_b = ProjectKey::new("t", "w", "p_b");

    pipeline
        .submit(make_request("doc_proj_a", "src_a", "Content in project A.", project_a.clone()))
        .await
        .unwrap();

    let retrieval = InMemoryRetrieval::new(store.clone());
    let chunks = store.all_chunks();
    let chunk_id = chunks[0].chunk_id.as_str().to_owned();

    let result = retrieval.explain_result(&chunk_id, "content", &project_b);
    assert!(result.is_none(), "explain_result must return None for chunk in different project");

    let result_a = retrieval.explain_result(&chunk_id, "content", &project_a);
    assert!(result_a.is_some(), "explain_result must return Some for chunk in correct project");
}
