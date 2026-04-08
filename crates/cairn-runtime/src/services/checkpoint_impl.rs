use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{
    CheckpointReadModel, CheckpointRecord, CheckpointStrategyReadModel,
};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::checkpoints::CheckpointService;
use crate::error::RuntimeError;

pub struct CheckpointServiceImpl<S> {
    store: Arc<S>,
}

impl<S> CheckpointServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> CheckpointService for CheckpointServiceImpl<S>
where
    S: EventLog + CheckpointReadModel + CheckpointStrategyReadModel + 'static,
{
    async fn save(
        &self,
        project: &ProjectKey,
        run_id: &RunId,
        checkpoint_id: CheckpointId,
    ) -> Result<CheckpointRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
            project: project.clone(),
            run_id: run_id.clone(),
            checkpoint_id: checkpoint_id.clone(),
            disposition: CheckpointDisposition::Latest,
            data: None,
        }));

        self.store.append(&[event]).await?;

        CheckpointReadModel::get(self.store.as_ref(), &checkpoint_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("checkpoint not found after save".into()))
    }

    async fn get(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointRecord>, RuntimeError> {
        Ok(CheckpointReadModel::get(self.store.as_ref(), checkpoint_id).await?)
    }

    async fn latest_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<CheckpointRecord>, RuntimeError> {
        Ok(self.store.latest_for_run(run_id).await?)
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, RuntimeError> {
        Ok(self.store.list_by_run(run_id, limit).await?)
    }

    async fn set_strategy(
        &self,
        run_id: &RunId,
        strategy_id: String,
        description: String,
        interval_ms: u64,
        max_checkpoints: u32,
        trigger_on_task_complete: bool,
    ) -> Result<CheckpointStrategy, RuntimeError> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let event = make_envelope(RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
            strategy_id: strategy_id.clone(),
            description: description.clone(),
            set_at_ms: now_ms,
            run_id: Some(run_id.clone()),
            interval_ms,
            max_checkpoints,
            trigger_on_task_complete,
        }));

        self.store.append(&[event]).await?;

        // Return the strategy from the projection (validates round-trip).
        CheckpointStrategyReadModel::get_by_run(self.store.as_ref(), run_id)
            .await
            .map_err(RuntimeError::Store)?
            .ok_or_else(|| RuntimeError::Internal("checkpoint strategy not found after set".into()))
    }

    async fn get_strategy(
        &self,
        run_id: &RunId,
    ) -> Result<Option<CheckpointStrategy>, RuntimeError> {
        CheckpointStrategyReadModel::get_by_run(self.store.as_ref(), run_id)
            .await
            .map_err(RuntimeError::Store)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::projections::CheckpointReadModel;
    use cairn_store::InMemoryStore;

    use crate::checkpoints::CheckpointService;

    use super::CheckpointServiceImpl;

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant", "ws", "proj")
    }

    #[tokio::test]
    async fn save_creates_latest_checkpoint() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store.clone());
        let project = test_project();
        let run_id = RunId::new("run_1");

        let cp = svc
            .save(&project, &run_id, CheckpointId::new("cp_1"))
            .await
            .unwrap();

        assert_eq!(cp.checkpoint_id, CheckpointId::new("cp_1"));
        assert_eq!(cp.disposition, CheckpointDisposition::Latest);
        assert_eq!(cp.run_id, run_id);
    }

    #[tokio::test]
    async fn second_save_supersedes_first() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store.clone());
        let project = test_project();
        let run_id = RunId::new("run_1");

        svc.save(&project, &run_id, CheckpointId::new("cp_1"))
            .await
            .unwrap();
        svc.save(&project, &run_id, CheckpointId::new("cp_2"))
            .await
            .unwrap();

        // First checkpoint should now be Superseded.
        let cp1 = svc.get(&CheckpointId::new("cp_1")).await.unwrap().unwrap();
        assert_eq!(
            cp1.disposition,
            CheckpointDisposition::Superseded,
            "first checkpoint must be superseded when second is saved"
        );

        // Second checkpoint should be Latest.
        let cp2 = svc.get(&CheckpointId::new("cp_2")).await.unwrap().unwrap();
        assert_eq!(cp2.disposition, CheckpointDisposition::Latest);

        // latest_for_run should return the second.
        let latest = svc.latest_for_run(&run_id).await.unwrap().unwrap();
        assert_eq!(latest.checkpoint_id, CheckpointId::new("cp_2"));
    }

    #[tokio::test]
    async fn list_by_run_returns_all_checkpoints() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store.clone());
        let project = test_project();
        let run_id = RunId::new("run_1");

        svc.save(&project, &run_id, CheckpointId::new("cp_1"))
            .await
            .unwrap();
        svc.save(&project, &run_id, CheckpointId::new("cp_2"))
            .await
            .unwrap();
        svc.save(&project, &run_id, CheckpointId::new("cp_3"))
            .await
            .unwrap();

        let all = svc.list_by_run(&run_id, 10).await.unwrap();
        assert_eq!(all.len(), 3);

        // Only the last should be Latest.
        let latest_count = all
            .iter()
            .filter(|cp| cp.disposition == CheckpointDisposition::Latest)
            .count();
        assert_eq!(latest_count, 1, "exactly one checkpoint should be Latest");
    }

    #[tokio::test]
    async fn get_nonexistent_returns_none() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store);

        let result = svc.get(&CheckpointId::new("missing")).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn latest_for_run_without_checkpoints_returns_none() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store);

        let result = svc.latest_for_run(&RunId::new("no_run")).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn different_runs_have_independent_checkpoints() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store.clone());
        let project = test_project();

        svc.save(&project, &RunId::new("run_a"), CheckpointId::new("cp_a"))
            .await
            .unwrap();
        svc.save(&project, &RunId::new("run_b"), CheckpointId::new("cp_b"))
            .await
            .unwrap();

        // Both should be Latest (different runs).
        let cp_a = svc.get(&CheckpointId::new("cp_a")).await.unwrap().unwrap();
        let cp_b = svc.get(&CheckpointId::new("cp_b")).await.unwrap().unwrap();
        assert_eq!(cp_a.disposition, CheckpointDisposition::Latest);
        assert_eq!(cp_b.disposition, CheckpointDisposition::Latest);
    }

    #[tokio::test]
    async fn set_strategy_records_and_retrieves() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store.clone());
        let run_id = RunId::new("run_strat");

        let strat = svc
            .set_strategy(&run_id, "s1".into(), "periodic".into(), 30_000, 5, true)
            .await
            .unwrap();

        assert_eq!(strat.strategy_id, "s1");
        assert_eq!(strat.interval_ms, 30_000);
        assert_eq!(strat.max_checkpoints, 5);
        assert!(strat.trigger_on_task_complete);

        // get_strategy round-trips
        let got = svc.get_strategy(&run_id).await.unwrap().unwrap();
        assert_eq!(got.strategy_id, "s1");
    }

    #[tokio::test]
    async fn get_strategy_returns_none_without_set() {
        let store = Arc::new(InMemoryStore::new());
        let svc = CheckpointServiceImpl::new(store);

        assert!(svc
            .get_strategy(&RunId::new("no_such"))
            .await
            .unwrap()
            .is_none());
    }
}
