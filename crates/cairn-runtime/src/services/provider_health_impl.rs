use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::{
    ProviderConnectionStatus, ProviderHealthRecord, ProviderHealthSchedule, ProviderHealthStatus,
};
use cairn_domain::*;
use cairn_store::projections::{
    ProviderBindingReadModel, ProviderConnectionReadModel, ProviderHealthReadModel,
    ProviderHealthScheduleReadModel,
};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::provider_health::ProviderHealthService;

pub struct ProviderHealthServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ProviderHealthServiceImpl<S> {
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

#[async_trait]
impl<S> ProviderHealthService for ProviderHealthServiceImpl<S>
where
    S: EventLog
        + ProviderConnectionReadModel
        + ProviderBindingReadModel
        + ProviderHealthReadModel
        + ProviderHealthScheduleReadModel
        + Send
        + Sync
        + 'static,
{
    async fn record_check(
        &self,
        connection_id: &ProviderConnectionId,
        latency_ms: u64,
        success: bool,
    ) -> Result<ProviderHealthRecord, RuntimeError> {
        let connection = ProviderConnectionReadModel::get(self.store.as_ref(), connection_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_connection",
                id: connection_id.to_string(),
            })?;

        let existing = ProviderHealthReadModel::get(self.store.as_ref(), connection_id).await?;
        let next_failures = if success {
            if matches!(
                existing.as_ref().map(|record| record.status),
                Some(ProviderHealthStatus::Degraded)
            ) {
                existing
                    .as_ref()
                    .map(|record| record.consecutive_failures)
                    .unwrap_or(0)
            } else {
                0
            }
        } else {
            existing
                .as_ref()
                .map(|record| record.consecutive_failures.saturating_add(1))
                .unwrap_or(1)
        };

        let checked_at_ms = now_ms();
        let checked_status = if success {
            if matches!(
                existing.as_ref().map(|record| record.status),
                Some(ProviderHealthStatus::Degraded)
            ) {
                ProviderHealthStatus::Degraded
            } else {
                ProviderHealthStatus::Healthy
            }
        } else if next_failures >= 3 {
            ProviderHealthStatus::Degraded
        } else {
            ProviderHealthStatus::Unreachable
        };

        let mut events = vec![make_envelope(RuntimeEvent::ProviderHealthChecked(
            ProviderHealthChecked {
                tenant_id: connection.tenant_id.clone(),
                connection_id: connection.provider_connection_id.clone(),
                status: checked_status,
                latency_ms: Some(latency_ms),
                checked_at_ms,
            },
        ))];

        if !success
            && next_failures >= 3
            && !matches!(
                existing.as_ref().map(|record| record.status),
                Some(ProviderHealthStatus::Degraded)
            )
        {
            events.push(make_envelope(RuntimeEvent::ProviderMarkedDegraded(
                ProviderMarkedDegraded {
                    tenant_id: connection.tenant_id.clone(),
                    connection_id: connection.provider_connection_id.clone(),
                    reason: format!("{next_failures} consecutive failed health checks"),
                    marked_at_ms: checked_at_ms,
                },
            )));

            let bindings = ProviderBindingReadModel::list_by_tenant(
                self.store.as_ref(),
                &connection.tenant_id,
                1000,
                0,
            )
            .await?;
            for binding in bindings.into_iter().filter(|binding| {
                binding.provider_connection_id == connection.provider_connection_id
                    && binding.active
            }) {
                events.push(make_envelope(RuntimeEvent::ProviderBindingStateChanged(
                    ProviderBindingStateChanged {
                        project: binding.project.clone(),
                        provider_binding_id: binding.provider_binding_id.clone(),
                        active: false,
                        changed_at: checked_at_ms,
                    },
                )));
            }
        }

        self.store.append(&events).await?;

        ProviderHealthReadModel::get(self.store.as_ref(), connection_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("provider health not found after record_check".to_owned())
            })
    }

    async fn mark_recovered(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<ProviderHealthRecord, RuntimeError> {
        let connection = ProviderConnectionReadModel::get(self.store.as_ref(), connection_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_connection",
                id: connection_id.to_string(),
            })?;

        let recovered_at_ms = now_ms();
        let mut events = vec![make_envelope(RuntimeEvent::ProviderRecovered(
            ProviderRecovered {
                tenant_id: connection.tenant_id.clone(),
                connection_id: connection.provider_connection_id.clone(),
                recovered_at_ms,
            },
        ))];

        let bindings = ProviderBindingReadModel::list_by_tenant(
            self.store.as_ref(),
            &connection.tenant_id,
            1000,
            0,
        )
        .await?;
        for binding in bindings.into_iter().filter(|binding| {
            binding.provider_connection_id == connection.provider_connection_id && !binding.active
        }) {
            events.push(make_envelope(RuntimeEvent::ProviderBindingStateChanged(
                ProviderBindingStateChanged {
                    project: binding.project.clone(),
                    provider_binding_id: binding.provider_binding_id.clone(),
                    active: true,
                    changed_at: recovered_at_ms,
                },
            )));
        }

        self.store.append(&events).await?;

        ProviderHealthReadModel::get(self.store.as_ref(), connection_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("provider health not found after recovery".to_owned())
            })
    }

    async fn get(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Option<ProviderHealthRecord>, RuntimeError> {
        Ok(ProviderHealthReadModel::get(self.store.as_ref(), connection_id).await?)
    }

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderHealthRecord>, RuntimeError> {
        Ok(
            ProviderHealthReadModel::list_by_tenant(self.store.as_ref(), tenant_id, limit, offset)
                .await?,
        )
    }

    async fn schedule_health_check(
        &self,
        connection_id: &ProviderConnectionId,
        interval_ms: u64,
    ) -> Result<ProviderHealthSchedule, RuntimeError> {
        let connection = ProviderConnectionReadModel::get(self.store.as_ref(), connection_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_connection",
                id: connection_id.to_string(),
            })?;

        let schedule_id = connection_id.as_str().to_owned();
        let event = make_envelope(RuntimeEvent::ProviderHealthScheduleSet(
            ProviderHealthScheduleSet {
                schedule_id: schedule_id.clone(),
                connection_id: connection_id.clone(),
                tenant_id: connection.tenant_id.clone(),
                interval_ms,
                enabled: true,
                set_at_ms: now_ms(),
            },
        ));
        self.store.append(&[event]).await?;

        ProviderHealthScheduleReadModel::get_schedule(self.store.as_ref(), &schedule_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("schedule not found after set".to_owned()))
    }

    async fn run_due_health_checks(&self) -> Result<Vec<ProviderHealthRecord>, RuntimeError> {
        let now = now_ms();
        let schedules =
            ProviderHealthScheduleReadModel::list_enabled_schedules(self.store.as_ref()).await?;

        let mut results = Vec::new();
        for schedule in schedules {
            let is_due = match schedule.last_run_ms {
                None => true,
                Some(last) => now >= last.saturating_add(schedule.interval_ms),
            };

            if !is_due {
                continue;
            }

            let connection =
                ProviderConnectionReadModel::get(self.store.as_ref(), &schedule.connection_id)
                    .await?;

            let success = matches!(
                connection.as_ref().map(|c| c.status),
                Some(ProviderConnectionStatus::Active)
            );

            let record = self
                .record_check(&schedule.connection_id, 0, success)
                .await?;

            let trigger_event = make_envelope(RuntimeEvent::ProviderHealthScheduleTriggered(
                ProviderHealthScheduleTriggered {
                    schedule_id: schedule.schedule_id.clone(),
                    connection_id: schedule.connection_id.clone(),
                    tenant_id: schedule.tenant_id.clone(),
                    triggered_at_ms: now,
                },
            ));
            self.store.append(&[trigger_event]).await?;

            results.push(record);
        }

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::providers::{OperationKind, ProviderHealthStatus};
    use cairn_domain::selectors::SelectorContext;
    use cairn_domain::{ProjectKey, ProviderConnectionId, ProviderModelId, TenantId, WorkspaceId};
    use cairn_store::projections::{ProviderBindingReadModel, ProviderHealthReadModel};
    use cairn_store::InMemoryStore;

    use crate::projects::ProjectService;
    use crate::provider_bindings::ProviderBindingService;
    use crate::provider_connections::{ProviderConnectionConfig, ProviderConnectionService};
    use crate::provider_health::ProviderHealthService;
    use crate::routing::RouteResolverService;
    use crate::services::{
        ProjectServiceImpl, ProviderBindingServiceImpl, ProviderConnectionServiceImpl,
        ProviderHealthServiceImpl, SimpleRouteResolver, TenantServiceImpl, WorkspaceServiceImpl,
    };
    use crate::tenants::TenantService;
    use crate::workspaces::WorkspaceService;
    use cairn_store::projections::ProviderHealthScheduleReadModel;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn provider_health_degrades_and_recovers_route_selection() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        let workspace_service = WorkspaceServiceImpl::new(store.clone());
        let project_service = ProjectServiceImpl::new(store.clone());
        let connection_service = ProviderConnectionServiceImpl::new(store.clone());
        let binding_service = ProviderBindingServiceImpl::new(store.clone());
        let health_service = ProviderHealthServiceImpl::new(store.clone());

        tenant_service
            .create(TenantId::new("tenant_health"), "Tenant".to_owned())
            .await
            .unwrap();
        workspace_service
            .create(
                TenantId::new("tenant_health"),
                WorkspaceId::new("ws_health"),
                "Workspace".to_owned(),
            )
            .await
            .unwrap();
        let project = ProjectKey::new("tenant_health", "ws_health", "project_health");
        project_service
            .create(project.clone(), "Project".to_owned())
            .await
            .unwrap();

        connection_service
            .create(
                TenantId::new("tenant_health"),
                ProviderConnectionId::new("conn_primary"),
                ProviderConnectionConfig {
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses".to_owned(),
                    supported_models: vec![],
                },
            )
            .await
            .unwrap();
        let binding_primary = binding_service
            .create(
                project.clone(),
                ProviderConnectionId::new("conn_primary"),
                OperationKind::Generate,
                ProviderModelId::new("gpt-5"),
                None,
            )
            .await
            .unwrap();
        sleep(Duration::from_millis(2)).await;

        connection_service
            .create(
                TenantId::new("tenant_health"),
                ProviderConnectionId::new("conn_backup"),
                ProviderConnectionConfig {
                    provider_family: "anthropic".to_owned(),
                    adapter_type: "messages".to_owned(),
                    supported_models: vec![],
                },
            )
            .await
            .unwrap();
        let binding_backup = binding_service
            .create(
                project.clone(),
                ProviderConnectionId::new("conn_backup"),
                OperationKind::Generate,
                ProviderModelId::new("claude-3-7-sonnet"),
                None,
            )
            .await
            .unwrap();

        for _ in 0..3 {
            health_service
                .record_check(&ProviderConnectionId::new("conn_primary"), 0, false)
                .await
                .unwrap();
        }

        let degraded = health_service
            .get(&ProviderConnectionId::new("conn_primary"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(degraded.status, ProviderHealthStatus::Degraded);
        assert_eq!(degraded.consecutive_failures, 3);

        let primary_binding =
            ProviderBindingReadModel::get(store.as_ref(), &binding_primary.provider_binding_id)
                .await
                .unwrap()
                .unwrap();
        assert!(!primary_binding.active);

        let resolver = SimpleRouteResolver::with_store(store.clone(), store.clone());
        let decision = resolver
            .resolve(
                &project,
                OperationKind::Generate,
                &SelectorContext::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            decision.selected_provider_binding_id,
            Some(binding_backup.provider_binding_id.clone())
        );

        health_service
            .mark_recovered(&ProviderConnectionId::new("conn_primary"))
            .await
            .unwrap();

        let recovered = ProviderHealthReadModel::get(
            store.as_ref(),
            &ProviderConnectionId::new("conn_primary"),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(recovered.status, ProviderHealthStatus::Healthy);
        assert_eq!(recovered.consecutive_failures, 0);

        let primary_binding =
            ProviderBindingReadModel::get(store.as_ref(), &binding_primary.provider_binding_id)
                .await
                .unwrap()
                .unwrap();
        assert!(primary_binding.active);

        let decision = resolver
            .resolve(
                &project,
                OperationKind::Generate,
                &SelectorContext::default(),
            )
            .await
            .unwrap();
        assert_eq!(
            decision.selected_provider_binding_id,
            Some(binding_primary.provider_binding_id)
        );
    }

    #[tokio::test]
    async fn health_schedule_triggers_and_updates_last_run_ms() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        let workspace_service = WorkspaceServiceImpl::new(store.clone());
        let connection_service = ProviderConnectionServiceImpl::new(store.clone());
        let health_service = ProviderHealthServiceImpl::new(store.clone());

        tenant_service
            .create(TenantId::new("tenant_sched"), "Tenant".to_owned())
            .await
            .unwrap();
        workspace_service
            .create(
                TenantId::new("tenant_sched"),
                WorkspaceId::new("ws_sched"),
                "Workspace".to_owned(),
            )
            .await
            .unwrap();

        connection_service
            .create(
                TenantId::new("tenant_sched"),
                ProviderConnectionId::new("conn_sched"),
                ProviderConnectionConfig {
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses".to_owned(),
                    supported_models: vec![],
                },
            )
            .await
            .unwrap();

        // Schedule check every 10ms
        health_service
            .schedule_health_check(&ProviderConnectionId::new("conn_sched"), 10)
            .await
            .unwrap();

        // Verify schedule exists with no last_run_ms
        let schedule = ProviderHealthScheduleReadModel::get_schedule(store.as_ref(), "conn_sched")
            .await
            .unwrap()
            .unwrap();
        assert!(schedule.last_run_ms.is_none());
        assert_eq!(schedule.interval_ms, 10);
        assert!(schedule.enabled);

        // Wait for schedule to become due
        sleep(Duration::from_millis(20)).await;

        // Run due checks
        let results = health_service.run_due_health_checks().await.unwrap();
        assert_eq!(results.len(), 1);

        // Assert last_run_ms is now set
        let updated = ProviderHealthScheduleReadModel::get_schedule(store.as_ref(), "conn_sched")
            .await
            .unwrap()
            .unwrap();
        assert!(updated.last_run_ms.is_some());
    }
}
