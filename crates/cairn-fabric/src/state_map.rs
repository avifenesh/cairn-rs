use cairn_domain::lifecycle::{FailureClass, RunState, TaskState};
use ff_core::state::PublicState;

use crate::constants::BLOCKING_WAITING_FOR_APPROVAL;

pub fn ff_public_state_to_run_state(state: PublicState) -> (RunState, Option<FailureClass>) {
    match state {
        PublicState::Waiting | PublicState::Delayed | PublicState::RateLimited => {
            (RunState::Pending, None)
        }
        PublicState::WaitingChildren => (RunState::WaitingDependency, None),
        PublicState::Active => (RunState::Running, None),
        PublicState::Suspended => (RunState::Paused, None),
        PublicState::Completed => (RunState::Completed, None),
        PublicState::Failed => (RunState::Failed, Some(FailureClass::ExecutionError)),
        PublicState::Cancelled => (RunState::Canceled, Some(FailureClass::CanceledByOperator)),
        PublicState::Expired => (RunState::Failed, Some(FailureClass::TimedOut)),
        PublicState::Skipped => (RunState::Failed, Some(FailureClass::DependencyFailed)),
    }
}

pub fn adjust_run_state_for_blocking_reason(state: RunState, blocking_reason: &str) -> RunState {
    if state == RunState::Paused && blocking_reason == BLOCKING_WAITING_FOR_APPROVAL {
        RunState::WaitingApproval
    } else {
        state
    }
}

pub fn adjust_task_state_for_blocking_reason(state: TaskState, blocking_reason: &str) -> TaskState {
    if state == TaskState::Paused && blocking_reason == BLOCKING_WAITING_FOR_APPROVAL {
        TaskState::WaitingApproval
    } else {
        state
    }
}

pub fn ff_public_state_to_task_state(state: PublicState) -> (TaskState, Option<FailureClass>) {
    match state {
        PublicState::Waiting | PublicState::Delayed | PublicState::RateLimited => {
            (TaskState::Queued, None)
        }
        PublicState::WaitingChildren => (TaskState::WaitingDependency, None),
        PublicState::Active => (TaskState::Running, None),
        PublicState::Suspended => (TaskState::Paused, None),
        PublicState::Completed => (TaskState::Completed, None),
        PublicState::Failed => (TaskState::Failed, Some(FailureClass::ExecutionError)),
        PublicState::Cancelled => (TaskState::Canceled, Some(FailureClass::CanceledByOperator)),
        PublicState::Expired => (TaskState::Failed, Some(FailureClass::TimedOut)),
        PublicState::Skipped => (TaskState::Failed, Some(FailureClass::DependencyFailed)),
    }
}

pub fn ff_run_state_to_public_states(state: RunState) -> &'static [PublicState] {
    match state {
        RunState::Pending => &[
            PublicState::Waiting,
            PublicState::Delayed,
            PublicState::RateLimited,
        ],
        RunState::Running => &[PublicState::Active],
        RunState::WaitingApproval => &[PublicState::Suspended],
        RunState::Paused => &[PublicState::Suspended],
        RunState::WaitingDependency => &[PublicState::WaitingChildren],
        RunState::Completed => &[PublicState::Completed],
        RunState::Failed => &[
            PublicState::Failed,
            PublicState::Expired,
            PublicState::Skipped,
        ],
        RunState::Canceled => &[PublicState::Cancelled],
    }
}

pub fn ff_task_state_to_public_states(state: TaskState) -> &'static [PublicState] {
    match state {
        TaskState::Queued => &[
            PublicState::Waiting,
            PublicState::Delayed,
            PublicState::RateLimited,
        ],
        TaskState::Leased | TaskState::Running => &[PublicState::Active],
        TaskState::WaitingApproval => &[PublicState::Suspended],
        TaskState::Paused => &[PublicState::Suspended],
        TaskState::WaitingDependency => &[PublicState::WaitingChildren],
        TaskState::RetryableFailed => &[PublicState::Delayed],
        TaskState::Completed => &[PublicState::Completed],
        TaskState::Failed => &[
            PublicState::Failed,
            PublicState::Expired,
            PublicState::Skipped,
        ],
        TaskState::Canceled => &[PublicState::Cancelled],
        TaskState::DeadLettered => &[PublicState::Failed],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_maps_to_pending() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Waiting);
        assert_eq!(run, RunState::Pending);
        assert!(fc.is_none());
    }

    #[test]
    fn delayed_maps_to_pending() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Delayed);
        assert_eq!(run, RunState::Pending);
        assert!(fc.is_none());
    }

    #[test]
    fn rate_limited_maps_to_pending() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::RateLimited);
        assert_eq!(run, RunState::Pending);
        assert!(fc.is_none());
    }

    #[test]
    fn waiting_children_maps_to_waiting_dependency() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::WaitingChildren);
        assert_eq!(run, RunState::WaitingDependency);
        assert!(fc.is_none());
    }

    #[test]
    fn active_maps_to_running() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Active);
        assert_eq!(run, RunState::Running);
        assert!(fc.is_none());
    }

    #[test]
    fn suspended_maps_to_paused() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Suspended);
        assert_eq!(run, RunState::Paused);
        assert!(fc.is_none());
    }

    #[test]
    fn completed_maps_to_completed() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Completed);
        assert_eq!(run, RunState::Completed);
        assert!(fc.is_none());
    }

    #[test]
    fn failed_maps_to_failed_with_execution_error() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Failed);
        assert_eq!(run, RunState::Failed);
        assert_eq!(fc, Some(FailureClass::ExecutionError));
    }

    #[test]
    fn cancelled_maps_to_canceled() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Cancelled);
        assert_eq!(run, RunState::Canceled);
        assert_eq!(fc, Some(FailureClass::CanceledByOperator));
    }

    #[test]
    fn expired_maps_to_failed_timed_out() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Expired);
        assert_eq!(run, RunState::Failed);
        assert_eq!(fc, Some(FailureClass::TimedOut));
    }

    #[test]
    fn skipped_maps_to_failed_dependency() {
        let (run, fc) = ff_public_state_to_run_state(PublicState::Skipped);
        assert_eq!(run, RunState::Failed);
        assert_eq!(fc, Some(FailureClass::DependencyFailed));
    }

    #[test]
    fn task_waiting_maps_to_queued() {
        let (task, fc) = ff_public_state_to_task_state(PublicState::Waiting);
        assert_eq!(task, TaskState::Queued);
        assert!(fc.is_none());
    }

    #[test]
    fn task_active_maps_to_running() {
        let (task, fc) = ff_public_state_to_task_state(PublicState::Active);
        assert_eq!(task, TaskState::Running);
        assert!(fc.is_none());
    }

    #[test]
    fn task_completed_maps_to_completed() {
        let (task, fc) = ff_public_state_to_task_state(PublicState::Completed);
        assert_eq!(task, TaskState::Completed);
        assert!(fc.is_none());
    }

    #[test]
    fn task_expired_maps_to_failed_timed_out() {
        let (task, fc) = ff_public_state_to_task_state(PublicState::Expired);
        assert_eq!(task, TaskState::Failed);
        assert_eq!(fc, Some(FailureClass::TimedOut));
    }

    #[test]
    fn adjust_run_paused_to_waiting_approval() {
        let adjusted =
            adjust_run_state_for_blocking_reason(RunState::Paused, "waiting_for_approval");
        assert_eq!(adjusted, RunState::WaitingApproval);
    }

    #[test]
    fn adjust_run_paused_other_reason_stays_paused() {
        let adjusted = adjust_run_state_for_blocking_reason(RunState::Paused, "waiting_for_signal");
        assert_eq!(adjusted, RunState::Paused);
    }

    #[test]
    fn adjust_run_non_paused_unchanged() {
        let adjusted =
            adjust_run_state_for_blocking_reason(RunState::Running, "waiting_for_approval");
        assert_eq!(adjusted, RunState::Running);
    }

    #[test]
    fn adjust_task_paused_to_waiting_approval() {
        let adjusted =
            adjust_task_state_for_blocking_reason(TaskState::Paused, "waiting_for_approval");
        assert_eq!(adjusted, TaskState::WaitingApproval);
    }

    #[test]
    fn adjust_task_paused_other_reason_stays_paused() {
        let adjusted = adjust_task_state_for_blocking_reason(TaskState::Paused, "operator_hold");
        assert_eq!(adjusted, TaskState::Paused);
    }

    #[test]
    fn adjust_task_non_paused_unchanged() {
        let adjusted =
            adjust_task_state_for_blocking_reason(TaskState::Running, "waiting_for_approval");
        assert_eq!(adjusted, TaskState::Running);
    }

    #[test]
    fn adjust_run_empty_reason_stays_paused() {
        let adjusted = adjust_run_state_for_blocking_reason(RunState::Paused, "");
        assert_eq!(adjusted, RunState::Paused);
    }

    #[test]
    fn all_public_states_covered_for_runs() {
        let states = [
            PublicState::Waiting,
            PublicState::Delayed,
            PublicState::RateLimited,
            PublicState::WaitingChildren,
            PublicState::Active,
            PublicState::Suspended,
            PublicState::Completed,
            PublicState::Failed,
            PublicState::Cancelled,
            PublicState::Expired,
            PublicState::Skipped,
        ];
        for state in states {
            let (run_state, _) = ff_public_state_to_run_state(state);
            assert!(!format!("{run_state:?}").is_empty());
        }
    }

    #[test]
    fn all_public_states_covered_for_tasks() {
        let states = [
            PublicState::Waiting,
            PublicState::Delayed,
            PublicState::RateLimited,
            PublicState::WaitingChildren,
            PublicState::Active,
            PublicState::Suspended,
            PublicState::Completed,
            PublicState::Failed,
            PublicState::Cancelled,
            PublicState::Expired,
            PublicState::Skipped,
        ];
        for state in states {
            let (task_state, _) = ff_public_state_to_task_state(state);
            assert!(!format!("{task_state:?}").is_empty());
        }
    }

    #[test]
    fn inverse_running_maps_to_active() {
        let states = ff_run_state_to_public_states(RunState::Running);
        assert_eq!(states, &[PublicState::Active]);
    }

    #[test]
    fn inverse_pending_maps_to_waiting_delayed_rate_limited() {
        let states = ff_run_state_to_public_states(RunState::Pending);
        assert_eq!(states.len(), 3);
        assert!(states.contains(&PublicState::Waiting));
        assert!(states.contains(&PublicState::Delayed));
        assert!(states.contains(&PublicState::RateLimited));
    }

    #[test]
    fn inverse_waiting_approval_maps_to_suspended() {
        let states = ff_run_state_to_public_states(RunState::WaitingApproval);
        assert_eq!(states, &[PublicState::Suspended]);
    }

    #[test]
    fn inverse_failed_includes_expired_and_skipped() {
        let states = ff_run_state_to_public_states(RunState::Failed);
        assert!(states.contains(&PublicState::Failed));
        assert!(states.contains(&PublicState::Expired));
        assert!(states.contains(&PublicState::Skipped));
    }

    #[test]
    fn inverse_canceled_maps_to_cancelled() {
        let states = ff_run_state_to_public_states(RunState::Canceled);
        assert_eq!(states, &[PublicState::Cancelled]);
    }

    #[test]
    fn inverse_task_queued_maps_to_waiting_variants() {
        let states = ff_task_state_to_public_states(TaskState::Queued);
        assert_eq!(states.len(), 3);
        assert!(states.contains(&PublicState::Waiting));
    }

    #[test]
    fn inverse_task_running_maps_to_active() {
        let states = ff_task_state_to_public_states(TaskState::Running);
        assert_eq!(states, &[PublicState::Active]);
    }

    #[test]
    fn inverse_all_run_states_return_nonempty() {
        let states = [
            RunState::Pending,
            RunState::Running,
            RunState::WaitingApproval,
            RunState::Paused,
            RunState::WaitingDependency,
            RunState::Completed,
            RunState::Failed,
            RunState::Canceled,
        ];
        for s in states {
            assert!(
                !ff_run_state_to_public_states(s).is_empty(),
                "{s:?} returned empty"
            );
        }
    }

    #[test]
    fn inverse_all_task_states_return_nonempty() {
        let states = [
            TaskState::Queued,
            TaskState::Leased,
            TaskState::Running,
            TaskState::WaitingApproval,
            TaskState::Paused,
            TaskState::WaitingDependency,
            TaskState::RetryableFailed,
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Canceled,
            TaskState::DeadLettered,
        ];
        for s in states {
            assert!(
                !ff_task_state_to_public_states(s).is_empty(),
                "{s:?} returned empty"
            );
        }
    }
}
