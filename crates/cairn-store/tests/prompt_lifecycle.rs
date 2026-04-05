//! Prompt lifecycle integration tests (RFC 006).
//!
//! Validates the full prompt management pipeline using only
//! `InMemoryStore` + `EventLog::append`, proving that the event-sourced
//! projection correctly maintains read-model state at every step.
//!
//! Pipeline under test:
//!   PromptAssetCreated
//!     → PromptVersionCreated  (version 1)
//!       → PromptReleaseCreated  (state = "draft")
//!         → PromptReleaseTransitioned  (draft → approved → active)
//!           → PromptVersionCreated  (version 2 for the same asset)

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, PromptAssetCreated,
    PromptAssetId, PromptReleaseCreated, PromptReleaseId, PromptReleaseTransitioned,
    PromptVersionCreated, PromptVersionId, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::{PromptAssetReadModel, PromptReleaseReadModel, PromptVersionReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_prompt"),
        workspace_id: WorkspaceId::new("w_prompt"),
        project_id: ProjectId::new("p_prompt"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── 1. PromptAsset creation ───────────────────────────────────────────────────

#[tokio::test]
async fn create_prompt_asset_produces_asset_record() {
    let store = InMemoryStore::new();
    let asset_id = PromptAssetId::new("asset_001");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                project: project(),
                prompt_asset_id: asset_id.clone(),
                name: "main-system-prompt".to_owned(),
                kind: "system".to_owned(),
                created_at: now_ms(),
            workspace_id: project().workspace_id,
            }),
        )])
        .await
        .unwrap();

    let record = PromptAssetReadModel::get(&store, &asset_id)
        .await
        .unwrap()
        .expect("PromptAssetRecord must exist after PromptAssetCreated");

    assert_eq!(record.prompt_asset_id, asset_id);
    assert_eq!(record.name, "main-system-prompt");
    assert_eq!(record.kind, "system");
    assert_eq!(record.project, project());
    assert_eq!(record.status, "draft", "new assets start in draft status");
}

// ── 2. PromptVersion linked to asset ─────────────────────────────────────────

#[tokio::test]
async fn create_prompt_version_produces_version_record() {
    let store = InMemoryStore::new();
    let asset_id = PromptAssetId::new("asset_002");
    let version_id = PromptVersionId::new("ver_002");
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "search-prompt".to_owned(),
                    kind: "user_template".to_owned(),
                    created_at: ts,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project(),
                    prompt_version_id: version_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: "sha256:aabbcc".to_owned(),
                    created_at: ts + 1,
            workspace_id: project().workspace_id,
                }),
            ),
        ])
        .await
        .unwrap();

    let version = PromptVersionReadModel::get(&store, &version_id)
        .await
        .unwrap()
        .expect("PromptVersionRecord must exist after PromptVersionCreated");

    assert_eq!(version.prompt_version_id, version_id);
    assert_eq!(version.prompt_asset_id, asset_id);
    assert_eq!(version.content_hash, "sha256:aabbcc");
    assert_eq!(version.version_number, 1, "first version is numbered 1");
}

// ── 3. PromptRelease creation starts in draft ─────────────────────────────────

#[tokio::test]
async fn create_prompt_release_starts_in_draft() {
    let store = InMemoryStore::new();
    let asset_id = PromptAssetId::new("asset_003");
    let version_id = PromptVersionId::new("ver_003");
    let release_id = PromptReleaseId::new("rel_003");
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "critic-prompt".to_owned(),
                    kind: "critic".to_owned(),
                    created_at: ts,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project(),
                    prompt_version_id: version_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: "sha256:001".to_owned(),
                    created_at: ts + 1,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
                    project: project(),
                    prompt_release_id: release_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    prompt_version_id: version_id.clone(),
                    created_at: ts + 2,
            release_tag: None,
            created_by: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let release = PromptReleaseReadModel::get(&store, &release_id)
        .await
        .unwrap()
        .expect("PromptReleaseRecord must exist after PromptReleaseCreated");

    assert_eq!(release.prompt_release_id, release_id);
    assert_eq!(release.prompt_asset_id, asset_id);
    assert_eq!(release.prompt_version_id, version_id);
    assert_eq!(release.state, "draft", "release must start in draft state");
    assert!(release.rollout_percent.is_none());
}

// ── 4. Full lifecycle: draft → approved → active ──────────────────────────────

#[tokio::test]
async fn prompt_release_transitions_draft_to_approved_to_active() {
    let store = InMemoryStore::new();
    let asset_id = PromptAssetId::new("asset_004");
    let version_id = PromptVersionId::new("ver_004");
    let release_id = PromptReleaseId::new("rel_004");
    let ts = now_ms();

    // Create asset + version + release.
    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "router-prompt".to_owned(),
                    kind: "router".to_owned(),
                    created_at: ts,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project(),
                    prompt_version_id: version_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: "sha256:router-v1".to_owned(),
                    created_at: ts + 1,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
                    project: project(),
                    prompt_release_id: release_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    prompt_version_id: version_id.clone(),
                    created_at: ts + 2,
            release_tag: None,
            created_by: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Transition: draft → approved.
    store
        .append(&[evt(
            "e4",
            RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
                project: project(),
                prompt_release_id: release_id.clone(),
                from_state: "draft".to_owned(),
                to_state: "approved".to_owned(),
                transitioned_at: ts + 3,
            actor: None,
            reason: None,
            }),
        )])
        .await
        .unwrap();

    let after_approve = PromptReleaseReadModel::get(&store, &release_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_approve.state, "approved", "state must be 'approved' after first transition");

    // Transition: approved → active.
    store
        .append(&[evt(
            "e5",
            RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
                project: project(),
                prompt_release_id: release_id.clone(),
                from_state: "approved".to_owned(),
                to_state: "active".to_owned(),
                transitioned_at: ts + 4,
            actor: None,
            reason: None,
            }),
        )])
        .await
        .unwrap();

    let after_activate = PromptReleaseReadModel::get(&store, &release_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_activate.state, "active", "state must be 'active' after second transition");
    assert_eq!(after_activate.updated_at, ts + 4, "updated_at must reflect transition timestamp");
}

// ── 5. active_for_selector finds the active release ──────────────────────────

#[tokio::test]
async fn active_release_is_returned_by_selector_query() {
    let store = InMemoryStore::new();
    let asset_id = PromptAssetId::new("asset_005");
    let version_id = PromptVersionId::new("ver_005");
    let release_id = PromptReleaseId::new("rel_005");
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "tool-prompt".to_owned(),
                    kind: "tool_prompt".to_owned(),
                    created_at: ts,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project(),
                    prompt_version_id: version_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: "sha256:tool-v1".to_owned(),
                    created_at: ts + 1,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
                    project: project(),
                    prompt_release_id: release_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    prompt_version_id: version_id.clone(),
                    created_at: ts + 2,
            release_tag: None,
            created_by: None,
                }),
            ),
            evt(
                "e4",
                RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
                    project: project(),
                    prompt_release_id: release_id.clone(),
                    from_state: "draft".to_owned(),
                    to_state: "approved".to_owned(),
                    transitioned_at: ts + 3,
            actor: None,
            reason: None,
                }),
            ),
            evt(
                "e5",
                RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
                    project: project(),
                    prompt_release_id: release_id.clone(),
                    from_state: "approved".to_owned(),
                    to_state: "active".to_owned(),
                    transitioned_at: ts + 4,
            actor: None,
            reason: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Non-existent asset returns None.
    let none = PromptReleaseReadModel::active_for_selector(
        &store,
        &project(),
        &PromptAssetId::new("unknown"),
        "user_abc",
    )
    .await
    .unwrap();
    assert!(none.is_none());

    // Active release is found for any selector.
    let found = PromptReleaseReadModel::active_for_selector(
        &store,
        &project(),
        &asset_id,
        "user_abc",
    )
    .await
    .unwrap()
    .expect("active release must be found");
    assert_eq!(found.prompt_release_id, release_id);
    assert_eq!(found.state, "active");
}

// ── 6. Second version increments version_number ───────────────────────────────

#[tokio::test]
async fn second_version_has_incremented_version_number() {
    let store = InMemoryStore::new();
    let asset_id = PromptAssetId::new("asset_006");
    let v1_id = PromptVersionId::new("ver_006_v1");
    let v2_id = PromptVersionId::new("ver_006_v2");
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "iterating-prompt".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts,
            workspace_id: project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: project(),
                    prompt_version_id: v1_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: "sha256:v1".to_owned(),
                    created_at: ts + 1,
            workspace_id: project().workspace_id,
                }),
            ),
        ])
        .await
        .unwrap();

    let v1 = PromptVersionReadModel::get(&store, &v1_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v1.version_number, 1, "first version is numbered 1");

    // Append a second version for the same asset.
    store
        .append(&[evt(
            "e3",
            RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                project: project(),
                prompt_version_id: v2_id.clone(),
                prompt_asset_id: asset_id.clone(),
                content_hash: "sha256:v2".to_owned(),
                created_at: ts + 2,
            workspace_id: project().workspace_id,
            }),
        )])
        .await
        .unwrap();

    let v2 = PromptVersionReadModel::get(&store, &v2_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v2.version_number, 2, "second version must be numbered 2");

    // list_by_asset returns both, ordered by created_at.
    let versions = PromptVersionReadModel::list_by_asset(&store, &asset_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].version_number, 1);
    assert_eq!(versions[1].version_number, 2);
}

// ── 7. list_by_project returns all releases for a project ────────────────────

#[tokio::test]
async fn list_by_project_returns_all_project_releases() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Create two independent assets, each with one release.
    for n in 1u32..=2 {
        let asset_id = PromptAssetId::new(format!("asset_007_{n}"));
        let version_id = PromptVersionId::new(format!("ver_007_{n}"));
        let release_id = PromptReleaseId::new(format!("rel_007_{n}"));

        store
            .append(&[
                evt(
                    &format!("ea{n}"),
                    RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                        project: project(),
                        prompt_asset_id: asset_id.clone(),
                        name: format!("prompt-{n}"),
                        kind: "system".to_owned(),
                        created_at: ts + n as u64,
                        workspace_id: project().workspace_id,
                    }),
                ),
                evt(
                    &format!("ev{n}"),
                    RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                        project: project(),
                        prompt_version_id: version_id.clone(),
                        prompt_asset_id: asset_id.clone(),
                        content_hash: format!("sha256:{n}"),
                        created_at: ts + n as u64 + 10,
                        workspace_id: project().workspace_id,
                    }),
                ),
                evt(
                    &format!("er{n}"),
                    RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
                        project: project(),
                        prompt_release_id: release_id.clone(),
                        prompt_asset_id: asset_id.clone(),
                        prompt_version_id: version_id.clone(),
                        created_at: ts + n as u64 + 20,
            release_tag: None,
            created_by: None,
                    }),
                ),
            ])
            .await
            .unwrap();
    }

    let releases = PromptReleaseReadModel::list_by_project(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(releases.len(), 2, "both releases must appear in list_by_project");

    let assets = PromptAssetReadModel::list_by_project(&store, &project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(assets.len(), 2, "both assets must appear in list_by_project");
}
