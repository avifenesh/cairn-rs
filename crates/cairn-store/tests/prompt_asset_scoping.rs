//! Prompt asset scoping tests (RFC 006).
//!
//! Validates that prompt assets and releases are correctly scoped to their
//! originating project. RFC 006 requires that every asset, version, and
//! release carries a project key so the operator can maintain independent
//! prompt libraries per project without cross-contamination.
//!
//! Scoping contract:
//!   list_by_project(project_a) never returns assets belonging to project_b
//!   Same asset name may appear in multiple projects (name is not a unique key)
//!   get(asset_id) returns the record regardless of which project owns it
//!   A release created for an asset inherits the asset's project key

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, PromptAssetCreated,
    PromptAssetId, PromptReleaseCreated, PromptReleaseId, PromptVersionCreated,
    PromptVersionId, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::{PromptAssetReadModel, PromptReleaseReadModel, PromptVersionReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str, workspace: &str, proj: &str) -> ProjectKey {
    ProjectKey {
        tenant_id:    TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
        project_id:   ProjectId::new(proj),
    }
}

fn proj_a() -> ProjectKey { project("t_scope", "w_scope", "proj_a") }
fn proj_b() -> ProjectKey { project("t_scope", "w_scope", "proj_b") }

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn create_asset(
    evt_id: &str,
    asset_id: &str,
    proj: ProjectKey,
    name: &str,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
        project:         proj,
        prompt_asset_id: PromptAssetId::new(asset_id),
        name:            name.to_owned(),
        kind:            "system".to_owned(),
        created_at:      ts,
    }))
}

// ── 1. list_by_project returns only matching project's assets ─────────────────

#[tokio::test]
async fn list_by_project_returns_only_matching_project_assets() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Two assets in proj_a, one in proj_b.
    store.append(&[
        create_asset("e1", "asset_a1", proj_a(), "planner-prompt",  ts),
        create_asset("e2", "asset_a2", proj_a(), "reviewer-prompt", ts + 1),
        create_asset("e3", "asset_b1", proj_b(), "executor-prompt", ts + 2),
    ]).await.unwrap();

    let a_assets = PromptAssetReadModel::list_by_project(&store, &proj_a(), 10, 0)
        .await.unwrap();
    assert_eq!(a_assets.len(), 2, "proj_a has 2 assets");
    assert!(a_assets.iter().all(|a| a.project == proj_a()));
    let a_ids: Vec<_> = a_assets.iter().map(|a| a.prompt_asset_id.as_str()).collect();
    assert!(a_ids.contains(&"asset_a1"));
    assert!(a_ids.contains(&"asset_a2"));
    assert!(!a_ids.contains(&"asset_b1"), "proj_b asset must not appear in proj_a list");

    let b_assets = PromptAssetReadModel::list_by_project(&store, &proj_b(), 10, 0)
        .await.unwrap();
    assert_eq!(b_assets.len(), 1);
    assert_eq!(b_assets[0].prompt_asset_id.as_str(), "asset_b1");

    // Unregistered project returns empty.
    let empty = PromptAssetReadModel::list_by_project(
        &store, &project("t_scope", "w_scope", "proj_c"), 10, 0,
    ).await.unwrap();
    assert!(empty.is_empty());
}

// ── 2. Asset names can be duplicated across projects ─────────────────────────

#[tokio::test]
async fn same_asset_name_allowed_in_different_projects() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Both projects have an asset named "system-prompt" — different IDs.
    store.append(&[
        create_asset("e1", "asset_dup_a", proj_a(), "system-prompt", ts),
        create_asset("e2", "asset_dup_b", proj_b(), "system-prompt", ts + 1),
    ]).await.unwrap();

    let rec_a = PromptAssetReadModel::get(&store, &PromptAssetId::new("asset_dup_a"))
        .await.unwrap().unwrap();
    let rec_b = PromptAssetReadModel::get(&store, &PromptAssetId::new("asset_dup_b"))
        .await.unwrap().unwrap();

    // Same name, different project — both exist.
    assert_eq!(rec_a.name, "system-prompt");
    assert_eq!(rec_b.name, "system-prompt");
    assert_ne!(rec_a.prompt_asset_id, rec_b.prompt_asset_id,
        "distinct IDs even with same name");
    assert_eq!(rec_a.project, proj_a());
    assert_eq!(rec_b.project, proj_b());

    // Each project's list contains only its own asset.
    let a_list = PromptAssetReadModel::list_by_project(&store, &proj_a(), 10, 0)
        .await.unwrap();
    assert_eq!(a_list.len(), 1);
    assert_eq!(a_list[0].prompt_asset_id.as_str(), "asset_dup_a");

    let b_list = PromptAssetReadModel::list_by_project(&store, &proj_b(), 10, 0)
        .await.unwrap();
    assert_eq!(b_list.len(), 1);
    assert_eq!(b_list[0].prompt_asset_id.as_str(), "asset_dup_b");
}

// ── 3. get() returns correct asset regardless of project ──────────────────────

#[tokio::test]
async fn get_returns_correct_asset_regardless_of_project() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store.append(&[
        create_asset("e1", "get_a", proj_a(), "asset-in-a", ts),
        create_asset("e2", "get_b", proj_b(), "asset-in-b", ts + 1),
    ]).await.unwrap();

    // Direct get works for either project without specifying the project.
    let a = PromptAssetReadModel::get(&store, &PromptAssetId::new("get_a"))
        .await.unwrap().expect("get_a must be retrievable");
    assert_eq!(a.name, "asset-in-a");
    assert_eq!(a.project, proj_a());

    let b = PromptAssetReadModel::get(&store, &PromptAssetId::new("get_b"))
        .await.unwrap().expect("get_b must be retrievable");
    assert_eq!(b.name, "asset-in-b");
    assert_eq!(b.project, proj_b());

    // Unknown ID returns None.
    let none = PromptAssetReadModel::get(&store, &PromptAssetId::new("nonexistent"))
        .await.unwrap();
    assert!(none.is_none());
}

// ── 4. Release links correctly to the asset's project ────────────────────────

#[tokio::test]
async fn release_inherits_asset_project() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Create the full stack for proj_a.
    store.append(&[
        create_asset("e1", "asset_rel", proj_a(), "release-test-prompt", ts),
        evt("e2", RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
            project:           proj_a(),
            prompt_version_id: PromptVersionId::new("ver_rel"),
            prompt_asset_id:   PromptAssetId::new("asset_rel"),
            content_hash:      "sha256:release_test".to_owned(),
            created_at:        ts + 1,
        })),
        evt("e3", RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
            project:           proj_a(),
            prompt_release_id: PromptReleaseId::new("rel_scope"),
            prompt_asset_id:   PromptAssetId::new("asset_rel"),
            prompt_version_id: PromptVersionId::new("ver_rel"),
            created_at:        ts + 2,
            release_tag: None,
            created_by: None,
        })),
    ]).await.unwrap();

    let release = PromptReleaseReadModel::get(&store, &PromptReleaseId::new("rel_scope"))
        .await.unwrap().expect("release must exist");

    // Release carries proj_a's project key.
    assert_eq!(release.project, proj_a(),
        "release must carry the same project key as its asset");
    assert_eq!(release.prompt_asset_id.as_str(), "asset_rel");
    assert_eq!(release.prompt_version_id.as_str(), "ver_rel");
    assert_eq!(release.state, "draft");

    // Release does NOT appear in proj_b's list.
    let b_releases = PromptReleaseReadModel::list_by_project(&store, &proj_b(), 10, 0)
        .await.unwrap();
    assert!(b_releases.is_empty(),
        "proj_b has no releases — proj_a release must not leak");

    // Release appears in proj_a's list.
    let a_releases = PromptReleaseReadModel::list_by_project(&store, &proj_a(), 10, 0)
        .await.unwrap();
    assert_eq!(a_releases.len(), 1);
    assert_eq!(a_releases[0].prompt_release_id.as_str(), "rel_scope");
}

// ── 5. list_by_project ordered by created_at ─────────────────────────────────

#[tokio::test]
async fn list_by_project_ordered_by_created_at() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Append in reverse order — list must return oldest first.
    store.append(&[
        create_asset("e1", "asset_ord_c", proj_a(), "third",  ts + 200),
        create_asset("e2", "asset_ord_a", proj_a(), "first",  ts),
        create_asset("e3", "asset_ord_b", proj_a(), "second", ts + 100),
    ]).await.unwrap();

    let assets = PromptAssetReadModel::list_by_project(&store, &proj_a(), 10, 0)
        .await.unwrap();
    assert_eq!(assets.len(), 3);
    assert_eq!(assets[0].prompt_asset_id.as_str(), "asset_ord_a", "oldest first");
    assert_eq!(assets[1].prompt_asset_id.as_str(), "asset_ord_b");
    assert_eq!(assets[2].prompt_asset_id.as_str(), "asset_ord_c", "newest last");
}

// ── 6. Version also scoped to its asset's project ────────────────────────────

#[tokio::test]
async fn version_carries_parent_asset_project() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    // Version in proj_a, version in proj_b — both have different assets.
    store.append(&[
        create_asset("e1", "ast_va", proj_a(), "va-asset", ts),
        create_asset("e2", "ast_vb", proj_b(), "vb-asset", ts + 1),
        evt("e3", RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
            project:           proj_a(),
            prompt_version_id: PromptVersionId::new("ver_va"),
            prompt_asset_id:   PromptAssetId::new("ast_va"),
            content_hash:      "sha256:va".to_owned(),
            created_at:        ts + 2,
        })),
        evt("e4", RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
            project:           proj_b(),
            prompt_version_id: PromptVersionId::new("ver_vb"),
            prompt_asset_id:   PromptAssetId::new("ast_vb"),
            content_hash:      "sha256:vb".to_owned(),
            created_at:        ts + 3,
        })),
    ]).await.unwrap();

    let va = PromptVersionReadModel::get(&store, &PromptVersionId::new("ver_va"))
        .await.unwrap().unwrap();
    assert_eq!(va.project, proj_a());
    assert_eq!(va.prompt_asset_id.as_str(), "ast_va");

    let vb = PromptVersionReadModel::get(&store, &PromptVersionId::new("ver_vb"))
        .await.unwrap().unwrap();
    assert_eq!(vb.project, proj_b());

    // list_by_asset is asset-scoped (not project-scoped) — returns only that asset's versions.
    let va_versions = PromptVersionReadModel::list_by_asset(
        &store, &PromptAssetId::new("ast_va"), 10, 0,
    ).await.unwrap();
    assert_eq!(va_versions.len(), 1);
    assert_eq!(va_versions[0].project, proj_a());

    let vb_versions = PromptVersionReadModel::list_by_asset(
        &store, &PromptAssetId::new("ast_vb"), 10, 0,
    ).await.unwrap();
    assert_eq!(vb_versions.len(), 1);
    assert_eq!(vb_versions[0].project, proj_b());
}

// ── 7. list_by_project pagination ────────────────────────────────────────────

#[tokio::test]
async fn list_by_project_respects_limit_and_offset() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    for i in 0u64..5 {
        store.append(&[create_asset(
            &format!("e{i}"),
            &format!("asset_pg_{i:02}"),
            proj_a(),
            &format!("prompt-{i}"),
            ts + i * 10,
        )]).await.unwrap();
    }

    let page1 = PromptAssetReadModel::list_by_project(&store, &proj_a(), 2, 0)
        .await.unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].prompt_asset_id.as_str(), "asset_pg_00");
    assert_eq!(page1[1].prompt_asset_id.as_str(), "asset_pg_01");

    let page2 = PromptAssetReadModel::list_by_project(&store, &proj_a(), 2, 2)
        .await.unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].prompt_asset_id.as_str(), "asset_pg_02");

    let page3 = PromptAssetReadModel::list_by_project(&store, &proj_a(), 2, 4)
        .await.unwrap();
    assert_eq!(page3.len(), 1);
    assert_eq!(page3[0].prompt_asset_id.as_str(), "asset_pg_04");
}
