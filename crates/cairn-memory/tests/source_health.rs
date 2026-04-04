//! RFC 003 source health monitoring integration tests.

use std::sync::Arc;

use cairn_api::memory_api::HealthEndpoints;
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::api_impl::HealthApiImpl;
use cairn_memory::diagnostics_impl::InMemoryDiagnostics;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Ingest from src_a (good feedback) and src_b (bad feedback).
/// GET /v1/memory/health: assert src_a is healthy, src_b is degraded.
#[tokio::test]
async fn source_health_good_vs_bad_feedback() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());
    let api = HealthApiImpl::new(diagnostics.clone());

    // Register src_a and src_b via ingest.
    diagnostics.record_ingest(
        &SourceId::new("src_a"),
        &project(),
        &KnowledgeDocumentId::new("doc_a"),
        3,
        150,
    );
    diagnostics.record_ingest(
        &SourceId::new("src_b"),
        &project(),
        &KnowledgeDocumentId::new("doc_b"),
        3,
        120,
    );

    // Good feedback for src_a (rating=5.0, was_used=true).
    for _ in 0..10 {
        diagnostics.record_retrieval_feedback(&SourceId::new("src_a"), "chunk_a", true, Some(5.0));
    }

    // Bad feedback for src_b (rating=1.0, not used) — error_rate > 0.1.
    for _ in 0..10 {
        diagnostics.record_retrieval_feedback(&SourceId::new("src_b"), "chunk_b", false, Some(1.0));
    }

    let health = api.get_health(&project()).await.unwrap();

    assert!(
        health.healthy.contains(&"src_a".to_owned()),
        "src_a should be healthy, health={health:?}"
    );
    assert!(
        health.degraded.contains(&"src_b".to_owned()),
        "src_b should be degraded (error_rate > 0.1), health={health:?}"
    );
    assert!(!health.degraded.contains(&"src_a".to_owned()), "src_a must not be degraded");
    assert!(!health.healthy.contains(&"src_b".to_owned()), "src_b must not be healthy");
}

/// A source with low query_hit_rate (< 0.2) is classified as at_risk.
#[tokio::test]
async fn source_health_low_hit_rate_is_at_risk() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());
    let api = HealthApiImpl::new(diagnostics.clone());

    diagnostics.record_ingest(
        &SourceId::new("src_poor"),
        &project(),
        &KnowledgeDocumentId::new("doc_poor"),
        2,
        80,
    );

    // 10 queries, only 1 hit → hit_rate = 0.1 < 0.2 → at_risk.
    diagnostics.record_query_hit(&SourceId::new("src_poor"), true);
    for _ in 0..9 {
        diagnostics.record_query_hit(&SourceId::new("src_poor"), false);
    }

    let health = api.get_health(&project()).await.unwrap();
    assert!(
        health.at_risk.contains(&"src_poor".to_owned()),
        "src_poor with hit_rate=0.1 should be at_risk, health={health:?}"
    );
}

/// A source with no feedback or query events is classified as healthy by default.
#[tokio::test]
async fn source_health_new_source_is_healthy() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());
    let api = HealthApiImpl::new(diagnostics.clone());

    diagnostics.record_ingest(
        &SourceId::new("src_fresh"),
        &project(),
        &KnowledgeDocumentId::new("doc_fresh"),
        5,
        500,
    );

    let health = api.get_health(&project()).await.unwrap();
    assert!(
        health.healthy.contains(&"src_fresh".to_owned()),
        "new source with no errors should be healthy"
    );
}

/// avg_chunk_size_bytes is computed from actual chunk text lengths.
#[tokio::test]
async fn source_health_avg_chunk_size_bytes_computed() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    // 2 chunks, 100 bytes each → avg = 100
    diagnostics.record_ingest(
        &SourceId::new("src_size"),
        &project(),
        &KnowledgeDocumentId::new("doc_sz1"),
        2,
        200,
    );

    let record = diagnostics.source_quality_sync(&SourceId::new("src_size")).unwrap();
    assert_eq!(record.avg_chunk_size_bytes, 100, "avg = 200 bytes / 2 chunks = 100");

    // Add more chunks: 4 chunks, 400 bytes → cumulative avg = (200+400)/(2+4) = 100
    diagnostics.record_ingest(
        &SourceId::new("src_size"),
        &project(),
        &KnowledgeDocumentId::new("doc_sz2"),
        4,
        400,
    );
    let record2 = diagnostics.source_quality_sync(&SourceId::new("src_size")).unwrap();
    assert_eq!(record2.avg_chunk_size_bytes, 100, "running avg should be 100");
}

/// retrieval_count increments on each record_retrieval_feedback call.
#[tokio::test]
async fn source_health_retrieval_count_increments_on_feedback() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    diagnostics.record_ingest(
        &SourceId::new("src_cnt"),
        &project(),
        &KnowledgeDocumentId::new("doc_cnt"),
        1,
        50,
    );

    for i in 1u64..=5 {
        diagnostics.record_retrieval_feedback(&SourceId::new("src_cnt"), "chunk", true, Some(4.0));
        let record = diagnostics.source_quality_sync(&SourceId::new("src_cnt")).unwrap();
        assert_eq!(record.retrieval_count, i);
    }
}

/// Full pipeline integration: ingest docs, run retrieval, check health.
#[tokio::test]
async fn source_health_pipeline_integration() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::with_diagnostics(store.clone(), diagnostics.clone());
    let api = HealthApiImpl::new(diagnostics.clone());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_pipeline_a"),
            source_id: SourceId::new("pipeline_src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Rust memory safety ownership systems programming content".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Manually record ingest in diagnostics (simulate what cairn-app wiring does).
    let chunks = store.all_current_chunks();
    let total_bytes: u64 = chunks.iter().map(|c| c.text.len() as u64).sum();
    diagnostics.record_ingest(
        &SourceId::new("pipeline_src"),
        &project(),
        &KnowledgeDocumentId::new("doc_pipeline_a"),
        chunks.len() as u64,
        total_bytes,
    );

    // Run a query.
    retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust memory safety".to_owned(),
            query_embedding: None,
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // Source should be healthy (no errors recorded).
    let health = api.get_health(&project()).await.unwrap();
    assert!(
        health.healthy.contains(&"pipeline_src".to_owned()),
        "pipeline_src should be healthy after clean retrieval, health={health:?}"
    );
}
