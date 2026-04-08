//! Provider model capability registry (RFC 009).

use std::sync::Arc;

use cairn_domain::providers::ProviderModelCapability;
use cairn_domain::*;
use cairn_store::projections::ProviderModelReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;

pub trait ProviderModelService: Send + Sync {
    fn register_model_sync(
        &self,
        tenant_id: TenantId,
        connection_id: ProviderConnectionId,
        model_id: String,
        capabilities: ProviderModelCapability,
    );
}

pub struct ProviderModelServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ProviderModelServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

impl<S> ProviderModelServiceImpl<S>
where
    S: EventLog + ProviderModelReadModel + Send + Sync + 'static,
{
    pub async fn register(
        &self,
        tenant_id: TenantId,
        connection_id: ProviderConnectionId,
        model_id: String,
        capabilities: ProviderModelCapability,
    ) -> Result<ProviderModelCapability, RuntimeError> {
        let event = make_envelope(RuntimeEvent::ProviderModelRegistered(
            ProviderModelRegistered {
                tenant_id,
                connection_id,
                model_id: model_id.clone(),
                capabilities_json: serde_json::to_string(&capabilities).unwrap_or_default(),
            },
        ));
        self.store.append(&[event]).await?;
        Ok(capabilities)
    }

    pub async fn get(
        &self,
        model_id: &str,
    ) -> Result<Option<ProviderModelCapability>, RuntimeError> {
        Ok(ProviderModelReadModel::get_model(self.store.as_ref(), model_id).await?)
    }

    pub async fn list(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Vec<ProviderModelCapability>, RuntimeError> {
        Ok(ProviderModelReadModel::list_by_connection(self.store.as_ref(), connection_id).await?)
    }
}
