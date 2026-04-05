use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::{RetentionPolicy, RetentionPolicySet, RetentionResult, RuntimeEvent, TenantId};
use cairn_store::projections::{RetentionMaintenance, RetentionPolicyReadModel, TenantReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::retention::RetentionService;

pub struct RetentionServiceImpl<S> {
    store: Arc<S>,
}

impl<S> RetentionServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> RetentionService for RetentionServiceImpl<S>
where
    S: EventLog + RetentionPolicyReadModel + RetentionMaintenance + TenantReadModel + Send + Sync + 'static,
{
    async fn set_policy(
        &self,
        tenant_id: TenantId,
        full_history_days: u32,
        current_state_days: u32,
        max_events_per_entity: u32,
    ) -> Result<RetentionPolicy, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        let policy_id = format!("retention_policy_{}", now_millis());
        let event = make_envelope(RuntimeEvent::RetentionPolicySet(RetentionPolicySet {
            tenant_id: tenant_id.clone(),
            policy_id,
            full_history_days,
            current_state_days,
            max_events_per_entity: Some(max_events_per_entity as u64),
        }));
        self.store.append(&[event]).await?;
        self.get_policy(&tenant_id).await?.ok_or_else(|| {
            RuntimeError::Internal("retention policy not found after set".to_owned())
        })
    }

    async fn get_policy(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Option<RetentionPolicy>, RuntimeError> {
        Ok(RetentionPolicyReadModel::get_by_tenant(self.store.as_ref(), tenant_id).await?)
    }

    async fn apply_retention(&self, tenant_id: &TenantId) -> Result<RetentionResult, RuntimeError> {
        if RetentionPolicyReadModel::get_by_tenant(self.store.as_ref(), tenant_id)
            .await?
            .is_none()
        {
            return Ok(RetentionResult {
                events_pruned: 0,
                entities_affected: 0,
            });
        }
        Ok(RetentionMaintenance::apply_retention(self.store.as_ref(), tenant_id).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{
        EventEnvelope, EventId, EventSource, ProjectKey, RetentionResult, RuntimeEvent,
        SessionCreated, SessionId, SessionState, SessionStateChanged, StateTransition, TenantId,
        WorkspaceId,
    };
    use cairn_store::{EntityRef, EventLog, InMemoryStore};

    use crate::retention::RetentionService;
    use crate::services::{RetentionServiceImpl, TenantServiceImpl, WorkspaceServiceImpl};
    use crate::tenants::TenantService;
    use crate::workspaces::WorkspaceService;

    fn make_runtime_event(payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
        EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_{}", super::now_millis())),
            EventSource::Runtime,
            payload,
        )
    }

    #[tokio::test]
    async fn retention_prunes_entity_history_to_tail_limit() {
        let store = Arc::new(InMemoryStore::new());
        let tenants = TenantServiceImpl::new(store.clone());
        let workspaces = WorkspaceServiceImpl::new(store.clone());
        let retention = RetentionServiceImpl::new(store.clone());
        let tenant_id = TenantId::new("tenant_retention");

        tenants
            .create(tenant_id.clone(), "Tenant".to_owned())
            .await
            .unwrap();
        workspaces
            .create(
                tenant_id.clone(),
                WorkspaceId::new("ws_retention"),
                "Workspace".to_owned(),
            )
            .await
            .unwrap();

        let project = ProjectKey::new("tenant_retention", "ws_retention", "project_retention");
        let session_id = SessionId::new("session_retention");
        store.append(&[make_runtime_event(RuntimeEvent::SessionCreated(SessionCreated {
            project: project.clone(),
            session_id: session_id.clone(),
        }))])
        .await
        .unwrap();

        for idx in 0..19 {
            store.append(&[make_runtime_event(RuntimeEvent::SessionStateChanged(
                SessionStateChanged {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    transition: StateTransition {
                        from: Some(SessionState::Open),
                        to: SessionState::Open,
                    },
                },
            ))])
            .await
            .unwrap();
            if idx == 18 {
                // no-op to keep loop structure obvious for 20 total events on one entity
            }
        }

        retention
            .set_policy(tenant_id.clone(), 0, 30, 5)
            .await
            .unwrap();
        let result = retention.apply_retention(&tenant_id).await.unwrap();
        assert!(result.events_pruned >= 15);
        assert_eq!(
            result,
            RetentionResult {
                events_pruned: result.events_pruned,
                entities_affected: 1,
            }
        );

        let remaining = store
            .read_by_entity(&EntityRef::Session(session_id), None, 100)
            .await
            .unwrap();
        assert!(remaining.len() <= 5);
    }
}
