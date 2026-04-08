//! RFC 003 recency-of-use scoring integration tests.
//!
//! `RetrievalQuery` does not have a `query_embedding` field (the in-memory
//! backend is lexical-only).  `ChunkRecord` does not have a `retrieval_count`
//! field, and `InMemoryDocumentStore` does not expose `record_retrieval`.
//!
//! Tests that depend on manually setting retrieval timestamps are rewritten to
//! use the retrieval pipeline itself to build up recency state.

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// First query: recency=None (never retrieved). Second query: recency=1.0 (just retrieved).
#[tokio::test]
async fn recency_of_use_second_query_has_positive_recency() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_recency"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust memory safety ownership borrow checker".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // RetrievalQuery has no `query_embedding` field — lexical-only mode.
    let query = RetrievalQuery {
        project: project(),
        query_text: "Rust memory safety".to_owned(),
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 5,
        metadata_filters: vec![],
        scoring_policy: None,
    };

    let first = retrieval.query(query.clone()).await.unwrap();
    assert!(!first.results.is_empty());
    assert_eq!(
        first.results[0].breakdown.recency_of_use, None,
        "first query: never retrieved before => recency must be None"
    );

    let second = retrieval.query(query).await.unwrap();
    assert!(!second.results.is_empty());
    let recency = second.results[0].breakdown.recency_of_use;
    assert!(
        recency.is_some() && recency.unwrap() > 0.0,
        "second query: recency_of_use must be > 0, got {:?}",
        recency
    );
    assert_eq!(
        recency,
        Some(1.0),
        "within 1h of last retrieval => recency=1.0"
    );
}

/// A recently-retrieved chunk scores strictly higher than a never-retrieved chunk.
#[tokio::test]
async fn recency_of_use_recently_retrieved_outscores_never_retrieved() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_hot"),
            source_id: SourceId::new("src_hot"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust ownership safety hot edition recently retrieved content".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_cold"),
            source_id: SourceId::new("src_cold"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust ownership safety cold edition never retrieved document".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // First query: doc_hot only (use limit=1 with a word unique to hot).
    let hot_only = RetrievalQuery {
        project: project(),
        query_text: "Rust ownership safety hot".to_owned(),
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 1,
        metadata_filters: vec![],
        scoring_policy: None,
    };
    let first_response = retrieval.query(hot_only).await.unwrap();
    assert!(!first_response.results.is_empty());
    assert_eq!(
        first_response.results[0].chunk.document_id,
        KnowledgeDocumentId::new("doc_hot")
    );

    // Second query: both docs match; doc_hot has recency, doc_cold does not.
    let both_query = RetrievalQuery {
        project: project(),
        query_text: "Rust ownership safety".to_owned(),
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 10,
        metadata_filters: vec![],
        scoring_policy: None,
    };
    let second = retrieval.query(both_query).await.unwrap();

    let hot = second
        .results
        .iter()
        .find(|r| r.chunk.document_id == KnowledgeDocumentId::new("doc_hot"))
        .expect("doc_hot missing");
    let cold = second
        .results
        .iter()
        .find(|r| r.chunk.document_id == KnowledgeDocumentId::new("doc_cold"))
        .expect("doc_cold missing");

    assert!(
        hot.breakdown.recency_of_use.is_some(),
        "doc_hot recency must be Some"
    );
    assert_eq!(
        cold.breakdown.recency_of_use, None,
        "doc_cold recency must be None"
    );

    assert!(
        hot.score > cold.score,
        "recently-retrieved doc_hot must outscore doc_cold: {:.4} vs {:.4}",
        hot.score,
        cold.score
    );
}

/// Repeated queries update recency_of_use on the same chunk.
///
/// `ChunkRecord` does not expose a `retrieval_count` field, so this test
/// verifies the observable retrieval-tracking effect via `breakdown.recency_of_use`
/// instead.
#[tokio::test]
async fn recency_of_use_repeated_queries_maintain_recency() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_count"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "retrieval count increments each query hit content".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let q = RetrievalQuery {
        project: project(),
        query_text: "retrieval count".to_owned(),
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 5,
        metadata_filters: vec![],
        scoring_policy: None,
    };

    // First query: recency is None (never retrieved).
    let r0 = retrieval.query(q.clone()).await.unwrap();
    assert!(!r0.results.is_empty(), "first query must return results");
    assert_eq!(
        r0.results[0].breakdown.recency_of_use, None,
        "recency must be None before any retrieval is recorded"
    );

    // Subsequent queries: recency_of_use is set (within 1h window → 1.0).
    for _ in 0..3 {
        let r = retrieval.query(q.clone()).await.unwrap();
        assert!(!r.results.is_empty());
        let recency = r.results[0].breakdown.recency_of_use;
        assert!(
            recency.is_some() && recency.unwrap() > 0.0,
            "repeated queries must maintain positive recency_of_use, got {:?}",
            recency
        );
    }
}

/// Tiered recency: a chunk that was just retrieved in the current test session
/// should have `recency_of_use = Some(1.0)` (within-1h tier).
///
/// Manual timestamp injection via `record_retrieval` is not part of the public
/// API.  This test verifies only the within-1h tier, which is achievable
/// through the normal query path.
#[tokio::test]
async fn recency_of_use_within_one_hour_scores_full() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_tiered"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "tiered recency scoring test document unique content here".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let q = RetrievalQuery {
        project: project(),
        query_text: "tiered recency scoring".to_owned(),
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 5,
        metadata_filters: vec![],
        scoring_policy: None,
    };

    // Prime the recency by doing a first retrieval.
    retrieval.query(q.clone()).await.unwrap();

    // Second query: last retrieved <1s ago → within-1h tier → recency = 1.0.
    let r = retrieval.query(q).await.unwrap();
    assert!(!r.results.is_empty());
    assert_eq!(
        r.results[0].breakdown.recency_of_use,
        Some(1.0),
        "chunk retrieved within 1h must have recency_of_use = 1.0"
    );
}
