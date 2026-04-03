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
            // Retryable: requeue. Terminal: fail.
            let (target_state, action) = if task.failure_class == Some(FailureClass::LeaseExpired) {
                // Already failed once with LeaseExpired — dead-letter it
                (
                    TaskState::DeadLettered,
                    RecoveryAction::TaskDeadLettered {
                        task_id: task.task_id.clone(),
                    },
                )
            } else {
                // First lease expiry — mark retryable
                (
                    TaskState::RetryableFailed,
                    RecoveryAction::TaskRequeued {
                        task_id: task.task_id.clone(),
                    },
                )
            };

            let events = vec![
                make_envelope(RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                    project: task.project.clone(),
                    run_id: None,
                    task_id: Some(task.task_id.clone()),
                    reason: "lease expired".to_owned(),
                })),
                make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: task.project.clone(),
                    task_id: task.task_id.clone(),
                    transition: StateTransition {
                        from: Some(task.state),
                        to: target_state,
                    },
                    failure_class: Some(FailureClass::LeaseExpired),
                })),
                make_envelope(RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                    project: task.project.clone(),
                    run_id: None,
                    task_id: Some(task.task_id.clone()),
                    recovered: true,
                })),
            ];

            self.store.append(&events).await?;
            actions.push(action);
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
