//! RFC 008: cross-workspace resource sharing service boundary.

use async_trait::async_trait;
use cairn_domain::resource_sharing::SharedResource;
use cairn_domain::{TenantId, WorkspaceId};

use crate::error::RuntimeError;

#[async_trait]
pub trait ResourceSharingService: Send + Sync {
    async fn share(
        &self,
        tenant_id: TenantId,
        source_workspace_id: WorkspaceId,
        target_workspace_id: WorkspaceId,
        resource_type: String,
        resource_id: String,
        permissions: Vec<String>,
    ) -> Result<SharedResource, RuntimeError>;

    async fn revoke(&self, share_id: &str) -> Result<(), RuntimeError>;

    async fn list_shares(
        &self,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<Vec<SharedResource>, RuntimeError>;

    async fn get_share(&self, share_id: &str) -> Result<Option<SharedResource>, RuntimeError>;
}
