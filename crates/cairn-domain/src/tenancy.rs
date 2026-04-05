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

impl Default for ProjectKey {
    fn default() -> Self {
        Self::new("", "", "")
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

/// Workspace role for access control (RFC 008).
///
/// Hierarchy (ascending privilege): Viewer < Member < Admin < Owner.
/// Use `has_at_least` to enforce minimum-role checks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRole {
    Owner,
    Admin,
    #[default]
    Member,
    Viewer,
}

impl WorkspaceRole {
    /// Numeric privilege level — higher is more privileged.
    pub fn level(self) -> u8 {
        match self {
            WorkspaceRole::Viewer => 1,
            WorkspaceRole::Member => 2,
            WorkspaceRole::Admin => 3,
            WorkspaceRole::Owner => 4,
        }
    }

    /// Returns true if this role meets or exceeds `minimum`.
    ///
    /// RFC 008: all permission checks must use this comparison, never
    /// direct equality, so future role additions slot in without auditing
    /// every check site.
    pub fn has_at_least(self, minimum: WorkspaceRole) -> bool {
        self.level() >= minimum.level()
    }
}

/// Workspace membership linking an operator to a workspace with a role.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMembership {
    pub workspace_id: WorkspaceId,
    pub operator_id: OperatorId,
    pub role: WorkspaceRole,
}

#[cfg(test)]
mod tests {
    use super::{OwnershipKey, ProjectKey, Scope, TenantKey, WorkspaceKey, WorkspaceRole};

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

    /// RFC 008: role hierarchy must be total and strictly ordered Viewer < Member < Admin < Owner.
    #[test]
    fn workspace_role_hierarchy_is_ordered() {
        use super::WorkspaceRole::*;

        // Each role must have a strictly higher level than the one below.
        assert!(Owner.level() > Admin.level());
        assert!(Admin.level() > Member.level());
        assert!(Member.level() > Viewer.level());

        // has_at_least enforces minimum-role checks.
        assert!(Owner.has_at_least(Owner));
        assert!(Owner.has_at_least(Viewer));
        assert!(Admin.has_at_least(Member));
        assert!(!Viewer.has_at_least(Member));
        assert!(!Member.has_at_least(Admin));
        assert!(!Admin.has_at_least(Owner));
    }

    /// RFC 008: a Viewer must not satisfy Member, Admin, or Owner requirements.
    #[test]
    fn viewer_cannot_meet_elevated_role_requirements() {
        use super::WorkspaceRole::*;
        assert!(!Viewer.has_at_least(Member));
        assert!(!Viewer.has_at_least(Admin));
        assert!(!Viewer.has_at_least(Owner));
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

// ── RFC 008 Gap Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod rfc008_tests {
    use super::*;

    /// RFC 008: all runtime-owned execution assets must be project-scoped.
    /// ProjectKey must carry ALL THREE scope components.
    #[test]
    fn rfc008_project_key_carries_all_three_scope_components() {
        let key = ProjectKey::new("acme_corp", "main_workspace", "agent_project");
        assert_eq!(key.tenant_id.as_str(), "acme_corp");
        assert_eq!(key.workspace_id.as_str(), "main_workspace");
        assert_eq!(key.project_id.as_str(), "agent_project");
    }

    /// RFC 008: scope hierarchy — System includes all scopes.
    #[test]
    fn rfc008_scope_hierarchy_system_includes_all() {
        assert!(Scope::System.includes(Scope::System));
        assert!(Scope::System.includes(Scope::Tenant));
        assert!(Scope::System.includes(Scope::Workspace));
        assert!(Scope::System.includes(Scope::Project));
    }

    /// RFC 008: scope hierarchy — Project does not include Workspace or Tenant.
    #[test]
    fn rfc008_project_scope_does_not_include_higher_scopes() {
        assert!(!Scope::Project.includes(Scope::Workspace));
        assert!(!Scope::Project.includes(Scope::Tenant));
        assert!(!Scope::Project.includes(Scope::System));
    }

    /// RFC 008: OwnershipKey must express every scope level.
    #[test]
    fn rfc008_ownership_key_covers_all_four_scope_levels() {
        let system = OwnershipKey::System;
        let tenant = OwnershipKey::Tenant(TenantKey::new("t1"));
        let workspace = OwnershipKey::Workspace(WorkspaceKey::new("t1", "w1"));
        let project = OwnershipKey::Project(ProjectKey::new("t1", "w1", "p1"));

        assert_eq!(system.scope(), Scope::System);
        assert_eq!(tenant.scope(), Scope::Tenant);
        assert_eq!(workspace.scope(), Scope::Workspace);
        assert_eq!(project.scope(), Scope::Project);
    }

    /// RFC 008: WorkspaceKey must contain its parent TenantId.
    /// This enforces that workspace-scoped entities always know their tenant.
    #[test]
    fn rfc008_workspace_key_contains_tenant_id() {
        let wk = WorkspaceKey::new("my_tenant", "my_workspace");
        assert_eq!(wk.tenant_id.as_str(), "my_tenant");
        // workspace_key() on ProjectKey must round-trip correctly
        let pk = ProjectKey::new("my_tenant", "my_workspace", "my_project");
        let derived_wk = pk.workspace_key();
        assert_eq!(derived_wk.tenant_id, wk.tenant_id);
        assert_eq!(derived_wk.workspace_id, wk.workspace_id);
    }

    /// RFC 008: default override chain — system < tenant < workspace < project.
    /// Scope::includes() must enforce this ordering for policy resolution.
    #[test]
    fn rfc008_default_override_chain_ordering() {
        // A higher scope "includes" a lower scope (can override it)
        assert!(Scope::Tenant.includes(Scope::Project));
        assert!(Scope::Workspace.includes(Scope::Project));
        assert!(Scope::Tenant.includes(Scope::Workspace));
        // But a lower scope cannot override a higher scope
        assert!(!Scope::Project.includes(Scope::Tenant));
        assert!(!Scope::Workspace.includes(Scope::Tenant));
    }
}
