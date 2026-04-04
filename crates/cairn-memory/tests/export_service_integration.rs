use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::bundles::{
    BundleType, DocumentExportFilters, ExportService, KnowledgeDocumentPayload,
};
use cairn_memory::export_service_impl::InMemoryExportService;
use cairn_memory::in_memory::InMemoryDocumentStore;
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_store::InMemoryStore;

#[tokio::test]
async fn export_documents_builds_curated_bundle_with_artifacts() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let prompt_store = Arc::new(InMemoryStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let export = InMemoryExportService::new(store.clone(), prompt_store, "operator");
    let project = ProjectKey::new("acme", "eng", "support");

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_export_1"),
            source_id: SourceId::new("src_export"),
            source_type: SourceType::PlainText,
            project: project.clone(),
            content: "First export document about onboarding.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_export_2"),
            source_id: SourceId::new("src_export"),
            source_type: SourceType::Markdown,
            project: project.clone(),
            content: "# Export Two\n\nSecond export document about recovery.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let bundle = export
        .export_documents(
            "Support Export",
            &project,
            &DocumentExportFilters::default(),
        )
        .await
        .unwrap();

    assert_eq!(bundle.bundle_type, BundleType::CuratedKnowledgePackBundle);
    assert_eq!(bundle.artifact_count, 2);
    assert_eq!(bundle.artifacts.len(), 2);
    assert_eq!(bundle.created_by, "operator");
    assert!(bundle.bundle_id.contains("knowledge_pack"));

    for artifact in &bundle.artifacts {
        assert!(!artifact.payload.is_null());
        let payload: KnowledgeDocumentPayload =
            serde_json::from_value(artifact.payload.clone()).unwrap();
        let text = match payload.content {
            cairn_memory::bundles::DocumentContent::InlineText { text } => text,
            other => panic!("expected inline text export payload, got {other:?}"),
        };
        assert!(!text.trim().is_empty());
        assert_eq!(payload.knowledge_pack_logical_id, bundle.bundle_id);
    }
}
