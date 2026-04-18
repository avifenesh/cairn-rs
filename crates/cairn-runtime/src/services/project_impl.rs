//! Concrete project service implementation.
//!
//! Manages project lifecycle by emitting `ProjectCreated` events
//! and reading back via the `ProjectReadModel` projection.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::ProjectReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::projects::ProjectService;

pub struct ProjectServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ProjectServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> ProjectService for ProjectServiceImpl<S>
where
    S: EventLog + ProjectReadModel + 'static,
{
    async fn create(
        &self,
        project: ProjectKey,
        name: String,
    ) -> Result<ProjectRecord, RuntimeError> {
        // Check for existing project.
        if ProjectReadModel::get_project(self.store.as_ref(), &project)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "project",
                id: project.project_id.to_string(),
            });
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = make_envelope(RuntimeEvent::ProjectCreated(ProjectCreated {
            project: project.clone(),
            name,
            created_at: now,
        }));

        self.store.append(&[event]).await?;

        ProjectReadModel::get_project(self.store.as_ref(), &project)
            .await?
            .ok_or_else(|| RuntimeError::Internal("project not found after create".into()))
    }

    async fn get(&self, project: &ProjectKey) -> Result<Option<ProjectRecord>, RuntimeError> {
        Ok(ProjectReadModel::get_project(self.store.as_ref(), project).await?)
    }

    async fn list_by_workspace(
        &self,
        tenant_id: &TenantId,
        workspace_id: &WorkspaceId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ProjectRecord>, RuntimeError> {
        Ok(self
            .store
            .list_by_workspace(tenant_id, workspace_id, limit, offset)
            .await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::InMemoryStore;

    use crate::projects::ProjectService;

    use super::ProjectServiceImpl;

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant_acme", "ws_main", "project_alpha")
    }

    #[tokio::test]
    async fn create_persists_and_returns_project() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProjectServiceImpl::new(store.clone());
        let project = test_project();

        let record = svc
            .create(project.clone(), "Alpha Project".to_owned())
            .await
            .unwrap();

        assert_eq!(record.project_id, project.project_id);
        assert_eq!(record.workspace_id, project.workspace_id);
        assert_eq!(record.tenant_id, project.tenant_id);
        assert_eq!(record.name, "Alpha Project");
    }

    #[tokio::test]
    async fn create_duplicate_returns_conflict() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProjectServiceImpl::new(store.clone());
        let project = test_project();

        svc.create(project.clone(), "Alpha".to_owned())
            .await
            .unwrap();

        let result = svc.create(project, "Alpha 2".to_owned()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_returns_created_project() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProjectServiceImpl::new(store);
        let project = test_project();

        svc.create(project.clone(), "Alpha".to_owned())
            .await
            .unwrap();

        let found = svc.get(&project).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().name, "Alpha");
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProjectServiceImpl::new(store);

        let result = svc.get(&test_project()).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_by_workspace_filters_correctly() {
        let store = Arc::new(InMemoryStore::new());
        let svc = ProjectServiceImpl::new(store);

        svc.create(ProjectKey::new("t1", "ws1", "p_a"), "A".to_owned())
            .await
            .unwrap();

        svc.create(ProjectKey::new("t1", "ws1", "p_b"), "B".to_owned())
            .await
            .unwrap();

        svc.create(ProjectKey::new("t1", "ws2", "p_c"), "C".to_owned())
            .await
            .unwrap();

        let results = svc
            .list_by_workspace(&TenantId::new("t1"), &WorkspaceId::new("ws1"), 10, 0)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        let other_results = svc
            .list_by_workspace(&TenantId::new("t1"), &WorkspaceId::new("ws2"), 10, 0)
            .await
            .unwrap();
        assert_eq!(other_results.len(), 1);
    }
}
