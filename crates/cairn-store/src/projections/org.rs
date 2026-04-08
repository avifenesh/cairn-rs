use async_trait::async_trait;
use cairn_domain::org::{ProjectRecord, TenantRecord, WorkspaceRecord};
use cairn_domain::{ProjectKey, TenantId, WorkspaceId};

use crate::error::StoreError;

/// Read-model for tenant current state.
#[async_trait]
pub trait TenantReadModel: Send + Sync {
    async fn get(&self, id: &TenantId) -> Result<Option<TenantRecord>, StoreError>;
    async fn list(&self, limit: usize, offset: usize) -> Result<Vec<TenantRecord>, StoreError>;
}

/// Read-model for workspace current state.
#[async_trait]
pub trait WorkspaceReadModel: Send + Sync {
    async fn get(&self, id: &WorkspaceId) -> Result<Option<WorkspaceRecord>, StoreError>;
    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WorkspaceRecord>, StoreError>;
}

/// Read-model for project current state.
#[async_trait]
pub trait ProjectReadModel: Send + Sync {
    async fn get_project(&self, project: &ProjectKey) -> Result<Option<ProjectRecord>, StoreError>;
    async fn list_by_workspace(
        &self,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProjectRecord>, StoreError>;
}
