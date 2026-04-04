use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::{ProviderBudget, ProviderBudgetPeriod};
use cairn_domain::ProviderBudgetSet;
use cairn_domain::{RuntimeEvent, TenantId};
use cairn_store::projections::ProviderBudgetReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::budgets::{BudgetService, BudgetStatus};
use crate::error::RuntimeError;

pub struct BudgetServiceImpl<S> {
    store: Arc<S>,
}

impl<S> BudgetServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn budget_id_for(tenant_id: &TenantId, period: ProviderBudgetPeriod) -> String {
    let period_label = match period {
        ProviderBudgetPeriod::Daily => "daily",
        ProviderBudgetPeriod::Monthly => "monthly",
    };
    format!("budget_{}_{}", tenant_id.as_str(), period_label)
}

#[async_trait]
impl<S> BudgetService for BudgetServiceImpl<S>
where
    S: EventLog + ProviderBudgetReadModel + Send + Sync + 'static,
{
    async fn set_budget(
        &self,
        tenant_id: TenantId,
        period: ProviderBudgetPeriod,
        limit_micros: u64,
        alert_threshold_percent: u32,
    ) -> Result<ProviderBudget, RuntimeError> {
        let event = make_envelope(RuntimeEvent::ProviderBudgetSet(ProviderBudgetSet {
            tenant_id: tenant_id.clone(),
            budget_id: budget_id_for(&tenant_id, period),
            period,
            limit_micros,
            alert_threshold_percent,
        }));
        self.store.append(&[event]).await?;
        self.get_budget(&tenant_id, period)
            .await?
            .ok_or_else(|| RuntimeError::Internal("budget not found after set".to_owned()))
    }

    async fn get_budget(
        &self,
        tenant_id: &TenantId,
        period: ProviderBudgetPeriod,
    ) -> Result<Option<ProviderBudget>, RuntimeError> {
        Ok(
            ProviderBudgetReadModel::get_by_tenant_period(self.store.as_ref(), tenant_id, period)
                .await?,
        )
    }

    async fn list_budgets(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ProviderBudget>, RuntimeError> {
        Ok(ProviderBudgetReadModel::list_by_tenant(self.store.as_ref(), tenant_id).await?)
    }

    async fn check_budget(&self, tenant_id: &TenantId) -> Result<BudgetStatus, RuntimeError> {
        let budgets =
            ProviderBudgetReadModel::list_by_tenant(self.store.as_ref(), tenant_id).await?;
        let mut status = BudgetStatus {
            remaining_micros: u64::MAX,
            percent_used: 0,
            alert_triggered: false,
            exceeded: false,
        };

        if budgets.is_empty() {
            status.remaining_micros = 0;
            return Ok(status);
        }

        for budget in budgets {
            let percent_used = if budget.limit_micros == 0 {
                100
            } else {
                ((budget.current_spend_micros.saturating_mul(100)) / budget.limit_micros) as u32
            };
            let remaining = budget
                .limit_micros
                .saturating_sub(budget.current_spend_micros);
            let alert_triggered = percent_used >= budget.alert_threshold_percent;
            let exceeded = budget.current_spend_micros > budget.limit_micros;

            if exceeded {
                status.exceeded = true;
            }
            if alert_triggered {
                status.alert_triggered = true;
            }
            if percent_used > status.percent_used {
                status.percent_used = percent_used;
            }
            status.remaining_micros = status.remaining_micros.min(remaining);
        }

        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::providers::{OperationKind, ProviderBudgetPeriod};
    use cairn_domain::selectors::SelectorContext;
    use cairn_domain::{
        EventEnvelope, EventId, EventSource, ProjectKey, ProviderBudgetAlertTriggered,
        ProviderBudgetExceeded, ProviderConnectionId, ProviderModelId, RuntimeEvent,
        SessionCostUpdated, TenantId, WorkspaceId,
    };
    use cairn_store::{EventLog, InMemoryStore};

    use crate::budgets::BudgetService;
    use crate::error::RuntimeError;
    use crate::projects::ProjectService;
    use crate::provider_bindings::ProviderBindingService;
    use crate::provider_connections::{ProviderConnectionConfig, ProviderConnectionService};
    use crate::routing::RouteResolverService;
    use crate::services::{
        BudgetServiceImpl, ProjectServiceImpl, ProviderBindingServiceImpl,
        ProviderConnectionServiceImpl, SimpleRouteResolver, TenantServiceImpl,
        WorkspaceServiceImpl,
    };
    use crate::tenants::TenantService;
    use crate::workspaces::WorkspaceService;

    #[tokio::test]
    async fn provider_budget_blocks_route_resolution_after_exceeding_limit() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        let workspace_service = WorkspaceServiceImpl::new(store.clone());
        let project_service = ProjectServiceImpl::new(store.clone());
        let connection_service = ProviderConnectionServiceImpl::new(store.clone());
        let binding_service = ProviderBindingServiceImpl::new(store.clone());
        let budget_service = BudgetServiceImpl::new(store.clone());

        tenant_service
            .create(TenantId::new("tenant_budget"), "Tenant".to_owned())
            .await
            .unwrap();
        workspace_service
            .create(
                TenantId::new("tenant_budget"),
                WorkspaceId::new("ws_budget"),
                "Workspace".to_owned(),
            )
            .await
            .unwrap();

        let project = ProjectKey::new("tenant_budget", "ws_budget", "project_budget");
        project_service
            .create(project.clone(), "Project".to_owned())
            .await
            .unwrap();
        connection_service
            .create(
                TenantId::new("tenant_budget"),
                ProviderConnectionId::new("conn_budget"),
                ProviderConnectionConfig {
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses_api".to_owned(),
                },
            )
            .await
            .unwrap();
        binding_service
            .create(
                project.clone(),
                ProviderConnectionId::new("conn_budget"),
                OperationKind::Generate,
                ProviderModelId::new("model_budget"),
                None,
            )
            .await
            .unwrap();

        budget_service
            .set_budget(
                TenantId::new("tenant_budget"),
                ProviderBudgetPeriod::Monthly,
                1_000,
                80,
            )
            .await
            .unwrap();

        store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_budget_session_cost"),
                EventSource::Runtime,
                RuntimeEvent::SessionCostUpdated(SessionCostUpdated {
                    session_id: "session_budget".into(),
                    tenant_id: TenantId::new("tenant_budget"),
                    delta_cost_micros: 1_100,
                    delta_tokens_in: 0,
                    delta_tokens_out: 0,
                    provider_call_id: "provider_call_budget".to_owned(),
                    updated_at_ms: 10,
                }),
            )])
            .await
            .unwrap();

        let resolver = SimpleRouteResolver::new(store.clone(), store.clone());
        let err = resolver
            .resolve(
                &project,
                OperationKind::Generate,
                &SelectorContext::default(),
            )
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            RuntimeError::PolicyDenied { ref reason } if reason.contains("provider budget exceeded")
        ));

        let events = store.read_stream(None, 20).await.unwrap();
        assert!(events.iter().any(|event| matches!(
            event.envelope.payload,
            RuntimeEvent::ProviderBudgetExceeded(ProviderBudgetExceeded { .. })
        )));
        assert!(events.iter().any(|event| matches!(
            event.envelope.payload,
            RuntimeEvent::ProviderBudgetAlertTriggered(ProviderBudgetAlertTriggered { .. })
        )));
    }
}
