use std::collections::HashMap;
use std::sync::Arc;

use cairn_memory::bundles::{
    ArtifactEntry, ArtifactKind, BundleEnvelope, BundleProvenance, BundleSourceType, BundleType,
    ChunkHint, ConflictResolutionStrategy, DocumentContent, ImportOutcome, ImportService,
    KnowledgeDocumentPayload, SourceScope,
};
use cairn_memory::import_service_impl::InMemoryImportService;
use cairn_memory::in_memory::InMemoryDocumentStore;

fn knowledge_doc_artifact(logical_id: &str, display_name: &str, text: &str) -> ArtifactEntry {
    ArtifactEntry {
        artifact_kind: ArtifactKind::KnowledgeDocument,
        artifact_logical_id: logical_id.to_owned(),
        artifact_display_name: display_name.to_owned(),
        origin_scope: SourceScope {
            tenant_id: Some("acme".to_owned()),
            workspace_id: Some("eng".to_owned()),
            project_id: Some("support".to_owned()),
        },
        origin_artifact_id: None,
        content_hash: logical_id.to_owned(),
        source_bundle_id: "bundle_curated".to_owned(),
        origin_timestamp: 1_710_000_000,
        metadata: HashMap::new(),
        payload: serde_json::to_value(KnowledgeDocumentPayload {
            knowledge_pack_logical_id: "bundle_curated".to_owned(),
            document_name: display_name.to_owned(),
            source_type: BundleSourceType::TextPlain,
            content: DocumentContent::InlineText {
                text: text.to_owned(),
            },
            metadata: HashMap::new(),
            chunk_hints: vec![ChunkHint {
                start_offset: 0,
                end_offset: text.len(),
                hint_text: Some("whole document".to_owned()),
            }],
            retrieval_hints: vec!["support".to_owned()],
        })
        .unwrap(),
        lineage: None,
        tags: vec!["curated".to_owned()],
    }
}

fn curated_bundle() -> BundleEnvelope {
    BundleEnvelope {
        bundle_schema_version: "1".to_owned(),
        bundle_type: BundleType::CuratedKnowledgePackBundle,
        bundle_id: "bundle_curated".to_owned(),
        bundle_name: "Curated Support Pack".to_owned(),
        created_at: 1_710_000_000,
        created_by: "operator".to_owned(),
        source_deployment_id: None,
        source_scope: SourceScope {
            tenant_id: Some("acme".to_owned()),
            workspace_id: Some("eng".to_owned()),
            project_id: Some("support".to_owned()),
        },
        artifact_count: 2,
        artifacts: vec![
            knowledge_doc_artifact(
                "doc_install",
                "Install Guide",
                "Install cairn with cargo install cairn-cli and verify the binary.",
            ),
            knowledge_doc_artifact(
                "doc_reset",
                "Password Reset",
                "Reset the password from the account portal and confirm the email challenge.",
            ),
        ],
        provenance: BundleProvenance {
            description: Some("Support knowledge pack".to_owned()),
            source_system: Some("curation".to_owned()),
            export_reason: Some("seed".to_owned()),
        },
    }
}

#[tokio::test]
async fn import_service_validate_plan_apply_and_skip_duplicates() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let import_service = InMemoryImportService::new(store.clone());
    let bundle = curated_bundle();
    let target_scope = SourceScope {
        tenant_id: Some("acme".to_owned()),
        workspace_id: Some("eng".to_owned()),
        project_id: Some("support".to_owned()),
    };

    let validation = import_service.validate(&bundle).await.unwrap();
    assert!(
        validation.errors.is_empty(),
        "validation errors: {:?}",
        validation.errors
    );
    assert!(validation.is_valid());

    let first_plan = import_service.plan(&bundle, &target_scope).await.unwrap();
    assert_eq!(first_plan.create_count, 2);
    assert_eq!(first_plan.skip_count, 0);
    assert!(first_plan
        .entries
        .iter()
        .all(|entry| entry.outcome == ImportOutcome::Create));

    let report = import_service.apply(&first_plan, &bundle).await.unwrap();
    assert_eq!(report.create_count, 2);
    assert_eq!(report.conflict_count, 0);
    assert!(report
        .entries
        .iter()
        .all(|entry| entry.outcome == ImportOutcome::Create));

    let second_plan = import_service.plan(&bundle, &target_scope).await.unwrap();
    assert_eq!(second_plan.create_count, 0);
    assert_eq!(second_plan.skip_count, 2);
    assert!(second_plan
        .entries
        .iter()
        .all(|entry| entry.outcome == ImportOutcome::Skip));
}

#[tokio::test]
async fn import_service_conflict_resolution_strategies_apply_as_requested() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let import_service = InMemoryImportService::new(store.clone());
    let bundle = curated_bundle();
    let target_scope = SourceScope {
        tenant_id: Some("acme".to_owned()),
        workspace_id: Some("eng".to_owned()),
        project_id: Some("support".to_owned()),
    };

    let first_plan = import_service.plan(&bundle, &target_scope).await.unwrap();
    import_service.apply(&first_plan, &bundle).await.unwrap();

    let mut overwrite_plan = import_service.plan(&bundle, &target_scope).await.unwrap();
    overwrite_plan.conflict_resolution = ConflictResolutionStrategy::Overwrite;
    let overwrite_report = import_service
        .apply(&overwrite_plan, &bundle)
        .await
        .unwrap();
    assert_eq!(overwrite_report.overwritten, 2);
    assert_eq!(overwrite_report.created, 0);

    let mut version_plan = import_service.plan(&bundle, &target_scope).await.unwrap();
    version_plan.conflict_resolution = ConflictResolutionStrategy::CreateVersion;
    let version_report = import_service.apply(&version_plan, &bundle).await.unwrap();
    assert_eq!(version_report.versioned, 2);
    assert_eq!(version_report.created, 0);
    assert!(version_report
        .entries
        .iter()
        .all(|entry| entry.created_object_id.as_deref().is_some()));

    let mut review_plan = import_service.plan(&bundle, &target_scope).await.unwrap();
    review_plan.conflict_resolution = ConflictResolutionStrategy::AskOperator;
    let review_report = import_service.apply(&review_plan, &bundle).await.unwrap();
    assert_eq!(review_report.pending_operator_review.len(), 2);
    assert_eq!(review_report.conflict_count, 2);
}

/// RFC 013 §5.1: "Every structured bundle must have one canonical envelope."
/// The bundle_schema_version MUST be present and MUST match a supported version.
#[tokio::test]
async fn import_validate_rejects_unsupported_schema_version() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let import_service = InMemoryImportService::new(store);

    let mut bad_version = curated_bundle();
    bad_version.bundle_schema_version = "99".to_owned();

    let report = import_service.validate(&bad_version).await.unwrap();
    assert!(
        !report.errors.is_empty(),
        "bundle with unsupported schema_version must produce validation errors"
    );
    assert!(
        report.errors.iter().any(|e| e.contains("bundle_schema_version") || e.contains("unsupported")),
        "error must mention bundle_schema_version, got: {:?}",
        report.errors
    );
}

#[tokio::test]
async fn import_validate_rejects_empty_schema_version() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let import_service = InMemoryImportService::new(store);

    let mut empty_version = curated_bundle();
    empty_version.bundle_schema_version = String::new();

    let report = import_service.validate(&empty_version).await.unwrap();
    assert!(
        !report.errors.is_empty(),
        "bundle with empty bundle_schema_version must produce validation errors"
    );
}

#[tokio::test]
async fn import_validate_accepts_version_1_bundle() {
    let store = Arc::new(InMemoryDocumentStore::new());
    let import_service = InMemoryImportService::new(store);

    let bundle = curated_bundle(); // bundle_schema_version = "1"
    let report = import_service.validate(&bundle).await.unwrap();
    assert!(
        report.errors.is_empty(),
        "valid version-1 bundle must pass validation, errors: {:?}",
        report.errors
    );
}
