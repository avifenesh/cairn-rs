//! Operator profile service boundary for tenant-scoped operator management.

use async_trait::async_trait;
use cairn_domain::org::OperatorProfile;
use cairn_domain::{OperatorId, TenantId, WorkspaceRole};

use crate::error::RuntimeError;

#[async_trait]
pub trait OperatorProfileService: Send + Sync {
    async fn create(
        &self,
        tenant_id: TenantId,
        display_name: String,
        email: String,
        role: WorkspaceRole,
    ) -> Result<OperatorProfile, RuntimeError>;

    async fn get(&self, profile_id: &OperatorId) -> Result<Option<OperatorProfile>, RuntimeError>;

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<OperatorProfile>, RuntimeError>;

    async fn update(
        &self,
        profile_id: &OperatorId,
        display_name: String,
        email: String,
    ) -> Result<OperatorProfile, RuntimeError>;

    /// RFC 008: update ergonomic/presentation preferences for an operator.
    ///
    /// Rejects any preference keys that could silently affect canonical runtime
    /// outcomes (provider routing, execution policy, etc.).
    async fn set_preferences(
        &self,
        profile_id: &OperatorId,
        preferences: serde_json::Value,
    ) -> Result<OperatorProfile, RuntimeError>;
}
