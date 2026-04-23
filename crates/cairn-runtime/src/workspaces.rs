//! Workspace service boundary for org hierarchy management.

use async_trait::async_trait;
use cairn_domain::{TenantId, WorkspaceId, WorkspaceRecord};

use crate::error::RuntimeError;

/// Workspace service boundary.
///
/// Manages workspace lifecycle within a tenant.
#[async_trait]
pub trait WorkspaceService: Send + Sync {
    /// Create a new workspace within a tenant.
    async fn create(
        &self,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        name: String,
    ) -> Result<WorkspaceRecord, RuntimeError>;

    /// Get a workspace by ID.
    async fn get(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspaceRecord>, RuntimeError>;

    /// List workspaces for a tenant with pagination. When
    /// `include_archived` is false (the default), soft-deleted
    /// workspaces are filtered out.
    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
        include_archived: bool,
    ) -> Result<Vec<WorkspaceRecord>, RuntimeError>;

    /// Soft-delete a workspace (issue #218). Emits `WorkspaceArchived`
    /// and marks the projection row with `archived_at`. Returns
    /// `RuntimeError::NotFound` if the workspace does not exist for the
    /// given tenant. Archiving an already-archived workspace is a no-op.
    async fn archive(
        &self,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
    ) -> Result<(), RuntimeError>;
}
