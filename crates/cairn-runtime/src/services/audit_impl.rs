use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::AuditLogReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::audits::AuditService;
use crate::error::RuntimeError;

static AUDIT_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct AuditServiceImpl<S> {
    store: Arc<S>,
}

impl<S> AuditServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn next_audit_id() -> String {
    let seq = AUDIT_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("audit_{seq}")
}

#[async_trait]
impl<S> AuditService for AuditServiceImpl<S>
where
    S: EventLog + AuditLogReadModel + Send + Sync + 'static,
{
    async fn record(
        &self,
        tenant_id: TenantId,
        actor_id: String,
        action: String,
        resource_type: String,
        resource_id: String,
        outcome: AuditOutcome,
        metadata: serde_json::Value,
    ) -> Result<AuditLogEntry, RuntimeError> {
        let entry = AuditLogEntry {
            entry_id: next_audit_id(),
            tenant_id: tenant_id.clone(),
            actor_id,
            action,
            resource_type,
            resource_id,
            outcome,
            request_id: None,
            ip_address: None,
            occurred_at_ms: now_ms(),
            metadata,
        };

        let envelope = make_envelope(RuntimeEvent::AuditLogEntryRecorded(AuditLogEntryRecorded {
            entry_id: entry.entry_id.clone(),
            tenant_id: entry.tenant_id.clone(),
            actor_id: entry.actor_id.clone(),
            action: entry.action.clone(),
            resource_type: entry.resource_type.clone(),
            resource_id: entry.resource_id.clone(),
            outcome: entry.outcome,
            occurred_at_ms: entry.occurred_at_ms,
        }));
        self.store.append(&[envelope]).await?;

        Ok(entry)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{AuditOutcome, TenantId};
    use cairn_store::InMemoryStore;

    use crate::audits::AuditService;

    use super::AuditServiceImpl;

    #[tokio::test]
    async fn record_creates_audit_entry() {
        let store = Arc::new(InMemoryStore::new());
        let service = AuditServiceImpl::new(store.clone());

        let entry = service
            .record(
                TenantId::new("tenant_audit"),
                "operator_1".to_owned(),
                "create_tenant".to_owned(),
                "tenant".to_owned(),
                "tenant_audit".to_owned(),
                AuditOutcome::Success,
                serde_json::json!({"source": "test"}),
            )
            .await
            .unwrap();

        assert_eq!(entry.tenant_id, TenantId::new("tenant_audit"));
        assert_eq!(entry.action, "create_tenant");
        assert_eq!(entry.resource_type, "tenant");
        assert_eq!(entry.resource_id, "tenant_audit");
    }
}
