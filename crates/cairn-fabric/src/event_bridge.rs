use std::sync::Arc;

use cairn_domain::events::EventEnvelope;
use cairn_domain::events::{
    EventSource, RunCreated, RunStateChanged, RuntimeEvent, SessionStateChanged, StateTransition,
    TaskCreated, TaskLeaseClaimed, TaskStateChanged,
};
use cairn_domain::ids::{EventId, RunId, SessionId, TaskId};
use cairn_domain::lifecycle::{FailureClass, RunState, SessionState, TaskState};
use cairn_domain::tenancy::ProjectKey;
use cairn_store::event_log::EventLog;
use tokio::sync::mpsc;

#[derive(Clone, Debug)]
pub enum BridgeEvent {
    ExecutionCreated {
        run_id: RunId,
        session_id: SessionId,
        project: ProjectKey,
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
    },
    ExecutionResumed {
        run_id: RunId,
        project: ProjectKey,
        prev_state: Option<RunState>,
    },
    TaskCreated {
        task_id: TaskId,
        project: ProjectKey,
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
    SessionArchived {
        session_id: SessionId,
        project: ProjectKey,
    },
}

pub struct EventBridge {
    tx: mpsc::Sender<BridgeEvent>,
}

impl EventBridge {
    // TODO: batch consumer appends for throughput — currently one EventLog::append
    // per event (~200-1000 events/sec with Postgres round-trips). Accumulate for
    // 10-50ms or batch_size=64, then append as a single &[EventEnvelope] call.
    pub fn new(event_log: Arc<dyn EventLog>) -> Self {
        let (tx, mut rx) = mpsc::channel::<BridgeEvent>(256);

        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                let runtime_event = bridge_event_to_runtime_event(&event);
                let envelope = EventEnvelope::for_runtime_event(
                    EventId::new(uuid::Uuid::new_v4().to_string()),
                    EventSource::Runtime,
                    runtime_event,
                );
                if let Err(e) = event_log.append(&[envelope]).await {
                    tracing::error!(error = %e, "event bridge: failed to append event");
                }
            }
        });

        Self { tx }
    }

    pub fn emit(&self, event: BridgeEvent) {
        let event_type = bridge_event_type_name(&event);
        if let Err(e) = self.tx.try_send(event) {
            tracing::warn!(error = %e, event_type, "event bridge: dropping event");
        }
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
        BridgeEvent::SessionArchived { .. } => "SessionArchived",
    }
}

fn bridge_event_to_runtime_event(event: &BridgeEvent) -> RuntimeEvent {
    match event {
        BridgeEvent::ExecutionCreated {
            run_id,
            session_id,
            project,
        } => RuntimeEvent::RunCreated(RunCreated {
            project: project.clone(),
            session_id: session_id.clone(),
            run_id: run_id.clone(),
            parent_run_id: None,
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
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: run_id.clone(),
            transition: StateTransition {
                from: *prev_state,
                to: RunState::Paused,
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
        };
        let runtime = bridge_event_to_runtime_event(&event);
        assert!(matches!(runtime, RuntimeEvent::RunCreated(_)));
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
    fn bridge_event_to_runtime_suspended() {
        let event = BridgeEvent::ExecutionSuspended {
            run_id: RunId::new("run_1"),
            project: ProjectKey::new("t", "w", "p"),
            prev_state: Some(RunState::Running),
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
}
