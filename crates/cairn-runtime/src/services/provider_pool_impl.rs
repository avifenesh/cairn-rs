//! RFC 009: provider connection pool service implementation.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::ProviderConnectionPool;
use cairn_domain::{
    ProviderConnectionId, ProviderPoolConnectionAdded, ProviderPoolConnectionRemoved,
    ProviderPoolCreated, RuntimeEvent, TenantId,
};
use cairn_store::projections::ProviderPoolReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::provider_pools::ProviderConnectionPoolService;

pub struct ProviderConnectionPoolServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ProviderConnectionPoolServiceImpl<S> {
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
impl<S> ProviderConnectionPoolService for ProviderConnectionPoolServiceImpl<S>
where
    S: EventLog + ProviderPoolReadModel + Send + Sync + 'static,
{
    async fn create_pool(
        &self,
        tenant_id: TenantId,
        pool_id: String,
        max_connections: u32,
    ) -> Result<ProviderConnectionPool, RuntimeError> {
        if ProviderPoolReadModel::get_pool(self.store.as_ref(), &pool_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "provider_pool",
                id: pool_id,
            });
        }

        let event = make_envelope(RuntimeEvent::ProviderPoolCreated(ProviderPoolCreated {
            pool_id: pool_id.clone(),
            tenant_id: tenant_id.clone(),
            max_connections,
            created_at_ms: now_ms(),
        }));
        self.store.append(&[event]).await?;

        ProviderPoolReadModel::get_pool(self.store.as_ref(), &pool_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("pool not found after create".into()))
    }

    async fn add_connection(
        &self,
        pool_id: &str,
        connection_id: ProviderConnectionId,
    ) -> Result<ProviderConnectionPool, RuntimeError> {
        let pool = ProviderPoolReadModel::get_pool(self.store.as_ref(), pool_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_pool",
                id: pool_id.to_owned(),
            })?;

        if pool.active_connections >= pool.max_connections {
            return Err(RuntimeError::PolicyDenied {
                reason: format!(
                    "pool_full: pool '{}' is at max capacity ({}/{})",
                    pool_id, pool.active_connections, pool.max_connections
                ),
            });
        }

        if pool.connection_ids.contains(&connection_id) {
            return Err(RuntimeError::Conflict {
                entity: "pool_connection",
                id: connection_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::ProviderPoolConnectionAdded(
            ProviderPoolConnectionAdded {
                pool_id: pool_id.to_owned(),
                tenant_id: pool.tenant_id.clone(),
                connection_id: connection_id.clone(),
                added_at_ms: now_ms(),
            },
        ));
        self.store.append(&[event]).await?;

        ProviderPoolReadModel::get_pool(self.store.as_ref(), pool_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("pool not found after add".into()))
    }

    async fn remove_connection(
        &self,
        pool_id: &str,
        connection_id: &ProviderConnectionId,
    ) -> Result<ProviderConnectionPool, RuntimeError> {
        let pool = ProviderPoolReadModel::get_pool(self.store.as_ref(), pool_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "provider_pool",
                id: pool_id.to_owned(),
            })?;

        if !pool.connection_ids.contains(connection_id) {
            return Err(RuntimeError::NotFound {
                entity: "pool_connection",
                id: connection_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::ProviderPoolConnectionRemoved(
            ProviderPoolConnectionRemoved {
                pool_id: pool_id.to_owned(),
                tenant_id: pool.tenant_id.clone(),
                connection_id: connection_id.clone(),
                removed_at_ms: now_ms(),
            },
        ));
        self.store.append(&[event]).await?;

        ProviderPoolReadModel::get_pool(self.store.as_ref(), pool_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("pool not found after remove".into()))
    }

    async fn get_pool(
        &self,
        pool_id: &str,
    ) -> Result<Option<ProviderConnectionPool>, RuntimeError> {
        Ok(ProviderPoolReadModel::get_pool(self.store.as_ref(), pool_id).await?)
    }

    async fn list_pools(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ProviderConnectionPool>, RuntimeError> {
        Ok(ProviderPoolReadModel::list_pools_by_tenant(self.store.as_ref(), tenant_id).await?)
    }

    async fn get_available(
        &self,
        pool_id: &str,
    ) -> Result<Option<ProviderConnectionId>, RuntimeError> {
        let pool = ProviderPoolReadModel::get_pool(self.store.as_ref(), pool_id).await?;
        Ok(pool.and_then(|p| p.connection_ids.first().cloned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{ProviderConnectionId, TenantId};
    use cairn_store::InMemoryStore;

    #[tokio::test]
    async fn provider_pool_create_and_get() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProviderConnectionPoolServiceImpl::new(store);
        let tenant = TenantId::new("t1");

        let pool = svc
            .create_pool(tenant.clone(), "pool_1".to_owned(), 3)
            .await
            .unwrap();

        assert_eq!(pool.pool_id, "pool_1");
        assert_eq!(pool.max_connections, 3);
        assert_eq!(pool.active_connections, 0);
    }

    #[tokio::test]
    async fn provider_pool_add_connections_and_assert_available() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProviderConnectionPoolServiceImpl::new(store);
        let tenant = TenantId::new("t2");

        svc.create_pool(tenant.clone(), "pool_2".to_owned(), 2)
            .await
            .unwrap();

        // Add first connection.
        let pool = svc
            .add_connection("pool_2", ProviderConnectionId::new("conn_a"))
            .await
            .unwrap();
        assert_eq!(pool.active_connections, 1);
        assert!(pool
            .connection_ids
            .contains(&ProviderConnectionId::new("conn_a")));

        // Add second connection.
        let pool = svc
            .add_connection("pool_2", ProviderConnectionId::new("conn_b"))
            .await
            .unwrap();
        assert_eq!(pool.active_connections, 2);
        assert!(pool
            .connection_ids
            .contains(&ProviderConnectionId::new("conn_b")));

        // get_available returns first connection.
        let available = svc.get_available("pool_2").await.unwrap();
        assert!(available.is_some(), "should have an available connection");

        // Adding 3rd connection when max=2 → pool_full error.
        let err = svc
            .add_connection("pool_2", ProviderConnectionId::new("conn_c"))
            .await
            .unwrap_err();

        assert!(
            matches!(&err, RuntimeError::PolicyDenied { reason } if reason.contains("pool_full")),
            "expected pool_full error, got: {err}"
        );
    }

    #[tokio::test]
    async fn provider_pool_remove_connection_frees_capacity() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProviderConnectionPoolServiceImpl::new(store);
        let tenant = TenantId::new("t3");

        svc.create_pool(tenant.clone(), "pool_3".to_owned(), 1)
            .await
            .unwrap();

        svc.add_connection("pool_3", ProviderConnectionId::new("conn_x"))
            .await
            .unwrap();

        // At capacity — adding another should fail.
        let err = svc
            .add_connection("pool_3", ProviderConnectionId::new("conn_y"))
            .await
            .unwrap_err();
        assert!(matches!(err, RuntimeError::PolicyDenied { .. }));

        // Remove conn_x → now have capacity.
        let pool = svc
            .remove_connection("pool_3", &ProviderConnectionId::new("conn_x"))
            .await
            .unwrap();
        assert_eq!(pool.active_connections, 0);

        // Now adding conn_y should succeed.
        let pool = svc
            .add_connection("pool_3", ProviderConnectionId::new("conn_y"))
            .await
            .unwrap();
        assert_eq!(pool.active_connections, 1);
    }
}
