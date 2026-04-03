use cairn_domain::policy::{ExecutionClass, PolicyVerdict};
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Canonical permission tokens declared by tools and plugins.
///
/// These align with RFC 007 install-time declared permissions
/// and invocation-time granted permissions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    FsRead,
    FsWrite,
    NetworkEgress,
    ProcessExec,
    CredentialAccess,
    MemoryAccess,
}

/// A set of permissions declared at install time by a tool or plugin.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeclaredPermissions {
    pub permissions: Vec<Permission>,
}

impl DeclaredPermissions {
    pub fn new(permissions: Vec<Permission>) -> Self {
        Self { permissions }
    }

    pub fn contains(&self, permission: &Permission) -> bool {
        self.permissions.contains(permission)
    }

    pub fn is_empty(&self) -> bool {
        self.permissions.is_empty()
    }
}

/// Permissions granted for a specific invocation, scoped to a project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationGrants {
    pub project: ProjectKey,
    pub execution_class: ExecutionClass,
    pub granted: Vec<Permission>,
}

impl InvocationGrants {
    pub fn is_granted(&self, permission: &Permission) -> bool {
        self.granted.contains(permission)
    }
}

/// Result of a permission check before tool or plugin invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermissionCheckResult {
    Granted(InvocationGrants),
    Denied(PolicyVerdict),
    HeldForApproval(PolicyVerdict),
}

/// Seam for permission checking. Implementors resolve whether a proposed
/// invocation is allowed given the current policy, scope, and actor.
pub trait PermissionGate {
    fn check(
        &self,
        project: &ProjectKey,
        required: &DeclaredPermissions,
        execution_class: ExecutionClass,
    ) -> PermissionCheckResult;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::policy::ExecutionClass;
    use cairn_domain::tenancy::ProjectKey;

    #[test]
    fn declared_permissions_containment() {
        let declared = DeclaredPermissions::new(vec![Permission::FsRead, Permission::ProcessExec]);

        assert!(declared.contains(&Permission::FsRead));
        assert!(!declared.contains(&Permission::NetworkEgress));
    }

    #[test]
    fn invocation_grants_check() {
        let grants = InvocationGrants {
            project: ProjectKey::new("t", "w", "p"),
            execution_class: ExecutionClass::SupervisedProcess,
            granted: vec![Permission::FsRead],
        };

        assert!(grants.is_granted(&Permission::FsRead));
        assert!(!grants.is_granted(&Permission::FsWrite));
    }

    #[test]
    fn permission_check_result_variants() {
        let grants = InvocationGrants {
            project: ProjectKey::new("t", "w", "p"),
            execution_class: ExecutionClass::SupervisedProcess,
            granted: vec![Permission::FsRead],
        };
        let result = PermissionCheckResult::Granted(grants);
        assert!(matches!(result, PermissionCheckResult::Granted(_)));

        let denied = PermissionCheckResult::Denied(PolicyVerdict::deny("not allowed"));
        assert!(matches!(denied, PermissionCheckResult::Denied(_)));

        let held = PermissionCheckResult::HeldForApproval(PolicyVerdict::hold("needs review"));
        assert!(matches!(held, PermissionCheckResult::HeldForApproval(_)));
    }
}
