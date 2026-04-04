//! RFC 013 artifact provenance tracking integration tests.

use std::sync::Arc;

use cairn_api::memory_api::{DocumentProvenanceEndpoints};
use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::api_impl::DocumentProvenanceApiImpl;
use cairn_memory::bundles::ExportService;
use cairn_memory::export_service_impl::InMemoryExportService;
use cairn_memory::in_memory::InMemoryDocumentStore;
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Ingest a doc with bundle_source_id. GET provenance. Assert source_bundle_id is set.
#[tokio::test]
async fn provenance_tracking_bundle_source_id_is_stored() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_prov"),
            source_id: SourceId::new("src_prov"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Provenance tracking test content for this document.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: Some("bundle_abc".to_owned()),
            import_id: None,
        })
        .await
        .unwrap();

    // Retrieve via DocumentProvenanceApiImpl (equivalent to GET /v1/memory/documents/:id/provenance)
    let api = DocumentProvenanceApiImpl::new(store.clone());
    let prov = api
        .get_document_provenance("doc_prov")
        .await
        .unwrap()
        .expect("provenance must be set for a bundle-sourced document");

    assert_eq!(
        prov.source_bundle_id.as_deref(),
        Some("bundle_abc"),
        "source_bundle_id must be 'bundle_abc', got {:?}",
        prov.source_bundle_id
    );
}

/// Ingest doc with import_id. Assert import_id propagates.
#[tokio::test]
async fn provenance_tracking_import_id_is_stored() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_import"),
            source_id: SourceId::new("src_import"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Import tracking test document with unique content here.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: Some("import_job_42".to_owned()),
        })
        .await
        .unwrap();

    let api = DocumentProvenanceApiImpl::new(store.clone());
    let prov = api
        .get_document_provenance("doc_import")
        .await
        .unwrap()
        .expect("provenance must be set");

    assert_eq!(
        prov.import_id.as_deref(),
        Some("import_job_42"),
        "import_id must be 'import_job_42'"
    );
    assert!(
        prov.imported_at_ms > 0,
        "imported_at_ms must be set"
    );
}

/// Document without provenance returns None.
#[tokio::test]
async fn provenance_tracking_no_provenance_returns_none() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_no_prov"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Document without any provenance metadata set during ingest.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let api = DocumentProvenanceApiImpl::new(store.clone());
    let prov = api.get_document_provenance("doc_no_prov").await.unwrap();
    assert!(
        prov.is_none(),
        "document without bundle_source_id or import_id must have no provenance"
    );
}

/// Export bundle includes provenance metadata in artifact.
#[tokio::test]
async fn provenance_tracking_export_includes_provenance_metadata() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let prompt_store = Arc::new(cairn_store::InMemoryStore::new());
    let export_svc = InMemoryExportService::new(store.clone(), prompt_store, "operator");

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_export_prov"),
            source_id: SourceId::new("src_export"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Export provenance test document unique content alpha beta.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: Some("bundle_xyz".to_owned()),
            import_id: Some("import_99".to_owned()),
        })
        .await
        .unwrap();

    let bundle = export_svc
        .export_documents(
            "test_bundle",
            &project(),
            &cairn_memory::bundles::DocumentExportFilters::default(),
        )
        .await
        .unwrap();

    assert!(!bundle.artifacts.is_empty(), "bundle must contain artifacts");

    let artifact = bundle
        .artifacts
        .iter()
        .find(|a| a.artifact_logical_id == "doc_export_prov")
        .expect("doc_export_prov must appear in export");

    // The source_bundle_id on the artifact should come from provenance.
    assert_eq!(
        artifact.source_bundle_id, "bundle_xyz",
        "artifact source_bundle_id must be 'bundle_xyz' from provenance"
    );

    // The metadata must contain the import_provenance key.
    assert!(
        artifact.metadata.contains_key("import_provenance"),
        "artifact metadata must contain 'import_provenance' key"
    );
    let prov_val = &artifact.metadata["import_provenance"];
    assert_eq!(
        prov_val["source_bundle_id"].as_str(),
        Some("bundle_xyz"),
        "import_provenance.source_bundle_id must be 'bundle_xyz'"
    );
    assert_eq!(
        prov_val["import_id"].as_str(),
        Some("import_99"),
        "import_provenance.import_id must be 'import_99'"
    );
}

/// Both bundle_source_id and import_id in same request.
#[tokio::test]
async fn provenance_tracking_both_fields_stored() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_both"),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Document with both bundle_source_id and import_id set uniquely.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: Some("bundle_full".to_owned()),
            import_id: Some("import_full".to_owned()),
        })
        .await
        .unwrap();

    let api = DocumentProvenanceApiImpl::new(store.clone());
    let prov = api.get_document_provenance("doc_both").await.unwrap().unwrap();

    assert_eq!(prov.source_bundle_id.as_deref(), Some("bundle_full"));
    assert_eq!(prov.import_id.as_deref(), Some("import_full"));
}
