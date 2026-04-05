use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::{RoutePolicy, RoutePolicyRule};
use cairn_domain::*;
use cairn_store::projections::{RoutePolicyReadModel, TenantReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::route_policies::RoutePolicyService;

pub struct RoutePolicyServiceImpl<S> {
    store: Arc<S>,
}

impl<S> RoutePolicyServiceImpl<S> {
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
impl<S> RoutePolicyService for RoutePolicyServiceImpl<S>
where
    S: EventLog + RoutePolicyReadModel + TenantReadModel + Send + Sync + 'static,
{
    async fn create(
        &self,
        tenant_id: TenantId,
        name: String,
        rules: Vec<RoutePolicyRule>,
    ) -> Result<RoutePolicy, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        let policy_id = format!("route_policy_{}", now_ms());
        let event = make_envelope(RuntimeEvent::RoutePolicyCreated(RoutePolicyCreated {
            tenant_id: tenant_id.clone(),
            policy_id: policy_id.clone(),
            name,
            rules,
            enabled: true,
        }));
        self.store.append(&[event]).await?;

        RoutePolicyReadModel::get(self.store.as_ref(), &policy_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("route policy not found after create".to_owned()))
    }

    async fn get(&self, policy_id: &str) -> Result<Option<RoutePolicy>, RuntimeError> {
        Ok(RoutePolicyReadModel::get(self.store.as_ref(), policy_id).await?)
    }
}
