use crate::ids::{RunId, SessionId, TaskId, ToolInvocationId};
use crate::policy::ExecutionClass;
use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Durable current-state lifecycle for tool execution visibility and replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInvocationState {
    Requested,
    Started,
    Completed,
    Failed,
    Canceled,
}

/// Canonical runtime outcome classification for durable tool invocation records.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInvocationOutcomeKind {
    Success,
    RetryableFailure,
    PermanentFailure,
    Timeout,
    Canceled,
    ProtocolViolation,
}

impl ToolInvocationOutcomeKind {
    pub fn is_success(self) -> bool {
        matches!(self, ToolInvocationOutcomeKind::Success)
    }

    pub fn is_failure(self) -> bool {
        matches!(
            self,
            ToolInvocationOutcomeKind::RetryableFailure
                | ToolInvocationOutcomeKind::PermanentFailure
                | ToolInvocationOutcomeKind::Timeout
                | ToolInvocationOutcomeKind::Canceled
                | ToolInvocationOutcomeKind::ProtocolViolation
        )
    }

    pub fn terminal_state(self) -> ToolInvocationState {
        match self {
            ToolInvocationOutcomeKind::Success => ToolInvocationState::Completed,
            ToolInvocationOutcomeKind::Canceled => ToolInvocationState::Canceled,
            ToolInvocationOutcomeKind::RetryableFailure
            | ToolInvocationOutcomeKind::PermanentFailure
            | ToolInvocationOutcomeKind::Timeout
            | ToolInvocationOutcomeKind::ProtocolViolation => ToolInvocationState::Failed,
        }
    }
}

/// Minimal durable target identity for built-in and plugin-backed tool calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "target_type", rename_all = "snake_case")]
pub enum ToolInvocationTarget {
    Builtin {
        tool_name: String,
    },
    Plugin {
        plugin_id: String,
        tool_name: String,
    },
}

/// Shared durable current-state record for tool invocation projections.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationRecord {
    pub invocation_id: ToolInvocationId,
    pub project: ProjectKey,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub target: ToolInvocationTarget,
    pub execution_class: ExecutionClass,
    pub prompt_release_id: Option<crate::ids::PromptReleaseId>,
    pub state: ToolInvocationState,
    pub version: u64,
    pub requested_at_ms: u64,
    pub started_at_ms: Option<u64>,
    pub finished_at_ms: Option<u64>,
    pub outcome: Option<ToolInvocationOutcomeKind>,
    pub error_message: Option<String>,
}

impl ToolInvocationRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new_requested(
        invocation_id: ToolInvocationId,
        project: ProjectKey,
        session_id: Option<SessionId>,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        target: ToolInvocationTarget,
        execution_class: ExecutionClass,
        requested_at_ms: u64,
    ) -> Self {
        let record = Self {
            invocation_id,
            project,
            session_id,
            run_id,
            task_id,
            target,
            execution_class,
            prompt_release_id: None,
            state: ToolInvocationState::Requested,
            version: 1,
            requested_at_ms,
            started_at_ms: None,
            finished_at_ms: None,
            outcome: None,
            error_message: None,
        };

        debug_assert!(validate_tool_invocation_record(&record).is_ok());
        record
    }

    pub fn mark_started(&self, started_at_ms: u64) -> Result<Self, ToolInvocationValidationError> {
        if !can_transition_tool_invocation(self.state, ToolInvocationState::Started) {
            return Err(ToolInvocationValidationError::InvalidTransition);
        }

        let mut next = self.clone();
        next.state = ToolInvocationState::Started;
        next.started_at_ms = Some(started_at_ms);
        next.version += 1;

        validate_tool_invocation_record(&next)?;
        Ok(next)
    }

    pub fn mark_finished(
        &self,
        outcome: ToolInvocationOutcomeKind,
        error_message: Option<String>,
        finished_at_ms: u64,
    ) -> Result<Self, ToolInvocationValidationError> {
        let target_state = outcome.terminal_state();
        if !can_transition_tool_invocation(self.state, target_state) {
            return Err(ToolInvocationValidationError::InvalidTransition);
        }

        let mut next = self.clone();
        next.state = target_state;
        next.outcome = Some(outcome);
        next.error_message = error_message;
        next.finished_at_ms = Some(finished_at_ms);
        next.version += 1;

        validate_tool_invocation_record(&next)?;
        Ok(next)
    }
}

impl ToolInvocationState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            ToolInvocationState::Completed
                | ToolInvocationState::Failed
                | ToolInvocationState::Canceled
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolInvocationValidationError {
    InvalidTransition,
    MissingTerminalOutcome,
    UnexpectedOutcomeForRequested,
    UnexpectedErrorMessageForSuccess,
    MissingErrorMessageForFailure,
}

pub fn can_transition_tool_invocation(from: ToolInvocationState, to: ToolInvocationState) -> bool {
    matches!(
        (from, to),
        (ToolInvocationState::Requested, ToolInvocationState::Started)
            | (ToolInvocationState::Requested, ToolInvocationState::Failed)
            | (
                ToolInvocationState::Requested,
                ToolInvocationState::Canceled
            )
            | (ToolInvocationState::Started, ToolInvocationState::Completed)
            | (ToolInvocationState::Started, ToolInvocationState::Failed)
            | (ToolInvocationState::Started, ToolInvocationState::Canceled)
    )
}

pub fn validate_tool_invocation_record(
    record: &ToolInvocationRecord,
) -> Result<(), ToolInvocationValidationError> {
    match record.state {
        ToolInvocationState::Requested => {
            if record.outcome.is_some() || record.finished_at_ms.is_some() {
                return Err(ToolInvocationValidationError::UnexpectedOutcomeForRequested);
            }
        }
        ToolInvocationState::Started => {
            if record.started_at_ms.is_none() {
                return Err(ToolInvocationValidationError::InvalidTransition);
            }
            if record.outcome.is_some() || record.finished_at_ms.is_some() {
                return Err(ToolInvocationValidationError::MissingTerminalOutcome);
            }
        }
        ToolInvocationState::Completed
        | ToolInvocationState::Failed
        | ToolInvocationState::Canceled => {
            let outcome = record
                .outcome
                .ok_or(ToolInvocationValidationError::MissingTerminalOutcome)?;

            if record.finished_at_ms.is_none() {
                return Err(ToolInvocationValidationError::MissingTerminalOutcome);
            }

            if matches!(record.state, ToolInvocationState::Completed) && !outcome.is_success() {
                return Err(ToolInvocationValidationError::InvalidTransition);
            }

            if matches!(
                record.state,
                ToolInvocationState::Failed | ToolInvocationState::Canceled
            ) && !outcome.is_failure()
            {
                return Err(ToolInvocationValidationError::InvalidTransition);
            }

            if outcome.is_success() && record.error_message.is_some() {
                return Err(ToolInvocationValidationError::UnexpectedErrorMessageForSuccess);
            }

            if outcome.is_failure() && record.error_message.is_none() {
                return Err(ToolInvocationValidationError::MissingErrorMessageForFailure);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        can_transition_tool_invocation, validate_tool_invocation_record, ToolInvocationOutcomeKind,
        ToolInvocationRecord, ToolInvocationState, ToolInvocationTarget,
        ToolInvocationValidationError,
    };
    use crate::policy::ExecutionClass;

    #[test]
    fn tool_invocation_terminal_states_are_explicit() {
        assert!(ToolInvocationState::Completed.is_terminal());
        assert!(ToolInvocationState::Failed.is_terminal());
        assert!(ToolInvocationState::Canceled.is_terminal());
        assert!(!ToolInvocationState::Requested.is_terminal());
    }

    #[test]
    fn tool_invocation_record_carries_target_and_outcome() {
        let record = ToolInvocationRecord {
            invocation_id: "inv_1".into(),
            project: crate::ProjectKey::new("t", "w", "p"),
            session_id: Some("session_1".into()),
            run_id: Some("run_1".into()),
            task_id: Some("task_1".into()),
            target: ToolInvocationTarget::Plugin {
                plugin_id: "com.example.git".to_owned(),
                tool_name: "git.status".to_owned(),
            },
            execution_class: ExecutionClass::SandboxedProcess,
            prompt_release_id: None,
            state: ToolInvocationState::Completed,
            version: 2,
            requested_at_ms: 10,
            started_at_ms: Some(11),
            finished_at_ms: Some(14),
            outcome: Some(ToolInvocationOutcomeKind::Success),
            error_message: None,
        };

        assert!(matches!(record.target, ToolInvocationTarget::Plugin { .. }));
    }

    #[test]
    fn outcome_kind_maps_to_terminal_state() {
        assert_eq!(
            ToolInvocationOutcomeKind::Success.terminal_state(),
            ToolInvocationState::Completed
        );
        assert_eq!(
            ToolInvocationOutcomeKind::Canceled.terminal_state(),
            ToolInvocationState::Canceled
        );
        assert_eq!(
            ToolInvocationOutcomeKind::Timeout.terminal_state(),
            ToolInvocationState::Failed
        );
    }

    #[test]
    fn tool_invocation_transitions_are_narrow() {
        assert!(can_transition_tool_invocation(
            ToolInvocationState::Requested,
            ToolInvocationState::Started
        ));
        assert!(can_transition_tool_invocation(
            ToolInvocationState::Started,
            ToolInvocationState::Completed
        ));
        assert!(!can_transition_tool_invocation(
            ToolInvocationState::Completed,
            ToolInvocationState::Started
        ));
    }

    #[test]
    fn completed_invocation_requires_success_outcome() {
        let record = ToolInvocationRecord {
            invocation_id: "inv_2".into(),
            project: crate::ProjectKey::new("t", "w", "p"),
            session_id: None,
            run_id: Some("run_1".into()),
            task_id: Some("task_1".into()),
            target: ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            prompt_release_id: None,
            state: ToolInvocationState::Completed,
            version: 3,
            requested_at_ms: 10,
            started_at_ms: Some(11),
            finished_at_ms: Some(12),
            outcome: Some(ToolInvocationOutcomeKind::RetryableFailure),
            error_message: Some("transient".to_owned()),
        };

        assert_eq!(
            validate_tool_invocation_record(&record),
            Err(ToolInvocationValidationError::InvalidTransition)
        );
    }

    #[test]
    fn record_methods_follow_shared_transition_rules() {
        let requested = ToolInvocationRecord::new_requested(
            "inv_4".into(),
            crate::ProjectKey::new("t", "w", "p"),
            None,
            Some("run_1".into()),
            Some("task_1".into()),
            ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            ExecutionClass::SupervisedProcess,
            10,
        );

        let started = requested.mark_started(11).unwrap();
        let finished = started
            .mark_finished(ToolInvocationOutcomeKind::Success, None, 12)
            .unwrap();

        assert_eq!(started.state, ToolInvocationState::Started);
        assert_eq!(finished.state, ToolInvocationState::Completed);
        assert_eq!(finished.outcome, Some(ToolInvocationOutcomeKind::Success));
    }

    #[test]
    fn canceled_finish_requires_context_and_preserves_terminal_state() {
        let requested = ToolInvocationRecord::new_requested(
            "inv_5".into(),
            crate::ProjectKey::new("t", "w", "p"),
            None,
            Some("run_1".into()),
            Some("task_1".into()),
            ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            ExecutionClass::SupervisedProcess,
            10,
        );

        assert_eq!(
            requested.mark_finished(ToolInvocationOutcomeKind::Canceled, None, 11),
            Err(ToolInvocationValidationError::MissingErrorMessageForFailure)
        );

        let canceled = requested
            .mark_finished(
                ToolInvocationOutcomeKind::Canceled,
                Some("canceled".to_owned()),
                11,
            )
            .unwrap();

        assert_eq!(canceled.state, ToolInvocationState::Canceled);
        assert_eq!(canceled.outcome, Some(ToolInvocationOutcomeKind::Canceled));
        assert_eq!(canceled.error_message.as_deref(), Some("canceled"));
        assert_eq!(canceled.finished_at_ms, Some(11));
        assert_eq!(canceled.version, 2);
    }

    #[test]
    fn successful_finish_rejects_error_context() {
        let requested = ToolInvocationRecord::new_requested(
            "inv_6".into(),
            crate::ProjectKey::new("t", "w", "p"),
            None,
            Some("run_1".into()),
            Some("task_1".into()),
            ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            ExecutionClass::SupervisedProcess,
            10,
        );
        let started = requested.mark_started(11).unwrap();

        assert_eq!(
            started.mark_finished(
                ToolInvocationOutcomeKind::Success,
                Some("should not exist".to_owned()),
                12
            ),
            Err(ToolInvocationValidationError::UnexpectedErrorMessageForSuccess)
        );
    }

    #[test]
    fn failed_invocation_requires_failure_context() {
        let record = ToolInvocationRecord {
            invocation_id: "inv_3".into(),
            project: crate::ProjectKey::new("t", "w", "p"),
            session_id: None,
            run_id: Some("run_1".into()),
            task_id: Some("task_1".into()),
            target: ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            prompt_release_id: None,
            state: ToolInvocationState::Failed,
            version: 3,
            requested_at_ms: 10,
            started_at_ms: Some(11),
            finished_at_ms: Some(12),
            outcome: Some(ToolInvocationOutcomeKind::PermanentFailure),
            error_message: None,
        };

        assert_eq!(
            validate_tool_invocation_record(&record),
            Err(ToolInvocationValidationError::MissingErrorMessageForFailure)
        );
    }
}
