use async_trait::async_trait;
use cairn_domain::ids::{OperatorId, TenantId};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Flat read-model record for an operator profile (RFC 008).
///
/// Derived from `cairn_domain::org::OperatorProfile` and stored in the
/// synchronous projection so callers do not need the full domain type.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OperatorProfileRecord {
    pub operator_id: OperatorId,
    pub tenant_id: TenantId,
    pub display_name: String,
    /// Contact email — `None` when the operator has not supplied one.
    #[serde(default)]
    pub email: Option<String>,
    /// Workspace role string (e.g. "admin", "member", "viewer").
    #[serde(default)]
    pub role: String,
    /// Unix milliseconds when the profile was first created.
    #[serde(default)]
    pub created_at: u64,
}

/// Read-model for operator profiles.
#[async_trait]
pub trait OperatorProfileReadModel: Send + Sync {
    /// Look up a single operator profile by its ID.
    async fn get(
        &self,
        operator_id: &OperatorId,
    ) -> Result<Option<OperatorProfileRecord>, StoreError>;

    /// List operator profiles for a tenant with limit/offset pagination.
    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<OperatorProfileRecord>, StoreError>;
}
