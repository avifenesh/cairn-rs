use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{
    ApprovalReadModel, QuotaReadModel, RunReadModel, RunRecord, SessionReadModel,
};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use super::quota_impl::enforce_run_quota;
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

impl<S> RunServiceImpl<S>
where
    S: EventLog + RunReadModel + SessionReadModel + QuotaReadModel + 'static,
{
    pub async fn start(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        self.start_internal(project, session_id, run_id, parent_run_id, None)
            .await
    }

    pub async fn start_command(&self, command: StartRun) -> Result<RunRecord, RuntimeError> {
        self.start_internal(
            &command.project,
            &command.session_id,
            command.run_id,
            command.parent_run_id,
            None,
        )
        .await
    }

    async fn start_internal(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
        correlation_id: Option<&str>,
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

        // Enforce run quota before creating the run.
        enforce_run_quota(self.store.as_ref(), &project.tenant_id).await?;

        let mut event = make_envelope(RuntimeEvent::RunCreated(RunCreated {
            project: project.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            parent_run_id,
            prompt_release_id: None,
            agent_role_id: None,
        }));
        if let Some(correlation_id) = correlation_id {
            event = event.with_correlation_id(correlation_id);
        }

        self.store.append(&[event]).await?;
        self.get_run(&run_id).await
    }

    pub async fn start_with_correlation(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
        correlation_id: impl AsRef<str>,
    ) -> Result<RunRecord, RuntimeError> {
        self.start_internal(
            project,
            session_id,
            run_id,
            parent_run_id,
            Some(correlation_id.as_ref()),
        )
        .await
    }
}

#[async_trait]
impl<S> RunService for RunServiceImpl<S>
where
    S: EventLog + RunReadModel + SessionReadModel + QuotaReadModel + ApprovalReadModel + 'static,
{
    async fn start(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        self.start_internal(project, session_id, run_id, parent_run_id, None)
            .await
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
        let run = self
            .transition_run(run_id, RunState::Completed, None)
            .await?;
        derive_and_update_session(self.store.as_ref(), &run.session_id).await?;
        Ok(run)
    }

    async fn fail(
        &self,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError> {
        let run = self
            .transition_run(run_id, RunState::Failed, Some(failure_class))
            .await?;
        derive_and_update_session(self.store.as_ref(), &run.session_id).await?;
        Ok(run)
    }

    async fn cancel(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        let run = self
            .transition_run(run_id, RunState::Canceled, None)
            .await?;
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
        // Block resume while the run has pending (unresolved) approvals.
        if ApprovalReadModel::has_pending_for_run(self.store.as_ref(), run_id).await? {
            return Err(RuntimeError::PolicyDenied {
                reason: format!("run {} has pending approvals", run_id.as_str()),
            });
        }

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

    async fn enter_waiting_approval(&self, run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        self.transition_run(run_id, RunState::WaitingApproval, None)
            .await
    }

    async fn resolve_approval(
        &self,
        run_id: &RunId,
        decision: ApprovalDecision,
    ) -> Result<RunRecord, RuntimeError> {
        match decision {
            ApprovalDecision::Approved => {
                let run = self.transition_run(run_id, RunState::Running, None).await?;
                derive_and_update_session(self.store.as_ref(), &run.session_id).await?;
                Ok(run)
            }
            ApprovalDecision::Rejected => {
                let run = self
                    .transition_run(
                        run_id,
                        RunState::Failed,
                        Some(FailureClass::ApprovalRejected),
                    )
                    .await?;
                derive_and_update_session(self.store.as_ref(), &run.session_id).await?;
                Ok(run)
            }
        }
    }

    async fn start_command(
        &self,
        command: cairn_domain::commands::StartRun,
    ) -> Result<RunRecord, RuntimeError> {
        // Override the trait default: route through the internal helper that
        // supports correlation ids, so consistency with start_with_correlation
        // is preserved.
        self.start_internal(
            &command.project,
            &command.session_id,
            command.run_id,
            command.parent_run_id,
            None,
        )
        .await
    }

    async fn start_with_correlation(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
        correlation_id: &str,
    ) -> Result<RunRecord, RuntimeError> {
        self.start_internal(
            project,
            session_id,
            run_id,
            parent_run_id,
            Some(correlation_id),
        )
        .await
    }

    async fn spawn_subagent(
        &self,
        project: &ProjectKey,
        parent_run_id: RunId,
        session_id: &SessionId,
        child_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        let child_run_id = child_run_id
            .unwrap_or_else(|| RunId::new(format!("subagent_{}", parent_run_id.as_str())));
        let event = super::event_helpers::make_envelope(cairn_domain::RuntimeEvent::RunCreated(
            cairn_domain::RunCreated {
                project: project.clone(),
                session_id: session_id.clone(),
                run_id: child_run_id.clone(),
                parent_run_id: Some(parent_run_id),
                prompt_release_id: None,
                agent_role_id: None,
            },
        ));
        self.store.append(&[event]).await?;
        cairn_store::projections::RunReadModel::get(self.store.as_ref(), &child_run_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("subagent run not found after create".into()))
    }

    async fn list_child_runs(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError> {
        // Scan event log for RunCreated events that reference this parent_run_id,
        // then fetch the current record for each child run found.
        let events = cairn_store::EventLog::read_stream(self.store.as_ref(), None, 10_000).await?;
        let child_run_ids: Vec<RunId> = events
            .into_iter()
            .filter_map(|stored| {
                if let cairn_domain::RuntimeEvent::RunCreated(e) = stored.envelope.payload {
                    if e.parent_run_id.as_ref() == Some(parent_run_id) {
                        return Some(e.run_id);
                    }
                }
                None
            })
            .take(limit)
            .collect();

        let mut records = Vec::new();
        for run_id in child_run_ids {
            if let Some(record) =
                cairn_store::projections::RunReadModel::get(self.store.as_ref(), &run_id).await?
            {
                records.push(record);
            }
        }
        Ok(records)
    }
}

// ── Additional helpers for cairn-app ────────────────────────────────────────

impl<S> RunServiceImpl<S>
where
    S: cairn_store::EventLog
        + cairn_store::projections::RunReadModel
        + cairn_store::projections::SessionReadModel
        + 'static,
{
    /// Set a checkpoint strategy for a run.
    ///
    /// Emits a `CheckpointStrategySet` event. The `strategy` string describes
    /// the trigger kind: "periodic", "on_tool_call", or "manual".
    ///
    /// Strategies containing "auto" or "on_task_complete" enable
    /// `trigger_on_task_complete`, which creates a checkpoint automatically
    /// when any task in the run completes.
    pub async fn set_checkpoint_strategy(
        &self,
        run_id: &cairn_domain::RunId,
        strategy: String,
    ) -> Result<(), crate::error::RuntimeError> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let trigger_on_task_complete =
            strategy.contains("auto") || strategy.contains("on_task_complete");

        let event =
            super::event_helpers::make_envelope(cairn_domain::RuntimeEvent::CheckpointStrategySet(
                cairn_domain::CheckpointStrategySet {
                    strategy_id: format!("strat_{}_{}", run_id.as_str(), now_ms),
                    description: strategy,
                    set_at_ms: now_ms,
                    run_id: Some(run_id.clone()),
                    interval_ms: 0,
                    max_checkpoints: 0,
                    trigger_on_task_complete,
                },
            ));

        self.store.append(&[event]).await?;
        Ok(())
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
            .start(
                &project(),
                &SessionId::new("sess_1"),
                RunId::new("run_1"),
                None,
            )
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
            .start(
                &project(),
                &SessionId::new("sess_2"),
                RunId::new("run_2"),
                None,
            )
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
            .start(
                &project(),
                &SessionId::new("sess_3"),
                RunId::new("run_a"),
                None,
            )
            .await
            .unwrap();
        run_svc
            .start(
                &project(),
                &SessionId::new("sess_3"),
                RunId::new("run_b"),
                None,
            )
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
            .start(
                &project(),
                &SessionId::new("sess_4"),
                RunId::new("dup"),
                None,
            )
            .await
            .unwrap();

        let result = run_svc
            .start(
                &project(),
                &SessionId::new("sess_4"),
                RunId::new("dup"),
                None,
            )
            .await;

        assert!(result.is_err());
    }
}
