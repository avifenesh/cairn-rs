//! Suspension helpers.
//!
//! Two sets of helpers live here:
//!
//! 1. **Typed 4-tuple helpers** (worker_sdk, CG-b and later):
//!    [`typed_approval`], [`typed_subagent`], [`typed_tool_result`],
//!    [`typed_signal`]. Each returns the exact 4-tuple
//!    `(SuspensionReasonCode, ResumeCondition,
//!    Option<(TimestampMs, TimeoutBehavior)>, ResumePolicy)` expected
//!    by `flowfabric::sdk::task::ClaimedTask::suspend`. No Lua glue,
//!    no intermediate struct. This is the path CG-b migrates
//!    worker_sdk onto.
//!
//! 2. **LEGACY [`SuspensionParams`] / [`ConditionMatcher`] factories**
//!    ([`for_approval`], [`for_subagent`], [`for_tool_result`],
//!    [`for_operator_hold`]). Used only by the service-layer suspend
//!    paths in `services/run_service.rs` and
//!    `services/task_service.rs`, which still go through the
//!    Lua-glue `build_suspend_input` + FCALL path because those
//!    callers hold a `LeaseFencingTriple`, not a `Handle`. Removal
//!    is blocked on FF upstream issue
//!    <https://github.com/avifenesh/FlowFabric/issues/322>
//!    (`EngineBackend::suspend_by_triple` typed trait method). Once
//!    that lands, CG-c migrates the 3 service-layer call sites onto
//!    the typed path and this legacy surface is deleted.

use flowfabric::core::contracts::{
    CompositeBody, CountKind, ResumeCondition, ResumePolicy, SignalMatcher, SuspensionReasonCode,
};
use flowfabric::core::types::TimestampMs;
use flowfabric::sdk::task::TimeoutBehavior;

use crate::helpers::sanitize_signal_component;

/// The 4-tuple shape consumed by
/// `flowfabric::sdk::task::ClaimedTask::suspend`.
///
/// Returned by the `typed_*` helpers in this module. Worker-side
/// `suspend_for_*` methods destructure this and call
/// `task.suspend(reason, cond, timeout, policy)` directly.
pub(crate) type TypedSuspendArgs = (
    SuspensionReasonCode,
    ResumeCondition,
    Option<(TimestampMs, TimeoutBehavior)>,
    ResumePolicy,
);

/// Compute the `Option<(TimestampMs, TimeoutBehavior)>` deadline for
/// a typed suspend call, using saturating arithmetic.
///
/// `u64 → i64` conversion clamps to `i64::MAX` via `.min(i64::MAX as
/// u64) as i64` and `.saturating_add` guarantees the result is a
/// valid future timestamp (no wrap, no debug panic). Matches the
/// arithmetic previously in `cg_a_suspend`.
fn compute_timeout(
    timeout_ms: Option<u64>,
    behavior: TimeoutBehavior,
) -> Option<(TimestampMs, TimeoutBehavior)> {
    timeout_ms.map(|ms| {
        let ms_i64 = ms.min(i64::MAX as u64) as i64;
        let deadline = TimestampMs::now().0.saturating_add(ms_i64);
        (TimestampMs::from_millis(deadline), behavior)
    })
}

/// Typed 4-tuple for approval suspension: `approval_granted:<id>` OR
/// `approval_rejected:<id>` resumes.
///
/// Uses a composite `DistinctWaitpoints n=1` condition so either
/// waitpoint satisfies. `ResumePolicy::normal()` matches cairn's
/// pre-0.9 default (consume matched signals, close waitpoint on
/// resume). `TimeoutBehavior::Escalate` preserves cairn's pre-0.9
/// "timeout bumps to operator review, does not auto-fail" semantic.
pub(crate) fn typed_approval(approval_id: &str, timeout_ms: Option<u64>) -> TypedSuspendArgs {
    let safe_id = sanitize_signal_component(approval_id);
    let granted = format!("approval_granted:{safe_id}");
    let rejected = format!("approval_rejected:{safe_id}");
    let cond = ResumeCondition::Composite(CompositeBody::count(
        1,
        CountKind::DistinctWaitpoints,
        None,
        vec![granted, rejected],
    ));
    (
        SuspensionReasonCode::WaitingForApproval,
        cond,
        compute_timeout(timeout_ms, TimeoutBehavior::Escalate),
        ResumePolicy::normal(),
    )
}

/// Typed 4-tuple for subagent suspension: `child_completed:<id>`
/// resumes.
///
/// `TimeoutBehavior::Fail` preserves cairn's pre-0.9 semantic that
/// an unbounded child wait that trips the deadline fails the parent.
pub(crate) fn typed_subagent(child_task_id: &str, deadline_ms: Option<u64>) -> TypedSuspendArgs {
    let safe_id = sanitize_signal_component(child_task_id);
    let waitpoint = format!("child_completed:{safe_id}");
    let cond = ResumeCondition::Single {
        waitpoint_key: waitpoint.clone(),
        matcher: SignalMatcher::ByName(waitpoint),
    };
    (
        // `waiting_for_children` is the FF blocking-reason string used
        // by the legacy path; on the typed surface the closest
        // upstream variant is `WaitingForSignal` (FF treats multi-
        // child satisfaction as a signal-count condition). Matches
        // the pre-existing `parse_reason` mapping in the worker_sdk
        // CG-a bridge.
        SuspensionReasonCode::WaitingForSignal,
        cond,
        compute_timeout(deadline_ms, TimeoutBehavior::Fail),
        ResumePolicy::normal(),
    )
}

/// Typed 4-tuple for tool-result suspension: `tool_result:<id>`
/// resumes.
///
/// `TimeoutBehavior::Expire` preserves cairn's pre-0.9 semantic that
/// a tool that never returns times out silently (does not fail the
/// run; the orchestrator decides what to do next based on the
/// timeout event).
pub(crate) fn typed_tool_result(invocation_id: &str, timeout_ms: Option<u64>) -> TypedSuspendArgs {
    let safe_id = sanitize_signal_component(invocation_id);
    let waitpoint = format!("tool_result:{safe_id}");
    let cond = ResumeCondition::Single {
        waitpoint_key: waitpoint.clone(),
        matcher: SignalMatcher::ByName(waitpoint),
    };
    (
        SuspensionReasonCode::WaitingForToolResult,
        cond,
        compute_timeout(timeout_ms, TimeoutBehavior::Expire),
        ResumePolicy::normal(),
    )
}

/// Typed 4-tuple for a generic signal suspension: `<signal_name>`
/// resumes.
///
/// Exposed for worker-side patterns that need to suspend on a single
/// caller-supplied signal with no ID-format assumptions. The caller
/// is responsible for passing an already-sanitized name; we still
/// run it through `sanitize_signal_component` as a defence-in-depth
/// measure (idempotent on already-clean input).
#[allow(dead_code)] // Wired up in CG-c when worker-side signal patterns land; kept
                    // public-in-crate so the typed path is complete on this PR.
pub(crate) fn typed_signal(signal_name: &str, timeout_ms: Option<u64>) -> TypedSuspendArgs {
    let safe = sanitize_signal_component(signal_name);
    let cond = ResumeCondition::Single {
        waitpoint_key: safe.clone(),
        matcher: SignalMatcher::ByName(safe),
    };
    (
        SuspensionReasonCode::WaitingForSignal,
        cond,
        compute_timeout(timeout_ms, TimeoutBehavior::Fail),
        ResumePolicy::normal(),
    )
}

// ─── LEGACY surface ────────────────────────────────────────────────
//
// Service-layer Lua-glue path. Kept `pub(crate)` + LEGACY-annotated
// until FF issue #322 (`suspend_by_triple`) lands and CG-c migrates
// the 3 service-layer call sites onto the typed path.

/// LEGACY: cairn-side signal matcher descriptor. Used only by the
/// service-layer Lua-glue suspend path in `run_service.rs` and
/// `task_service.rs`. Removal blocked on FF#322.
#[derive(Clone, Debug)]
pub(crate) struct ConditionMatcher {
    pub signal_name: String,
}

/// LEGACY: intermediate suspension params struct for the service-
/// layer Lua-glue path. Used only by `build_suspend_input` in
/// `run_service.rs` and `task_service.rs`. Removal blocked on FF#322.
#[derive(Clone, Debug)]
pub(crate) struct SuspensionParams {
    pub reason_code: String,
    pub condition_matchers: Vec<ConditionMatcher>,
    pub timeout_ms: Option<u64>,
    pub timeout_behavior: TimeoutBehavior,
}

/// LEGACY: factory for the service-layer approval-suspend path.
/// Removal blocked on FF#322.
pub(crate) fn for_approval(approval_id: &str, timeout_ms: Option<u64>) -> SuspensionParams {
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

/// LEGACY: factory for the service-layer tool-result-suspend path.
/// Removal blocked on FF#322.
pub(crate) fn for_tool_result(invocation_id: &str, timeout_ms: Option<u64>) -> SuspensionParams {
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

/// LEGACY: factory for the service-layer operator-hold path.
/// Removal blocked on FF#322.
pub(crate) fn for_operator_hold() -> SuspensionParams {
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

    // ─── legacy factory tests (unchanged from CG-a) ─────────────

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
    fn different_invocations_produce_different_signals() {
        let a = for_tool_result("inv_1", None);
        let b = for_tool_result("inv_2", None);
        assert_ne!(
            a.condition_matchers[0].signal_name,
            b.condition_matchers[0].signal_name
        );
    }

    // ─── typed 4-tuple helper tests (new in CG-b) ────────────────

    #[test]
    fn typed_approval_maps_reason_and_composite() {
        let (reason, cond, timeout, _policy) = typed_approval("appr_7", Some(45_000));
        assert_eq!(reason, SuspensionReasonCode::WaitingForApproval);
        // Composite n=1 DistinctWaitpoints over granted|rejected.
        match cond {
            ResumeCondition::Composite(body) => {
                // Either of the two waitpoints satisfies.
                let json =
                    serde_json::to_string(&body).expect("composite body serialises for inspection");
                assert!(
                    json.contains("approval_granted:appr_7"),
                    "composite includes granted waitpoint: {json}"
                );
                assert!(
                    json.contains("approval_rejected:appr_7"),
                    "composite includes rejected waitpoint: {json}"
                );
            }
            other => panic!("expected Composite for approval, got {other:?}"),
        }
        // Timeout present with Escalate behavior.
        let (_, behavior) = timeout.expect("timeout present when timeout_ms supplied");
        assert_eq!(behavior, TimeoutBehavior::Escalate);
    }

    #[test]
    fn typed_approval_without_timeout_is_none() {
        let (_, _, timeout, _) = typed_approval("appr_8", None);
        assert!(timeout.is_none());
    }

    #[test]
    fn typed_subagent_is_single_waitpoint_with_fail_timeout() {
        let (reason, cond, timeout, _policy) = typed_subagent("task_42", Some(10_000));
        // Subagent maps to WaitingForSignal on the typed surface
        // (see doc-comment on typed_subagent for rationale).
        assert_eq!(reason, SuspensionReasonCode::WaitingForSignal);
        match cond {
            ResumeCondition::Single { waitpoint_key, .. } => {
                assert_eq!(waitpoint_key, "child_completed:task_42");
            }
            other => panic!("expected Single for subagent, got {other:?}"),
        }
        let (_, behavior) = timeout.expect("deadline_ms supplied");
        assert_eq!(behavior, TimeoutBehavior::Fail);
    }

    #[test]
    fn typed_tool_result_uses_expire_behavior() {
        let (reason, cond, timeout, _policy) = typed_tool_result("inv_9", Some(5_000));
        assert_eq!(reason, SuspensionReasonCode::WaitingForToolResult);
        match cond {
            ResumeCondition::Single { waitpoint_key, .. } => {
                assert_eq!(waitpoint_key, "tool_result:inv_9");
            }
            other => panic!("expected Single for tool_result, got {other:?}"),
        }
        let (_, behavior) = timeout.expect("timeout_ms supplied");
        assert_eq!(behavior, TimeoutBehavior::Expire);
    }

    #[test]
    fn typed_signal_passes_through_name() {
        let (reason, cond, _, _) = typed_signal("custom_signal_x", None);
        assert_eq!(reason, SuspensionReasonCode::WaitingForSignal);
        match cond {
            ResumeCondition::Single { waitpoint_key, .. } => {
                // sanitize_signal_component is idempotent on clean input.
                assert_eq!(waitpoint_key, "custom_signal_x");
            }
            other => panic!("expected Single for generic signal, got {other:?}"),
        }
    }

    #[test]
    fn typed_timeout_saturates_on_u64_max() {
        // Giant timeout must clamp rather than wrap or panic.
        let (_, _, timeout, _) = typed_tool_result("inv_big", Some(u64::MAX));
        let (at, _behavior) = timeout.expect("timeout_ms supplied");
        // Saturated to i64::MAX (approximately), not a negative wrap.
        assert!(at.0 > 0, "saturating add must not wrap negative");
    }
}
