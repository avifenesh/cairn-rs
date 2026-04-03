use serde::{Deserialize, Serialize};

/// Session-level lifecycle for long-lived execution context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Open,
    Completed,
    Failed,
    Archived,
}

/// Run lifecycle inside a session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    Pending,
    Running,
    WaitingApproval,
    Paused,
    WaitingDependency,
    Completed,
    Failed,
    Canceled,
}

/// Task lifecycle for leased and schedulable work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Queued,
    Leased,
    Running,
    WaitingApproval,
    Paused,
    WaitingDependency,
    RetryableFailed,
    Completed,
    Failed,
    Canceled,
    DeadLettered,
}

/// Checkpoint disposition is intentionally narrow in v1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckpointDisposition {
    Latest,
    Superseded,
}

/// Failure classes remain explicit even when timeout collapses into failed state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    TimedOut,
    DependencyFailed,
    ApprovalRejected,
    PolicyDenied,
    ExecutionError,
    LeaseExpired,
    CanceledByOperator,
}

/// Canonical pause reasons in v1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseReason {
    pub kind: PauseReasonKind,
    pub detail: Option<String>,
    /// Optional timestamp (unix ms) after which the entity should auto-resume.
    pub resume_after_ms: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PauseReasonKind {
    OperatorPause,
    RuntimeSuspension,
    ToolRequestedSuspension,
    PolicyHold,
}

/// Resume trigger sources are deliberately explicit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResumeTrigger {
    OperatorResume,
    ResumeAfterTimer,
    RuntimeSignal,
}

/// Explicit resume targets keep pause/resume semantics narrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunResumeTarget {
    Pending,
    Running,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskResumeTarget {
    Queued,
    Running,
}

impl SessionState {
    pub fn is_terminal(self) -> bool {
        !matches!(self, SessionState::Open)
    }
}

impl RunState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Completed | RunState::Failed | RunState::Canceled
        )
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        can_transition_run_state(self, next)
    }
}

impl TaskState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskState::Completed
                | TaskState::Failed
                | TaskState::Canceled
                | TaskState::DeadLettered
        )
    }

    pub fn is_retryable(self) -> bool {
        matches!(self, TaskState::RetryableFailed)
    }

    pub fn can_transition_to(self, next: Self) -> bool {
        can_transition_task_state(self, next)
    }
}

/// Canonical v1 run-state transition helper from RFC 005.
pub fn can_transition_run_state(from: RunState, to: RunState) -> bool {
    matches!(
        (from, to),
        (RunState::Pending, RunState::Running)
            | (RunState::Pending, RunState::WaitingApproval)
            | (RunState::Pending, RunState::WaitingDependency)
            | (RunState::Pending, RunState::Paused)
            | (RunState::Pending, RunState::Failed)
            | (RunState::Pending, RunState::Canceled)
            | (RunState::Running, RunState::WaitingApproval)
            | (RunState::Running, RunState::WaitingDependency)
            | (RunState::Running, RunState::Paused)
            | (RunState::Running, RunState::Completed)
            | (RunState::Running, RunState::Failed)
            | (RunState::Running, RunState::Canceled)
            | (RunState::WaitingApproval, RunState::Running)
            | (RunState::WaitingApproval, RunState::Paused)
            | (RunState::WaitingApproval, RunState::Failed)
            | (RunState::WaitingApproval, RunState::Canceled)
            | (RunState::Paused, RunState::Pending)
            | (RunState::Paused, RunState::Running)
            | (RunState::Paused, RunState::Failed)
            | (RunState::Paused, RunState::Canceled)
            | (RunState::WaitingDependency, RunState::Running)
            | (RunState::WaitingDependency, RunState::Paused)
            | (RunState::WaitingDependency, RunState::Completed)
            | (RunState::WaitingDependency, RunState::Failed)
            | (RunState::WaitingDependency, RunState::Canceled)
    )
}

/// Canonical v1 task-state transition helper from RFC 005.
pub fn can_transition_task_state(from: TaskState, to: TaskState) -> bool {
    matches!(
        (from, to),
        (TaskState::Queued, TaskState::Leased)
            | (TaskState::Queued, TaskState::Paused)
            | (TaskState::Queued, TaskState::Canceled)
            | (TaskState::Queued, TaskState::DeadLettered)
            | (TaskState::Leased, TaskState::Running)
            | (TaskState::Leased, TaskState::Queued)
            | (TaskState::Leased, TaskState::Paused)
            | (TaskState::Leased, TaskState::RetryableFailed)
            | (TaskState::Leased, TaskState::Failed)
            | (TaskState::Leased, TaskState::Canceled)
            | (TaskState::Leased, TaskState::DeadLettered)
            | (TaskState::Running, TaskState::WaitingApproval)
            | (TaskState::Running, TaskState::WaitingDependency)
            | (TaskState::Running, TaskState::Paused)
            | (TaskState::Running, TaskState::Completed)
            | (TaskState::Running, TaskState::RetryableFailed)
            | (TaskState::Running, TaskState::Failed)
            | (TaskState::Running, TaskState::Canceled)
            | (TaskState::WaitingApproval, TaskState::Running)
            | (TaskState::WaitingApproval, TaskState::Paused)
            | (TaskState::WaitingApproval, TaskState::RetryableFailed)
            | (TaskState::WaitingApproval, TaskState::Failed)
            | (TaskState::WaitingApproval, TaskState::Canceled)
            | (TaskState::Paused, TaskState::Queued)
            | (TaskState::Paused, TaskState::Running)
            | (TaskState::Paused, TaskState::RetryableFailed)
            | (TaskState::Paused, TaskState::Failed)
            | (TaskState::Paused, TaskState::Canceled)
            | (TaskState::WaitingDependency, TaskState::Running)
            | (TaskState::WaitingDependency, TaskState::Paused)
            | (TaskState::WaitingDependency, TaskState::Completed)
            | (TaskState::WaitingDependency, TaskState::RetryableFailed)
            | (TaskState::WaitingDependency, TaskState::Failed)
            | (TaskState::WaitingDependency, TaskState::Canceled)
            | (TaskState::RetryableFailed, TaskState::Queued)
            | (TaskState::RetryableFailed, TaskState::Failed)
            | (TaskState::RetryableFailed, TaskState::Canceled)
            | (TaskState::RetryableFailed, TaskState::DeadLettered)
    )
}

pub fn can_resume_run_to(target: RunResumeTarget) -> bool {
    matches!(target, RunResumeTarget::Pending | RunResumeTarget::Running)
}

pub fn can_resume_task_to(target: TaskResumeTarget) -> bool {
    matches!(target, TaskResumeTarget::Queued | TaskResumeTarget::Running)
}

/// Derives session state using the v1 rules from RFC 005.
pub fn derive_session_state(
    is_archived: bool,
    any_run_non_terminal: bool,
    latest_root_run_terminal: Option<RunState>,
) -> SessionState {
    if is_archived {
        SessionState::Archived
    } else if any_run_non_terminal {
        SessionState::Open
    } else {
        match latest_root_run_terminal {
            Some(RunState::Failed) => SessionState::Failed,
            Some(RunState::Completed | RunState::Canceled) => SessionState::Completed,
            _ => SessionState::Open,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        can_resume_run_to, can_resume_task_to, derive_session_state, RunResumeTarget, RunState,
        SessionState, TaskResumeTarget, TaskState,
    };

    #[test]
    fn session_derivation_prefers_archive() {
        let state = derive_session_state(true, true, Some(RunState::Failed));
        assert_eq!(state, SessionState::Archived);
    }

    #[test]
    fn session_stays_open_when_any_run_is_non_terminal() {
        let state = derive_session_state(false, true, Some(RunState::Completed));
        assert_eq!(state, SessionState::Open);
    }

    #[test]
    fn session_follows_latest_root_run_terminal_outcome() {
        assert_eq!(
            derive_session_state(false, false, Some(RunState::Failed)),
            SessionState::Failed
        );
        assert_eq!(
            derive_session_state(false, false, Some(RunState::Completed)),
            SessionState::Completed
        );
        assert_eq!(
            derive_session_state(false, false, Some(RunState::Canceled)),
            SessionState::Completed
        );
    }

    #[test]
    fn task_retryable_failed_is_not_terminal() {
        assert!(TaskState::RetryableFailed.is_retryable());
        assert!(!TaskState::RetryableFailed.is_terminal());
        assert!(TaskState::DeadLettered.is_terminal());
    }

    #[test]
    fn run_state_transitions_follow_v1_contract() {
        assert!(RunState::Pending.can_transition_to(RunState::Running));
        assert!(RunState::Running.can_transition_to(RunState::Paused));
        assert!(RunState::Paused.can_transition_to(RunState::Running));
        assert!(RunState::WaitingDependency.can_transition_to(RunState::Completed));
        assert!(!RunState::Completed.can_transition_to(RunState::Running));
        assert!(!RunState::Canceled.can_transition_to(RunState::Pending));
    }

    #[test]
    fn task_state_transitions_follow_v1_contract() {
        assert!(TaskState::Queued.can_transition_to(TaskState::Leased));
        assert!(TaskState::Leased.can_transition_to(TaskState::Running));
        assert!(TaskState::Running.can_transition_to(TaskState::RetryableFailed));
        assert!(TaskState::RetryableFailed.can_transition_to(TaskState::Queued));
        assert!(TaskState::Paused.can_transition_to(TaskState::Running));
        assert!(!TaskState::Completed.can_transition_to(TaskState::Running));
        assert!(!TaskState::DeadLettered.can_transition_to(TaskState::Queued));
    }

    #[test]
    fn resume_targets_remain_narrow() {
        assert!(can_resume_run_to(RunResumeTarget::Pending));
        assert!(can_resume_run_to(RunResumeTarget::Running));
        assert!(can_resume_task_to(TaskResumeTarget::Queued));
        assert!(can_resume_task_to(TaskResumeTarget::Running));
    }
}
