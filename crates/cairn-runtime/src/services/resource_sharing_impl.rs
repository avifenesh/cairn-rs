//! RFC 008: cross-workspace resource sharing service implementation.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::resource_sharing::SharedResource;
use cairn_domain::{ResourceShareRevoked, ResourceShared, RuntimeEvent, TenantId, WorkspaceId};
use cairn_store::projections::ResourceSharingReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::resource_sharing::ResourceSharingService;

static SHARE_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_share_id() -> String {
    let n = SHARE_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("share_{n}")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub struct ResourceSharingServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ResourceSharingServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> ResourceSharingService for ResourceSharingServiceImpl<S>
where
    S: EventLog + ResourceSharingReadModel + Send + Sync + 'static,
{
    async fn share(
        &self,
        tenant_id: TenantId,
        source_workspace_id: WorkspaceId,
        target_workspace_id: WorkspaceId,
        resource_type: String,
        resource_id: String,
        permissions: Vec<String>,
    ) -> Result<SharedResource, RuntimeError> {
        let share_id = next_share_id();
        let now = now_ms();
        let event = make_envelope(RuntimeEvent::ResourceShared(ResourceShared {
            share_id: share_id.clone(),
            tenant_id: tenant_id.clone(),
            source_workspace_id: source_workspace_id.clone(),
            target_workspace_id: target_workspace_id.clone(),
            resource_type: resource_type.clone(),
            resource_id: resource_id.clone(),
            permissions: permissions.clone(),
            grantee: String::new(),
            shared_at_ms: now,
        }));
        self.store.append(&[event]).await?;
        Ok(SharedResource {
            share_id,
            tenant_id,
            source_workspace_id,
            target_workspace_id,
            resource_type,
            resource_id,
            permissions,
            shared_at_ms: now,
        })
    }

    async fn revoke(&self, share_id: &str) -> Result<(), RuntimeError> {
        let existing = ResourceSharingReadModel::get_share(self.store.as_ref(), share_id).await?;
        let share = existing.ok_or_else(|| RuntimeError::NotFound {
            entity: "resource_share",
            id: share_id.to_owned(),
        })?;
        let event = make_envelope(RuntimeEvent::ResourceShareRevoked(ResourceShareRevoked {
            share_id: share_id.to_owned(),
            tenant_id: share.tenant_id,
            revoked_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;
        Ok(())
    }

    async fn list_shares(
        &self,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<SharedResource>, RuntimeError> {
        Ok(ResourceSharingReadModel::list_shares_for_workspace(
            self.store.as_ref(),
            tenant_id,
            workspace_id,
        )
        .await?)
    }

    async fn get_share(&self, share_id: &str) -> Result<Option<SharedResource>, RuntimeError> {
        Ok(ResourceSharingReadModel::get_share(self.store.as_ref(), share_id).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{TenantId, WorkspaceId};

    use cairn_store::InMemoryStore;

    use crate::prompt_assets::PromptAssetService;
    use crate::prompt_versions::PromptVersionService;
    use crate::resource_sharing::ResourceSharingService;
    use crate::services::{
        PromptAssetServiceImpl, PromptVersionServiceImpl, ResourceSharingServiceImpl,
        TenantServiceImpl, WorkspaceServiceImpl,
    };
    use crate::tenants::TenantService;
    use crate::workspaces::WorkspaceService;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::{PromptAssetId, PromptVersionId};

    async fn setup_tenant_and_workspaces(
        store: &Arc<InMemoryStore>,
        tenant_id: &str,
    ) -> (ProjectKey, ProjectKey) {
        let tenant_svc = TenantServiceImpl::new(store.clone());
        let ws_svc = WorkspaceServiceImpl::new(store.clone());

        tenant_svc
            .create(TenantId::new(tenant_id), "Test Tenant".to_owned())
            .await
            .unwrap();
        ws_svc
            .create(
                TenantId::new(tenant_id),
                WorkspaceId::new("ws_a"),
                "Workspace A".to_owned(),
            )
            .await
            .unwrap();
        ws_svc
            .create(
                TenantId::new(tenant_id),
                WorkspaceId::new("ws_b"),
                "Workspace B".to_owned(),
            )
            .await
            .unwrap();

        let ws_a = ProjectKey::new(tenant_id, "ws_a", "default");
        let ws_b = ProjectKey::new(tenant_id, "ws_b", "default");
        (ws_a, ws_b)
    }

    #[tokio::test]
    async fn resource_sharing_share_and_revoke_controls_version_creation() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_id = "tenant_sharing";
        let (ws_a, ws_b) = setup_tenant_and_workspaces(&store, tenant_id).await;

        let asset_svc = PromptAssetServiceImpl::new(store.clone());
        let version_svc = PromptVersionServiceImpl::new(store.clone());
        let sharing_svc = ResourceSharingServiceImpl::new(store.clone());

        // Create asset in ws_a.
        asset_svc
            .create(
                &ws_a,
                PromptAssetId::new("asset_shared"),
                "Shared Asset".to_owned(),
                "system".to_owned(),
            )
            .await
            .unwrap();

        // Creating version from ws_b should FAIL (no share yet).
        let err = version_svc
            .create(
                &ws_b,
                PromptVersionId::new("ver_1"),
                PromptAssetId::new("asset_shared"),
                "hash_1".to_owned(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err, RuntimeError::PolicyDenied { .. }),
            "expected PolicyDenied before sharing, got: {err}"
        );

        // Share asset_shared from ws_a to ws_b.
        let share = sharing_svc
            .share(
                TenantId::new(tenant_id),
                WorkspaceId::new("ws_a"),
                WorkspaceId::new("ws_b"),
                "prompt_asset".to_owned(),
                "asset_shared".to_owned(),
                vec!["read".to_owned(), "version".to_owned()],
            )
            .await
            .unwrap();
        let share_id = share.share_id.clone();

        // Creating version from ws_b should now SUCCEED.
        let ver = version_svc
            .create(
                &ws_b,
                PromptVersionId::new("ver_2"),
                PromptAssetId::new("asset_shared"),
                "hash_2".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(ver.prompt_asset_id, PromptAssetId::new("asset_shared"));

        // Revoke the share.
        sharing_svc.revoke(&share_id).await.unwrap();

        // Creating another version from ws_b should FAIL again.
        let err2 = version_svc
            .create(
                &ws_b,
                PromptVersionId::new("ver_3"),
                PromptAssetId::new("asset_shared"),
                "hash_3".to_owned(),
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err2, RuntimeError::PolicyDenied { .. }),
            "expected PolicyDenied after revoke, got: {err2}"
        );
    }

    #[tokio::test]
    async fn resource_sharing_list_shares_for_workspace() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_id = "tenant_list_shares";
        let (ws_a, ws_b) = setup_tenant_and_workspaces(&store, tenant_id).await;
        let sharing_svc = ResourceSharingServiceImpl::new(store.clone());

        sharing_svc
            .share(
                TenantId::new(tenant_id),
                WorkspaceId::new("ws_a"),
                WorkspaceId::new("ws_b"),
                "prompt_asset".to_owned(),
                "asset_1".to_owned(),
                vec![],
            )
            .await
            .unwrap();

        let shares = sharing_svc
            .list_shares(&TenantId::new(tenant_id), &WorkspaceId::new("ws_b"))
            .await
            .unwrap();
        assert_eq!(shares.len(), 1);
        assert_eq!(shares[0].resource_id, "asset_1");

        let _ = ws_a;
        let _ = ws_b;
    }
}
