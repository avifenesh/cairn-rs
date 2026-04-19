use ff_sdk::task::{ConditionMatcher, TimeoutBehavior};

use crate::helpers::sanitize_signal_component;

#[derive(Clone, Debug)]
pub struct SuspensionParams {
    pub reason_code: String,
    pub condition_matchers: Vec<ConditionMatcher>,
    pub timeout_ms: Option<u64>,
    pub timeout_behavior: TimeoutBehavior,
}

pub fn for_approval(approval_id: &str, timeout_ms: Option<u64>) -> SuspensionParams {
    let safe_id = sanitize_signal_component(approval_id);
    SuspensionParams {
        reason_code: crate::constants::BLOCKING_WAITING_FOR_APPROVAL.to_owned(),
        condition_matchers: vec![
            ConditionMatcher {
                signal_name: format!("approval_granted:{safe_id}"),
            },
            ConditionMatcher {
                signal_name: format!("approval_rejected:{safe_id}"),
            },
        ],
        timeout_ms,
        timeout_behavior: TimeoutBehavior::Escalate,
    }
}

pub fn for_subagent(child_task_id: &str, deadline_ms: Option<u64>) -> SuspensionParams {
    let safe_id = sanitize_signal_component(child_task_id);
    SuspensionParams {
        reason_code: "waiting_for_children".into(),
        condition_matchers: vec![ConditionMatcher {
            signal_name: format!("child_completed:{safe_id}"),
        }],
        timeout_ms: deadline_ms,
        timeout_behavior: TimeoutBehavior::Fail,
    }
}

pub fn for_tool_result(invocation_id: &str, timeout_ms: Option<u64>) -> SuspensionParams {
    let safe_id = sanitize_signal_component(invocation_id);
    SuspensionParams {
        reason_code: "waiting_for_tool_result".into(),
        condition_matchers: vec![ConditionMatcher {
            signal_name: format!("tool_result:{safe_id}"),
        }],
        timeout_ms,
        timeout_behavior: TimeoutBehavior::Expire,
    }
}

pub fn for_operator_hold() -> SuspensionParams {
    SuspensionParams {
        reason_code: "operator_hold".into(),
        condition_matchers: vec![ConditionMatcher {
            signal_name: "__cairn_operator_resume__".into(),
        }],
        timeout_ms: None,
        timeout_behavior: TimeoutBehavior::Escalate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_suspension_has_two_matchers() {
        let params = for_approval("appr_1", Some(60_000));
        assert_eq!(params.reason_code, "waiting_for_approval");
        assert_eq!(params.condition_matchers.len(), 2);
        assert_eq!(
            params.condition_matchers[0].signal_name,
            "approval_granted:appr_1"
        );
        assert_eq!(
            params.condition_matchers[1].signal_name,
            "approval_rejected:appr_1"
        );
        assert_eq!(params.timeout_ms, Some(60_000));
        assert_eq!(params.timeout_behavior, TimeoutBehavior::Escalate);
    }

    #[test]
    fn approval_without_timeout() {
        let params = for_approval("appr_2", None);
        assert!(params.timeout_ms.is_none());
        assert_eq!(params.timeout_behavior, TimeoutBehavior::Escalate);
    }

    #[test]
    fn subagent_suspension_encodes_child_id() {
        let params = for_subagent("task_child_1", Some(120_000));
        assert_eq!(params.reason_code, "waiting_for_children");
        assert_eq!(params.condition_matchers.len(), 1);
        assert_eq!(
            params.condition_matchers[0].signal_name,
            "child_completed:task_child_1"
        );
        assert_eq!(params.timeout_ms, Some(120_000));
        assert_eq!(params.timeout_behavior, TimeoutBehavior::Fail);
    }

    #[test]
    fn subagent_without_deadline() {
        let params = for_subagent("task_99", None);
        assert!(params.timeout_ms.is_none());
        assert_eq!(params.timeout_behavior, TimeoutBehavior::Fail);
    }

    #[test]
    fn tool_result_suspension_encodes_invocation_id() {
        let params = for_tool_result("inv_42", Some(30_000));
        assert_eq!(params.reason_code, "waiting_for_tool_result");
        assert_eq!(params.condition_matchers.len(), 1);
        assert_eq!(
            params.condition_matchers[0].signal_name,
            "tool_result:inv_42"
        );
        assert_eq!(params.timeout_ms, Some(30_000));
        assert_eq!(params.timeout_behavior, TimeoutBehavior::Expire);
    }

    #[test]
    fn tool_result_without_timeout() {
        let params = for_tool_result("inv_99", None);
        assert!(params.timeout_ms.is_none());
    }

    #[test]
    fn operator_hold_uses_sentinel_matcher() {
        let params = for_operator_hold();
        assert_eq!(params.reason_code, "operator_hold");
        assert_eq!(params.condition_matchers.len(), 1);
        assert_eq!(
            params.condition_matchers[0].signal_name,
            "__cairn_operator_resume__"
        );
        assert!(params.timeout_ms.is_none());
        assert_eq!(params.timeout_behavior, TimeoutBehavior::Escalate);
    }

    #[test]
    fn different_children_produce_different_signals() {
        let a = for_subagent("child_a", None);
        let b = for_subagent("child_b", None);
        assert_ne!(
            a.condition_matchers[0].signal_name,
            b.condition_matchers[0].signal_name
        );
    }

    #[test]
    fn different_invocations_produce_different_signals() {
        let a = for_tool_result("inv_1", None);
        let b = for_tool_result("inv_2", None);
        assert_ne!(
            a.condition_matchers[0].signal_name,
            b.condition_matchers[0].signal_name
        );
    }
}
