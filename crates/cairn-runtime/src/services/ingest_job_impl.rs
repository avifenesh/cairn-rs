//! Concrete ingest job service implementation.
//!
//! Starts and completes memory ingest jobs by appending events
//! and reading back via the IngestJobReadModel projection.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::IngestJobReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::ingest_jobs::IngestJobService;

pub struct IngestJobServiceImpl<S> {
    store: Arc<S>,
}

impl<S> IngestJobServiceImpl<S> {
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
impl<S> IngestJobService for IngestJobServiceImpl<S>
where
    S: EventLog + IngestJobReadModel + 'static,
{
    async fn start(
        &self,
        project: &ProjectKey,
        job_id: IngestJobId,
        source_id: Option<SourceId>,
        document_count: u32,
    ) -> Result<IngestJobRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::IngestJobStarted(IngestJobStarted {
            project: project.clone(),
            job_id: job_id.clone(),
            source_id,
            document_count,
            started_at: now_ms(),
        }));

        self.store.append(&[event]).await?;

        IngestJobReadModel::get(self.store.as_ref(), &job_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("ingest job not found after start".into()))
    }

    async fn complete(
        &self,
        project: &ProjectKey,
        job_id: IngestJobId,
        success: bool,
        error_message: Option<String>,
    ) -> Result<IngestJobRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
            project: project.clone(),
            job_id: job_id.clone(),
            success,
            error_message,
            completed_at: now_ms(),
        }));

        self.store.append(&[event]).await?;

        IngestJobReadModel::get(self.store.as_ref(), &job_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("ingest job not found after complete".into()))
    }

    async fn get(&self, job_id: &IngestJobId) -> Result<Option<IngestJobRecord>, RuntimeError> {
        Ok(IngestJobReadModel::get(self.store.as_ref(), job_id).await?)
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<IngestJobRecord>, RuntimeError> {
        Ok(self.store.list_by_project(project, limit, offset).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::InMemoryStore;

    use crate::ingest_jobs::IngestJobService;

    use super::IngestJobServiceImpl;

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant_acme", "ws_main", "project_alpha")
    }

    #[tokio::test]
    async fn start_persists_and_returns_job() {
        let store = Arc::new(InMemoryStore::new());
        let svc = IngestJobServiceImpl::new(store);
        let project = test_project();

        let record = svc
            .start(
                &project,
                IngestJobId::new("job_1"),
                Some(SourceId::new("src_1")),
                5,
            )
            .await
            .unwrap();

        assert_eq!(record.id, IngestJobId::new("job_1"));
        assert_eq!(record.state, IngestJobState::Processing);
        assert_eq!(record.document_count, 5);
    }

    #[tokio::test]
    async fn complete_success_transitions_state() {
        let store = Arc::new(InMemoryStore::new());
        let svc = IngestJobServiceImpl::new(store);
        let project = test_project();

        svc.start(
            &project,
            IngestJobId::new("job_2"),
            None,
            3,
        )
        .await
        .unwrap();

        let record = svc
            .complete(&project, IngestJobId::new("job_2"), true, None)
            .await
            .unwrap();

        assert_eq!(record.state, IngestJobState::Completed);
        assert!(record.error_message.is_none());
    }

    #[tokio::test]
    async fn complete_failure_records_error() {
        let store = Arc::new(InMemoryStore::new());
        let svc = IngestJobServiceImpl::new(store);
        let project = test_project();

        svc.start(
            &project,
            IngestJobId::new("job_3"),
            None,
            1,
        )
        .await
        .unwrap();

        let record = svc
            .complete(
                &project,
                IngestJobId::new("job_3"),
                false,
                Some("parse failed".to_owned()),
            )
            .await
            .unwrap();

        assert_eq!(record.state, IngestJobState::Failed);
        assert_eq!(record.error_message.as_deref(), Some("parse failed"));
    }

    #[tokio::test]
    async fn get_returns_started_job() {
        let store = Arc::new(InMemoryStore::new());
        let svc = IngestJobServiceImpl::new(store);
        let project = test_project();

        svc.start(
            &project,
            IngestJobId::new("job_4"),
            None,
            2,
        )
        .await
        .unwrap();

        let found = svc.get(&IngestJobId::new("job_4")).await.unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().document_count, 2);
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let store = Arc::new(InMemoryStore::new());
        let svc = IngestJobServiceImpl::new(store);

        let result = svc.get(&IngestJobId::new("job_missing")).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_by_project_returns_matching_jobs() {
        let store = Arc::new(InMemoryStore::new());
        let svc = IngestJobServiceImpl::new(store);
        let project = test_project();
        let other = ProjectKey::new("other", "ws", "proj");

        svc.start(&project, IngestJobId::new("job_a"), None, 1)
            .await
            .unwrap();
        svc.start(&project, IngestJobId::new("job_b"), None, 2)
            .await
            .unwrap();
        svc.start(&other, IngestJobId::new("job_c"), None, 3)
            .await
            .unwrap();

        let results = svc.list_by_project(&project, 10, 0).await.unwrap();
        assert_eq!(results.len(), 2);

        let other_results = svc.list_by_project(&other, 10, 0).await.unwrap();
        assert_eq!(other_results.len(), 1);
    }
}
