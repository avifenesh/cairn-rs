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
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub role: crate::tenancy::WorkspaceRole,
    #[serde(default)]
    pub preferences: serde_json::Value,
}

/// Validate operator preference JSON. Returns Ok if valid, Err with reason if not.
///
/// Rejects preference keys that could affect runtime behavior (routing, model selection, etc.).
pub fn validate_operator_preferences(prefs: &serde_json::Value) -> Result<(), String> {
    const DISALLOWED_KEYS: &[&str] = &[
        "provider_routing",
        "model",
        "provider",
        "routing",
        "execution_class",
        "timeout_ms",
    ];

    if let serde_json::Value::Object(map) = prefs {
        for key in map.keys() {
            if DISALLOWED_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "preference key '{}' is not allowed because it affects runtime behavior",
                    key
                ));
            }
        }
    }

    Ok(())
}
