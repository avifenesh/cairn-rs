use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{RunReadModel, RunRecord, SessionReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use super::session_impl::derive_and_update_session;
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

impl<S: EventLog + RunReadModel + SessionReadModel + 'static> RunServiceImpl<S> {
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
            pause_reason: None,
            resume_trigger: None,
        }));

        self.store.append(&[event]).await?;
        self.get_run(run_id).await
    }
}

#[async_trait]
impl<S> RunService for RunServiceImpl<S>
where
    S: EventLog + RunReadModel + SessionReadModel + 'static,
{
    async fn start(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        if RunReadModel::get(self.store.as_ref(), &run_id)
            .await?
            .is_some()
        {
            return Err(RuntimeError::Conflict {
                entity: "run",
                id: run_id.to_string(),
            });
        }

        let event = make_envelope(RuntimeEvent::RunCreated(RunCreated {
            project: project.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            parent_run_id,
            prompt_release_id: None,
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
        let run = self.transition_run(run_id, RunState::Completed, None).await?;
        derive_and_update_session(self.store.as_ref(), &run.session_id).await?;
        Ok(run)
    }

    async fn fail(
        &self,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError> {
        let run = self.transition_run(run_id, RunState::Failed, Some(failure_class)).await?;
        derive_and_update_session(self.store.as_ref(), &run.session_id).await?;
        Ok(run)
    }

    async fn cancel(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        let run = self.transition_run(run_id, RunState::Canceled, None).await?;
        derive_and_update_session(self.store.as_ref(), &run.session_id).await?;
        Ok(run)
    }

    async fn pause(&self, run_id: &RunId, reason: PauseReason) -> Result<RunRecord, RuntimeError> {
        let run = self.get_run(run_id).await?;

        if !can_transition_run_state(run.state, RunState::Paused) {
            return Err(RuntimeError::InvalidTransition {
                entity: "run",
                from: format!("{:?}", run.state),
                to: format!("{:?}", RunState::Paused),
            });
        }

        let event = make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
            project: run.project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: Some(run.state),
                to: RunState::Paused,
            },
            failure_class: None,
            pause_reason: Some(reason),
            resume_trigger: None,
        }));

        self.store.append(&[event]).await?;
        self.get_run(run_id).await
    }

    async fn resume(
        &self,
        run_id: &RunId,
        trigger: ResumeTrigger,
        target: RunResumeTarget,
    ) -> Result<RunRecord, RuntimeError> {
        let run = self.get_run(run_id).await?;
        let to = match target {
            RunResumeTarget::Pending => RunState::Pending,
            RunResumeTarget::Running => RunState::Running,
        };

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
            failure_class: None,
            pause_reason: None,
            resume_trigger: Some(trigger),
        }));

        self.store.append(&[event]).await?;
        self.get_run(run_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::projections::SessionReadModel;
    use cairn_store::InMemoryStore;

    use super::super::session_impl::SessionServiceImpl;
    use super::RunServiceImpl;
    use crate::runs::RunService;
    use crate::sessions::SessionService;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[tokio::test]
    async fn session_derives_completed_when_run_completes() {
        let store = Arc::new(InMemoryStore::new());
        let session_svc = SessionServiceImpl::new(store.clone());
        let run_svc = RunServiceImpl::new(store.clone());

        session_svc
            .create(&project(), SessionId::new("sess_1"))
            .await
            .unwrap();

        run_svc
            .start(&project(), &SessionId::new("sess_1"), RunId::new("run_1"), None)
            .await
            .unwrap();

        // Transition through Running before completing.
        run_svc
            .transition_run(&RunId::new("run_1"), RunState::Running, None)
            .await
            .unwrap();
        run_svc.complete(&RunId::new("run_1")).await.unwrap();

        let session = SessionReadModel::get(store.as_ref(), &SessionId::new("sess_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Completed);
    }

    #[tokio::test]
    async fn session_derives_failed_when_run_fails() {
        let store = Arc::new(InMemoryStore::new());
        let session_svc = SessionServiceImpl::new(store.clone());
        let run_svc = RunServiceImpl::new(store.clone());

        session_svc
            .create(&project(), SessionId::new("sess_2"))
            .await
            .unwrap();

        run_svc
            .start(&project(), &SessionId::new("sess_2"), RunId::new("run_2"), None)
            .await
            .unwrap();

        run_svc
            .fail(&RunId::new("run_2"), FailureClass::ExecutionError)
            .await
            .unwrap();

        let session = SessionReadModel::get(store.as_ref(), &SessionId::new("sess_2"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Failed);
    }

    #[tokio::test]
    async fn session_stays_open_with_non_terminal_runs() {
        let store = Arc::new(InMemoryStore::new());
        let session_svc = SessionServiceImpl::new(store.clone());
        let run_svc = RunServiceImpl::new(store.clone());

        session_svc
            .create(&project(), SessionId::new("sess_3"))
            .await
            .unwrap();

        run_svc
            .start(&project(), &SessionId::new("sess_3"), RunId::new("run_a"), None)
            .await
            .unwrap();
        run_svc
            .start(&project(), &SessionId::new("sess_3"), RunId::new("run_b"), None)
            .await
            .unwrap();

        run_svc
            .transition_run(&RunId::new("run_a"), RunState::Running, None)
            .await
            .unwrap();
        run_svc.complete(&RunId::new("run_a")).await.unwrap();

        let session = SessionReadModel::get(store.as_ref(), &SessionId::new("sess_3"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Open);
    }

    #[tokio::test]
    async fn duplicate_run_start_returns_conflict() {
        let store = Arc::new(InMemoryStore::new());
        let run_svc = RunServiceImpl::new(store.clone());
        let session_svc = SessionServiceImpl::new(store.clone());

        session_svc
            .create(&project(), SessionId::new("sess_4"))
            .await
            .unwrap();

        run_svc
            .start(&project(), &SessionId::new("sess_4"), RunId::new("dup"), None)
            .await
            .unwrap();

        let result = run_svc
            .start(&project(), &SessionId::new("sess_4"), RunId::new("dup"), None)
            .await;

        assert!(result.is_err());
    }
}
