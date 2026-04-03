use crate::ids::{OperatorId, ProjectId, TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// Canonical product ownership layers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    System,
    Tenant,
    Workspace,
    Project,
}

/// Top-level product ownership key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantKey {
    pub tenant_id: TenantId,
}

/// Team or environment ownership key inside a tenant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceKey {
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
}

/// Runtime ownership key for execution-truth entities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectKey {
    pub tenant_id: TenantId,
    pub workspace_id: WorkspaceId,
    pub project_id: ProjectId,
}

/// Tenant-scoped operator profile ownership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorProfileKey {
    pub tenant_id: TenantId,
    pub operator_id: OperatorId,
}

/// Single enum that can be attached to envelopes and read-model facts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "scope", rename_all = "snake_case")]
pub enum OwnershipKey {
    System,
    Tenant(TenantKey),
    Workspace(WorkspaceKey),
    Project(ProjectKey),
}

impl Scope {
    pub fn includes(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Scope::System, _)
                | (
                    Scope::Tenant,
                    Scope::Tenant | Scope::Workspace | Scope::Project
                )
                | (Scope::Workspace, Scope::Workspace | Scope::Project)
                | (Scope::Project, Scope::Project)
        )
    }
}

impl TenantKey {
    pub fn new(tenant_id: impl Into<TenantId>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
        }
    }
}

impl WorkspaceKey {
    pub fn new(tenant_id: impl Into<TenantId>, workspace_id: impl Into<WorkspaceId>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            workspace_id: workspace_id.into(),
        }
    }
}

impl ProjectKey {
    pub fn new(
        tenant_id: impl Into<TenantId>,
        workspace_id: impl Into<WorkspaceId>,
        project_id: impl Into<ProjectId>,
    ) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            workspace_id: workspace_id.into(),
            project_id: project_id.into(),
        }
    }

    pub fn workspace_key(&self) -> WorkspaceKey {
        WorkspaceKey {
            tenant_id: self.tenant_id.clone(),
            workspace_id: self.workspace_id.clone(),
        }
    }
}

impl OperatorProfileKey {
    pub fn new(tenant_id: impl Into<TenantId>, operator_id: impl Into<OperatorId>) -> Self {
        Self {
            tenant_id: tenant_id.into(),
            operator_id: operator_id.into(),
        }
    }
}

impl OwnershipKey {
    pub fn scope(&self) -> Scope {
        match self {
            OwnershipKey::System => Scope::System,
            OwnershipKey::Tenant(_) => Scope::Tenant,
            OwnershipKey::Workspace(_) => Scope::Workspace,
            OwnershipKey::Project(_) => Scope::Project,
        }
    }
}

impl From<TenantKey> for OwnershipKey {
    fn from(value: TenantKey) -> Self {
        OwnershipKey::Tenant(value)
    }
}

impl From<WorkspaceKey> for OwnershipKey {
    fn from(value: WorkspaceKey) -> Self {
        OwnershipKey::Workspace(value)
    }
}

impl From<ProjectKey> for OwnershipKey {
    fn from(value: ProjectKey) -> Self {
        OwnershipKey::Project(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{OwnershipKey, ProjectKey, Scope, TenantKey, WorkspaceKey};

    #[test]
    fn scope_hierarchy_matches_rfc_order() {
        assert!(Scope::System.includes(Scope::Project));
        assert!(Scope::Tenant.includes(Scope::Workspace));
        assert!(Scope::Workspace.includes(Scope::Project));
        assert!(!Scope::Project.includes(Scope::Workspace));
    }

    #[test]
    fn ownership_key_reports_scope() {
        let tenant = OwnershipKey::Tenant(TenantKey::new("tenant"));
        let workspace = OwnershipKey::Workspace(WorkspaceKey::new("tenant", "workspace"));
        let project = OwnershipKey::Project(ProjectKey::new("tenant", "workspace", "project"));

        assert_eq!(tenant.scope(), Scope::Tenant);
        assert_eq!(workspace.scope(), Scope::Workspace);
        assert_eq!(project.scope(), Scope::Project);
    }

    #[test]
    fn ownership_key_conversions_preserve_scope() {
        let tenant: OwnershipKey = TenantKey::new("tenant").into();
        let workspace: OwnershipKey = WorkspaceKey::new("tenant", "workspace").into();
        let project: OwnershipKey = ProjectKey::new("tenant", "workspace", "project").into();

        assert_eq!(tenant.scope(), Scope::Tenant);
        assert_eq!(workspace.scope(), Scope::Workspace);
        assert_eq!(project.scope(), Scope::Project);
    }
}
