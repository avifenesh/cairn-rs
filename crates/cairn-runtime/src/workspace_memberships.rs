//! Workspace membership service boundary for RFC 008 membership management.

use async_trait::async_trait;
use cairn_domain::{WorkspaceKey, WorkspaceMembership, WorkspaceRole};

use crate::error::RuntimeError;

#[async_trait]
pub trait WorkspaceMembershipService: Send + Sync {
    async fn add_member(
        &self,
        workspace_key: WorkspaceKey,
        member_id: String,
        role: WorkspaceRole,
    ) -> Result<WorkspaceMembership, RuntimeError>;

    async fn list_members(
        &self,
        workspace_key: &WorkspaceKey,
    ) -> Result<Vec<WorkspaceMembership>, RuntimeError>;

    async fn remove_member(
        &self,
        workspace_key: WorkspaceKey,
        member_id: String,
    ) -> Result<(), RuntimeError>;
}
