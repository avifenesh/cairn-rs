use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::*;
use cairn_store::projections::{CheckpointReadModel, RunReadModel, TaskReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::recovery::{RecoveryAction, RecoveryService, RecoverySummary};

pub struct RecoveryServiceImpl<S> {
    store: Arc<S>,
}

impl<S> RecoveryServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> RecoveryService for RecoveryServiceImpl<S>
where
    S: EventLog + TaskReadModel + RunReadModel + CheckpointReadModel + 'static,
{
    async fn recover_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<RecoverySummary, RuntimeError> {
        let expired = self.store.list_expired_leases(now, limit).await?;
        let scanned = expired.len();
        let mut actions = Vec::new();

        for task in &expired {
            // Already failed once with LeaseExpired — dead-letter it.
            if task.failure_class == Some(FailureClass::LeaseExpired) {
                let events = vec![
                    make_envelope(RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                        project: task.project.clone(),
                        run_id: None,
                        task_id: Some(task.task_id.clone()),
                        reason: "lease expired (repeat) — dead-lettering".to_owned(),
                    })),
                    make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                        project: task.project.clone(),
                        task_id: task.task_id.clone(),
                        transition: StateTransition {
                            from: Some(task.state),
                            to: TaskState::DeadLettered,
                        },
                        failure_class: Some(FailureClass::LeaseExpired),
                        pause_reason: None,
                        resume_trigger: None,
                    })),
                    make_envelope(RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                        project: task.project.clone(),
                        run_id: None,
                        task_id: Some(task.task_id.clone()),
                        recovered: false,
                    })),
                ];
                self.store.append(&events).await?;
                actions.push(RecoveryAction::TaskDeadLettered {
                    task_id: task.task_id.clone(),
                });
                continue;
            }

            // First lease expiry: mark retryable, then immediately requeue
            // so the task returns to the scheduling pool.
            let events = vec![
                make_envelope(RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                    project: task.project.clone(),
                    run_id: None,
                    task_id: Some(task.task_id.clone()),
                    reason: "lease expired (no heartbeat) — requeueing".to_owned(),
                })),
                // Step 1: running/leased → retryable_failed (records the failure)
                make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: task.project.clone(),
                    task_id: task.task_id.clone(),
                    transition: StateTransition {
                        from: Some(task.state),
                        to: TaskState::RetryableFailed,
                    },
                    failure_class: Some(FailureClass::LeaseExpired),
                    pause_reason: None,
                    resume_trigger: None,
                })),
                // Step 2: retryable_failed → queued (completes the recovery cycle)
                make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: task.project.clone(),
                    task_id: task.task_id.clone(),
                    transition: StateTransition {
                        from: Some(TaskState::RetryableFailed),
                        to: TaskState::Queued,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                })),
                make_envelope(RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                    project: task.project.clone(),
                    run_id: None,
                    task_id: Some(task.task_id.clone()),
                    recovered: true,
                })),
            ];
            self.store.append(&events).await?;
            actions.push(RecoveryAction::TaskRequeued {
                task_id: task.task_id.clone(),
            });
        }

        Ok(RecoverySummary { actions, scanned })
    }

    async fn recover_interrupted_runs(
        &self,
        limit: usize,
    ) -> Result<RecoverySummary, RuntimeError> {
        // Find runs stuck in Running state. If they have a latest checkpoint,
        // attempt resume from checkpoint. Otherwise, fail them.
        let running = cairn_store::projections::RunReadModel::list_by_state(
            self.store.as_ref(),
            RunState::Running,
            limit,
        )
        .await?;
        let scanned = running.len();
        let mut actions = Vec::new();

        for run in &running {
            let has_checkpoint = cairn_store::projections::CheckpointReadModel::latest_for_run(
                self.store.as_ref(),
                &run.run_id,
            )
            .await?
            .is_some();

            let events = vec![make_envelope(RuntimeEvent::RecoveryAttempted(
                RecoveryAttempted {
                    project: run.project.clone(),
                    run_id: Some(run.run_id.clone()),
                    task_id: None,
                    reason: if has_checkpoint {
                        "interrupted run with checkpoint".to_owned()
                    } else {
                        "interrupted run without checkpoint".to_owned()
                    },
                },
            ))];
            self.store.append(&events).await?;

            if has_checkpoint {
                actions.push(RecoveryAction::RunResumedFromCheckpoint {
                    run_id: run.run_id.clone(),
                });
            } else {
                // Fail runs without checkpoints
                let fail_events = vec![
                    make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                        project: run.project.clone(),
                        run_id: run.run_id.clone(),
                        transition: StateTransition {
                            from: Some(RunState::Running),
                            to: RunState::Failed,
                        },
                        failure_class: Some(FailureClass::ExecutionError),
                        pause_reason: None,
                        resume_trigger: None,
                    })),
                    make_envelope(RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                        project: run.project.clone(),
                        run_id: Some(run.run_id.clone()),
                        task_id: None,
                        recovered: false,
                    })),
                ];
                self.store.append(&fail_events).await?;
                actions.push(RecoveryAction::RunFailed {
                    run_id: run.run_id.clone(),
                });
            }
        }

        Ok(RecoverySummary { actions, scanned })
    }

    async fn resolve_stale_dependencies(
        &self,
        limit: usize,
    ) -> Result<RecoverySummary, RuntimeError> {
        // Find runs stuck in WaitingDependency, then check if all their
        // child tasks are terminal using any_non_terminal_children.
        let waiting = cairn_store::projections::RunReadModel::list_by_state(
            self.store.as_ref(),
            RunState::WaitingDependency,
            limit,
        )
        .await?;
        let scanned = waiting.len();
        let mut actions = Vec::new();

        for run in &waiting {
            let has_active_children =
                cairn_store::projections::TaskReadModel::any_non_terminal_children(
                    self.store.as_ref(),
                    &run.run_id,
                )
                .await?;

            if has_active_children {
                // Dependency still active — skip this run
                continue;
            }

            // All children terminal (or none exist) — resume the parent
            let events = vec![
                make_envelope(RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                    project: run.project.clone(),
                    run_id: Some(run.run_id.clone()),
                    task_id: None,
                    reason: "all child tasks terminal, resuming parent".to_owned(),
                })),
                make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: run.project.clone(),
                    run_id: run.run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::WaitingDependency),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                })),
                make_envelope(RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                    project: run.project.clone(),
                    run_id: Some(run.run_id.clone()),
                    task_id: None,
                    recovered: true,
                })),
            ];

            self.store.append(&events).await?;
            actions.push(RecoveryAction::DependencyResolved {
                run_id: run.run_id.clone(),
            });
        }

        Ok(RecoverySummary { actions, scanned })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::*;
    use cairn_store::projections::TaskReadModel;
    use cairn_store::InMemoryStore;

    use super::RecoveryServiceImpl;
    use crate::recovery::{RecoveryAction, RecoveryService};
    use crate::services::TaskServiceImpl;
    use crate::tasks::TaskService;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[tokio::test]
    async fn recovery_sweep_requeues_expired_task() {
        let store = Arc::new(InMemoryStore::new());
        let task_svc = TaskServiceImpl::new(store.clone());
        let recovery_svc = RecoveryServiceImpl::new(store.clone());

        // Create and claim a task with a very short lease
        let task_id = TaskId::new("expired_task");
        task_svc
            .submit(&project(), task_id.clone(), None, None, 0)
            .await
            .unwrap();
        task_svc
            .claim(&task_id, "worker".into(), 1) // 1ms lease — expires immediately
            .await
            .unwrap();

        // Advance "now" past the lease
        let far_future = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 120_000;

        let summary = recovery_svc
            .recover_expired_leases(far_future, 100)
            .await
            .unwrap();

        assert_eq!(summary.scanned, 1);
        assert!(matches!(
            &summary.actions[0],
            RecoveryAction::TaskRequeued { task_id: id } if id.as_str() == "expired_task"
        ));

        // Task should be back in Queued with lease cleared
        let task = TaskReadModel::get(store.as_ref(), &task_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(task.state, TaskState::Queued);
        assert!(task.lease_owner.is_none());
        assert!(task.lease_expires_at.is_none());
    }
}
