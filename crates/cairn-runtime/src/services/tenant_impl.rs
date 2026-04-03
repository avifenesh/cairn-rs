//! Concrete tenant service implementation.
//!
//! Manages tenant lifecycle by emitting `TenantCreated` events
//! and reading back via the `TenantReadModel` projection.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::TenantReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::tenants::TenantService;

pub struct TenantServiceImpl<S> {
    store: Arc<S>,
}

impl<S> TenantServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> TenantService for TenantServiceImpl<S>
where
    S: EventLog + TenantReadModel + 'static,
{
    async fn create(
        &self,
        tenant_id: TenantId,
        name: String,
    ) -> Result<TenantRecord, RuntimeError> {
        // Check for existing tenant.
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Use a synthetic ProjectKey scoped to the tenant.
        let project = ProjectKey::new(
            tenant_id.clone(),
            "__system__",
            "__system__",
        );

        let event = make_envelope(RuntimeEvent::TenantCreated(TenantCreated {
            project,
            tenant_id: tenant_id.clone(),
            name,
            created_at: now,
        }));

        self.store.append(&[event]).await?;

        TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("tenant not found after create".into()))
    }

    async fn get(&self, tenant_id: &TenantId) -> Result<Option<TenantRecord>, RuntimeError> {
        Ok(TenantReadModel::get(self.store.as_ref(), tenant_id).await?)
    }

    async fn list(
        &self,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TenantRecord>, RuntimeError> {
        Ok(TenantReadModel::list(self.store.as_ref(), limit, offset).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::InMemoryStore;

    use crate::tenants::TenantService;

    use super::TenantServiceImpl;

    #[tokio::test]
    async fn create_persists_and_returns_tenant() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TenantServiceImpl::new(store.clone());

        let record = svc
            .create(TenantId::new("tenant_acme"), "Acme Corp".to_owned())
            .await
            .unwrap();

        assert_eq!(record.tenant_id, TenantId::new("tenant_acme"));
        assert_eq!(record.name, "Acme Corp");
    }

    #[tokio::test]
    async fn create_duplicate_returns_conflict() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TenantServiceImpl::new(store.clone());

        svc.create(TenantId::new("tenant_acme"), "Acme Corp".to_owned())
            .await
            .unwrap();

        let result = svc
            .create(TenantId::new("tenant_acme"), "Acme Corp 2".to_owned())
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_returns_created_tenant() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TenantServiceImpl::new(store);

        svc.create(TenantId::new("tenant_1"), "Tenant One".to_owned())
            .await
            .unwrap();

        let found = svc.get(&TenantId::new("tenant_1")).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Tenant One");
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TenantServiceImpl::new(store);

        let result = svc.get(&TenantId::new("missing")).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_returns_all_tenants() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TenantServiceImpl::new(store);

        svc.create(TenantId::new("t1"), "One".to_owned())
            .await
            .unwrap();
        svc.create(TenantId::new("t2"), "Two".to_owned())
            .await
            .unwrap();

        let results = svc.list(10, 0).await.unwrap();
        assert_eq!(results.len(), 2);
    }
}
