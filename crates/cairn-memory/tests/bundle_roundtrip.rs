//! Bundle round-trip: proves knowledge pack bundle can be serialized,
//! deserialized, and its documents fed into the ingest pipeline
//! without adding new retrieval scope.

use cairn_domain::{KnowledgeDocumentId, ProjectKey, SourceId};
use cairn_memory::bundles::*;
use cairn_memory::in_memory::InMemoryDocumentStore;
use cairn_memory::ingest::{IngestRequest, IngestService, SourceType};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use std::collections::HashMap;
use std::sync::Arc;

#[tokio::test]
async fn bundle_documents_ingest_through_existing_pipeline() {
    // Build a knowledge pack bundle.
    let bundle = BundleEnvelope {
        bundle_schema_version: "1".to_owned(),
        bundle_type: BundleType::CuratedKnowledgePackBundle,
        bundle_id: "pack_onboard".to_owned(),
        bundle_name: "Onboarding Pack".to_owned(),
        created_at: 1000,
        created_by: Some("admin".to_owned()),
        source_deployment_id: None,
        source_scope: SourceScope {
            tenant_id: Some("acme".to_owned()),
            workspace_id: Some("eng".to_owned()),
            project_id: None,
        },
        artifact_count: 2,
        artifacts: vec![
            ArtifactEntry {
                artifact_kind: ArtifactKind::KnowledgeDocument,
                artifact_logical_id: "doc_setup".to_owned(),
                artifact_display_name: "Setup Guide".to_owned(),
                origin_scope: SourceScope {
                    tenant_id: Some("acme".to_owned()),
                    workspace_id: Some("eng".to_owned()),
                    project_id: Some("support".to_owned()),
                },
                origin_artifact_id: None,
                content_hash: "hash_setup".to_owned(),
                source_bundle_id: "pack_onboard".to_owned(),
                origin_timestamp: 900,
                metadata: HashMap::new(),
                payload: cairn_memory::bundles::ArtifactPayload::InlineJson(serde_json::json!({
                    "knowledge_pack_logical_id": "pack_onboard",
                    "document_name": "Setup Guide",
                    "source_type": "text_plain",
                    "content": {"kind": "inline_text", "text": "Install the CLI with cargo install cairn-cli."}
                })),
                provenance: cairn_memory::bundles::ArtifactProvenance::default(),
                lineage: None,
                tags: vec!["onboarding".to_owned()],
            },
            ArtifactEntry {
                artifact_kind: ArtifactKind::KnowledgeDocument,
                artifact_logical_id: "doc_faq".to_owned(),
                artifact_display_name: "FAQ".to_owned(),
                origin_scope: SourceScope {
                    tenant_id: Some("acme".to_owned()),
                    workspace_id: Some("eng".to_owned()),
                    project_id: Some("support".to_owned()),
                },
                origin_artifact_id: None,
                content_hash: "hash_faq".to_owned(),
                source_bundle_id: "pack_onboard".to_owned(),
                origin_timestamp: 950,
                metadata: HashMap::new(),
                payload: cairn_memory::bundles::ArtifactPayload::InlineJson(serde_json::json!({
                    "knowledge_pack_logical_id": "pack_onboard",
                    "document_name": "FAQ",
                    "source_type": "text_markdown",
                    "content": {"kind": "inline_text", "text": "# FAQ\n\nQ: How do I reset my password?\nA: Use the forgot password link."}
                })),
                provenance: cairn_memory::bundles::ArtifactProvenance::default(),
                lineage: None,
                tags: vec!["onboarding".to_owned()],
            },
        ],
        provenance: BundleProvenance {
            description: Some("Onboarding knowledge pack".to_owned()),
            source_system: None,
            export_reason: None,
            origin: None,
            production_method: None,
            source_version: None,
        },
    };

    // Round-trip through JSON.
    let json = serde_json::to_string(&bundle).unwrap();
    let parsed: BundleEnvelope = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.artifacts.len(), 2);
    assert_eq!(parsed.bundle_type, BundleType::CuratedKnowledgePackBundle);

    // Extract document content and ingest through existing pipeline.
    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let project = ProjectKey::new("acme", "eng", "support");

    for artifact in &parsed.artifacts {
        let payload = artifact.payload.as_value();
        let content = payload["content"]["text"].as_str().unwrap_or("").to_owned();
        let content = content.as_str();
        let source_type = match payload["source_type"].as_str().unwrap_or("") {
            "text_plain" => SourceType::PlainText,
            "text_markdown" => SourceType::Markdown,
            _ => SourceType::PlainText,
        };

        pipeline
            .submit(IngestRequest {
                document_id: KnowledgeDocumentId::new(&artifact.artifact_logical_id),
                source_id: SourceId::new(&parsed.bundle_id),
                source_type,
                project: project.clone(),
                content: content.to_owned(),
                import_id: None,
                corpus_id: None,
                bundle_source_id: None,
                tags: vec![],
            })
            .await
            .unwrap();
    }

    // Verify both documents ingested.
    let chunks = store.all_chunks();
    assert!(chunks.len() >= 2, "bundle documents should produce chunks");
    assert!(
        chunks.iter().any(|c| c.text.contains("cargo install")),
        "setup guide content should be ingested"
    );
    assert!(
        chunks.iter().any(|c| c.text.contains("password")),
        "FAQ content should be ingested"
    );
}

/// Tests submit_pack() — the pipeline's native bundle ingest path.
#[tokio::test]
async fn submit_pack_ingests_knowledge_documents() {
    use cairn_memory::ingest::IngestPackRequest;

    let store = Arc::new(InMemoryDocumentStore::new());
    let pipeline = IngestPipeline::new(store.clone(), ParagraphChunker::default());
    let project = ProjectKey::new("acme", "eng", "support");

    let bundle_json = serde_json::json!({
        "bundle_schema_version": "1",
        "bundle_type": "curated_knowledge_pack_bundle",
        "bundle_id": "pack_test",
        "bundle_name": "Test Pack",
        "created_at": 1000,
        "source_scope": {"tenant_id": "acme", "workspace_id": "eng"},
        "artifact_count": 2,
        "artifacts": [
            {
                "artifact_kind": "knowledge_document",
                "artifact_logical_id": "doc_alpha",
                "artifact_display_name": "Alpha Doc",
                "origin_scope": {"tenant_id": "acme"},
                "content_hash": "h1",
                "source_bundle_id": "pack_test",
                "origin_timestamp": 900,
                "metadata": {},
                "tags": [],
                "payload": {
                    "source_type": "text_plain",
                    "content": {"kind": "inline_text", "text": "Alpha document about Rust concurrency."}
                }
            },
            {
                "artifact_kind": "knowledge_document",
                "artifact_logical_id": "doc_beta",
                "artifact_display_name": "Beta Doc",
                "origin_scope": {"tenant_id": "acme"},
                "content_hash": "h2",
                "source_bundle_id": "pack_test",
                "origin_timestamp": 950,
                "metadata": {},
                "tags": [],
                "payload": {
                    "source_type": "text_markdown",
                    "content": {"kind": "inline_text", "text": "# Beta\n\nBeta document about async await patterns."}
                }
            }
        ],
        "provenance": {"description": "test pack"}
    })
    .to_string();

    pipeline
        .submit_pack(IngestPackRequest {
            pack_id: cairn_domain::KnowledgePackId::new("pack_test"),
            project: project.clone(),
            bundle_json,
        })
        .await
        .unwrap();

    let chunks = store.all_chunks();
    assert!(chunks.len() >= 2, "pack should produce chunks");
    assert!(chunks.iter().any(|c| c.text.contains("concurrency")));
    assert!(chunks.iter().any(|c| c.text.contains("async await")));

    // Both docs should show as completed.
    let s1 = pipeline
        .status(&KnowledgeDocumentId::new("doc_alpha"))
        .await
        .unwrap();
    assert_eq!(s1, Some(cairn_memory::ingest::IngestStatus::Completed));

    let s2 = pipeline
        .status(&KnowledgeDocumentId::new("doc_beta"))
        .await
        .unwrap();
    assert_eq!(s2, Some(cairn_memory::ingest::IngestStatus::Completed));
}
