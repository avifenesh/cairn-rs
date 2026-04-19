use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use cairn_domain::events::EventEnvelope;
use cairn_domain::events::{
    EventSource, RunCreated, RunStateChanged, RuntimeEvent, SessionCreated, SessionStateChanged,
    StateTransition, TaskCreated, TaskLeaseClaimed, TaskStateChanged,
};
use cairn_domain::ids::{EventId, RunId, SessionId, TaskId};
use cairn_domain::lifecycle::{FailureClass, RunState, SessionState, TaskState};
use cairn_domain::tenancy::ProjectKey;
use cairn_store::event_log::EventLog;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub enum BridgeEvent {
    ExecutionCreated {
        run_id: RunId,
        session_id: SessionId,
        project: ProjectKey,
        /// Parent run id for subagent / child runs. `None` for top-level.
        /// FF's exec_core already carries this in the `cairn.parent_run_id`
        /// tag, but the bridge must thread it through to `RunCreated` so the
        /// cairn-store projection can reconstruct the run tree.
        parent_run_id: Option<RunId>,
        /// External correlation id (sqeq ingress etc.). Tagged onto the
        /// resulting `EventEnvelope.correlation_id` so audit / SSE
        /// downstreams can join back to the originating request. `None`
        /// for internal starts.
        correlation_id: Option<String>,
    },
    ExecutionCompleted {
        run_id: RunId,
        project: ProjectKey,
        prev_state: Option<RunState>,
    },
    ExecutionFailed {
        run_id: RunId,
        project: ProjectKey,
        failure_class: FailureClass,
        prev_state: Option<RunState>,
    },
    ExecutionCancelled {
        run_id: RunId,
        project: ProjectKey,
        prev_state: Option<RunState>,
    },
    ExecutionSuspended {
        run_id: RunId,
        project: ProjectKey,
        prev_state: Option<RunState>,
        /// Post-suspension state from the service's `read_run_record`
        /// (already adjusted for FF blocking_reason). Callers emit the
        /// observed state so projection/SSE don't drift from HGETALL.
        /// Approval-gated suspensions become `WaitingApproval`; plain
        /// operator pauses stay `Paused`.
        to: RunState,
    },
    ExecutionResumed {
        run_id: RunId,
        project: ProjectKey,
        prev_state: Option<RunState>,
    },
    TaskCreated {
        task_id: TaskId,
        project: ProjectKey,
        /// RFC-011 Phase 2: the session this task is scoped to. Carried
        /// through the bridge so every downstream consumer (projection,
        /// FF tag write, audit) sees the same session binding that the
        /// ExecutionId was minted against via
        /// `id_map::session_task_to_execution_id`. `None` for bare
        /// (session-less) submissions.
        session_id: Option<SessionId>,
        parent_run_id: Option<RunId>,
        parent_task_id: Option<TaskId>,
    },
    TaskLeaseClaimed {
        task_id: TaskId,
        project: ProjectKey,
        lease_owner: String,
        lease_epoch: u64,
        lease_expires_at_ms: u64,
    },
    TaskStateChanged {
        task_id: TaskId,
        project: ProjectKey,
        to: TaskState,
        failure_class: Option<FailureClass>,
    },
    ExecutionRetryScheduled {
        run_id: RunId,
        project: ProjectKey,
        /// FF attempt counter. Carried so downstream observability can
        /// show retry progress; not persisted in RunStateChanged today.
        attempt: u32,
        /// Previous public_state before retry was scheduled. FF can retry
        /// from `Running`, `Suspended` (waitpoint expiry), or `Delayed`
        /// (chained retries) — hardcoding `Running` falsifies history in
        /// the projection.
        prev_state: Option<RunState>,
    },
    SessionCreated {
        session_id: SessionId,
        project: ProjectKey,
    },
    SessionArchived {
        session_id: SessionId,
        project: ProjectKey,
    },
}

pub struct EventBridge {
    tx: mpsc::Sender<BridgeEvent>,
    cancel: CancellationToken,
    append_failures: Arc<AtomicU64>,
    /// Counts events dropped because the consumer channel was closed
    /// (i.e. the bridge background task exited before the producer).
    /// Distinct from `append_failures`, which counts events that reached
    /// the consumer but failed to persist to the event log.
    emit_failures: Arc<AtomicU64>,
}

const MAX_RETRY_ATTEMPTS: u32 = 3;
const RETRY_BACKOFF_MS: u64 = 100;

impl EventBridge {
    pub fn start(event_log: Arc<dyn EventLog + Send + Sync>) -> (Self, JoinHandle<()>) {
        let (tx, rx) = mpsc::channel::<BridgeEvent>(1024);
        let cancel = CancellationToken::new();
        let append_failures = Arc::new(AtomicU64::new(0));

        let handle = tokio::spawn(Self::run_consumer(
            rx,
            event_log,
            cancel.clone(),
            append_failures.clone(),
        ));

        let bridge = Self {
            tx,
            cancel,
            append_failures,
            emit_failures: Arc::new(AtomicU64::new(0)),
        };
        (bridge, handle)
    }

    /// Number of events dropped because the consumer channel was closed.
    /// Exposed so tests and operator metrics can observe bridge-side loss.
    pub fn emit_failures(&self) -> u64 {
        self.emit_failures.load(Ordering::Relaxed)
    }

    async fn run_consumer(
        mut rx: mpsc::Receiver<BridgeEvent>,
        event_log: Arc<dyn EventLog + Send + Sync>,
        cancel: CancellationToken,
        append_failures: Arc<AtomicU64>,
    ) {
        loop {
            let event = tokio::select! {
                biased;
                ev = rx.recv() => match ev {
                    Some(e) => e,
                    None => break,
                },
                () = cancel.cancelled() => {
                    break;
                }
            };
            Self::append_with_retry(&event_log, &event, &append_failures).await;
        }

        // Drain remaining events after stop signal.
        rx.close();
        while let Some(event) = rx.recv().await {
            Self::append_with_retry(&event_log, &event, &append_failures).await;
        }
    }

    async fn append_with_retry(
        event_log: &Arc<dyn EventLog + Send + Sync>,
        event: &BridgeEvent,
        append_failures: &AtomicU64,
    ) {
        let runtime_event = bridge_event_to_runtime_event(event);
        let mut envelope = EventEnvelope::for_runtime_event(
            EventId::new(uuid::Uuid::new_v4().to_string()),
            EventSource::Runtime,
            runtime_event,
        );
        if let Some(corr) = bridge_event_correlation_id(event) {
            envelope = envelope.with_correlation_id(corr);
        }
        let event_type = bridge_event_type_name(event);

        for attempt in 0..MAX_RETRY_ATTEMPTS {
            match event_log.append(std::slice::from_ref(&envelope)).await {
                Ok(_) => return,
                Err(e) => {
                    if attempt + 1 < MAX_RETRY_ATTEMPTS {
                        tracing::warn!(
                            attempt = attempt + 1,
                            event_type,
                            error = %e,
                            "event bridge: append failed, retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(
                            RETRY_BACKOFF_MS * (1 << attempt) as u64,
                        ))
                        .await;
                    } else {
                        append_failures.fetch_add(1, Ordering::Relaxed);
                        tracing::error!(
                            event_type,
                            error = %e,
                            total_failures = append_failures.load(Ordering::Relaxed),
                            "event bridge: append failed after {MAX_RETRY_ATTEMPTS} attempts"
                        );
                    }
                }
            }
        }
    }

    pub async fn emit(&self, event: BridgeEvent) {
        let event_type = bridge_event_type_name(&event);
        if let Err(e) = self.tx.send(event).await {
            // `fetch_add` returns the previous value; add 1 for the
            // post-increment count without a separate (race-prone) load.
            let total = self.emit_failures.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::error!(
                event_type,
                error = %e,
                total_emit_failures = total,
                "event bridge: channel closed — event dropped, projection will have a gap"
            );
        }
    }

    pub fn stop(&self) {
        self.cancel.cancel();
    }

    pub fn append_failure_count(&self) -> u64 {
        self.append_failures.load(Ordering::Relaxed)
    }
}

fn bridge_event_type_name(event: &BridgeEvent) -> &'static str {
    match event {
        BridgeEvent::ExecutionCreated { .. } => "ExecutionCreated",
        BridgeEvent::ExecutionCompleted { .. } => "ExecutionCompleted",
        BridgeEvent::ExecutionFailed { .. } => "ExecutionFailed",
        BridgeEvent::ExecutionCancelled { .. } => "ExecutionCancelled",
        BridgeEvent::ExecutionSuspended { .. } => "ExecutionSuspended",
        BridgeEvent::ExecutionResumed { .. } => "ExecutionResumed",
        BridgeEvent::TaskCreated { .. } => "TaskCreated",
        BridgeEvent::TaskLeaseClaimed { .. } => "TaskLeaseClaimed",
        BridgeEvent::TaskStateChanged { .. } => "TaskStateChanged",
        BridgeEvent::ExecutionRetryScheduled { .. } => "ExecutionRetryScheduled",
        BridgeEvent::SessionCreated { .. } => "SessionCreated",
        BridgeEvent::SessionArchived { .. } => "SessionArchived",
    }
}

/// External correlation id carried by the bridge event, if any. Only
/// `ExecutionCreated` carries one today — sqeq ingress threads a
/// request-level correlation through `start_with_correlation`.
fn bridge_event_correlation_id(event: &BridgeEvent) -> Option<&str> {
    match event {
        BridgeEvent::ExecutionCreated {
            correlation_id: Some(c),
            ..
        } => Some(c.as_str()),
        _ => None,
    }
}

fn bridge_event_to_runtime_event(event: &BridgeEvent) -> RuntimeEvent {
    match event {
        BridgeEvent::ExecutionCreated {
            run_id,
            session_id,
            project,
            parent_run_id,
            correlation_id: _,
        } => RuntimeEvent::RunCreated(RunCreated {
            project: project.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            parent_run_id: parent_run_id.clone(),
            prompt_release_id: None,
            agent_role_id: None,
        }),
        BridgeEvent::ExecutionCompleted {
            run_id,
            project,
            prev_state,
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: *prev_state,
                to: RunState::Completed,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
        BridgeEvent::ExecutionFailed {
            run_id,
            project,
            failure_class,
            prev_state,
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: *prev_state,
                to: RunState::Failed,
            },
            failure_class: Some(*failure_class),
            pause_reason: None,
            resume_trigger: None,
        }),
        BridgeEvent::ExecutionCancelled {
            run_id,
            project,
            prev_state,
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: *prev_state,
                to: RunState::Canceled,
            },
            failure_class: Some(FailureClass::CanceledByOperator),
            pause_reason: None,
            resume_trigger: None,
        }),
        BridgeEvent::ExecutionSuspended {
            run_id,
            project,
            prev_state,
            to,
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: *prev_state,
                to: *to,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
        BridgeEvent::ExecutionResumed {
            run_id,
            project,
            prev_state,
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: *prev_state,
                to: RunState::Running,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
        BridgeEvent::TaskCreated {
            task_id,
            project,
            session_id: _,
            parent_run_id,
            parent_task_id,
        } => RuntimeEvent::TaskCreated(TaskCreated {
            project: project.clone(),
            task_id: task_id.clone(),
            parent_run_id: parent_run_id.clone(),
            parent_task_id: parent_task_id.clone(),
            prompt_release_id: None,
        }),
        BridgeEvent::TaskLeaseClaimed {
            task_id,
            project,
            lease_owner,
            lease_epoch,
            lease_expires_at_ms,
        } => RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
            project: project.clone(),
            task_id: task_id.clone(),
            lease_owner: lease_owner.clone(),
            lease_token: *lease_epoch,
            lease_expires_at_ms: *lease_expires_at_ms,
        }),
        BridgeEvent::TaskStateChanged {
            task_id,
            project,
            to,
            failure_class,
        } => RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: project.clone(),
            task_id: task_id.clone(),
            transition: StateTransition {
                from: None,
                to: *to,
            },
            failure_class: *failure_class,
            pause_reason: None,
            resume_trigger: None,
        }),
        BridgeEvent::ExecutionRetryScheduled {
            run_id,
            project,
            prev_state,
            attempt: _,
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: *prev_state,
                to: RunState::Pending,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
        BridgeEvent::SessionCreated {
            session_id,
            project,
        } => RuntimeEvent::SessionCreated(SessionCreated {
            project: project.clone(),
            session_id: session_id.clone(),
        }),
        BridgeEvent::SessionArchived {
            session_id,
            project,
        } => RuntimeEvent::SessionStateChanged(SessionStateChanged {
            project: project.clone(),
            session_id: session_id.clone(),
            transition: StateTransition {
                from: Some(SessionState::Open),
                to: SessionState::Archived,
            },
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_event_to_runtime_created() {
        let event = BridgeEvent::ExecutionCreated {
            run_id: RunId::new("run_1"),
            session_id: SessionId::new("sess_1"),
            project: ProjectKey::new("t", "w", "p"),
            parent_run_id: None,
            correlation_id: None,
        };
        let runtime = bridge_event_to_runtime_event(&event);
        assert!(matches!(runtime, RuntimeEvent::RunCreated(_)));
    }

    // T4-C2 regression: parent_run_id threads through to the RunCreated
    // projection so subagent run trees don't orphan.
    #[test]
    fn bridge_event_to_runtime_created_propagates_parent_run_id() {
        let event = BridgeEvent::ExecutionCreated {
            run_id: RunId::new("child_run"),
            session_id: SessionId::new("sess_1"),
            project: ProjectKey::new("t", "w", "p"),
            parent_run_id: Some(RunId::new("parent_run")),
            correlation_id: None,
        };
        match bridge_event_to_runtime_event(&event) {
            RuntimeEvent::RunCreated(rc) => {
                assert_eq!(rc.parent_run_id, Some(RunId::new("parent_run")));
            }
            _ => panic!("expected RunCreated"),
        }
    }

    #[test]
    fn bridge_event_correlation_id_extracts_execution_created() {
        let with_corr = BridgeEvent::ExecutionCreated {
            run_id: RunId::new("run_1"),
            session_id: SessionId::new("sess_1"),
            project: ProjectKey::new("t", "w", "p"),
            parent_run_id: None,
            correlation_id: Some("corr_xyz".to_owned()),
        };
        assert_eq!(bridge_event_correlation_id(&with_corr), Some("corr_xyz"));

        let without_corr = BridgeEvent::ExecutionCreated {
            run_id: RunId::new("run_1"),
            session_id: SessionId::new("sess_1"),
            project: ProjectKey::new("t", "w", "p"),
            parent_run_id: None,
            correlation_id: None,
        };
        assert_eq!(bridge_event_correlation_id(&without_corr), None);

        // Other variants never carry a correlation today.
        let other = BridgeEvent::SessionCreated {
            session_id: SessionId::new("sess_1"),
            project: ProjectKey::new("t", "w", "p"),
        };
        assert_eq!(bridge_event_correlation_id(&other), None);
    }

    #[test]
    fn bridge_event_to_runtime_completed() {
        let event = BridgeEvent::ExecutionCompleted {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: Some(RunState::Running),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert_eq!(rsc.transition.from, Some(RunState::Running));
                assert_eq!(rsc.transition.to, RunState::Completed);
                assert!(rsc.failure_class.is_none());
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    #[test]
    fn bridge_event_to_runtime_completed_from_waiting() {
        let event = BridgeEvent::ExecutionCompleted {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: Some(RunState::WaitingDependency),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert_eq!(rsc.transition.from, Some(RunState::WaitingDependency));
                assert_eq!(rsc.transition.to, RunState::Completed);
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    #[test]
    fn bridge_event_to_runtime_failed() {
        let event = BridgeEvent::ExecutionFailed {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            failure_class: FailureClass::TimedOut,
            prev_state: Some(RunState::Running),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert_eq!(rsc.transition.from, Some(RunState::Running));
                assert_eq!(rsc.transition.to, RunState::Failed);
                assert_eq!(rsc.failure_class, Some(FailureClass::TimedOut));
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    #[test]
    fn bridge_event_to_runtime_cancelled() {
        let event = BridgeEvent::ExecutionCancelled {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: Some(RunState::Running),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert_eq!(rsc.transition.from, Some(RunState::Running));
                assert_eq!(rsc.transition.to, RunState::Canceled);
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    #[test]
    fn bridge_event_to_runtime_suspended_pauses_by_default() {
        let event = BridgeEvent::ExecutionSuspended {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: Some(RunState::Running),
            to: RunState::Paused,
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert_eq!(rsc.transition.from, Some(RunState::Running));
                assert_eq!(rsc.transition.to, RunState::Paused);
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    // T4-C1 regression: suspended-for-approval must land as WaitingApproval
    // in the projection, not Paused.
    #[test]
    fn bridge_event_to_runtime_suspended_for_approval() {
        let event = BridgeEvent::ExecutionSuspended {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: Some(RunState::Running),
            to: RunState::WaitingApproval,
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert_eq!(rsc.transition.from, Some(RunState::Running));
                assert_eq!(rsc.transition.to, RunState::WaitingApproval);
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    #[test]
    fn bridge_event_to_runtime_resumed() {
        let event = BridgeEvent::ExecutionResumed {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: Some(RunState::Paused),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert_eq!(rsc.transition.from, Some(RunState::Paused));
                assert_eq!(rsc.transition.to, RunState::Running);
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    #[test]
    fn bridge_event_prev_state_none_produces_none_from() {
        let event = BridgeEvent::ExecutionCompleted {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: None,
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::RunStateChanged(rsc) => {
                assert!(rsc.transition.from.is_none());
                assert_eq!(rsc.transition.to, RunState::Completed);
            }
            _ => panic!("expected RunStateChanged"),
        }
    }

    #[test]
    fn bridge_event_task_created() {
        let event = BridgeEvent::TaskCreated {
            task_id: TaskId::new("task_1"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: Some(SessionId::new("sess_1")),
            parent_run_id: Some(RunId::new("run_1")),
            parent_task_id: None,
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::TaskCreated(tc) => {
                assert_eq!(tc.task_id.as_str(), "task_1");
                assert_eq!(tc.project.tenant_id.as_str(), "t");
                assert_eq!(tc.parent_run_id.as_ref().unwrap().as_str(), "run_1");
                assert!(tc.parent_task_id.is_none());
                assert!(tc.prompt_release_id.is_none());
            }
            _ => panic!("expected TaskCreated"),
        }
    }

    #[test]
    fn bridge_event_task_created_with_parent_task() {
        let event = BridgeEvent::TaskCreated {
            task_id: TaskId::new("task_child"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: None,
            parent_run_id: Some(RunId::new("run_1")),
            parent_task_id: Some(TaskId::new("task_parent")),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::TaskCreated(tc) => {
                assert_eq!(tc.task_id.as_str(), "task_child");
                assert_eq!(tc.parent_task_id.as_ref().unwrap().as_str(), "task_parent");
            }
            _ => panic!("expected TaskCreated"),
        }
    }

    #[test]
    fn bridge_event_task_state_changed_completed() {
        let event = BridgeEvent::TaskStateChanged {
            task_id: TaskId::new("task_3"),
            project: ProjectKey::new("t", "w", "p"),
            to: TaskState::Completed,
            failure_class: None,
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::TaskStateChanged(tsc) => {
                assert_eq!(tsc.task_id.as_str(), "task_3");
                assert_eq!(tsc.transition.to, TaskState::Completed);
                assert!(tsc.transition.from.is_none());
                assert!(tsc.failure_class.is_none());
            }
            _ => panic!("expected TaskStateChanged"),
        }
    }

    #[test]
    fn bridge_event_task_state_changed_failed_preserves_class() {
        let event = BridgeEvent::TaskStateChanged {
            task_id: TaskId::new("task_4"),
            project: ProjectKey::new("t", "w", "p"),
            to: TaskState::Failed,
            failure_class: Some(FailureClass::TimedOut),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::TaskStateChanged(tsc) => {
                assert_eq!(tsc.transition.to, TaskState::Failed);
                assert_eq!(tsc.failure_class, Some(FailureClass::TimedOut));
            }
            _ => panic!("expected TaskStateChanged"),
        }
    }

    #[test]
    fn bridge_task_lease_claimed_uses_epoch_as_token() {
        let event = BridgeEvent::TaskLeaseClaimed {
            task_id: TaskId::new("task_1"),
            project: ProjectKey::new("t", "w", "p"),
            lease_owner: "worker_a".to_owned(),
            lease_epoch: 7,
            lease_expires_at_ms: 99_000,
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::TaskLeaseClaimed(tlc) => {
                assert_eq!(tlc.task_id.as_str(), "task_1");
                assert_eq!(tlc.lease_owner, "worker_a");
                assert_eq!(tlc.lease_token, 7);
                assert_eq!(tlc.lease_expires_at_ms, 99_000);
            }
            _ => panic!("expected TaskLeaseClaimed"),
        }
    }

    #[test]
    fn bridge_session_archived_emits_session_state_changed() {
        let event = BridgeEvent::SessionArchived {
            session_id: SessionId::new("sess_1"),
            project: ProjectKey::new("t", "w", "p"),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::SessionStateChanged(ssc) => {
                assert_eq!(ssc.session_id.as_str(), "sess_1");
                assert_eq!(ssc.transition.from, Some(SessionState::Open));
                assert_eq!(ssc.transition.to, SessionState::Archived);
            }
            _ => panic!("expected SessionStateChanged, got {runtime:?}"),
        }
    }

    #[test]
    fn bridge_session_created_emits_session_created_envelope() {
        // SessionCreated must map to a SessionCreated envelope — handlers
        // that read a session by id before starting a run depend on this.
        let event = BridgeEvent::SessionCreated {
            session_id: SessionId::new("sess_brand_new"),
            project: ProjectKey::new("tenant_x", "workspace_y", "project_z"),
        };
        let runtime = bridge_event_to_runtime_event(&event);
        match runtime {
            RuntimeEvent::SessionCreated(sc) => {
                assert_eq!(sc.session_id.as_str(), "sess_brand_new");
                assert_eq!(sc.project.tenant_id.as_str(), "tenant_x");
                assert_eq!(sc.project.workspace_id.as_str(), "workspace_y");
                assert_eq!(sc.project.project_id.as_str(), "project_z");
            }
            _ => panic!("expected SessionCreated, got {runtime:?}"),
        }
    }

    #[tokio::test]
    async fn bridge_session_created_round_trips_through_event_log() {
        // End-to-end: start an EventBridge, emit SessionCreated, let the
        // consumer drain, verify SessionReadModel sees the projection.
        // Mirrors the existing TaskCreated / ExecutionCreated round-trip
        // tests above — proves the new variant reaches cairn-store so
        // FabricSessionServiceAdapter::get succeeds on the next request.
        use cairn_store::projections::SessionReadModel;
        use cairn_store::InMemoryStore;

        let store = Arc::new(InMemoryStore::new());
        let event_log: Arc<dyn EventLog + Send + Sync> = store.clone();
        let (bridge, handle) = EventBridge::start(event_log);

        let session_id = SessionId::new("sess_rt_1");
        let project = ProjectKey::new("t_rt", "w_rt", "p_rt");

        bridge
            .emit(BridgeEvent::SessionCreated {
                session_id: session_id.clone(),
                project: project.clone(),
            })
            .await;

        // Stop the bridge and wait for the consumer to drain. `stop`
        // cancels the loop AFTER processing everything already in the
        // channel (the loop uses `biased` select so recv drains before
        // the cancel arm fires).
        bridge.stop();
        let _ = handle.await;

        let record = SessionReadModel::get(store.as_ref(), &session_id)
            .await
            .expect("projection read must not error")
            .expect("SessionCreated must populate SessionReadModel");
        assert_eq!(record.session_id, session_id);
        assert_eq!(record.project, project);
    }
}
