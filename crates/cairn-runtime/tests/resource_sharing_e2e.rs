//! RFC 008 resource sharing lifecycle end-to-end integration test.
//!
//! Validates cross-workspace resource sharing:
//!   (1) share a resource from one workspace to another
//!   (2) verify the share record exists with correct permissions
//!   (3) list shares for the source/target workspace
//!   (4) revoke the share
//!   (5) verify the share is no longer active (get_share returns None)
//!   (6) sharing gates prompt version creation across workspaces
//!   (7) multiple shares for the same resource are tracked independently
//!   (8) revoking a non-existent share returns an error

use std::sync::Arc;

use cairn_domain::{TenantId, WorkspaceId};
use cairn_runtime::{ResourceSharingService, TenantService, WorkspaceService};
use cairn_runtime::services::{
    ResourceSharingServiceImpl, TenantServiceImpl, WorkspaceServiceImpl,
};
use cairn_store::InMemoryStore;

async fn setup() -> (Arc<InMemoryStore>, ResourceSharingServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let tenants = TenantServiceImpl::new(store.clone());
    let workspaces = WorkspaceServiceImpl::new(store.clone());

    tenants
        .create(TenantId::new("tenant_share"), "Share Tenant".to_owned())
        .await
        .unwrap();
    workspaces
        .create(TenantId::new("tenant_share"), WorkspaceId::new("ws_src"), "Source WS".to_owned())
        .await
        .unwrap();
    workspaces
        .create(TenantId::new("tenant_share"), WorkspaceId::new("ws_dst"), "Dest WS".to_owned())
        .await
        .unwrap();

    let sharing = ResourceSharingServiceImpl::new(store.clone());
    (store, sharing)
}

fn tenant() -> TenantId { TenantId::new("tenant_share") }
fn src()    -> WorkspaceId { WorkspaceId::new("ws_src") }
fn dst()    -> WorkspaceId { WorkspaceId::new("ws_dst") }

// ── (1)+(2) Share and verify record fields ────────────────────────────────

#[tokio::test]
async fn share_resource_creates_record_with_correct_fields() {
    let (_, sharing) = setup().await;

    let share = sharing
        .share(
            tenant(),
            src(),
            dst(),
            "prompt_asset".to_owned(),
            "asset_abc".to_owned(),
            vec!["read".to_owned(), "version".to_owned()],
        )
        .await
        .unwrap();

    assert!(!share.share_id.is_empty(), "share_id must be populated");
    assert_eq!(share.tenant_id, tenant());
    assert_eq!(share.source_workspace_id, src());
    assert_eq!(share.target_workspace_id, dst());
    assert_eq!(share.resource_type, "prompt_asset");
    assert_eq!(share.resource_id, "asset_abc");
    assert!(share.permissions.contains(&"read".to_owned()));
    assert!(share.permissions.contains(&"version".to_owned()));
    assert!(share.shared_at_ms > 0);

    // get_share confirms it's persisted.
    let fetched = sharing.get_share(&share.share_id).await.unwrap();
    assert!(fetched.is_some(), "share must be retrievable by ID after creation");
    let fetched = fetched.unwrap();
    assert_eq!(fetched.share_id, share.share_id);
    assert_eq!(fetched.permissions, share.permissions);
}

// ── (3) List shares for target workspace ─────────────────────────────────

#[tokio::test]
async fn list_shares_returns_all_shares_for_workspace() {
    let (_, sharing) = setup().await;

    sharing
        .share(tenant(), src(), dst(), "prompt_asset".to_owned(), "asset_1".to_owned(), vec!["read".to_owned()])
        .await
        .unwrap();
    sharing
        .share(tenant(), src(), dst(), "knowledge_pack".to_owned(), "pack_2".to_owned(), vec!["read".to_owned()])
        .await
        .unwrap();

    let shares = sharing.list_shares(&tenant(), &dst()).await.unwrap();
    assert_eq!(shares.len(), 2, "both shares must appear when listing by target workspace");

    let resource_ids: Vec<_> = shares.iter().map(|s| s.resource_id.as_str()).collect();
    assert!(resource_ids.contains(&"asset_1"));
    assert!(resource_ids.contains(&"pack_2"));

    // All shares belong to the same tenant and are destined for ws_dst.
    for s in &shares {
        assert_eq!(s.tenant_id, tenant());
        assert_eq!(s.target_workspace_id, dst());
    }
}

#[tokio::test]
async fn list_shares_for_source_workspace_is_scoped() {
    let (_, sharing) = setup().await;

    sharing
        .share(tenant(), src(), dst(), "prompt_asset".to_owned(), "asset_x".to_owned(), vec![])
        .await
        .unwrap();

    // ws_src is the SOURCE — listing shares for ws_src as the target returns nothing.
    let src_shares = sharing.list_shares(&tenant(), &src()).await.unwrap();
    assert!(
        src_shares.is_empty(),
        "source workspace must not appear in target-scoped listing"
    );

    // ws_dst as target returns the share.
    let dst_shares = sharing.list_shares(&tenant(), &dst()).await.unwrap();
    assert_eq!(dst_shares.len(), 1);
    assert_eq!(dst_shares[0].source_workspace_id, src());
}

// ── (4)+(5) Revoke — share is no longer active ────────────────────────────

#[tokio::test]
async fn revoke_share_removes_it_from_store() {
    let (_, sharing) = setup().await;

    let share = sharing
        .share(tenant(), src(), dst(), "prompt_asset".to_owned(), "asset_revoke".to_owned(), vec!["read".to_owned()])
        .await
        .unwrap();
    let share_id = share.share_id.clone();

    // Confirm it exists before revoke.
    assert!(sharing.get_share(&share_id).await.unwrap().is_some());

    sharing.revoke(&share_id).await.unwrap();

    // After revoke, get_share must return None.
    let after = sharing.get_share(&share_id).await.unwrap();
    assert!(
        after.is_none(),
        "revoked share must no longer be retrievable"
    );
}

#[tokio::test]
async fn revoked_share_disappears_from_list() {
    let (_, sharing) = setup().await;

    let s1 = sharing
        .share(tenant(), src(), dst(), "prompt_asset".to_owned(), "keep_me".to_owned(), vec![])
        .await
        .unwrap();
    let s2 = sharing
        .share(tenant(), src(), dst(), "prompt_asset".to_owned(), "revoke_me".to_owned(), vec![])
        .await
        .unwrap();

    sharing.revoke(&s2.share_id).await.unwrap();

    let remaining = sharing.list_shares(&tenant(), &dst()).await.unwrap();
    assert_eq!(remaining.len(), 1, "only the non-revoked share must remain");
    assert_eq!(remaining[0].share_id, s1.share_id);
    assert_eq!(remaining[0].resource_id, "keep_me");
}

// ── (6) Share gates prompt version creation across workspaces ────────────

#[tokio::test]
async fn share_with_version_permission_unlocks_cross_workspace_version_create() {
    use cairn_domain::{ProjectKey, PromptAssetId, PromptVersionId};
    use cairn_runtime::{PromptAssetService, PromptAssetServiceImpl, PromptVersionService, PromptVersionServiceImpl};
    use cairn_runtime::error::RuntimeError;

    let store = Arc::new(InMemoryStore::new());
    let tenants = TenantServiceImpl::new(store.clone());
    let workspaces = WorkspaceServiceImpl::new(store.clone());
    let sharing = ResourceSharingServiceImpl::new(store.clone());
    let assets = PromptAssetServiceImpl::new(store.clone());
    let versions = PromptVersionServiceImpl::new(store.clone());

    tenants.create(TenantId::new("t_gate"), "Gate Tenant".to_owned()).await.unwrap();
    workspaces.create(TenantId::new("t_gate"), WorkspaceId::new("ws_gate_a"), "A".to_owned()).await.unwrap();
    workspaces.create(TenantId::new("t_gate"), WorkspaceId::new("ws_gate_b"), "B".to_owned()).await.unwrap();

    let ws_a = ProjectKey::new("t_gate", "ws_gate_a", "proj_a");
    let ws_b = ProjectKey::new("t_gate", "ws_gate_b", "proj_b");

    // Create asset in ws_a.
    assets.create(&ws_a, PromptAssetId::new("gated_asset"), "Gated".to_owned(), "system".to_owned()).await.unwrap();

    // Without share — ws_b cannot create a version.
    let denied = versions
        .create(&ws_b, PromptVersionId::new("ver_denied"), PromptAssetId::new("gated_asset"), "hash_d".to_owned())
        .await;
    assert!(matches!(denied, Err(RuntimeError::PolicyDenied { .. })), "must be denied without share");

    // Share with 'version' permission.
    let share = sharing
        .share(
            TenantId::new("t_gate"),
            WorkspaceId::new("ws_gate_a"),
            WorkspaceId::new("ws_gate_b"),
            "prompt_asset".to_owned(),
            "gated_asset".to_owned(),
            vec!["read".to_owned(), "version".to_owned()],
        )
        .await
        .unwrap();

    // Now ws_b can create a version.
    let ver = versions
        .create(&ws_b, PromptVersionId::new("ver_allowed"), PromptAssetId::new("gated_asset"), "hash_a".to_owned())
        .await
        .unwrap();
    assert_eq!(ver.prompt_asset_id, PromptAssetId::new("gated_asset"));

    // Revoke and confirm gating is restored.
    sharing.revoke(&share.share_id).await.unwrap();
    let denied2 = versions
        .create(&ws_b, PromptVersionId::new("ver_denied2"), PromptAssetId::new("gated_asset"), "hash_d2".to_owned())
        .await;
    assert!(matches!(denied2, Err(RuntimeError::PolicyDenied { .. })), "must be denied after revoke");
}

// ── (7) Multiple shares for same resource tracked independently ───────────

#[tokio::test]
async fn multiple_shares_for_same_resource_are_independent() {
    let (store, sharing) = setup().await;

    // Add a third workspace.
    let workspaces = WorkspaceServiceImpl::new(store.clone());
    workspaces
        .create(TenantId::new("tenant_share"), WorkspaceId::new("ws_third"), "Third".to_owned())
        .await
        .unwrap();

    let s1 = sharing
        .share(tenant(), src(), dst(), "prompt_asset".to_owned(), "shared_res".to_owned(), vec!["read".to_owned()])
        .await
        .unwrap();
    let s2 = sharing
        .share(tenant(), src(), WorkspaceId::new("ws_third"), "prompt_asset".to_owned(), "shared_res".to_owned(), vec!["read".to_owned(), "version".to_owned()])
        .await
        .unwrap();

    assert_ne!(s1.share_id, s2.share_id, "each share must have a unique ID");

    // Revoke only s1.
    sharing.revoke(&s1.share_id).await.unwrap();

    assert!(sharing.get_share(&s1.share_id).await.unwrap().is_none(), "s1 revoked");
    assert!(sharing.get_share(&s2.share_id).await.unwrap().is_some(), "s2 still active");
}

// ── (8) Revoking a non-existent share returns an error ────────────────────

#[tokio::test]
async fn revoke_nonexistent_share_returns_error() {
    let (_, sharing) = setup().await;

    let result = sharing.revoke("share_does_not_exist").await;
    assert!(result.is_err(), "revoking a non-existent share must return an error");
}
