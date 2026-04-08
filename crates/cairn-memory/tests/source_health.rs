//! RFC 003 source health monitoring integration tests.

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::diagnostics::DiagnosticsService;
use cairn_memory::diagnostics_impl::InMemoryDiagnostics;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Ingest from src_a (good feedback) and src_b (bad feedback).
/// src_a should have higher relevance than src_b.
#[tokio::test]
async fn source_health_good_vs_bad_feedback() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    // Register src_a and src_b via ingest.
    diagnostics.record_ingest(&SourceId::new("src_a"), &project(), 3);
    diagnostics.record_ingest(&SourceId::new("src_b"), &project(), 3);

    // Good feedback for src_a (high relevance).
    for _ in 0..10 {
        diagnostics.record_retrieval_hit(&SourceId::new("src_a"), 0.9);
    }

    // Bad feedback for src_b (low relevance).
    for _ in 0..10 {
        diagnostics.record_retrieval_hit(&SourceId::new("src_b"), 0.1);
    }

    let quality_a = diagnostics
        .source_quality(&SourceId::new("src_a"))
        .await
        .unwrap()
        .expect("src_a should have a quality record");
    let quality_b = diagnostics
        .source_quality(&SourceId::new("src_b"))
        .await
        .unwrap()
        .expect("src_b should have a quality record");

    assert!(
        quality_a.avg_relevance_score > quality_b.avg_relevance_score,
        "src_a (good) should have higher avg_relevance_score than src_b (bad): {:.2} vs {:.2}",
        quality_a.avg_relevance_score,
        quality_b.avg_relevance_score
    );
}

/// A source with no retrieval events should still have a quality record after ingest.
#[tokio::test]
async fn source_health_new_source_has_quality_record() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    diagnostics.record_ingest(&SourceId::new("src_fresh"), &project(), 5);

    let quality = diagnostics
        .source_quality(&SourceId::new("src_fresh"))
        .await
        .unwrap()
        .expect("new source should have a quality record after ingest");

    assert_eq!(quality.total_chunks, 5);
    assert_eq!(quality.total_retrievals, 0);
}

/// total_chunks accumulates across multiple ingest calls.
#[tokio::test]
async fn source_health_chunk_count_accumulates() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    // 2 chunks first ingest.
    diagnostics.record_ingest(&SourceId::new("src_size"), &project(), 2);

    let record = diagnostics
        .source_quality(&SourceId::new("src_size"))
        .await
        .unwrap()
        .expect("should have record");
    assert_eq!(record.total_chunks, 2);

    // Add more chunks: cumulative total = 2 + 4 = 6.
    diagnostics.record_ingest(&SourceId::new("src_size"), &project(), 4);
    let record2 = diagnostics
        .source_quality(&SourceId::new("src_size"))
        .await
        .unwrap()
        .expect("should have record");
    assert_eq!(
        record2.total_chunks, 6,
        "total_chunks should accumulate: 2 + 4 = 6"
    );
}

/// total_retrievals increments on each record_retrieval_hit call.
#[tokio::test]
async fn source_health_retrieval_count_increments_on_hit() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    diagnostics.record_ingest(&SourceId::new("src_cnt"), &project(), 1);

    for i in 1u64..=5 {
        diagnostics.record_retrieval_hit(&SourceId::new("src_cnt"), 0.7);
        let record = diagnostics
            .source_quality(&SourceId::new("src_cnt"))
            .await
            .unwrap()
            .expect("should have record");
        assert_eq!(record.total_retrievals, i);
    }
}

/// Full pipeline integration: ingest docs, run retrieval, check diagnostics.
#[tokio::test]
async fn source_health_pipeline_integration() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::with_diagnostics(store.clone(), diagnostics.clone());

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
    diagnostics.record_ingest(
        &SourceId::new("pipeline_src"),
        &project(),
        chunks.len() as u64,
    );

    // Run a query.
    retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "Rust memory safety".to_owned(),
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 5,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    // Source should have a quality record after ingest.
    let quality = diagnostics
        .source_quality(&SourceId::new("pipeline_src"))
        .await
        .unwrap()
        .expect("pipeline_src should have a quality record after ingest");
    assert!(
        quality.total_chunks > 0,
        "pipeline_src should have chunks after ingest, got {}",
        quality.total_chunks
    );
}
