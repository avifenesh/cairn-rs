//! Prompt version diff tests (RFC 001).
//!
//! Validates the prompt versioning pipeline: content_hash, version_number
//! auto-increment, ordering, and cross-workspace isolation.
//!
//! RFC 001 requires that every prompt content change is captured as a new
//! immutable PromptVersion with a distinct content_hash, so operators can
//! audit exactly what changed between prompt releases.
//!
//! Note on workspace field:
//!   PromptVersionRecord.workspace is populated as "" by the current
//!   projection (populated by higher layers). Cross-workspace isolation
//!   is tested via project.workspace_id on the ProjectKey stored on each
//!   record, which IS populated by the projection.

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, PromptAssetCreated, PromptAssetId,
    PromptVersionCreated, PromptVersionId, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::{PromptAssetReadModel, PromptVersionReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(workspace: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_ver"),
        workspace_id: WorkspaceId::new(workspace),
        project_id: ProjectId::new(format!("p_{workspace}")),
    }
}

fn default_project() -> ProjectKey {
    project("ws_default")
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

/// Compute a deterministic content hash from a content string.
/// In production this would be a real SHA-256; here we use a stable stub.
fn content_hash(content: &str) -> String {
    // Simple deterministic stub: len + first/last char codes simulate uniqueness.
    let bytes = content.as_bytes();
    let first = bytes.first().copied().unwrap_or(0);
    let last = bytes.last().copied().unwrap_or(0);
    format!(
        "sha256:{:08x}{:08x}{:08x}",
        content.len(),
        first as u32,
        last as u32
    )
}

// ── 1. Create prompt asset ────────────────────────────────────────────────────

#[tokio::test]
async fn create_prompt_asset_is_stored() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_id = PromptAssetId::new("asset_diff");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                project: default_project(),
                prompt_asset_id: asset_id.clone(),
                name: "greeting-prompt".to_owned(),
                kind: "system".to_owned(),
                created_at: ts,
                workspace_id: default_project().workspace_id,
            }),
        )])
        .await
        .unwrap();

    let asset = PromptAssetReadModel::get(&store, &asset_id)
        .await
        .unwrap()
        .expect("asset must exist");
    assert_eq!(asset.prompt_asset_id, asset_id);
    assert_eq!(asset.name, "greeting-prompt");
    assert_eq!(asset.kind, "system");
}

// ── 2. Create v1 with "Hello world" content ───────────────────────────────────

#[tokio::test]
async fn create_version_v1_with_hello_world() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_id = PromptAssetId::new("asset_v1");
    let v1_id = PromptVersionId::new("ver_v1");
    let hash_v1 = content_hash("Hello world");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: default_project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "greeting".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: v1_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: hash_v1.clone(),
                    created_at: ts + 1,
                    workspace_id: default_project().workspace_id,
                }),
            ),
        ])
        .await
        .unwrap();

    let v1 = PromptVersionReadModel::get(&store, &v1_id)
        .await
        .unwrap()
        .expect("v1 must exist");
    assert_eq!(v1.prompt_version_id, v1_id);
    assert_eq!(v1.prompt_asset_id, asset_id);
    assert_eq!(v1.content_hash, hash_v1);
    assert_eq!(v1.version_number, 1, "first version is numbered 1");
}

// ── 3. Create v2 with "Hello brave world" — different hash ───────────────────

#[tokio::test]
async fn create_version_v2_with_different_content() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_id = PromptAssetId::new("asset_v12");
    let v1_id = PromptVersionId::new("ver_v12_1");
    let v2_id = PromptVersionId::new("ver_v12_2");
    let hash_v1 = content_hash("Hello world");
    let hash_v2 = content_hash("Hello brave world");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: default_project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "greeting".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: v1_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: hash_v1.clone(),
                    created_at: ts + 1,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: v2_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: hash_v2.clone(),
                    created_at: ts + 2,
                    workspace_id: default_project().workspace_id,
                }),
            ),
        ])
        .await
        .unwrap();

    let v2 = PromptVersionReadModel::get(&store, &v2_id)
        .await
        .unwrap()
        .expect("v2 must exist");
    assert_eq!(v2.content_hash, hash_v2);
    assert_ne!(
        hash_v1, hash_v2,
        "different content must produce different hashes"
    );
}

// ── 4. Both versions retrievable via PromptVersionReadModel ───────────────────

#[tokio::test]
async fn both_versions_are_retrievable() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_id = PromptAssetId::new("asset_both");
    let v1_id = PromptVersionId::new("ver_both_1");
    let v2_id = PromptVersionId::new("ver_both_2");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: default_project(),
                    prompt_asset_id: asset_id.clone(),
                    name: "both".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: v1_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: content_hash("Hello world"),
                    created_at: ts + 1,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: v2_id.clone(),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: content_hash("Hello brave world"),
                    created_at: ts + 2,
                    workspace_id: default_project().workspace_id,
                }),
            ),
        ])
        .await
        .unwrap();

    // Both retrievable by ID.
    let found_v1 = PromptVersionReadModel::get(&store, &v1_id).await.unwrap();
    let found_v2 = PromptVersionReadModel::get(&store, &v2_id).await.unwrap();
    assert!(found_v1.is_some(), "v1 must be retrievable by ID");
    assert!(found_v2.is_some(), "v2 must be retrievable by ID");

    // They are distinct records.
    assert_ne!(
        found_v1.unwrap().content_hash,
        found_v2.unwrap().content_hash
    );
}

// ── 5. list_by_asset returns versions in created_at order ────────────────────

#[tokio::test]
async fn list_by_asset_returns_versions_in_created_at_order() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_id = PromptAssetId::new("asset_order");

    // Create 4 versions with strictly increasing timestamps.
    let contents = ["v1 content", "v2 content", "v3 content", "v4 content"];
    store
        .append(&[evt(
            "e0",
            RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                project: default_project(),
                prompt_asset_id: asset_id.clone(),
                name: "ordered".to_owned(),
                kind: "system".to_owned(),
                created_at: ts,
                workspace_id: default_project().workspace_id,
            }),
        )])
        .await
        .unwrap();

    for (i, content) in contents.iter().enumerate() {
        store
            .append(&[evt(
                &format!("e{}", i + 1),
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: PromptVersionId::new(format!("ver_ord_{i}")),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: content_hash(content),
                    created_at: ts + (i as u64 + 1) * 10,
                    workspace_id: default_project().workspace_id,
                }),
            )])
            .await
            .unwrap();
    }

    let versions = PromptVersionReadModel::list_by_asset(&store, &asset_id, 10, 0)
        .await
        .unwrap();

    assert_eq!(versions.len(), 4);

    // Sorted by created_at ascending.
    for pair in versions.windows(2) {
        assert!(
            pair[0].created_at <= pair[1].created_at,
            "list_by_asset must be sorted by created_at ascending"
        );
    }

    // Version IDs in insertion order.
    assert_eq!(versions[0].prompt_version_id.as_str(), "ver_ord_0");
    assert_eq!(versions[3].prompt_version_id.as_str(), "ver_ord_3");
}

// ── 6. version_number auto-increments ────────────────────────────────────────

#[tokio::test]
async fn version_number_auto_increments_per_asset() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_id = PromptAssetId::new("asset_inc");

    store
        .append(&[evt(
            "e0",
            RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                project: default_project(),
                prompt_asset_id: asset_id.clone(),
                name: "incrementing".to_owned(),
                kind: "system".to_owned(),
                created_at: ts,
                workspace_id: default_project().workspace_id,
            }),
        )])
        .await
        .unwrap();

    for i in 1u32..=5 {
        store
            .append(&[evt(
                &format!("e{i}"),
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: PromptVersionId::new(format!("ver_inc_{i}")),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: format!("sha256:content_{i}"),
                    created_at: ts + i as u64 * 10,
                    workspace_id: default_project().workspace_id,
                }),
            )])
            .await
            .unwrap();

        let v = PromptVersionReadModel::get(&store, &PromptVersionId::new(format!("ver_inc_{i}")))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            v.version_number, i,
            "version {i} must have version_number={i}"
        );
    }

    // Final state: 5 versions, numbered 1-5.
    let all = PromptVersionReadModel::list_by_asset(&store, &asset_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 5);
    for (i, v) in all.iter().enumerate() {
        assert_eq!(
            v.version_number,
            (i + 1) as u32,
            "position {i} must have version_number {}",
            i + 1
        );
    }
}

// ── 7. content_hash differs between versions ──────────────────────────────────

#[tokio::test]
async fn content_hash_differs_between_versions() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_id = PromptAssetId::new("asset_hash");

    let prompts = [
        ("Hello world", "sha256:hello_world"),
        ("Hello brave world", "sha256:hello_brave_world"),
        ("Hello brave new world", "sha256:hello_brave_new_world"),
    ];

    store
        .append(&[evt(
            "e0",
            RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                project: default_project(),
                prompt_asset_id: asset_id.clone(),
                name: "hashing".to_owned(),
                kind: "system".to_owned(),
                created_at: ts,
                workspace_id: default_project().workspace_id,
            }),
        )])
        .await
        .unwrap();

    for (i, (_, hash)) in prompts.iter().enumerate() {
        store
            .append(&[evt(
                &format!("e{}", i + 1),
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: PromptVersionId::new(format!("ver_hash_{i}")),
                    prompt_asset_id: asset_id.clone(),
                    content_hash: hash.to_string(),
                    created_at: ts + (i as u64 + 1) * 5,
                    workspace_id: default_project().workspace_id,
                }),
            )])
            .await
            .unwrap();
    }

    let versions = PromptVersionReadModel::list_by_asset(&store, &asset_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(versions.len(), 3);

    // All content_hashes are distinct.
    let hashes: std::collections::HashSet<_> =
        versions.iter().map(|v| v.content_hash.as_str()).collect();
    assert_eq!(
        hashes.len(),
        3,
        "every version must have a unique content_hash"
    );

    // Specific hash values are preserved.
    assert_eq!(versions[0].content_hash, "sha256:hello_world");
    assert_eq!(versions[1].content_hash, "sha256:hello_brave_world");
    assert_eq!(versions[2].content_hash, "sha256:hello_brave_new_world");
}

// ── 8. Cross-workspace isolation ─────────────────────────────────────────────

#[tokio::test]
async fn versions_are_isolated_by_workspace() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let proj_ws_a = project("ws_alpha");
    let proj_ws_b = project("ws_beta");

    // Asset A in workspace alpha.
    let asset_a = PromptAssetId::new("asset_ws_a");
    // Asset B in workspace beta.
    let asset_b = PromptAssetId::new("asset_ws_b");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: proj_ws_a.clone(),
                    prompt_asset_id: asset_a.clone(),
                    name: "alpha-prompt".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts,
                    workspace_id: proj_ws_a.workspace_id.clone(),
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: proj_ws_a.clone(),
                    prompt_version_id: PromptVersionId::new("ver_a1"),
                    prompt_asset_id: asset_a.clone(),
                    content_hash: "sha256:alpha_v1".to_owned(),
                    created_at: ts + 1,
                    workspace_id: proj_ws_a.workspace_id.clone(),
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: proj_ws_a.clone(),
                    prompt_version_id: PromptVersionId::new("ver_a2"),
                    prompt_asset_id: asset_a.clone(),
                    content_hash: "sha256:alpha_v2".to_owned(),
                    created_at: ts + 2,
                    workspace_id: proj_ws_a.workspace_id.clone(),
                }),
            ),
            evt(
                "e4",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: proj_ws_b.clone(),
                    prompt_asset_id: asset_b.clone(),
                    name: "beta-prompt".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts + 10,
                    workspace_id: proj_ws_b.workspace_id.clone(),
                }),
            ),
            evt(
                "e5",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: proj_ws_b.clone(),
                    prompt_version_id: PromptVersionId::new("ver_b1"),
                    prompt_asset_id: asset_b.clone(),
                    content_hash: "sha256:beta_v1".to_owned(),
                    created_at: ts + 11,
                    workspace_id: proj_ws_b.workspace_id.clone(),
                }),
            ),
        ])
        .await
        .unwrap();

    // Workspace alpha: asset_a has 2 versions.
    let alpha_versions = PromptVersionReadModel::list_by_asset(&store, &asset_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(alpha_versions.len(), 2);
    assert!(
        alpha_versions
            .iter()
            .all(|v| v.project.workspace_id.as_str() == "ws_alpha"),
        "all alpha versions must carry workspace_id=ws_alpha"
    );

    // Workspace beta: asset_b has 1 version.
    let beta_versions = PromptVersionReadModel::list_by_asset(&store, &asset_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(beta_versions.len(), 1);
    assert_eq!(beta_versions[0].project.workspace_id.as_str(), "ws_beta");

    // Alpha and beta version sets are disjoint.
    let alpha_ids: std::collections::HashSet<_> = alpha_versions
        .iter()
        .map(|v| v.prompt_version_id.as_str())
        .collect();
    let beta_ids: std::collections::HashSet<_> = beta_versions
        .iter()
        .map(|v| v.prompt_version_id.as_str())
        .collect();
    assert!(
        alpha_ids.is_disjoint(&beta_ids),
        "alpha and beta version IDs must not overlap"
    );
}

// ── 9. Version numbers are per-asset, not global ──────────────────────────────

#[tokio::test]
async fn version_numbers_are_per_asset_not_global() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let asset_x = PromptAssetId::new("asset_vx");
    let asset_y = PromptAssetId::new("asset_vy");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: default_project(),
                    prompt_asset_id: asset_x.clone(),
                    name: "x".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: default_project(),
                    prompt_asset_id: asset_y.clone(),
                    name: "y".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts + 1,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            // Version 1 for asset_x.
            evt(
                "e3",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: PromptVersionId::new("vx1"),
                    prompt_asset_id: asset_x.clone(),
                    content_hash: "sha256:x1".to_owned(),
                    created_at: ts + 2,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            // Version 1 for asset_y — independent counter.
            evt(
                "e4",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: PromptVersionId::new("vy1"),
                    prompt_asset_id: asset_y.clone(),
                    content_hash: "sha256:y1".to_owned(),
                    created_at: ts + 3,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            // Version 2 for asset_x.
            evt(
                "e5",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: PromptVersionId::new("vx2"),
                    prompt_asset_id: asset_x.clone(),
                    content_hash: "sha256:x2".to_owned(),
                    created_at: ts + 4,
                    workspace_id: default_project().workspace_id,
                }),
            ),
            // Version 2 for asset_y.
            evt(
                "e6",
                RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
                    project: default_project(),
                    prompt_version_id: PromptVersionId::new("vy2"),
                    prompt_asset_id: asset_y.clone(),
                    content_hash: "sha256:y2".to_owned(),
                    created_at: ts + 5,
                    workspace_id: default_project().workspace_id,
                }),
            ),
        ])
        .await
        .unwrap();

    // asset_x: vx1=1, vx2=2.
    let vx1 = PromptVersionReadModel::get(&store, &PromptVersionId::new("vx1"))
        .await
        .unwrap()
        .unwrap();
    let vx2 = PromptVersionReadModel::get(&store, &PromptVersionId::new("vx2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(vx1.version_number, 1);
    assert_eq!(vx2.version_number, 2);

    // asset_y: vy1=1, vy2=2 — independent counter, not affected by asset_x versions.
    let vy1 = PromptVersionReadModel::get(&store, &PromptVersionId::new("vy1"))
        .await
        .unwrap()
        .unwrap();
    let vy2 = PromptVersionReadModel::get(&store, &PromptVersionId::new("vy2"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        vy1.version_number, 1,
        "asset_y version counter is independent of asset_x"
    );
    assert_eq!(vy2.version_number, 2);
}

// ── 10. list_by_project returns only assets for the project ───────────────────

#[tokio::test]
async fn list_by_project_returns_only_project_assets() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    let proj_x = project("ws_x");
    let proj_y = project("ws_y");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: proj_x.clone(),
                    prompt_asset_id: PromptAssetId::new("ax1"),
                    name: "x-asset-1".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts,
                    workspace_id: proj_x.workspace_id.clone(),
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: proj_x.clone(),
                    prompt_asset_id: PromptAssetId::new("ax2"),
                    name: "x-asset-2".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts + 1,
                    workspace_id: proj_x.workspace_id.clone(),
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
                    project: proj_y.clone(),
                    prompt_asset_id: PromptAssetId::new("ay1"),
                    name: "y-asset-1".to_owned(),
                    kind: "system".to_owned(),
                    created_at: ts + 2,
                    workspace_id: proj_y.workspace_id.clone(),
                }),
            ),
        ])
        .await
        .unwrap();

    let x_assets = PromptAssetReadModel::list_by_project(&store, &proj_x, 10, 0)
        .await
        .unwrap();
    assert_eq!(x_assets.len(), 2);
    assert!(x_assets
        .iter()
        .all(|a| a.project.workspace_id.as_str() == "ws_x"));

    let y_assets = PromptAssetReadModel::list_by_project(&store, &proj_y, 10, 0)
        .await
        .unwrap();
    assert_eq!(y_assets.len(), 1);
    assert_eq!(y_assets[0].prompt_asset_id.as_str(), "ay1");
}
