//! Concrete workspace service implementation.
//!
//! Manages workspace lifecycle by emitting `WorkspaceCreated` events
//! and reading back via the `WorkspaceReadModel` projection.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::WorkspaceReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::workspaces::WorkspaceService;

pub struct WorkspaceServiceImpl<S> {
    store: Arc<S>,
}

impl<S> WorkspaceServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> WorkspaceService for WorkspaceServiceImpl<S>
where
    S: EventLog + WorkspaceReadModel + 'static,
{
    async fn create(
        &self,
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
        name: String,
    ) -> Result<WorkspaceRecord, RuntimeError> {
        // Check for existing workspace.
        if WorkspaceReadModel::get(self.store.as_ref(), &workspace_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "workspace",
                id: workspace_id.to_string(),
            });
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        // Use a synthetic ProjectKey scoped to the tenant/workspace.
        let project = ProjectKey::new(tenant_id.clone(), workspace_id.clone(), "__system__");

        let event = make_envelope(RuntimeEvent::WorkspaceCreated(WorkspaceCreated {
            project,
            workspace_id: workspace_id.clone(),
            tenant_id,
            name,
            created_at: now,
        }));

        self.store.append(&[event]).await?;

        WorkspaceReadModel::get(self.store.as_ref(), &workspace_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("workspace not found after create".into()))
    }

    async fn get(
        &self,
        workspace_id: &WorkspaceId,
    ) -> Result<Option<WorkspaceRecord>, RuntimeError> {
        Ok(WorkspaceReadModel::get(self.store.as_ref(), workspace_id).await?)
    }

    async fn list_by_tenant(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<WorkspaceRecord>, RuntimeError> {
        Ok(self.store.list_by_tenant(tenant_id, limit, offset).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::InMemoryStore;

    use crate::workspaces::WorkspaceService;

    use super::WorkspaceServiceImpl;

    #[tokio::test]
    async fn create_persists_and_returns_workspace() {
        let store = Arc::new(InMemoryStore::new());
        let svc = WorkspaceServiceImpl::new(store.clone());

        let record = svc
            .create(
                TenantId::new("tenant_acme"),
                WorkspaceId::new("ws_main"),
                "Main Workspace".to_owned(),
            )
            .await
            .unwrap();

        assert_eq!(record.workspace_id, WorkspaceId::new("ws_main"));
        assert_eq!(record.tenant_id, TenantId::new("tenant_acme"));
        assert_eq!(record.name, "Main Workspace");
    }

    #[tokio::test]
    async fn create_duplicate_returns_conflict() {
        let store = Arc::new(InMemoryStore::new());
        let svc = WorkspaceServiceImpl::new(store.clone());

        svc.create(
            TenantId::new("tenant_acme"),
            WorkspaceId::new("ws_main"),
            "Main".to_owned(),
        )
        .await
        .unwrap();

        let result = svc
            .create(
                TenantId::new("tenant_acme"),
                WorkspaceId::new("ws_main"),
                "Main 2".to_owned(),
            )
            .await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_returns_created_workspace() {
        let store = Arc::new(InMemoryStore::new());
        let svc = WorkspaceServiceImpl::new(store);

        svc.create(
            TenantId::new("t1"),
            WorkspaceId::new("ws_1"),
            "WS One".to_owned(),
        )
        .await
        .unwrap();

        let found = svc.get(&WorkspaceId::new("ws_1")).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "WS One");
    }

    #[tokio::test]
    async fn list_by_tenant_filters_correctly() {
        let store = Arc::new(InMemoryStore::new());
        let svc = WorkspaceServiceImpl::new(store);

        svc.create(
            TenantId::new("t1"),
            WorkspaceId::new("ws_a"),
            "A".to_owned(),
        )
        .await
        .unwrap();

        svc.create(
            TenantId::new("t1"),
            WorkspaceId::new("ws_b"),
            "B".to_owned(),
        )
        .await
        .unwrap();

        svc.create(
            TenantId::new("t2"),
            WorkspaceId::new("ws_c"),
            "C".to_owned(),
        )
        .await
        .unwrap();

        let results = svc
            .list_by_tenant(&TenantId::new("t1"), 10, 0)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        let other_results = svc
            .list_by_tenant(&TenantId::new("t2"), 10, 0)
            .await
            .unwrap();
        assert_eq!(other_results.len(), 1);
    }
}
