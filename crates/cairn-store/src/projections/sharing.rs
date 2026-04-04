//! RFC 008: read-model for cross-workspace resource shares.

use async_trait::async_trait;
use cairn_domain::resource_sharing::SharedResource;
use cairn_domain::{TenantId, WorkspaceId};

use crate::error::StoreError;

#[async_trait]
pub trait ResourceSharingReadModel: Send + Sync {
    async fn get_share(&self, share_id: &str) -> Result<Option<SharedResource>, StoreError>;

    async fn list_shares_for_workspace(
        &self,
        tenant_id: &TenantId,
        target_workspace_id: &WorkspaceId,
    ) -> Result<Vec<SharedResource>, StoreError>;

    /// Find an active share for a specific resource to a specific workspace.
    async fn get_share_for_resource(
        &self,
        tenant_id: &TenantId,
        target_workspace_id: &WorkspaceId,
        resource_type: &str,
        resource_id: &str,
    ) -> Result<Option<SharedResource>, StoreError>;
}
