//! Concrete provider connection service implementation.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::{ProviderConnectionRecord, ProviderConnectionStatus};
use cairn_domain::*;
use cairn_store::projections::{ProviderConnectionReadModel, TenantReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::provider_connections::{ProviderConnectionConfig, ProviderConnectionService};

pub struct ProviderConnectionServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ProviderConnectionServiceImpl<S> {
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
impl<S> ProviderConnectionService for ProviderConnectionServiceImpl<S>
where
    S: EventLog + ProviderConnectionReadModel + TenantReadModel + Send + Sync + 'static,
{
    async fn create(
        &self,
        tenant_id: TenantId,
        provider_connection_id: ProviderConnectionId,
        config: ProviderConnectionConfig,
    ) -> Result<ProviderConnectionRecord, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        if ProviderConnectionReadModel::get(self.store.as_ref(), &provider_connection_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "provider_connection",
                id: provider_connection_id.to_string(),
            });
        }

        let registered_at = now_ms();
        let event = make_envelope(RuntimeEvent::ProviderConnectionRegistered(
            ProviderConnectionRegistered {
                tenant: TenantKey::new(tenant_id.clone()),
                provider_connection_id: provider_connection_id.clone(),
                provider_family: config.provider_family,
                adapter_type: config.adapter_type,
                supported_models: config.supported_models,
                status: ProviderConnectionStatus::Active,
                registered_at,
            },
        ));

        self.store.append(&[event]).await?;

        ProviderConnectionReadModel::get(self.store.as_ref(), &provider_connection_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("provider connection not found after create".to_owned())
            })
    }

    async fn get(
        &self,
        id: &ProviderConnectionId,
    ) -> Result<Option<ProviderConnectionRecord>, RuntimeError> {
        Ok(ProviderConnectionReadModel::get(self.store.as_ref(), id).await?)
    }

    async fn update(
        &self,
        id: &ProviderConnectionId,
        config: ProviderConnectionConfig,
    ) -> Result<ProviderConnectionRecord, RuntimeError> {
        let existing = ProviderConnectionReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_connection",
                id: id.to_string(),
            })?;

        // Re-register with updated config (same ID, same tenant).
        let event = make_envelope(RuntimeEvent::ProviderConnectionRegistered(
            ProviderConnectionRegistered {
                tenant: TenantKey::new(existing.tenant_id.clone()),
                provider_connection_id: id.clone(),
                provider_family: config.provider_family,
                adapter_type: config.adapter_type,
                supported_models: config.supported_models,
                status: existing.status,
                registered_at: now_ms(),
            },
        ));

        self.store.append(&[event]).await?;

        ProviderConnectionReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("provider connection not found after update".to_owned())
            })
    }

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProviderConnectionRecord>, RuntimeError> {
        Ok(ProviderConnectionReadModel::list_by_tenant(
            self.store.as_ref(),
            tenant_id,
            limit,
            offset,
        )
        .await?)
    }

    async fn delete(&self, id: &ProviderConnectionId) -> Result<(), RuntimeError> {
        let existing = ProviderConnectionReadModel::get(self.store.as_ref(), id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_connection",
                id: id.to_string(),
            })?;

        let event = make_envelope(RuntimeEvent::ProviderConnectionDeleted(
            ProviderConnectionDeleted {
                tenant: TenantKey::new(existing.tenant_id.clone()),
                provider_connection_id: id.clone(),
                deleted_at: now_ms(),
            },
        ));

        self.store.append(&[event]).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{ProviderConnectionId, TenantId};
    use cairn_store::InMemoryStore;

    use crate::provider_connections::{ProviderConnectionConfig, ProviderConnectionService};
    use crate::services::{ProviderConnectionServiceImpl, TenantServiceImpl};
    use crate::tenants::TenantService;

    /// F40: hard-delete allows re-creating with the same ID. Before the
    /// fix, DELETE appended a malformed Registered event and the create
    /// path hit `RuntimeError::Conflict` on the second attempt.
    #[tokio::test]
    async fn delete_then_recreate_same_id_succeeds() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        tenant_service
            .create(TenantId::new("tenant_f40"), "F40".to_owned())
            .await
            .unwrap();

        let service = ProviderConnectionServiceImpl::new(store);
        let id = ProviderConnectionId::new("conn_f40");

        service
            .create(
                TenantId::new("tenant_f40"),
                id.clone(),
                ProviderConnectionConfig {
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses_api".to_owned(),
                    supported_models: vec!["gpt-4".to_owned()],
                },
            )
            .await
            .expect("first create");

        service.delete(&id).await.expect("delete");

        assert!(
            service.get(&id).await.unwrap().is_none(),
            "projection row must be gone after delete",
        );

        let recreated = service
            .create(
                TenantId::new("tenant_f40"),
                id.clone(),
                ProviderConnectionConfig {
                    provider_family: "anthropic".to_owned(),
                    adapter_type: "messages_api".to_owned(),
                    supported_models: vec!["claude-opus".to_owned()],
                },
            )
            .await
            .expect("recreate after delete");

        assert_eq!(recreated.provider_family, "anthropic");
        assert_eq!(recreated.adapter_type, "messages_api");
    }

    /// F40: delete on a non-existent connection surfaces `NotFound`,
    /// which maps to 404 at the HTTP layer.
    #[tokio::test]
    async fn delete_missing_connection_is_not_found() {
        let store = Arc::new(InMemoryStore::new());
        let service = ProviderConnectionServiceImpl::new(store);
        let err = service
            .delete(&ProviderConnectionId::new("ghost"))
            .await
            .expect_err("delete on missing");
        assert!(
            matches!(err, crate::RuntimeError::NotFound { entity, .. } if entity == "provider_connection"),
            "expected NotFound, got {err:?}",
        );
    }

    #[tokio::test]
    async fn create_and_get_round_trip() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        tenant_service
            .create(TenantId::new("tenant_acme"), "Acme".to_owned())
            .await
            .unwrap();

        let service = ProviderConnectionServiceImpl::new(store);
        let created = service
            .create(
                TenantId::new("tenant_acme"),
                ProviderConnectionId::new("conn_openai"),
                ProviderConnectionConfig {
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses_api".to_owned(),
                    supported_models: vec![],
                },
            )
            .await
            .unwrap();

        let fetched = service
            .get(&ProviderConnectionId::new("conn_openai"))
            .await
            .unwrap()
            .unwrap();

        assert_eq!(created, fetched);
        assert_eq!(fetched.provider_family, "openai");
        assert_eq!(fetched.adapter_type, "responses_api");
    }
}
