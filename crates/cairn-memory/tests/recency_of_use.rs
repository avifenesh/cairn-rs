//! RFC 003 recency-of-use scoring integration tests.

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

    let query = RetrievalQuery {
        project: project(),
        query_text: "Rust memory safety".to_owned(),
        query_embedding: None,
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
    assert_eq!(recency, Some(1.0), "within 1h of last retrieval => recency=1.0");
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
        query_embedding: None,
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
        query_embedding: None,
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

    assert!(hot.breakdown.recency_of_use.is_some(), "doc_hot recency must be Some");
    assert_eq!(cold.breakdown.recency_of_use, None, "doc_cold recency must be None");

    assert!(
        hot.score > cold.score,
        "recently-retrieved doc_hot must outscore doc_cold: {:.4} vs {:.4}",
        hot.score,
        cold.score
    );
}

/// retrieval_count increments on each query that returns the chunk.
#[tokio::test]
async fn recency_of_use_retrieval_count_increments() {
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
        query_embedding: None,
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 5,
        metadata_filters: vec![],
        scoring_policy: None,
    };

    for expected_count in 1u32..=3 {
        retrieval.query(q.clone()).await.unwrap();
        let chunks = store.all_current_chunks();
        let chunk = chunks
            .iter()
            .find(|c| c.document_id == KnowledgeDocumentId::new("doc_count"))
            .unwrap();
        assert_eq!(chunk.retrieval_count, expected_count);
    }
}

/// Tiered recency: within 1h=1.0, within 24h=0.7, within 7d=0.4, older=0.1.
#[tokio::test]
async fn recency_of_use_tiered_scoring_values() {
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

    let now = cairn_memory::retrieval::now_ms();
    let chunks = store.all_current_chunks();
    let chunk_id = chunks[0].chunk_id.as_str().to_owned();

    let q = RetrievalQuery {
        project: project(),
        query_text: "tiered recency scoring".to_owned(),
        query_embedding: None,
        mode: RetrievalMode::LexicalOnly,
        reranker: RerankerStrategy::None,
        limit: 5,
        metadata_filters: vec![],
        scoring_policy: None,
    };

    store.record_retrieval(&chunk_id, now.saturating_sub(1_800_000)); // 30min
    let r1 = retrieval.query(q.clone()).await.unwrap();
    assert_eq!(r1.results[0].breakdown.recency_of_use, Some(1.0));

    store.record_retrieval(&chunk_id, now.saturating_sub(7_200_000)); // 2h
    let r2 = retrieval.query(q.clone()).await.unwrap();
    assert_eq!(r2.results[0].breakdown.recency_of_use, Some(0.7));

    store.record_retrieval(&chunk_id, now.saturating_sub(3 * 86_400_000)); // 3d
    let r3 = retrieval.query(q.clone()).await.unwrap();
    assert_eq!(r3.results[0].breakdown.recency_of_use, Some(0.4));

    store.record_retrieval(&chunk_id, now.saturating_sub(10 * 86_400_000)); // 10d
    let r4 = retrieval.query(q.clone()).await.unwrap();
    assert_eq!(r4.results[0].breakdown.recency_of_use, Some(0.1));
}
