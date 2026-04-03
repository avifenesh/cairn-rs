use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{RunReadModel, RunRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::runs::RunService;

pub struct RunServiceImpl<S> {
    store: Arc<S>,
}

impl<S> RunServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

impl<S: EventLog + RunReadModel + 'static> RunServiceImpl<S> {
    async fn get_run(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        RunReadModel::get(self.store.as_ref(), run_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "run",
                id: run_id.to_string(),
            })
    }

    async fn transition_run(
        &self,
        run_id: &RunId,
        to: RunState,
        failure_class: Option<FailureClass>,
    ) -> Result<RunRecord, RuntimeError> {
        let run = self.get_run(run_id).await?;

        if !can_transition_run_state(run.state, to) {
            return Err(RuntimeError::InvalidTransition {
                entity: "run",
                from: format!("{:?}", run.state),
                to: format!("{to:?}"),
            });
        }

        let event = make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
            project: run.project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: Some(run.state),
                to,
            },
            failure_class,
        }));

        self.store.append(&[event]).await?;
        self.get_run(run_id).await
    }
}

#[async_trait]
impl<S> RunService for RunServiceImpl<S>
where
    S: EventLog + RunReadModel + 'static,
{
    async fn start(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::RunCreated(RunCreated {
            project: project.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            parent_run_id,
        }));

        self.store.append(&[event]).await?;
        self.get_run(&run_id).await
    }

    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, RuntimeError> {
        Ok(RunReadModel::get(self.store.as_ref(), run_id).await?)
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError> {
        Ok(self
            .store
            .list_by_session(session_id, limit, offset)
            .await?)
    }

    async fn complete(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        self.transition_run(run_id, RunState::Completed, None).await
    }

    async fn fail(
        &self,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError> {
        self.transition_run(run_id, RunState::Failed, Some(failure_class))
            .await
    }

    async fn cancel(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        self.transition_run(run_id, RunState::Canceled, None).await
    }

    async fn pause(&self, run_id: &RunId, _reason: PauseReason) -> Result<RunRecord, RuntimeError> {
        self.transition_run(run_id, RunState::Paused, None).await
    }

    async fn resume(
        &self,
        run_id: &RunId,
        _trigger: ResumeTrigger,
        target: RunResumeTarget,
    ) -> Result<RunRecord, RuntimeError> {
        let to = match target {
            RunResumeTarget::Pending => RunState::Pending,
            RunResumeTarget::Running => RunState::Running,
        };
        self.transition_run(run_id, to, None).await
    }
}
