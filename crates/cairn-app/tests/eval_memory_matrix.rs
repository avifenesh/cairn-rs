//! Integration test: eval memory quality matrix builds from IngestDiagnostics.

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_evals::EvalRunService;
use cairn_evals::services::eval_service::{MemoryDiagnosticsSource, SourceQualitySnapshot};
use cairn_memory::diagnostics::DiagnosticsService;
use cairn_memory::diagnostics_impl::InMemoryDiagnostics;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Adapter from InMemoryDiagnostics to cairn-evals MemoryDiagnosticsSource.
struct DiagnosticsAdapter(Arc<InMemoryDiagnostics>);

#[async_trait::async_trait]
impl MemoryDiagnosticsSource for DiagnosticsAdapter {
    async fn list_source_quality(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<SourceQualitySnapshot>, String> {
        let records = DiagnosticsService::list_source_quality(self.0.as_ref(), project, limit)
            .await
            .map_err(|e| e.to_string())?;
        Ok(records
            .into_iter()
            .map(|r| SourceQualitySnapshot {
                source_id: r.source_id.clone(),
                total_chunks: r.total_chunks,
                credibility_score: r.credibility_score,
                retrieval_count: r.retrieval_count,
                query_hit_rate: r.query_hit_rate,
                error_rate: r.error_rate,
                last_ingested_at: r.last_ingested_at,
            })
            .collect())
    }
}

#[tokio::test]
async fn eval_memory_matrix_two_sources_appear_with_correct_chunk_count() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    // Ingest docs from 2 different sources.
    diagnostics.record_ingest(
        &SourceId::new("src_alpha"),
        &project(),
        &KnowledgeDocumentId::new("doc_a1"),
        5,
        250,
    );
    diagnostics.record_ingest(
        &SourceId::new("src_alpha"),
        &project(),
        &KnowledgeDocumentId::new("doc_a2"),
        3,
        150,
    );
    diagnostics.record_ingest(
        &SourceId::new("src_beta"),
        &project(),
        &KnowledgeDocumentId::new("doc_b1"),
        7,
        350,
    );

    // Record some quality metrics.
    diagnostics.record_retrieval_feedback(&SourceId::new("src_alpha"), "chunk_a", true, Some(4.5));
    diagnostics.record_retrieval_feedback(&SourceId::new("src_beta"), "chunk_b", false, Some(1.0));

    let adapter: Arc<dyn MemoryDiagnosticsSource> = Arc::new(DiagnosticsAdapter(diagnostics));
    let eval_svc = EvalRunService::with_diagnostics(adapter);
    let matrix = eval_svc.build_memory_quality_matrix(&project()).await.unwrap();

    assert_eq!(
        matrix.rows.len(), 2,
        "should have one row per source: {:?}",
        matrix.rows.iter().map(|r| &r.source_id).collect::<Vec<_>>()
    );

    let alpha = matrix.rows.iter().find(|r| r.source_id == "src_alpha").expect("src_alpha");
    let beta  = matrix.rows.iter().find(|r| r.source_id == "src_beta").expect("src_beta");

    assert_eq!(alpha.chunk_count, 8, "src_alpha: 5+3=8 chunks");
    assert_eq!(beta.chunk_count, 7,  "src_beta: 7 chunks");

    assert!(
        beta.error_rate > 0.0,
        "src_beta bad feedback → error_rate > 0, got {}",
        beta.error_rate
    );
    assert_eq!(alpha.error_rate, 0.0, "src_alpha good feedback → error_rate=0");
}

#[tokio::test]
async fn eval_memory_matrix_empty_when_no_diagnostics() {
    let eval_svc = EvalRunService::new();
    let matrix = eval_svc.build_memory_quality_matrix(&project()).await.unwrap();
    assert!(matrix.rows.is_empty(), "no diagnostics → empty matrix");
}

#[tokio::test]
async fn eval_memory_matrix_row_fields_populated() {
    let diagnostics = Arc::new(InMemoryDiagnostics::new());

    diagnostics.record_ingest(
        &SourceId::new("src_full"),
        &project(),
        &KnowledgeDocumentId::new("doc_f1"),
        10,
        1000,
    );
    for _ in 0..3 {
        diagnostics.record_retrieval_feedback(&SourceId::new("src_full"), "c", true, Some(5.0));
    }

    let adapter: Arc<dyn MemoryDiagnosticsSource> = Arc::new(DiagnosticsAdapter(diagnostics));
    let eval_svc = EvalRunService::with_diagnostics(adapter);
    let matrix = eval_svc.build_memory_quality_matrix(&project()).await.unwrap();

    assert_eq!(matrix.rows.len(), 1);
    let row = &matrix.rows[0];
    assert_eq!(row.source_id, "src_full");
    assert_eq!(row.chunk_count, 10);
    assert!(row.retrieval_count > 0, "retrieval_count must reflect feedback calls");
    assert!(row.last_ingested_ms > 0, "last_ingested_ms must be set");
    assert!(
        row.avg_credibility_score >= 0.0 && row.avg_credibility_score <= 1.0,
        "credibility score must be in [0, 1]"
    );
}
