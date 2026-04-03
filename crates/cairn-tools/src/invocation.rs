use async_trait::async_trait;
use cairn_domain::ids::{RunId, SessionId, TaskId, ToolInvocationId};
use cairn_domain::policy::ExecutionClass;
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::tool_invocation::{
    ToolInvocationOutcomeKind, ToolInvocationRecord, ToolInvocationTarget,
};
use cairn_store::error::StoreError;
use serde::{Deserialize, Serialize};

use crate::builtin::ToolOutcome;

/// Request to begin a tool invocation through the durable pipeline.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvocationRequest {
    pub invocation_id: ToolInvocationId,
    pub project: ProjectKey,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub target: ToolInvocationTarget,
    pub execution_class: ExecutionClass,
}

/// Durable invocation result combining the tool outcome with record metadata.
#[derive(Clone, Debug)]
pub struct InvocationResult {
    pub record: ToolInvocationRecord,
    pub outcome_kind: ToolInvocationOutcomeKind,
}

/// Maps a `ToolOutcome` (local execution result) to the shared
/// `ToolInvocationOutcomeKind` used by durable records.
pub fn outcome_to_kind(outcome: &ToolOutcome) -> ToolInvocationOutcomeKind {
    match outcome {
        ToolOutcome::Success { .. } => ToolInvocationOutcomeKind::Success,
        ToolOutcome::RetryableFailure { .. } => ToolInvocationOutcomeKind::RetryableFailure,
        ToolOutcome::PermanentFailure { .. } => ToolInvocationOutcomeKind::PermanentFailure,
        ToolOutcome::Timeout => ToolInvocationOutcomeKind::Timeout,
        ToolOutcome::Canceled => ToolInvocationOutcomeKind::Canceled,
    }
}

/// Extracts an error message from a `ToolOutcome`, if applicable.
pub fn outcome_error_message(outcome: &ToolOutcome) -> Option<String> {
    match outcome {
        ToolOutcome::RetryableFailure { reason } => Some(reason.clone()),
        ToolOutcome::PermanentFailure { reason } => Some(reason.clone()),
        ToolOutcome::Timeout => Some("timeout".to_owned()),
        ToolOutcome::Canceled => Some("canceled".to_owned()),
        _ => None,
    }
}

/// Creates the initial `ToolInvocationRecord` in `Requested` state.
pub fn create_requested_record(
    request: &InvocationRequest,
    requested_at_ms: u64,
) -> ToolInvocationRecord {
    ToolInvocationRecord::new_requested(
        request.invocation_id.clone(),
        request.project.clone(),
        request.session_id.clone(),
        request.run_id.clone(),
        request.task_id.clone(),
        request.target.clone(),
        request.execution_class,
        requested_at_ms,
    )
}

/// Advances a record to `Started` state.
pub fn mark_started(record: &ToolInvocationRecord, started_at_ms: u64) -> ToolInvocationRecord {
    record
        .mark_started(started_at_ms)
        .expect("tool pipeline should only request valid invocation start transitions")
}

/// Advances a record to its terminal state based on the tool outcome.
pub fn mark_finished(
    record: &ToolInvocationRecord,
    outcome: &ToolOutcome,
    finished_at_ms: u64,
) -> ToolInvocationRecord {
    record
        .mark_finished(
            outcome_to_kind(outcome),
            outcome_error_message(outcome),
            finished_at_ms,
        )
        .expect("tool pipeline should only request valid invocation finish transitions")
}

/// Service trait for durable tool invocation lifecycle.
///
/// Implementors coordinate between the permission gate, tool host,
/// event log, and tool invocation read model to produce a fully
/// persisted invocation lifecycle.
#[async_trait]
pub trait InvocationService: Send + Sync {
    /// Record a tool invocation request. Returns the initial record.
    async fn request(
        &self,
        request: InvocationRequest,
        requested_at_ms: u64,
    ) -> Result<ToolInvocationRecord, StoreError>;

    /// Mark a previously requested invocation as started.
    async fn start(
        &self,
        invocation_id: &ToolInvocationId,
        started_at_ms: u64,
    ) -> Result<ToolInvocationRecord, StoreError>;

    /// Record the terminal outcome of an invocation.
    async fn finish(
        &self,
        invocation_id: &ToolInvocationId,
        outcome: &ToolOutcome,
        finished_at_ms: u64,
    ) -> Result<ToolInvocationRecord, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::ToolInvocationId;
    use cairn_domain::policy::ExecutionClass;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::tool_invocation::{
        validate_tool_invocation_record, ToolInvocationState, ToolInvocationTarget,
    };

    fn test_request() -> InvocationRequest {
        InvocationRequest {
            invocation_id: ToolInvocationId::new("inv_1"),
            project: ProjectKey::new("t", "w", "p"),
            session_id: Some("sess_1".into()),
            run_id: Some("run_1".into()),
            task_id: Some("task_1".into()),
            target: ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
        }
    }

    #[test]
    fn create_requested_record_sets_initial_state() {
        let record = create_requested_record(&test_request(), 1000);

        assert_eq!(record.state, ToolInvocationState::Requested);
        assert_eq!(record.version, 1);
        assert_eq!(record.requested_at_ms, 1000);
        assert!(record.started_at_ms.is_none());
        assert!(record.finished_at_ms.is_none());
        assert!(record.outcome.is_none());
        assert!(validate_tool_invocation_record(&record).is_ok());
    }

    #[test]
    fn mark_started_advances_state() {
        let record = create_requested_record(&test_request(), 1000);
        let started = mark_started(&record, 1001);

        assert_eq!(started.state, ToolInvocationState::Started);
        assert_eq!(started.started_at_ms, Some(1001));
        assert_eq!(started.version, 2);
        assert!(validate_tool_invocation_record(&started).is_ok());
    }

    #[test]
    fn mark_finished_with_success() {
        let record = create_requested_record(&test_request(), 1000);
        let started = mark_started(&record, 1001);
        let outcome = ToolOutcome::Success {
            output: serde_json::json!({"text": "ok"}),
        };
        let finished = mark_finished(&started, &outcome, 1005);

        assert_eq!(finished.state, ToolInvocationState::Completed);
        assert_eq!(finished.outcome, Some(ToolInvocationOutcomeKind::Success));
        assert!(finished.error_message.is_none());
        assert_eq!(finished.finished_at_ms, Some(1005));
        assert_eq!(finished.version, 3);
        assert!(validate_tool_invocation_record(&finished).is_ok());
    }

    #[test]
    fn mark_finished_with_failure() {
        let record = create_requested_record(&test_request(), 1000);
        let started = mark_started(&record, 1001);
        let outcome = ToolOutcome::PermanentFailure {
            reason: "bad input".to_owned(),
        };
        let finished = mark_finished(&started, &outcome, 1005);

        assert_eq!(finished.state, ToolInvocationState::Failed);
        assert_eq!(
            finished.outcome,
            Some(ToolInvocationOutcomeKind::PermanentFailure)
        );
        assert_eq!(finished.error_message, Some("bad input".to_owned()));
        assert!(validate_tool_invocation_record(&finished).is_ok());
    }

    #[test]
    fn mark_finished_with_timeout() {
        let record = create_requested_record(&test_request(), 1000);
        let started = mark_started(&record, 1001);
        let finished = mark_finished(&started, &ToolOutcome::Timeout, 1005);

        assert_eq!(finished.state, ToolInvocationState::Failed);
        assert_eq!(finished.outcome, Some(ToolInvocationOutcomeKind::Timeout));
        assert_eq!(finished.error_message, Some("timeout".to_owned()));
        assert!(validate_tool_invocation_record(&finished).is_ok());
    }

    #[test]
    fn mark_finished_with_cancel() {
        let record = create_requested_record(&test_request(), 1000);
        let started = mark_started(&record, 1001);
        let finished = mark_finished(&started, &ToolOutcome::Canceled, 1005);

        assert_eq!(finished.state, ToolInvocationState::Canceled);
        assert_eq!(finished.outcome, Some(ToolInvocationOutcomeKind::Canceled));
        assert_eq!(finished.error_message.as_deref(), Some("canceled"));
        assert!(validate_tool_invocation_record(&finished).is_ok());
    }

    #[test]
    fn outcome_kind_mapping_is_exhaustive() {
        let cases: Vec<(ToolOutcome, ToolInvocationOutcomeKind)> = vec![
            (
                ToolOutcome::Success {
                    output: serde_json::json!(null),
                },
                ToolInvocationOutcomeKind::Success,
            ),
            (
                ToolOutcome::RetryableFailure {
                    reason: "x".to_owned(),
                },
                ToolInvocationOutcomeKind::RetryableFailure,
            ),
            (
                ToolOutcome::PermanentFailure {
                    reason: "x".to_owned(),
                },
                ToolInvocationOutcomeKind::PermanentFailure,
            ),
            (ToolOutcome::Timeout, ToolInvocationOutcomeKind::Timeout),
            (ToolOutcome::Canceled, ToolInvocationOutcomeKind::Canceled),
        ];
        for (outcome, expected) in cases {
            assert_eq!(outcome_to_kind(&outcome), expected);
        }
    }
}
