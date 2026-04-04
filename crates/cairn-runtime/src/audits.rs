use async_trait::async_trait;
use cairn_domain::{AuditLogEntry, AuditOutcome, TenantId};

use crate::error::RuntimeError;

#[async_trait]
pub trait AuditService: Send + Sync {
    async fn record(
        &self,
        tenant_id: TenantId,
        actor_id: String,
        action: String,
        resource_type: String,
        resource_id: String,
        outcome: AuditOutcome,
        metadata: serde_json::Value,
    ) -> Result<AuditLogEntry, RuntimeError>;
}
