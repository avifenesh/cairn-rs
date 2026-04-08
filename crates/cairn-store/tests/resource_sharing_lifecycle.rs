//! RFC 002 — Resource sharing lifecycle tests.
//!
//! Validates cross-workspace resource sharing through the event log and
//! synchronous projection:
//!
//! - `ResourceShared` creates a `SharedResource` grant linking source to target
//!   workspace with a resource type, id, and permissions.
//! - `ResourceShareRevoked` removes the grant; subsequent reads return `None`.
//! - `list_shares_for_workspace` and `get_share_for_resource` scope queries
//!   to the correct tenant + target workspace.
//! - Revoking a share that never existed is a safe no-op.
//! - Cross-workspace isolation: grants to workspace A are not visible to B.

use cairn_domain::{
    events::{ResourceShareRevoked, ResourceShared},
    tenancy::OwnershipKey,
    EventEnvelope, EventId, EventSource, RuntimeEvent, TenantId, WorkspaceId,
};
use cairn_store::{projections::ResourceSharingReadModel, EventLog, InMemoryStore};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn tenant_ownership(tenant_id: &str) -> OwnershipKey {
    OwnershipKey::Tenant(cairn_domain::tenancy::TenantKey::new(TenantId::new(
        tenant_id,
    )))
}

async fn share_resource(
    store: &InMemoryStore,
    event_id: &str,
    share_id: &str,
    tenant_id: &str,
    source_ws: &str,
    target_ws: &str,
    resource_type: &str,
    resource_id: &str,
    permissions: Vec<&str>,
    at: u64,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        tenant_ownership(tenant_id),
        RuntimeEvent::ResourceShared(ResourceShared {
            share_id: share_id.to_owned(),
            resource_type: resource_type.to_owned(),
            grantee: target_ws.to_owned(),
            shared_at_ms: at,
            tenant_id: TenantId::new(tenant_id),
            source_workspace_id: WorkspaceId::new(source_ws),
            target_workspace_id: WorkspaceId::new(target_ws),
            resource_id: resource_id.to_owned(),
            permissions: permissions.into_iter().map(ToOwned::to_owned).collect(),
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn revoke_share(
    store: &InMemoryStore,
    event_id: &str,
    share_id: &str,
    tenant_id: &str,
    at: u64,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        tenant_ownership(tenant_id),
        RuntimeEvent::ResourceShareRevoked(ResourceShareRevoked {
            share_id: share_id.to_owned(),
            revoked_at_ms: at,
            tenant_id: TenantId::new(tenant_id),
        }),
    );
    store.append(&[env]).await.unwrap();
}

// ── 1. ResourceShared stores the grant ───────────────────────────────────────

#[tokio::test]
async fn resource_shared_appears_in_read_model() {
    let store = InMemoryStore::new();

    share_resource(
        &store,
        "e1",
        "share_1",
        "tenant_a",
        "ws_src",
        "ws_tgt",
        "prompt_asset",
        "asset_42",
        vec!["read"],
        1_000,
    )
    .await;

    let record = ResourceSharingReadModel::get_share(&store, "share_1")
        .await
        .unwrap()
        .expect("share must exist after ResourceShared");

    assert_eq!(record.share_id, "share_1");
    assert_eq!(record.tenant_id.as_str(), "tenant_a");
    assert_eq!(record.source_workspace_id.as_str(), "ws_src");
    assert_eq!(record.target_workspace_id.as_str(), "ws_tgt");
    assert_eq!(record.resource_type, "prompt_asset");
    assert_eq!(record.resource_id, "asset_42");
    assert_eq!(record.permissions, vec!["read"]);
    assert_eq!(record.shared_at_ms, 1_000);
}

#[tokio::test]
async fn get_share_returns_none_for_unknown_id() {
    let store = InMemoryStore::new();
    let result = ResourceSharingReadModel::get_share(&store, "ghost_share")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn resource_shared_with_multiple_permissions_preserves_all() {
    let store = InMemoryStore::new();

    share_resource(
        &store,
        "e1",
        "share_perms",
        "tenant_p",
        "ws_from",
        "ws_to",
        "corpus",
        "corpus_99",
        vec!["read", "embed", "export"],
        2_000,
    )
    .await;

    let rec = ResourceSharingReadModel::get_share(&store, "share_perms")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(rec.permissions.len(), 3);
    assert!(rec.permissions.contains(&"read".to_owned()));
    assert!(rec.permissions.contains(&"embed".to_owned()));
    assert!(rec.permissions.contains(&"export".to_owned()));
}

// ── 3. ResourceShareRevoked removes the grant ────────────────────────────────

#[tokio::test]
async fn revoked_share_removed_from_read_model() {
    let store = InMemoryStore::new();

    share_resource(
        &store,
        "e1",
        "share_rev",
        "tenant_r",
        "ws_a",
        "ws_b",
        "source",
        "src_1",
        vec!["read"],
        1_000,
    )
    .await;

    // Confirm the grant exists before revocation.
    let before = ResourceSharingReadModel::get_share(&store, "share_rev")
        .await
        .unwrap();
    assert!(before.is_some(), "share must exist before revocation");

    revoke_share(&store, "e2", "share_rev", "tenant_r", 2_000).await;

    // (4) Verify share removed.
    let after = ResourceSharingReadModel::get_share(&store, "share_rev")
        .await
        .unwrap();
    assert!(
        after.is_none(),
        "share must be absent after ResourceShareRevoked"
    );
}

#[tokio::test]
async fn revocation_does_not_affect_other_shares() {
    let store = InMemoryStore::new();

    share_resource(
        &store,
        "e1",
        "share_keep",
        "tenant_k",
        "ws_1",
        "ws_2",
        "prompt_asset",
        "pa_1",
        vec![],
        1_000,
    )
    .await;
    share_resource(
        &store,
        "e2",
        "share_gone",
        "tenant_k",
        "ws_1",
        "ws_2",
        "prompt_asset",
        "pa_2",
        vec![],
        1_000,
    )
    .await;

    revoke_share(&store, "e3", "share_gone", "tenant_k", 2_000).await;

    let kept = ResourceSharingReadModel::get_share(&store, "share_keep")
        .await
        .unwrap();
    let gone = ResourceSharingReadModel::get_share(&store, "share_gone")
        .await
        .unwrap();

    assert!(kept.is_some(), "unrevoked share must remain");
    assert!(gone.is_none(), "revoked share must be absent");
}

// ── 5. Cross-workspace share scoping ─────────────────────────────────────────

#[tokio::test]
async fn list_shares_for_workspace_returns_only_target_workspace_shares() {
    let store = InMemoryStore::new();
    let tenant = "tenant_scope";

    share_resource(
        &store,
        "e1",
        "sh_to_ws_a",
        tenant,
        "ws_src",
        "ws_a",
        "prompt_asset",
        "pa_1",
        vec![],
        1_000,
    )
    .await;
    share_resource(
        &store,
        "e2",
        "sh_to_ws_a2",
        tenant,
        "ws_src",
        "ws_a",
        "corpus",
        "corp_1",
        vec![],
        1_000,
    )
    .await;
    share_resource(
        &store,
        "e3",
        "sh_to_ws_b",
        tenant,
        "ws_src",
        "ws_b",
        "prompt_asset",
        "pa_2",
        vec![],
        1_000,
    )
    .await;

    let ws_a_shares = ResourceSharingReadModel::list_shares_for_workspace(
        &store,
        &TenantId::new(tenant),
        &WorkspaceId::new("ws_a"),
    )
    .await
    .unwrap();

    let ws_b_shares = ResourceSharingReadModel::list_shares_for_workspace(
        &store,
        &TenantId::new(tenant),
        &WorkspaceId::new("ws_b"),
    )
    .await
    .unwrap();

    assert_eq!(ws_a_shares.len(), 2, "ws_a must have 2 shares");
    assert_eq!(ws_b_shares.len(), 1, "ws_b must have 1 share");

    let ws_a_ids: Vec<&str> = ws_a_shares.iter().map(|s| s.share_id.as_str()).collect();
    assert!(ws_a_ids.contains(&"sh_to_ws_a") && ws_a_ids.contains(&"sh_to_ws_a2"));
    assert_eq!(ws_b_shares[0].share_id, "sh_to_ws_b");
}

#[tokio::test]
async fn list_shares_for_workspace_returns_empty_for_unknown_workspace() {
    let store = InMemoryStore::new();
    share_resource(
        &store,
        "e1",
        "sh_x",
        "tenant_empty",
        "ws_src",
        "ws_known",
        "source",
        "s1",
        vec![],
        1_000,
    )
    .await;

    let result = ResourceSharingReadModel::list_shares_for_workspace(
        &store,
        &TenantId::new("tenant_empty"),
        &WorkspaceId::new("ws_unknown"),
    )
    .await
    .unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn shares_are_scoped_by_tenant() {
    let store = InMemoryStore::new();

    share_resource(
        &store,
        "e1",
        "sh_ta",
        "tenant_ta",
        "ws_1",
        "ws_2",
        "prompt_asset",
        "pa",
        vec![],
        1_000,
    )
    .await;
    share_resource(
        &store,
        "e2",
        "sh_tb",
        "tenant_tb",
        "ws_1",
        "ws_2",
        "prompt_asset",
        "pa",
        vec![],
        1_000,
    )
    .await;

    let ta_shares = ResourceSharingReadModel::list_shares_for_workspace(
        &store,
        &TenantId::new("tenant_ta"),
        &WorkspaceId::new("ws_2"),
    )
    .await
    .unwrap();
    let tb_shares = ResourceSharingReadModel::list_shares_for_workspace(
        &store,
        &TenantId::new("tenant_tb"),
        &WorkspaceId::new("ws_2"),
    )
    .await
    .unwrap();

    assert_eq!(ta_shares.len(), 1, "tenant_ta must see only its own share");
    assert_eq!(tb_shares.len(), 1, "tenant_tb must see only its own share");
    assert_eq!(ta_shares[0].share_id, "sh_ta");
    assert_eq!(tb_shares[0].share_id, "sh_tb");
}

#[tokio::test]
async fn get_share_for_resource_finds_exact_grant() {
    let store = InMemoryStore::new();
    let tenant = "tenant_exact";

    share_resource(
        &store,
        "e1",
        "sh_exact",
        tenant,
        "ws_a",
        "ws_b",
        "corpus",
        "corp_xyz",
        vec!["read"],
        1_000,
    )
    .await;
    // Red herring: same resource_type + resource_id but different target workspace.
    share_resource(
        &store,
        "e2",
        "sh_other_ws",
        tenant,
        "ws_a",
        "ws_c",
        "corpus",
        "corp_xyz",
        vec![],
        1_000,
    )
    .await;

    let found = ResourceSharingReadModel::get_share_for_resource(
        &store,
        &TenantId::new(tenant),
        &WorkspaceId::new("ws_b"),
        "corpus",
        "corp_xyz",
    )
    .await
    .unwrap();

    assert!(found.is_some(), "exact share must be found");
    assert_eq!(found.unwrap().share_id, "sh_exact");
}

#[tokio::test]
async fn get_share_for_resource_returns_none_after_revocation() {
    let store = InMemoryStore::new();
    let tenant = "tenant_rev_res";

    share_resource(
        &store,
        "e1",
        "sh_rev_res",
        tenant,
        "ws_src",
        "ws_tgt",
        "prompt_asset",
        "pa_r",
        vec![],
        1_000,
    )
    .await;
    revoke_share(&store, "e2", "sh_rev_res", tenant, 2_000).await;

    let result = ResourceSharingReadModel::get_share_for_resource(
        &store,
        &TenantId::new(tenant),
        &WorkspaceId::new("ws_tgt"),
        "prompt_asset",
        "pa_r",
    )
    .await
    .unwrap();

    assert!(
        result.is_none(),
        "revoked share must not be returned by get_share_for_resource"
    );
}

// ── 6. Revoking a non-existent share is safe ──────────────────────────────────

#[tokio::test]
async fn revoking_nonexistent_share_is_a_no_op() {
    let store = InMemoryStore::new();

    // Revoke a share that was never created.
    revoke_share(&store, "e1", "never_existed", "tenant_noop", 1_000).await;

    let result = ResourceSharingReadModel::get_share(&store, "never_existed")
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "revoking nonexistent share must not create a phantom record"
    );
}

#[tokio::test]
async fn double_revocation_is_safe() {
    let store = InMemoryStore::new();

    share_resource(
        &store,
        "e1",
        "share_double",
        "tenant_d",
        "ws_a",
        "ws_b",
        "source",
        "s1",
        vec![],
        1_000,
    )
    .await;
    revoke_share(&store, "e2", "share_double", "tenant_d", 2_000).await;
    // Revoke a second time — must not panic or create unexpected state.
    revoke_share(&store, "e3", "share_double", "tenant_d", 3_000).await;

    let result = ResourceSharingReadModel::get_share(&store, "share_double")
        .await
        .unwrap();
    assert!(result.is_none());
}

// ── 7. Event log completeness ─────────────────────────────────────────────────

#[tokio::test]
async fn share_and_revoke_events_are_in_log() {
    let store = InMemoryStore::new();

    share_resource(
        &store,
        "e1",
        "sh_log",
        "tenant_log",
        "ws_1",
        "ws_2",
        "corpus",
        "c1",
        vec![],
        1_000,
    )
    .await;
    revoke_share(&store, "e2", "sh_log", "tenant_log", 2_000).await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 2);

    assert!(
        matches!(&all[0].envelope.payload, RuntimeEvent::ResourceShared(e)
        if e.share_id == "sh_log" && e.resource_id == "c1")
    );
    assert!(
        matches!(&all[1].envelope.payload, RuntimeEvent::ResourceShareRevoked(e)
        if e.share_id == "sh_log" && e.revoked_at_ms == 2_000)
    );
}

#[tokio::test]
async fn multiple_shares_coexist_independently() {
    let store = InMemoryStore::new();
    let tenant = "tenant_multi";

    for i in 0..5u32 {
        share_resource(
            &store,
            &format!("e{i}"),
            &format!("sh_{i}"),
            tenant,
            "ws_from",
            &format!("ws_to_{i}"),
            "prompt_asset",
            &format!("asset_{i}"),
            vec!["read"],
            1_000 + i as u64,
        )
        .await;
    }

    // Revoke only the middle one.
    revoke_share(&store, "e_rev", "sh_2", tenant, 9_000).await;

    for i in 0..5u32 {
        let result = ResourceSharingReadModel::get_share(&store, &format!("sh_{i}"))
            .await
            .unwrap();
        if i == 2 {
            assert!(result.is_none(), "sh_2 must be revoked");
        } else {
            assert!(result.is_some(), "sh_{i} must still exist");
        }
    }
}
