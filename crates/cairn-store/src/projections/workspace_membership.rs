use async_trait::async_trait;
use cairn_domain::{OperatorId, WorkspaceId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Legacy per-member record (raw string IDs, used by existing projections).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceMemberRecord {
    pub workspace_id: String,
    pub operator_id: String,
    pub role: cairn_domain::tenancy::WorkspaceRole,
    pub added_at_ms: u64,
}

/// RFC 008 workspace membership record with strongly-typed domain IDs.
///
/// Replaces the raw-string `WorkspaceMemberRecord` for new query paths that
/// need to compose with typed domain operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceMembershipRecord {
    pub workspace_id: WorkspaceId,
    pub operator_id: OperatorId,
    /// Role name (e.g. "owner", "admin", "member", "viewer").
    pub role: String,
    /// Unix milliseconds when the operator joined the workspace.
    pub joined_at: u64,
}

#[async_trait]
pub trait WorkspaceMembershipReadModel: Send + Sync {
    async fn list_workspace_members(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<WorkspaceMemberRecord>, StoreError>;

    async fn get_member(
        &self,
        workspace_key: &cairn_domain::tenancy::WorkspaceKey,
        operator_id: &str,
    ) -> Result<Option<WorkspaceMemberRecord>, StoreError>;

    async fn add_workspace_member(
        &self,
        record: WorkspaceMemberRecord,
    ) -> Result<(), StoreError>;

    async fn remove_workspace_member(
        &self,
        workspace_id: &str,
        operator_id: &str,
    ) -> Result<(), StoreError>;

    /// Look up a single membership by workspace and operator.
    ///
    /// Returns `None` if the operator is not a member of the workspace.
    /// Default no-op implementation returns `None`; concrete stores should
    /// override once the typed projection is wired.
    async fn get(
        &self,
        workspace_id: &WorkspaceId,
        operator_id: &OperatorId,
    ) -> Result<Option<WorkspaceMembershipRecord>, StoreError> {
        let _ = (workspace_id, operator_id);
        Ok(None)
    }

    /// List all members of a workspace, paginated.
    ///
    /// Default no-op implementation returns an empty vec; concrete stores
    /// should override once the typed projection is wired.
    async fn list_by_workspace(
        &self,
        workspace_id: &WorkspaceId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WorkspaceMembershipRecord>, StoreError> {
        let _ = (workspace_id, limit, offset);
        Ok(vec![])
    }
}
