//! Project service boundary for org hierarchy management.

use async_trait::async_trait;
use cairn_domain::{ProjectKey, ProjectRecord, TenantId, WorkspaceId};

use crate::error::RuntimeError;

/// Project service boundary.
///
/// Manages project lifecycle within a workspace.
#[async_trait]
pub trait ProjectService: Send + Sync {
    /// Create a new project within a workspace.
    async fn create(
        &self,
        project: ProjectKey,
        name: String,
    ) -> Result<ProjectRecord, RuntimeError>;

    /// Get a project by its composite key.
    async fn get(
        &self,
        project: &ProjectKey,
    ) -> Result<Option<ProjectRecord>, RuntimeError>;

    /// List projects for a workspace with pagination.
    async fn list_by_workspace(
        &self,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProjectRecord>, RuntimeError>;
}
