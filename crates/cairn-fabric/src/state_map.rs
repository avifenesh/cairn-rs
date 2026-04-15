use cairn_domain::lifecycle::{FailureClass, RunState, TaskState};
use ff_core::state::PublicState;

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

// Coupled to FF Lua's REASON_TO_BLOCKING table in helpers.lua. If cairn adds
// new pause reasons that map to new blocking_reason strings, add entries here.
pub fn adjust_run_state_for_blocking_reason(state: RunState, blocking_reason: &str) -> RunState {
    if state == RunState::Paused && blocking_reason == "waiting_for_approval" {
        RunState::WaitingApproval
    } else {
        state
    }
}

pub fn adjust_task_state_for_blocking_reason(state: TaskState, blocking_reason: &str) -> TaskState {
    if state == TaskState::Paused && blocking_reason == "waiting_for_approval" {
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
}
