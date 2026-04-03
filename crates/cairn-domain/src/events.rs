use crate::errors::RuntimeEntityRef;
use crate::ids::{
    ApprovalId, CheckpointId, EventId, MailboxMessageId, RunId, SessionId, SignalId, TaskId,
    ToolInvocationId,
};
use crate::lifecycle::{CheckpointDisposition, FailureClass, RunState, SessionState, TaskState};
use crate::policy::{ApprovalDecision, ApprovalRequirement, ExecutionClass};
use crate::tenancy::{OwnershipKey, ProjectKey};
use crate::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
use crate::workers::ExternalWorkerReport;
use serde::{Deserialize, Serialize};

/// Shared event envelope for canonical product events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub event_id: EventId,
    pub source: EventSource,
    pub ownership: OwnershipKey,
    pub causation_id: Option<crate::ids::CommandId>,
    pub correlation_id: Option<String>,
    pub payload: T,
}

impl<T> EventEnvelope<T> {
    pub fn new(
        event_id: impl Into<EventId>,
        source: EventSource,
        ownership: impl Into<OwnershipKey>,
        payload: T,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            source,
            ownership: ownership.into(),
            causation_id: None,
            correlation_id: None,
            payload,
        }
    }

    pub fn with_causation_id(mut self, causation_id: impl Into<crate::ids::CommandId>) -> Self {
        self.causation_id = Some(causation_id.into());
        self
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }
}

impl EventEnvelope<RuntimeEvent> {
    pub fn for_runtime_event(
        event_id: impl Into<EventId>,
        source: EventSource,
        payload: RuntimeEvent,
    ) -> Self {
        let ownership = payload.project().clone();
        Self::new(event_id, source, ownership, payload)
    }

    pub fn project(&self) -> &ProjectKey {
        self.payload.project()
    }

    pub fn primary_entity_ref(&self) -> Option<RuntimeEntityRef> {
        self.payload.primary_entity_ref()
    }
}

/// Event source information used by runtime, operators, and workers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source_type", rename_all = "snake_case")]
pub enum EventSource {
    Operator { operator_id: crate::ids::OperatorId },
    Runtime,
    Scheduler,
    ExternalWorker { worker: String },
    System,
}

/// Minimal runtime event set used as the Week 1 shared contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeEvent {
    SessionCreated(SessionCreated),
    SessionStateChanged(SessionStateChanged),
    RunCreated(RunCreated),
    RunStateChanged(RunStateChanged),
    TaskCreated(TaskCreated),
    TaskLeaseClaimed(TaskLeaseClaimed),
    TaskLeaseHeartbeated(TaskLeaseHeartbeated),
    TaskStateChanged(TaskStateChanged),
    ApprovalRequested(ApprovalRequested),
    ApprovalResolved(ApprovalResolved),
    CheckpointRecorded(CheckpointRecorded),
    CheckpointRestored(CheckpointRestored),
    MailboxMessageAppended(MailboxMessageAppended),
    ToolInvocationStarted(ToolInvocationStarted),
    ToolInvocationCompleted(ToolInvocationCompleted),
    ToolInvocationFailed(ToolInvocationFailed),
    SignalIngested(SignalIngested),
    ExternalWorkerReported(ExternalWorkerReported),
    SubagentSpawned(SubagentSpawned),
    RecoveryAttempted(RecoveryAttempted),
    RecoveryCompleted(RecoveryCompleted),
}

impl RuntimeEvent {
    pub fn project(&self) -> &ProjectKey {
        match self {
            RuntimeEvent::SessionCreated(event) => &event.project,
            RuntimeEvent::SessionStateChanged(event) => &event.project,
            RuntimeEvent::RunCreated(event) => &event.project,
            RuntimeEvent::RunStateChanged(event) => &event.project,
            RuntimeEvent::TaskCreated(event) => &event.project,
            RuntimeEvent::TaskLeaseClaimed(event) => &event.project,
            RuntimeEvent::TaskLeaseHeartbeated(event) => &event.project,
            RuntimeEvent::TaskStateChanged(event) => &event.project,
            RuntimeEvent::ApprovalRequested(event) => &event.project,
            RuntimeEvent::ApprovalResolved(event) => &event.project,
            RuntimeEvent::CheckpointRecorded(event) => &event.project,
            RuntimeEvent::CheckpointRestored(event) => &event.project,
            RuntimeEvent::MailboxMessageAppended(event) => &event.project,
            RuntimeEvent::ToolInvocationStarted(event) => &event.project,
            RuntimeEvent::ToolInvocationCompleted(event) => &event.project,
            RuntimeEvent::ToolInvocationFailed(event) => &event.project,
            RuntimeEvent::SignalIngested(event) => &event.project,
            RuntimeEvent::ExternalWorkerReported(event) => &event.report.project,
            RuntimeEvent::SubagentSpawned(event) => &event.project,
            RuntimeEvent::RecoveryAttempted(event) => &event.project,
            RuntimeEvent::RecoveryCompleted(event) => &event.project,
        }
    }

    pub fn primary_entity_ref(&self) -> Option<RuntimeEntityRef> {
        match self {
            RuntimeEvent::SessionCreated(event) => Some(RuntimeEntityRef::Session {
                session_id: event.session_id.clone(),
            }),
            RuntimeEvent::SessionStateChanged(event) => Some(RuntimeEntityRef::Session {
                session_id: event.session_id.clone(),
            }),
            RuntimeEvent::RunCreated(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
            }),
            RuntimeEvent::RunStateChanged(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
            }),
            RuntimeEvent::TaskCreated(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::TaskLeaseClaimed(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::TaskLeaseHeartbeated(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::TaskStateChanged(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::ApprovalRequested(event) => Some(RuntimeEntityRef::Approval {
                approval_id: event.approval_id.clone(),
            }),
            RuntimeEvent::ApprovalResolved(event) => Some(RuntimeEntityRef::Approval {
                approval_id: event.approval_id.clone(),
            }),
            RuntimeEvent::CheckpointRecorded(event) => Some(RuntimeEntityRef::Checkpoint {
                checkpoint_id: event.checkpoint_id.clone(),
            }),
            RuntimeEvent::CheckpointRestored(event) => Some(RuntimeEntityRef::Checkpoint {
                checkpoint_id: event.checkpoint_id.clone(),
            }),
            RuntimeEvent::MailboxMessageAppended(event) => Some(RuntimeEntityRef::MailboxMessage {
                message_id: event.message_id.clone(),
            }),
            RuntimeEvent::ToolInvocationStarted(event) => Some(RuntimeEntityRef::ToolInvocation {
                invocation_id: event.invocation_id.clone(),
            }),
            RuntimeEvent::ToolInvocationCompleted(event) => {
                Some(RuntimeEntityRef::ToolInvocation {
                    invocation_id: event.invocation_id.clone(),
                })
            }
            RuntimeEvent::ToolInvocationFailed(event) => Some(RuntimeEntityRef::ToolInvocation {
                invocation_id: event.invocation_id.clone(),
            }),
            RuntimeEvent::SignalIngested(event) => Some(RuntimeEntityRef::Signal {
                signal_id: event.signal_id.clone(),
            }),
            RuntimeEvent::ExternalWorkerReported(event) => Some(RuntimeEntityRef::Task {
                task_id: event.report.task_id.clone(),
            }),
            RuntimeEvent::SubagentSpawned(event) => Some(RuntimeEntityRef::Task {
                task_id: event.child_task_id.clone(),
            }),
            RuntimeEvent::RecoveryAttempted(event) => event
                .task_id
                .clone()
                .map(|task_id| RuntimeEntityRef::Task { task_id })
                .or_else(|| {
                    event
                        .run_id
                        .clone()
                        .map(|run_id| RuntimeEntityRef::Run { run_id })
                }),
            RuntimeEvent::RecoveryCompleted(event) => event
                .task_id
                .clone()
                .map(|task_id| RuntimeEntityRef::Task { task_id })
                .or_else(|| {
                    event
                        .run_id
                        .clone()
                        .map(|run_id| RuntimeEntityRef::Run { run_id })
                }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransition<S> {
    pub from: Option<S>,
    pub to: S,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCreated {
    pub project: ProjectKey,
    pub session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStateChanged {
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub transition: StateTransition<SessionState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCreated {
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub parent_run_id: Option<RunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStateChanged {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub transition: StateTransition<RunState>,
    pub failure_class: Option<FailureClass>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCreated {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub parent_run_id: Option<RunId>,
    pub parent_task_id: Option<TaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLeaseClaimed {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub lease_owner: String,
    pub lease_token: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLeaseHeartbeated {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub lease_token: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStateChanged {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub transition: StateTransition<TaskState>,
    pub failure_class: Option<FailureClass>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequested {
    pub project: ProjectKey,
    pub approval_id: ApprovalId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub requirement: ApprovalRequirement,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalResolved {
    pub project: ProjectKey,
    pub approval_id: ApprovalId,
    pub decision: ApprovalDecision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecorded {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub checkpoint_id: CheckpointId,
    pub disposition: CheckpointDisposition,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRestored {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub checkpoint_id: CheckpointId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxMessageAppended {
    pub project: ProjectKey,
    pub message_id: MailboxMessageId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationStarted {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub target: ToolInvocationTarget,
    pub execution_class: ExecutionClass,
    pub requested_at_ms: u64,
    pub started_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationCompleted {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    pub finished_at_ms: u64,
    pub outcome: ToolInvocationOutcomeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationFailed {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    pub finished_at_ms: u64,
    pub outcome: ToolInvocationOutcomeKind,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerReported {
    pub report: ExternalWorkerReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSpawned {
    pub project: ProjectKey,
    pub parent_run_id: RunId,
    pub parent_task_id: Option<TaskId>,
    pub child_task_id: TaskId,
    pub child_session_id: SessionId,
    pub child_run_id: Option<RunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryAttempted {
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalIngested {
    pub project: ProjectKey,
    pub signal_id: SignalId,
    pub source: String,
    pub payload: serde_json::Value,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCompleted {
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub recovered: bool,
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalRequested, EventEnvelope, EventSource, ExternalWorkerReported, RuntimeEvent,
        SessionCreated, TaskCreated, ToolInvocationFailed, ToolInvocationStarted,
    };
    use crate::ids::{ApprovalId, CommandId, EventId, TaskId};
    use crate::policy::ExecutionClass;
    use crate::tenancy::{OwnershipKey, ProjectKey};
    use crate::tool_invocation::ToolInvocationTarget;
    use crate::workers::ExternalWorkerReport;

    #[test]
    fn runtime_event_envelope_carries_project_ownership() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let event = EventEnvelope::for_runtime_event(
            EventId::new("evt_1"),
            EventSource::Runtime,
            RuntimeEvent::SessionCreated(SessionCreated {
                project,
                session_id: "session_1".into(),
            }),
        )
        .with_correlation_id("corr_1");

        assert!(matches!(event.payload, RuntimeEvent::SessionCreated(_)));
        assert!(matches!(event.ownership, OwnershipKey::Project(_)));
    }

    #[test]
    fn event_envelope_builders_set_causation_and_correlation() {
        let event = EventEnvelope::new(
            EventId::new("evt_2"),
            EventSource::Runtime,
            ProjectKey::new("tenant", "workspace", "project"),
            RuntimeEvent::SessionCreated(SessionCreated {
                project: ProjectKey::new("tenant", "workspace", "project"),
                session_id: "session_2".into(),
            }),
        )
        .with_causation_id(CommandId::new("cmd_2"))
        .with_correlation_id("corr_2");

        assert_eq!(
            event.causation_id.as_ref().map(|id| id.as_str()),
            Some("cmd_2")
        );
        assert_eq!(event.correlation_id.as_deref(), Some("corr_2"));
    }

    #[test]
    fn runtime_event_envelope_carries_tool_invocation_payload() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let event = EventEnvelope {
            event_id: EventId::new("evt_tool_1"),
            source: EventSource::Runtime,
            ownership: OwnershipKey::Project(project.clone()),
            causation_id: None,
            correlation_id: Some("corr_tool_1".to_owned()),
            payload: RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project,
                invocation_id: "inv_1".into(),
                session_id: Some("session_1".into()),
                run_id: Some("run_1".into()),
                task_id: Some("task_1".into()),
                target: ToolInvocationTarget::Plugin {
                    plugin_id: "com.example.git".to_owned(),
                    tool_name: "git.status".to_owned(),
                },
                execution_class: ExecutionClass::SandboxedProcess,
                requested_at_ms: 10,
                started_at_ms: 11,
            }),
        };

        assert!(matches!(
            event.payload,
            RuntimeEvent::ToolInvocationStarted(_)
        ));
    }

    #[test]
    fn runtime_event_envelope_carries_external_worker_payload() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let event = EventEnvelope {
            event_id: EventId::new("evt_worker_1"),
            source: EventSource::ExternalWorker {
                worker: "worker_1".to_owned(),
            },
            ownership: OwnershipKey::Project(project.clone()),
            causation_id: None,
            correlation_id: Some("corr_worker_1".to_owned()),
            payload: RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
                report: ExternalWorkerReport {
                    project,
                    worker_id: "worker_1".into(),
                    run_id: Some("run_1".into()),
                    task_id: "task_1".into(),
                    lease_token: 3,
                    reported_at_ms: 99,
                    progress: None,
                    outcome: None,
                },
            }),
        };

        assert!(matches!(
            event.payload,
            RuntimeEvent::ExternalWorkerReported(_)
        ));
    }

    #[test]
    fn runtime_event_reports_project_and_primary_entity() {
        let event = RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
            project: ProjectKey::new("tenant", "workspace", "project"),
            invocation_id: "inv_8".into(),
            task_id: Some("task_8".into()),
            tool_name: "fs.write".to_owned(),
            finished_at_ms: 14,
            outcome: crate::tool_invocation::ToolInvocationOutcomeKind::PermanentFailure,
            error_message: Some("bad input".to_owned()),
        });

        assert_eq!(event.project().project_id.as_str(), "project");
        assert!(matches!(
            event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::ToolInvocation { .. })
        ));
    }

    #[test]
    fn task_and_approval_events_already_carry_identity_for_enrichment() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let task_event = RuntimeEvent::TaskCreated(TaskCreated {
            project: project.clone(),
            task_id: TaskId::new("task_9"),
            parent_run_id: None,
            parent_task_id: None,
        });
        let approval_event = RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project.clone(),
            approval_id: ApprovalId::new("approval_9"),
            run_id: None,
            task_id: Some(TaskId::new("task_9")),
            requirement: crate::policy::ApprovalRequirement::Required,
        });

        assert_eq!(task_event.project(), &project);
        assert_eq!(approval_event.project(), &project);
        assert!(matches!(
            task_event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Task { .. })
        ));
        assert!(matches!(
            approval_event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Approval { .. })
        ));
        assert_eq!(
            EventEnvelope::for_runtime_event(EventId::new("evt_task_9"), EventSource::Runtime, task_event)
                .project(),
            &project
        );
        assert_eq!(
            EventEnvelope::for_runtime_event(
                EventId::new("evt_approval_9"),
                EventSource::Runtime,
                approval_event
            )
            .project(),
            &project
        );
    }

    #[test]
    fn event_envelope_reports_project_and_primary_entity() {
        let event = EventEnvelope::for_runtime_event(
            EventId::new("evt_3"),
            EventSource::Runtime,
            RuntimeEvent::SessionCreated(SessionCreated {
                project: ProjectKey::new("tenant", "workspace", "project"),
                session_id: "session_3".into(),
            }),
        );

        assert_eq!(event.project().project_id.as_str(), "project");
        assert!(matches!(
            event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Session { .. })
        ));
    }
}
