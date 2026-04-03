use crate::ids::{RunId, TaskId, WorkerId};
use crate::lifecycle::{FailureClass, PauseReason};
use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

/// Canonical leased task record shared across runtime, store, and external workers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLease {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub lease_owner: WorkerId,
    pub lease_token: u64,
    pub lease_expires_at_ms: u64,
}

/// Narrow progress payload that external workers may report through runtime-owned APIs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerProgress {
    pub message: Option<String>,
    pub percent_milli: Option<u16>,
}

/// Outcome classifications external workers may report without becoming canonical event writers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum ExternalWorkerOutcome {
    Completed,
    Failed { failure_class: FailureClass },
    Canceled,
    Suspended { reason: PauseReason },
}

/// Shared payload for worker heartbeats, progress, and terminal outcome reporting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerReport {
    pub project: ProjectKey,
    pub worker_id: WorkerId,
    pub run_id: Option<RunId>,
    pub task_id: TaskId,
    pub lease_token: u64,
    pub reported_at_ms: u64,
    pub progress: Option<ExternalWorkerProgress>,
    pub outcome: Option<ExternalWorkerOutcome>,
}

impl ExternalWorkerReport {
    pub fn is_terminal(&self) -> bool {
        self.outcome.is_some()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalWorkerReportValidationError {
    ProjectMismatch,
    TaskMismatch,
    WorkerMismatch,
    LeaseTokenMismatch,
}

pub fn validate_external_worker_report(
    report: &ExternalWorkerReport,
    lease: &TaskLease,
) -> Result<(), ExternalWorkerReportValidationError> {
    if report.project != lease.project {
        return Err(ExternalWorkerReportValidationError::ProjectMismatch);
    }

    if report.task_id != lease.task_id {
        return Err(ExternalWorkerReportValidationError::TaskMismatch);
    }

    if report.worker_id != lease.lease_owner {
        return Err(ExternalWorkerReportValidationError::WorkerMismatch);
    }

    if report.lease_token != lease.lease_token {
        return Err(ExternalWorkerReportValidationError::LeaseTokenMismatch);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        validate_external_worker_report, ExternalWorkerOutcome, ExternalWorkerProgress,
        ExternalWorkerReport, ExternalWorkerReportValidationError, TaskLease,
    };
    use crate::lifecycle::{FailureClass, PauseReason, PauseReasonKind};

    #[test]
    fn external_worker_report_knows_when_outcome_is_terminal() {
        let report = ExternalWorkerReport {
            project: crate::ProjectKey::new("t", "w", "p"),
            worker_id: "worker_1".into(),
            run_id: Some("run_1".into()),
            task_id: "task_1".into(),
            lease_token: 9,
            reported_at_ms: 10,
            progress: Some(ExternalWorkerProgress {
                message: Some("halfway".to_owned()),
                percent_milli: Some(500),
            }),
            outcome: None,
        };

        assert!(!report.is_terminal());
    }

    #[test]
    fn external_worker_outcome_supports_failure_and_suspension() {
        let failure = ExternalWorkerOutcome::Failed {
            failure_class: FailureClass::ExecutionError,
        };
        let suspended = ExternalWorkerOutcome::Suspended {
            reason: PauseReason {
                kind: PauseReasonKind::RuntimeSuspension,
                detail: Some("waiting for capacity".to_owned()),
                resume_after_ms: None,
            },
        };

        assert!(matches!(failure, ExternalWorkerOutcome::Failed { .. }));
        assert!(matches!(suspended, ExternalWorkerOutcome::Suspended { .. }));
    }

    #[test]
    fn task_lease_keeps_worker_identity_and_token() {
        let lease = TaskLease {
            project: crate::ProjectKey::new("t", "w", "p"),
            task_id: "task_1".into(),
            lease_owner: "worker_42".into(),
            lease_token: 7,
            lease_expires_at_ms: 999,
        };

        assert_eq!(lease.lease_token, 7);
        assert_eq!(lease.lease_owner.as_str(), "worker_42");
    }

    #[test]
    fn worker_report_must_match_lease_identity() {
        let lease = TaskLease {
            project: crate::ProjectKey::new("t", "w", "p"),
            task_id: "task_1".into(),
            lease_owner: "worker_42".into(),
            lease_token: 7,
            lease_expires_at_ms: 999,
        };
        let report = ExternalWorkerReport {
            project: crate::ProjectKey::new("t", "w", "p"),
            worker_id: "worker_42".into(),
            run_id: Some("run_1".into()),
            task_id: "task_1".into(),
            lease_token: 8,
            reported_at_ms: 10,
            progress: None,
            outcome: None,
        };

        assert_eq!(
            validate_external_worker_report(&report, &lease),
            Err(ExternalWorkerReportValidationError::LeaseTokenMismatch)
        );
    }
}
