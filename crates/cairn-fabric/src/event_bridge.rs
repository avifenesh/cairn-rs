use std::sync::Arc;

use cairn_domain::events::EventEnvelope;
use cairn_domain::events::{
    EventSource, RunCreated, RunStateChanged, RuntimeEvent, StateTransition, TaskCreated,
    TaskLeaseClaimed, TaskStateChanged,
};
use cairn_domain::ids::{EventId, RunId, SessionId, TaskId};
use cairn_domain::lifecycle::{FailureClass, RunState, TaskState};
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
            lease_expires_at_ms,
        } => RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
            project: project.clone(),
            task_id: task_id.clone(),
            lease_owner: lease_owner.clone(),
            lease_token: 1,
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
        } => RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project.clone(),
            run_id: RunId::new(format!("session:{}", session_id.as_str())),
            transition: StateTransition {
                from: Some(RunState::Running),
                to: RunState::Canceled,
            },
            failure_class: Some(FailureClass::CanceledByOperator),
            pause_reason: None,
            resume_trigger: None,
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
}
