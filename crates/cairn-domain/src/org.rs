use crate::ids::{OperatorId, ProjectId, TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Tenant entity record for multi-tenant organization hierarchy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantRecord {
    pub tenant_id: TenantId,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Workspace entity record scoped to a tenant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRecord {
    pub workspace_id: WorkspaceId,
    pub tenant_id: TenantId,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Project entity record scoped to a workspace within a tenant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRecord {
    pub project_id: ProjectId,
    pub workspace_id: WorkspaceId,
    pub tenant_id: TenantId,
    pub name: String,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Operator profile scoped to a tenant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorProfile {
    pub operator_id: OperatorId,
    pub tenant_id: TenantId,
    pub display_name: String,
    pub preferences: serde_json::Value,
}
