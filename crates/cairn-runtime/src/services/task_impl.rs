//! `TaskServiceImpl` — dev/CI path backing for [`crate::tasks::TaskService`].
//! See the module comment on `run_impl.rs` for the dev-vs-production
//! selection mechanism.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{CheckpointStrategyReadModel, TaskReadModel, TaskRecord};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::tasks::TaskService;

/// In-memory dev-path implementation of [`crate::tasks::TaskService`].
pub struct TaskServiceImpl<S> {
    store: Arc<S>,
}

impl<S> TaskServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

impl<S: EventLog + TaskReadModel + 'static> TaskServiceImpl<S> {
    async fn get_task(&self, task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        TaskReadModel::get(self.store.as_ref(), task_id)
            .await?
            .ok_or_else(|| RuntimeError::NotFound {
                entity: "task",
                id: task_id.to_string(),
            })
    }

    async fn transition_task(
        &self,
        task_id: &TaskId,
        to: TaskState,
        failure_class: Option<FailureClass>,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if !can_transition_task_state(task.state, to) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: format!("{to:?}"),
            });
        }

        let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: task.project.clone(),
            task_id: task_id.clone(),
            transition: StateTransition {
                from: Some(task.state),
                to,
            },
            failure_class,
            pause_reason: None,
            resume_trigger: None,
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }
}

#[async_trait]
impl<S> TaskService for TaskServiceImpl<S>
where
    S: EventLog + TaskReadModel + CheckpointStrategyReadModel + 'static,
{
    async fn submit(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: TaskId,
        parent_run_id: Option<RunId>,
        parent_task_id: Option<TaskId>,
        _priority: u32,
    ) -> Result<TaskRecord, RuntimeError> {
        let event = make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
            project: project.clone(),
            task_id: task_id.clone(),
            parent_run_id,
            parent_task_id,
            prompt_release_id: None,
            session_id: session_id.cloned(),
        }));

        self.store.append(&[event]).await?;
        self.get_task(&task_id).await
    }

    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, RuntimeError> {
        Ok(TaskReadModel::get(self.store.as_ref(), task_id).await?)
    }

    async fn claim(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if !can_transition_task_state(task.state, TaskState::Leased) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: "Leased".into(),
            });
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let events = vec![
            make_envelope(RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                project: task.project.clone(),
                task_id: task_id.clone(),
                lease_owner,
                lease_token: task.version + 1,
                lease_expires_at_ms: now_ms + lease_duration_ms,
            })),
            make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: task.project.clone(),
                task_id: task_id.clone(),
                transition: StateTransition {
                    from: Some(TaskState::Queued),
                    to: TaskState::Leased,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            })),
        ];

        self.store.append(&events).await?;
        self.get_task(task_id).await
    }

    async fn heartbeat(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if task.state != TaskState::Leased && task.state != TaskState::Running {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: "heartbeat".into(),
            });
        }

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let event = make_envelope(RuntimeEvent::TaskLeaseHeartbeated(TaskLeaseHeartbeated {
            project: task.project.clone(),
            task_id: task_id.clone(),
            lease_token: task.version,
            lease_expires_at_ms: now_ms + lease_extension_ms,
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }

    async fn start(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        self.transition_task(task_id, TaskState::Running, None)
            .await
    }

    async fn complete(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        let result = self
            .transition_task(task_id, TaskState::Completed, None)
            .await?;
        // Task dependencies are FF-authoritative; there is nothing for
        // the in-memory runtime to resolve on completion.

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Auto-checkpoint: if the parent run has a strategy with
        // trigger_on_task_complete, emit a CheckpointRecorded event.
        if let Some(ref run_id) = result.parent_run_id {
            if let Ok(Some(strategy)) =
                CheckpointStrategyReadModel::get_by_run(self.store.as_ref(), run_id).await
            {
                if strategy.trigger_on_task_complete {
                    let cp_id = CheckpointId::new(format!(
                        "cp_auto_{}_{}_{now_ms}",
                        run_id.as_str(),
                        task_id.as_str()
                    ));
                    let event =
                        make_envelope(RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
                            project: result.project.clone(),
                            run_id: run_id.clone(),
                            checkpoint_id: cp_id,
                            disposition: CheckpointDisposition::Latest,
                            data: None,
                        }));
                    // Propagate — if the operator asked for auto-checkpoint
                    // on task complete, a silent drop is worse than a
                    // visible failure. Pre-T3-H1 the append error was
                    // `let _`-discarded.
                    self.store.append(&[event]).await?;
                }
            }
        }

        Ok(result)
    }

    async fn declare_dependency(
        &self,
        _dependent_task_id: &TaskId,
        _prerequisite_task_id: &TaskId,
    ) -> Result<TaskDependencyRecord, RuntimeError> {
        // `in-memory-runtime` is a dev/CI-only path (CLAUDE.md). Task
        // dependencies are FF-authoritative under the default Fabric
        // backend: `ff_stage_dependency_edge` on the flow partition
        // plus `ff_apply_dependency_to_child` on the child's execution
        // partition. Reconstructing that edge state in-memory would
        // duplicate FF's invariants with guaranteed drift, so this
        // path is intentionally absent.
        Err(RuntimeError::Validation {
            reason: "task dependencies require the Fabric backend; \
                     in-memory-runtime is dev-only and does not model \
                     FF flow edges"
                .to_owned(),
        })
    }

    async fn check_dependencies(
        &self,
        _task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, RuntimeError> {
        // See declare_dependency — the in-memory runtime is
        // intentionally dep-less. Returning an empty list (rather
        // than an error) lets callers that only want "is anything
        // blocking me?" short-circuit without noise, since the answer
        // under this backend is trivially "no, nothing can be
        // declared".
        Ok(Vec::new())
    }

    async fn fail(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, RuntimeError> {
        // Choose retryable vs terminal based on failure class
        let target = match failure_class {
            FailureClass::LeaseExpired | FailureClass::DependencyFailed => {
                TaskState::RetryableFailed
            }
            _ => TaskState::Failed,
        };
        self.transition_task(task_id, target, Some(failure_class))
            .await
    }

    async fn cancel(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        self.transition_task(task_id, TaskState::Canceled, None)
            .await
    }

    async fn dead_letter(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        self.transition_task(task_id, TaskState::DeadLettered, None)
            .await
    }

    /// RFC 005: return all dead-lettered tasks for a project (the dead-letter queue).
    async fn list_dead_lettered(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(self
            .store
            .list_by_state(project, TaskState::DeadLettered, limit)
            .await?
            .into_iter()
            .skip(offset)
            .collect())
    }

    async fn pause(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
        reason: PauseReason,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;

        if !can_transition_task_state(task.state, TaskState::Paused) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: format!("{:?}", TaskState::Paused),
            });
        }

        let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: task.project.clone(),
            task_id: task_id.clone(),
            transition: StateTransition {
                from: Some(task.state),
                to: TaskState::Paused,
            },
            failure_class: None,
            pause_reason: Some(reason),
            resume_trigger: None,
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }

    async fn resume(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
        trigger: ResumeTrigger,
        target: TaskResumeTarget,
    ) -> Result<TaskRecord, RuntimeError> {
        let task = self.get_task(task_id).await?;
        let to = match target {
            TaskResumeTarget::Queued => TaskState::Queued,
            TaskResumeTarget::Running => TaskState::Running,
        };

        if !can_transition_task_state(task.state, to) {
            return Err(RuntimeError::InvalidTransition {
                entity: "task",
                from: format!("{:?}", task.state),
                to: format!("{to:?}"),
            });
        }

        let event = make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: task.project.clone(),
            task_id: task_id.clone(),
            transition: StateTransition {
                from: Some(task.state),
                to,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: Some(trigger),
        }));

        self.store.append(&[event]).await?;
        self.get_task(task_id).await
    }

    async fn list_by_state(
        &self,
        project: &ProjectKey,
        state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(self.store.list_by_state(project, state, limit).await?)
    }

    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(self.store.list_expired_leases(now, limit).await?)
    }

    async fn release_lease(
        &self,
        _session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        self.transition_task(task_id, TaskState::Queued, None).await
    }

    /// Spawn a child task linked to a parent run/task and emit `SubagentSpawned`.
    ///
    /// Creates the child task via `TaskCreated`, then emits `SubagentSpawned`
    /// so the projection links parent_run_id / parent_task_id on the child.
    async fn spawn_subagent(
        &self,
        project: &ProjectKey,
        parent_run_id: RunId,
        parent_task_id: Option<TaskId>,
        child_task_id: TaskId,
        child_session_id: SessionId,
        child_run_id: Option<RunId>,
    ) -> Result<TaskRecord, RuntimeError> {
        let events = vec![
            make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: child_task_id.clone(),
                parent_run_id: Some(parent_run_id.clone()),
                parent_task_id: parent_task_id.clone(),
                prompt_release_id: None,
                // A subagent task belongs to the child session, not the
                // parent's — persist that binding up front so the resolver
                // returns the child session without walking parent_run_id.
                session_id: Some(child_session_id.clone()),
            })),
            make_envelope(RuntimeEvent::SubagentSpawned(SubagentSpawned {
                project: project.clone(),
                parent_run_id,
                parent_task_id,
                child_task_id: child_task_id.clone(),
                child_session_id,
                child_run_id,
            })),
        ];

        self.store.append(&events).await?;
        self.get_task(&child_task_id).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;

    use cairn_store::InMemoryStore;

    use super::TaskServiceImpl;
    use crate::tasks::TaskService;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[tokio::test]
    async fn pause_clears_lease_timer() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TaskServiceImpl::new(store.clone());

        let task_id = TaskId::new("task_pause");
        svc.submit(&project(), None, task_id.clone(), None, None, 0)
            .await
            .unwrap();

        // Claim a lease
        let claimed = svc
            .claim(None, &task_id, "worker-1".into(), 60_000)
            .await
            .unwrap();
        assert!(claimed.lease_owner.is_some());
        assert!(claimed.lease_expires_at.is_some());

        // Start running
        svc.start(None, &task_id).await.unwrap();

        // Pause — should clear lease fields
        let paused = svc
            .pause(
                None,
                &task_id,
                PauseReason {
                    kind: PauseReasonKind::OperatorPause,
                    detail: None,
                    resume_after_ms: None,
                    actor: None,
                },
            )
            .await
            .unwrap();

        assert_eq!(paused.state, TaskState::Paused);
        assert!(paused.lease_owner.is_none(), "pause must clear lease_owner");
        assert!(
            paused.lease_expires_at.is_none(),
            "pause must clear lease_expires_at"
        );
    }

    #[tokio::test]
    async fn resume_from_pause_to_queued() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TaskServiceImpl::new(store.clone());

        let task_id = TaskId::new("task_resume");
        svc.submit(&project(), None, task_id.clone(), None, None, 0)
            .await
            .unwrap();

        svc.claim(None, &task_id, "w".into(), 60_000).await.unwrap();
        svc.start(None, &task_id).await.unwrap();
        svc.pause(
            None,
            &task_id,
            PauseReason {
                kind: PauseReasonKind::OperatorPause,
                detail: None,
                resume_after_ms: None,
                actor: None,
            },
        )
        .await
        .unwrap();

        let resumed = svc
            .resume(
                None,
                &task_id,
                ResumeTrigger::OperatorResume,
                TaskResumeTarget::Queued,
            )
            .await
            .unwrap();

        assert_eq!(resumed.state, TaskState::Queued);
        assert!(resumed.resume_trigger.is_some());
    }

    #[tokio::test]
    async fn spawn_subagent_links_child_to_parent() {
        let store = Arc::new(InMemoryStore::new());
        let svc = TaskServiceImpl::new(store.clone());

        let parent_task_id = TaskId::new("parent_task");
        svc.submit(&project(), None, parent_task_id.clone(), None, None, 0)
            .await
            .unwrap();

        let child = svc
            .spawn_subagent(
                &project(),
                RunId::new("parent_run"),
                Some(parent_task_id.clone()),
                TaskId::new("child_task"),
                SessionId::new("child_sess"),
                Some(RunId::new("child_run")),
            )
            .await
            .unwrap();

        assert_eq!(child.task_id, TaskId::new("child_task"));
        assert_eq!(
            child.parent_run_id.as_ref().unwrap(),
            &RunId::new("parent_run")
        );
        assert_eq!(child.parent_task_id.as_ref().unwrap(), &parent_task_id);
    }
}
