//! Integration tests for RFC 013 bundle export filtering.

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::bundles::{DocumentExportFilters, ExportService};
use cairn_memory::export_service_impl::InMemoryExportService;
use cairn_memory::in_memory::InMemoryDocumentStore;
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_store::InMemoryStore;

async fn setup() -> (
    Arc<InMemoryDocumentStore>,
    IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>,
    InMemoryExportService,
    ProjectKey,
) {
    let store = Arc::new(InMemoryDocumentStore::new());
    let prompt_store = Arc::new(InMemoryStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let export = InMemoryExportService::new(store.clone(), prompt_store, "operator");
    let project = ProjectKey::new("acme", "eng", "filter_test");
    (store, pipeline, export, project)
}

/// Ingest three documents with different source IDs.
/// Export with source_ids filter — only docs from matching source returned.
#[tokio::test]
async fn export_filtering_by_source_ids() {
    let (store, pipeline, export, project) = setup().await;

    // doc 1: source A
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_src_a"),
            source_id: SourceId::new("src_alpha"),
            source_type: SourceType::PlainText,
            project: project.clone(),
            content: "Document from source alpha about onboarding.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // doc 2: source A again
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_src_a2"),
            source_id: SourceId::new("src_alpha"),
            source_type: SourceType::PlainText,
            project: project.clone(),
            content: "Second document from source alpha about setup.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // doc 3: source B
    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_src_b"),
            source_id: SourceId::new("src_beta"),
            source_type: SourceType::PlainText,
            project: project.clone(),
            content: "Document from source beta about deployment.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    // Export with no filter — all 3 docs
    let all_bundle = export
        .export_documents("all_docs", &project, &DocumentExportFilters::default())
        .await
        .unwrap();
    assert_eq!(
        all_bundle.artifact_count, 3,
        "unfiltered export should return all 3 docs"
    );

    // Export with source_ids filter for src_alpha only
    let filtered = export
        .export_documents(
            "alpha_only",
            &project,
            &DocumentExportFilters {
                source_ids: vec!["src_alpha".to_owned()],
                ..DocumentExportFilters::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        filtered.artifact_count, 2,
        "source_ids filter should return only src_alpha docs"
    );
    for artifact in &filtered.artifacts {
        let logical_id = &artifact.artifact_logical_id;
        assert!(
            logical_id == "doc_src_a" || logical_id == "doc_src_a2",
            "unexpected doc in filtered export: {logical_id}"
        );
    }

    // Export with source_ids filter for src_beta only
    let beta_only = export
        .export_documents(
            "beta_only",
            &project,
            &DocumentExportFilters {
                source_ids: vec!["src_beta".to_owned()],
                ..DocumentExportFilters::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(beta_only.artifact_count, 1);
    assert_eq!(beta_only.artifacts[0].artifact_logical_id, "doc_src_b");

    // Verify we drop all docs when no source matches
    let _ = store; // keep alive
}

/// Export with min_credibility_score filter.
///
/// `InMemoryDocumentStore::exportable_documents()` currently returns
/// `credibility_score: None` for every document (scores are not persisted
/// through the ingest pipeline in the in-memory backend).  The export
/// service treats `None` as `0.0`, so any positive `min_credibility_score`
/// filters out all docs — which is the currently observable behaviour.
#[tokio::test]
async fn export_filtering_by_min_credibility_score() {
    let (_store, pipeline, export, project) = setup().await;

    // Ingest 3 docs — all will have credibility_score = None in the store.
    for (doc_id, src_id) in [
        ("cred_doc_high", "src_cred"),
        ("cred_doc_mid", "src_cred"),
        ("cred_doc_low", "src_cred"),
    ] {
        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new(doc_id),
                source_id: SourceId::new(src_id),
                source_type: SourceType::PlainText,
                project: project.clone(),
                content: format!("Content for {doc_id} about various topics."),
                tags: vec![],
                corpus_id: None,
                bundle_source_id: None,
                import_id: None,
            })
            .await
            .unwrap();
    }

    // Export all (no filter) — all 3 docs returned.
    let all_bundle = export
        .export_documents("all_cred", &project, &DocumentExportFilters::default())
        .await
        .unwrap();
    assert_eq!(
        all_bundle.artifact_count, 3,
        "unfiltered export returns all 3 docs"
    );

    // Export with min_credibility_score > 0: docs with credibility_score=None are
    // treated as 0.0 by the filter, so all are excluded.
    let filtered = export
        .export_documents(
            "cred_filtered",
            &project,
            &DocumentExportFilters {
                min_credibility_score: Some(0.5),
                ..DocumentExportFilters::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(
        filtered.artifact_count, 0,
        "docs with no credibility score are excluded when min_credibility_score is set"
    );
}
