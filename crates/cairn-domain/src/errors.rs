use crate::ids::{
    ApprovalId, CheckpointId, MailboxMessageId, RunId, SessionId, SignalId, TaskId,
    ToolInvocationId,
};
use crate::tenancy::OwnershipKey;
use serde::{Deserialize, Serialize};

/// Runtime-critical entities that participate in command validation and conflict handling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeEntityKind {
    Session,
    Run,
    Task,
    Approval,
    Checkpoint,
    MailboxMessage,
    Signal,
    ToolInvocation,
}

/// Shared runtime entity identifier envelope for API/runtime/store error reporting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "entity", rename_all = "snake_case")]
pub enum RuntimeEntityRef {
    Session { session_id: SessionId },
    Run { run_id: RunId },
    Task { task_id: TaskId },
    Approval { approval_id: ApprovalId },
    Checkpoint { checkpoint_id: CheckpointId },
    MailboxMessage { message_id: MailboxMessageId },
    Signal { signal_id: SignalId },
    ToolInvocation { invocation_id: ToolInvocationId },
}

impl RuntimeEntityRef {
    pub fn kind(&self) -> RuntimeEntityKind {
        match self {
            RuntimeEntityRef::Session { .. } => RuntimeEntityKind::Session,
            RuntimeEntityRef::Run { .. } => RuntimeEntityKind::Run,
            RuntimeEntityRef::Task { .. } => RuntimeEntityKind::Task,
            RuntimeEntityRef::Approval { .. } => RuntimeEntityKind::Approval,
            RuntimeEntityRef::Checkpoint { .. } => RuntimeEntityKind::Checkpoint,
            RuntimeEntityRef::MailboxMessage { .. } => RuntimeEntityKind::MailboxMessage,
            RuntimeEntityRef::Signal { .. } => RuntimeEntityKind::Signal,
            RuntimeEntityRef::ToolInvocation { .. } => RuntimeEntityKind::ToolInvocation,
        }
    }
}

/// Canonical command-validation failures that should be shared across runtime/API/store boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum CommandValidationError {
    NotFound {
        entity: RuntimeEntityRef,
    },
    AlreadyExists {
        entity: RuntimeEntityRef,
    },
    InvalidOwnership {
        expected: OwnershipKey,
        actual: OwnershipKey,
    },
    InvalidStateTransition {
        entity: RuntimeEntityRef,
        from: Option<String>,
        to: String,
    },
    EntityAlreadyTerminal {
        entity: RuntimeEntityRef,
        state: String,
    },
    ApprovalAlreadyResolved {
        approval_id: ApprovalId,
    },
    LeaseRequired {
        task_id: TaskId,
    },
    LeaseExpired {
        task_id: TaskId,
        lease_token: u64,
        lease_expires_at_ms: u64,
    },
    LeaseTokenMismatch {
        task_id: TaskId,
        expected_lease_token: u64,
        actual_lease_token: u64,
    },
    PolicyDenied {
        reason: String,
    },
}

/// Optimistic-concurrency and runtime-truth conflicts that may be retried by callers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "error", rename_all = "snake_case")]
pub enum RuntimeConflictError {
    StaleVersion {
        entity: RuntimeEntityRef,
        expected_version: u64,
        actual_version: u64,
    },
    LeaseAlreadyClaimed {
        task_id: TaskId,
        current_lease_token: u64,
        current_lease_owner: String,
    },
    ConcurrentMutation {
        entity: RuntimeEntityRef,
    },
}

#[cfg(test)]
mod tests {
    use super::{
        CommandValidationError, RuntimeConflictError, RuntimeEntityKind, RuntimeEntityRef,
    };
    use crate::tenancy::OwnershipKey;

    #[test]
    fn runtime_entity_ref_reports_kind() {
        let entity = RuntimeEntityRef::Task {
            task_id: "task_1".into(),
        };

        assert_eq!(entity.kind(), RuntimeEntityKind::Task);
    }

    #[test]
    fn validation_errors_keep_runtime_context() {
        let error = CommandValidationError::InvalidOwnership {
            expected: OwnershipKey::Project(crate::ProjectKey::new("t", "w", "expected")),
            actual: OwnershipKey::Project(crate::ProjectKey::new("t", "w", "actual")),
        };

        assert!(matches!(
            error,
            CommandValidationError::InvalidOwnership { .. }
        ));
    }

    #[test]
    fn conflict_errors_are_retry_friendly() {
        let error = RuntimeConflictError::StaleVersion {
            entity: RuntimeEntityRef::Run {
                run_id: "run_1".into(),
            },
            expected_version: 3,
            actual_version: 4,
        };

        assert!(matches!(error, RuntimeConflictError::StaleVersion { .. }));
    }
}
