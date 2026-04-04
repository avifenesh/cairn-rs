//! RFC 003 Gap 3: Unsupported or low-confidence parser outputs must fail explicitly
//! rather than silently entering the canonical corpus.

use cairn_domain::{KnowledgeDocumentId, KnowledgePackId, ProjectKey, SourceId};
use cairn_memory::ingest::{IngestPackRequest, IngestRequest, IngestService, SourceType};
use cairn_memory::in_memory::InMemoryDocumentStore;
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use std::sync::Arc;

/// RFC 003: ingesting a malformed (invalid JSON) KnowledgePack must fail explicitly
/// with IngestError::ParseFailed and must not silently insert chunks.
#[tokio::test]
async fn test_ingest_malformed_source_fails_explicitly() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let chunker = ParagraphChunker::default();
    let pipeline = IngestPipeline::new(store.clone(), chunker);

    let project = ProjectKey::new("t", "w", "p");

    let result = pipeline
        .submit_pack(IngestPackRequest {
            pack_id: KnowledgePackId::new("pack_malformed"),
            project: project.clone(),
            bundle_json: "this is not json at all }{{{".to_owned(),
        })
        .await;

    assert!(
        result.is_err(),
        "ingesting malformed JSON bundle must return an error"
    );

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("parse failed") || err_str.contains("ParseFailed") || err_str.contains("invalid"),
        "error must indicate a parse failure, got: {}",
        err_str
    );

    let chunks = store.all_chunks();
    assert!(
        chunks.is_empty(),
        "malformed ingest must not silently insert chunks; found {} chunk(s)",
        chunks.len()
    );
}

/// RFC 003: ingesting a valid JSON that is the wrong bundle type must also fail explicitly.
#[tokio::test]
async fn test_ingest_wrong_bundle_type_fails_explicitly() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let chunker = ParagraphChunker::default();
    let pipeline = IngestPipeline::new(store.clone(), chunker);

    let project = ProjectKey::new("t", "w", "p");

    let wrong_bundle = serde_json::json!({
        "bundle_schema_version": "1",
        "bundle_type": "unknown_bundle_type_xyz",
        "bundle_id": "bundle_wrong",
        "bundle_name": "Wrong",
        "created_at": 1000,
        "created_by": "operator",
        "source_deployment_id": null,
        "source_scope": { "tenant_id": "t", "workspace_id": "w", "project_id": "p" },
        "artifact_count": 0,
        "artifacts": [],
        "provenance": { "description": null, "source_system": null, "export_reason": null }
    });

    let result = pipeline
        .submit_pack(IngestPackRequest {
            pack_id: KnowledgePackId::new("pack_wrong_type"),
            project: project.clone(),
            bundle_json: wrong_bundle.to_string(),
        })
        .await;

    assert!(
        result.is_err(),
        "ingesting a bundle with wrong type must return an error"
    );

    let err = result.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("parse failed") || err_str.contains("ParseFailed"),
        "error must be a ParseFailed variant, got: {}",
        err_str
    );

    assert!(store.all_chunks().is_empty());
}

/// RFC 003: ingesting a KnowledgePack source type with empty content produces no chunks.
#[tokio::test]
async fn test_ingest_empty_knowledge_pack_content_produces_no_chunks() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let chunker = ParagraphChunker::default();
    let pipeline = IngestPipeline::new(store.clone(), chunker);

    let project = ProjectKey::new("t", "w", "p");

    let result = pipeline
        .submit(IngestRequest {
            document_id: KnowledgeDocumentId::new("doc_empty_pack"),
            source_id: SourceId::new("src_empty"),
            source_type: SourceType::KnowledgePack,
            project: project.clone(),
            content: "".to_owned(),
        })
        .await;

    assert!(result.is_ok(), "empty content submission should not panic, got: {:?}", result.err());
    let chunks = store.all_chunks();
    assert!(
        chunks.is_empty(),
        "empty KnowledgePack content must not produce chunks; found {}",
        chunks.len()
    );
}
