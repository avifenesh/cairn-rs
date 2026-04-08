//! RFC 013 bundle import/export round-trip integration tests.
//!
//! Validates the bundle pipeline:
//! - BundleEnvelope construction with PromptAsset artifacts.
//! - validate_bundle_schema_version accepts v1 and rejects unknown versions.
//! - ImportPlan construction and count verification.
//! - BundleType discriminator for both PromptLibraryBundle and CuratedKnowledgePackBundle.
//! - ArtifactEntry carries content_hash and artifact_logical_id correctly.
//! - Bundle serializes to/from JSON without data loss.

use std::collections::HashMap;

use cairn_memory::bundles::{
    validate_bundle_schema_version, ArtifactEntry, ArtifactKind, ArtifactPayload,
    ArtifactProvenance, BundleEnvelope, BundleProvenance, BundleType, ConflictResolutionStrategy,
    ImportOutcome, ImportPlan, ImportPlanEntry, SourceScope,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn source_scope() -> SourceScope {
    SourceScope {
        tenant_id: Some("tenant_bundle".to_owned()),
        workspace_id: Some("ws_bundle".to_owned()),
        project_id: Some("proj_bundle".to_owned()),
    }
}

fn provenance() -> BundleProvenance {
    BundleProvenance {
        description: Some("test bundle".to_owned()),
        source_system: Some("test-harness".to_owned()),
        export_reason: Some("integration-test".to_owned()),
        origin: None,
        production_method: None,
        source_version: None,
    }
}

fn prompt_asset_artifact(logical_id: &str, name: &str, content_hash: &str) -> ArtifactEntry {
    ArtifactEntry {
        artifact_kind: ArtifactKind::PromptAsset,
        artifact_logical_id: logical_id.to_owned(),
        artifact_display_name: name.to_owned(),
        origin_scope: source_scope(),
        origin_artifact_id: Some(format!("pa_{logical_id}")),
        content_hash: content_hash.to_owned(),
        source_bundle_id: "bundle_prompt_lib_001".to_owned(),
        origin_timestamp: 1_700_000_000,
        metadata: HashMap::new(),
        payload: ArtifactPayload::InlineJson(serde_json::json!({
            "name": name,
            "kind": "assistant",
            "status": "published",
            "library_scope_hint": "workspace",
            "metadata": {}
        })),
        provenance: ArtifactProvenance {
            origin: None,
            production_method: None,
            created_at: None,
        },
        lineage: Some(format!("origin::{logical_id}")),
        tags: vec!["prompt".to_owned(), "assistant".to_owned()],
    }
}

fn two_artifact_prompt_bundle(bundle_type: BundleType) -> BundleEnvelope {
    let artifacts = vec![
        prompt_asset_artifact("prompt_asset_system", "System Prompt", "hash_abc123"),
        prompt_asset_artifact("prompt_asset_retrieval", "Retrieval Prompt", "hash_def456"),
    ];
    BundleEnvelope {
        bundle_schema_version: "1".to_owned(),
        bundle_type,
        bundle_id: "bundle_prompt_lib_001".to_owned(),
        bundle_name: "Prompt Library Bundle".to_owned(),
        created_at: 1_700_000_000,
        created_by: Some("operator_test".to_owned()),
        source_deployment_id: Some("deploy_001".to_owned()),
        source_scope: source_scope(),
        artifact_count: artifacts.len(),
        artifacts,
        provenance: provenance(),
    }
}

fn import_plan_from_bundle(bundle: &BundleEnvelope) -> ImportPlan {
    let entries: Vec<ImportPlanEntry> = bundle
        .artifacts
        .iter()
        .map(|a| ImportPlanEntry {
            artifact_logical_id: a.artifact_logical_id.clone(),
            artifact_kind: a.artifact_kind,
            outcome: ImportOutcome::Create,
            reason: "new artifact — not present in target deployment".to_owned(),
            existing_id: None,
        })
        .collect();

    let create_count = entries.len();
    ImportPlan {
        bundle_id: bundle.bundle_id.clone(),
        target_scope: source_scope(),
        create_count,
        reuse_count: 0,
        update_count: 0,
        skip_count: 0,
        conflict_count: 0,
        entries,
        conflict_resolution: ConflictResolutionStrategy::Skip,
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Create BundleEnvelope with 2 PromptAsset artifacts;
/// (2) validate_bundle_schema_version accepts the v1 envelope.
#[test]
fn bundle_with_two_prompt_assets_is_valid() {
    let bundle = two_artifact_prompt_bundle(BundleType::PromptLibraryBundle);

    assert_eq!(
        bundle.artifacts.len(),
        2,
        "bundle must carry exactly 2 artifacts"
    );
    assert_eq!(
        bundle.artifact_count, 2,
        "artifact_count must match actual artifact count"
    );

    // Step 2: validate schema version.
    validate_bundle_schema_version(&bundle)
        .expect("bundle with schema_version='1' must pass validation");
}

/// validate_bundle_schema_version rejects empty and unsupported versions.
#[test]
fn validate_bundle_schema_version_rejects_invalid() {
    let mut bundle = two_artifact_prompt_bundle(BundleType::PromptLibraryBundle);

    // Empty version.
    bundle.bundle_schema_version = String::new();
    assert!(
        validate_bundle_schema_version(&bundle).is_err(),
        "empty bundle_schema_version must fail validation"
    );

    // Unknown version.
    bundle.bundle_schema_version = "99".to_owned();
    let err = validate_bundle_schema_version(&bundle).unwrap_err();
    assert!(
        err.contains("unsupported"),
        "error message must say 'unsupported', got: {err}"
    );

    // Whitespace-only version.
    bundle.bundle_schema_version = "   ".to_owned();
    assert!(
        validate_bundle_schema_version(&bundle).is_err(),
        "whitespace-only bundle_schema_version must fail validation"
    );
}

/// (3) Create ImportPlan from the bundle; (4) verify plan has 2 'create' outcomes.
#[test]
fn import_plan_has_two_create_outcomes() {
    let bundle = two_artifact_prompt_bundle(BundleType::PromptLibraryBundle);
    let plan = import_plan_from_bundle(&bundle);

    assert_eq!(
        plan.bundle_id, bundle.bundle_id,
        "plan must reference the bundle ID"
    );
    assert_eq!(
        plan.entries.len(),
        2,
        "plan must have one entry per artifact"
    );
    assert_eq!(
        plan.create_count, 2,
        "both artifacts must be Create outcomes"
    );
    assert_eq!(plan.reuse_count, 0);
    assert_eq!(plan.conflict_count, 0);
    assert!(
        !plan.has_conflicts(),
        "plan with no conflicts must report has_conflicts=false"
    );

    // All entries must be Create.
    for entry in &plan.entries {
        assert_eq!(
            entry.outcome,
            ImportOutcome::Create,
            "entry for '{}' must have Create outcome",
            entry.artifact_logical_id
        );
    }

    // summarize_counts must agree with individual fields.
    let (create, reuse, update, skip, conflict) = ImportPlan::summarize_counts(&plan.entries);
    assert_eq!(create, 2);
    assert_eq!(reuse, 0);
    assert_eq!(update, 0);
    assert_eq!(skip, 0);
    assert_eq!(conflict, 0);
}

/// (5) BundleType discriminator works for both PromptLibraryBundle and
/// CuratedKnowledgePackBundle: each type round-trips through JSON correctly.
#[test]
fn bundle_type_discriminator_works_for_both_types() {
    for bundle_type in [
        BundleType::PromptLibraryBundle,
        BundleType::CuratedKnowledgePackBundle,
    ] {
        let bundle = two_artifact_prompt_bundle(bundle_type);

        // Type is preserved.
        assert_eq!(
            bundle.bundle_type, bundle_type,
            "bundle_type must be set correctly"
        );

        // Validate that schema version is accepted for both bundle types.
        validate_bundle_schema_version(&bundle).unwrap_or_else(|e| panic!("{bundle_type:?}: {e}"));

        // Discriminator round-trips through JSON.
        let json = serde_json::to_string(&bundle).expect("bundle must serialize to JSON");
        let recovered: BundleEnvelope =
            serde_json::from_str(&json).expect("bundle must deserialize from JSON");
        assert_eq!(
            recovered.bundle_type, bundle_type,
            "bundle_type must survive JSON round-trip for {bundle_type:?}"
        );
    }

    // Verify serde discriminator values.
    let prompt_lib_json = serde_json::to_string(&BundleType::PromptLibraryBundle).unwrap();
    let curated_json = serde_json::to_string(&BundleType::CuratedKnowledgePackBundle).unwrap();
    assert_eq!(prompt_lib_json, r#""prompt_library_bundle""#);
    assert_eq!(curated_json, r#""curated_knowledge_pack_bundle""#);
}

/// (6) ArtifactEntry carries content_hash and artifact_logical_id correctly
/// and those values survive a JSON round-trip.
#[test]
fn artifact_entry_carries_content_hash_and_logical_id() {
    let bundle = two_artifact_prompt_bundle(BundleType::PromptLibraryBundle);

    // Verify directly.
    let system_artifact = bundle
        .artifacts
        .iter()
        .find(|a| a.artifact_logical_id == "prompt_asset_system")
        .expect("system artifact must exist");
    assert_eq!(
        system_artifact.content_hash, "hash_abc123",
        "content_hash must be stored correctly"
    );
    assert_eq!(system_artifact.artifact_kind, ArtifactKind::PromptAsset);
    assert!(system_artifact.lineage.is_some(), "lineage must be present");
    assert!(!system_artifact.tags.is_empty(), "tags must be present");

    let retrieval_artifact = bundle
        .artifacts
        .iter()
        .find(|a| a.artifact_logical_id == "prompt_asset_retrieval")
        .expect("retrieval artifact must exist");
    assert_eq!(retrieval_artifact.content_hash, "hash_def456");

    // Both hashes are distinct.
    assert_ne!(
        system_artifact.content_hash, retrieval_artifact.content_hash,
        "distinct artifacts must have distinct content hashes"
    );

    // content_hash and artifact_logical_id survive JSON round-trip.
    let json = serde_json::to_string(&bundle).unwrap();
    let recovered: BundleEnvelope = serde_json::from_str(&json).unwrap();

    for original in &bundle.artifacts {
        let roundtripped = recovered
            .artifacts
            .iter()
            .find(|a| a.artifact_logical_id == original.artifact_logical_id)
            .expect("artifact must be present after round-trip");

        assert_eq!(
            roundtripped.artifact_logical_id, original.artifact_logical_id,
            "artifact_logical_id must survive round-trip"
        );
        assert_eq!(
            roundtripped.content_hash, original.content_hash,
            "content_hash must survive round-trip"
        );
    }
}

/// ImportPlan count helpers work correctly for mixed-outcome plans.
#[test]
fn import_plan_summarize_counts_mixed_outcomes() {
    let entries = vec![
        ImportPlanEntry {
            artifact_logical_id: "a1".to_owned(),
            artifact_kind: ArtifactKind::PromptAsset,
            outcome: ImportOutcome::Create,
            reason: "new".to_owned(),
            existing_id: None,
        },
        ImportPlanEntry {
            artifact_logical_id: "a2".to_owned(),
            artifact_kind: ArtifactKind::PromptAsset,
            outcome: ImportOutcome::Create,
            reason: "new".to_owned(),
            existing_id: None,
        },
        ImportPlanEntry {
            artifact_logical_id: "a3".to_owned(),
            artifact_kind: ArtifactKind::PromptVersion,
            outcome: ImportOutcome::Reuse,
            reason: "same hash".to_owned(),
            existing_id: Some("pv_existing".to_owned()),
        },
        ImportPlanEntry {
            artifact_logical_id: "a4".to_owned(),
            artifact_kind: ArtifactKind::KnowledgePack,
            outcome: ImportOutcome::Skip,
            reason: "excluded by filter".to_owned(),
            existing_id: None,
        },
        ImportPlanEntry {
            artifact_logical_id: "a5".to_owned(),
            artifact_kind: ArtifactKind::KnowledgeDocument,
            outcome: ImportOutcome::Conflict,
            reason: "name collision".to_owned(),
            existing_id: None,
        },
    ];

    let (create, reuse, update, skip, conflict) = ImportPlan::summarize_counts(&entries);
    assert_eq!(create, 2);
    assert_eq!(reuse, 1);
    assert_eq!(update, 0);
    assert_eq!(skip, 1);
    assert_eq!(conflict, 1);

    let plan = ImportPlan {
        bundle_id: "bnd_mixed".to_owned(),
        target_scope: source_scope(),
        entries,
        create_count: create,
        reuse_count: reuse,
        update_count: update,
        skip_count: skip,
        conflict_count: conflict,
        conflict_resolution: ConflictResolutionStrategy::Skip,
    };

    assert!(
        plan.has_conflicts(),
        "plan with 1 conflict must report has_conflicts=true"
    );
}

/// Full round-trip: serialize BundleEnvelope to JSON and back, preserving all fields.
#[test]
fn bundle_full_json_round_trip() {
    let original = two_artifact_prompt_bundle(BundleType::PromptLibraryBundle);

    let json = serde_json::to_string_pretty(&original).expect("bundle must serialize to JSON");
    let recovered: BundleEnvelope =
        serde_json::from_str(&json).expect("bundle must deserialize from JSON");

    assert_eq!(
        recovered.bundle_schema_version,
        original.bundle_schema_version
    );
    assert_eq!(recovered.bundle_type, original.bundle_type);
    assert_eq!(recovered.bundle_id, original.bundle_id);
    assert_eq!(recovered.bundle_name, original.bundle_name);
    assert_eq!(recovered.artifact_count, original.artifact_count);
    assert_eq!(recovered.artifacts.len(), original.artifacts.len());
    assert_eq!(recovered.created_by, original.created_by);
    assert_eq!(
        recovered.source_deployment_id,
        original.source_deployment_id
    );
}
