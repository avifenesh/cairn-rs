//! RFC 003 document versioning integration tests.
//!
//! Verifies: multi-version ingest, superseded chunk filtering,
//! same-hash deduplication, and API route boundary.

use std::sync::Arc;

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::api_impl::{DocumentVersionApiImpl, MemoryApiImpl};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::{DocumentVersionReadModel, IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_memory::retrieval::{RerankerStrategy, RetrievalMode, RetrievalQuery, RetrievalService};
use cairn_api::memory_api::DocumentVersionEndpoints;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

/// Ingest the same document ID twice with different content.
/// Asserts exactly 2 versions are registered and hashes differ.
#[tokio::test]
async fn versioning_two_different_ingests_creates_two_versions() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    let doc_id = KnowledgeDocumentId::new("doc_versioned");
    let src = SourceId::new("src");

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: src.clone(),
            source_type: SourceType::PlainText,
            project: project(),
            content: "First version: alpha content about widgets and sprockets.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: src.clone(),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Second version: beta content about gadgets and doodads.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let versions = DocumentVersionReadModel::list_versions(store.as_ref(), &doc_id)
        .await
        .unwrap();

    assert_eq!(versions.len(), 2, "expected 2 versions, got {}", versions.len());
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[1].version, 2);
    assert_ne!(
        versions[0].content_hash, versions[1].content_hash,
        "v1 and v2 must have different content hashes"
    );
    assert_eq!(
        versions[1].changed_fields,
        vec!["content".to_owned()],
        "v2 changed_fields should record content change"
    );
}

/// After re-ingesting a document, retrieval must return only current (v2) chunks.
/// Superseded v1 chunks must not appear in search results.
#[tokio::test]
async fn versioning_search_returns_only_current_chunks() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let retrieval = InMemoryRetrieval::new(store.clone());

    let doc_id = KnowledgeDocumentId::new("doc_search_versioned");

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Obsolete first edition content: legacy widgets.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Updated second edition content: modern gadgets.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let response = retrieval
        .query(RetrievalQuery {
            project: project(),
            query_text: "content edition".to_owned(),
            query_embedding: None,
            mode: RetrievalMode::LexicalOnly,
            reranker: RerankerStrategy::None,
            limit: 20,
            metadata_filters: vec![],
            scoring_policy: None,
        })
        .await
        .unwrap();

    assert!(!response.results.is_empty(), "should return current chunks");

    for result in &response.results {
        assert!(
            !result.chunk.superseded,
            "superseded chunks must not appear in search results (chunk_id={})",
            result.chunk.chunk_id
        );
    }

    let texts: Vec<&str> = response
        .results
        .iter()
        .map(|r| r.chunk.text.as_str())
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("gadgets")),
        "current v2 content (gadgets) must be findable"
    );
    assert!(
        !texts.iter().any(|t| t.contains("widgets")),
        "superseded v1 content (widgets) must not appear in results"
    );
}

/// Re-ingesting the same content (identical hash) must skip version creation.
#[tokio::test]
async fn versioning_same_hash_skips_new_version() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());

    let doc_id = KnowledgeDocumentId::new("doc_same_hash");
    let content = "Identical content — hash will match on second ingest.".to_owned();

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: content.clone(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: content.clone(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let versions = DocumentVersionReadModel::list_versions(store.as_ref(), &doc_id)
        .await
        .unwrap();

    assert_eq!(versions.len(), 1, "same content must not create a second version");
}

/// API route: GET /v1/memory/documents/:id returns document info with current version.
#[tokio::test]
async fn versioning_api_get_document_returns_current_version() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let api = DocumentVersionApiImpl::new(store.clone());

    let doc_id = KnowledgeDocumentId::new("doc_api_get");

    // Before ingest — document does not exist.
    let result = api.get_document("doc_api_get").await.unwrap();
    assert!(result.is_none(), "unknown document should return None");

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "API test content about retrieval pipelines.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let info = api
        .get_document("doc_api_get")
        .await
        .unwrap()
        .expect("document should exist after ingest");

    assert_eq!(info.document_id, "doc_api_get");
    let cv = info.current_version.expect("current_version should be populated");
    assert_eq!(cv.version, 1);
    assert!(!cv.content_hash.is_empty());
}

/// API route: GET /v1/memory/documents/:id/versions lists all versions.
#[tokio::test]
async fn versioning_api_list_versions_returns_all_versions() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let api = DocumentVersionApiImpl::new(store.clone());

    let doc_id = KnowledgeDocumentId::new("doc_api_versions");

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Version one: original text about pipelines.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    pipeline
        .submit(IngestRequest {
            document_id: doc_id.clone(),
            source_id: SourceId::new("src"),
            source_type: SourceType::PlainText,
            project: project(),
            content: "Version two: revised text about orchestration.".to_owned(),
            tags: vec![],
            corpus_id: None,
            bundle_source_id: None,
            import_id: None,
        })
        .await
        .unwrap();

    let versions = api.list_document_versions("doc_api_versions").await.unwrap();

    assert_eq!(versions.len(), 2, "expected 2 versions from API route");
    assert_eq!(versions[0].version, 1);
    assert_eq!(versions[1].version, 2);
    assert_ne!(versions[0].content_hash, versions[1].content_hash);
}
