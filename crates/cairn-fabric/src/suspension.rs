//! Suspension helpers — typed path to FF 0.10 `EngineBackend::suspend`
//! and `EngineBackend::suspend_by_triple`.
//!
//! Two surfaces live here:
//!
//! 1. **Worker-side 4-tuple helpers** ([`typed_approval`],
//!    [`typed_subagent`], [`typed_tool_result`], [`typed_signal`]).
//!    Each returns the `(SuspensionReasonCode, ResumeCondition,
//!    Option<(TimestampMs, TimeoutBehavior)>, ResumePolicy)` tuple
//!    that `flowfabric::sdk::task::ClaimedTask::suspend` consumes.
//!    CG-b wired worker_sdk onto these.
//! 2. **Service-side [`SuspendArgs`] builders**
//!    ([`build_suspend_args`] + the [`SuspendCase`] variants). CG-c
//!    adopted FF#322 — the service-layer pause / waiting-approval /
//!    policy-hold paths now build a typed [`SuspendArgs`] +
//!    [`LeaseFence`] triple and call
//!    `EngineBackend::suspend_by_triple` directly. No Lua-ARGV
//!    translation lives in cairn anymore; every suspension shape is
//!    expressed as typed fields the FF trait validates on its side.

use flowfabric::core::contracts::{
    CompositeBody, CountKind, ResumeCondition, ResumePolicy, SignalMatcher, SuspendArgs,
    SuspensionReasonCode, SuspensionRequester, TimeoutBehavior, WaitpointBinding,
};
use flowfabric::core::types::{
    AttemptId, ExecutionId, LeaseEpoch, LeaseFence, LeaseId, SuspensionId, TimestampMs,
    WaitpointId,
};

use crate::engine::ExecutionLeaseContext;
use crate::error::FabricError;
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
/// valid future timestamp (no wrap, no debug panic).
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

/// Build the approval-suspend `ResumeCondition` — any signal
/// delivered to the single bound waitpoint resumes.
///
/// `waitpoint_key` must match the `WaitpointBinding::Fresh.waitpoint_key`
/// the caller passes to `SuspendArgs::new`. The `Single { matcher:
/// Wildcard }` shape tells the Lua evaluator to accept any signal
/// name — and the `resolve_approval` / `resolve_task_approval` callers
/// only ever deliver `approval_granted:<id>` or `approval_rejected:<id>`
/// to this waitpoint, so the functional semantic (first approval
/// signal resumes) is preserved without a typed composite that would
/// need to carry both signal-name alternatives.
///
/// Why not `Composite(Count { matcher: ByName(...) })`: the v0.10 Lua
/// evaluator's `matcher_accepts` compares a single `SignalMatcher`
/// against the incoming signal name — it does not support an
/// "either-of-two-names" matcher at the Count node. The old Lua-glue
/// path expressed this via `required_signal_names: [granted, rejected]`
/// which was a bespoke v0.9-era `signal_set` condition; the typed
/// trait has no direct equivalent. Wildcard + caller-controlled
/// delivery set is the idiomatic translation.
///
/// Shared between the worker `suspend_for_approval` path and the
/// service-layer `enter_waiting_approval` path.
fn approval_resume_condition(waitpoint_key: &str) -> ResumeCondition {
    ResumeCondition::Single {
        waitpoint_key: waitpoint_key.to_owned(),
        matcher: SignalMatcher::Wildcard,
    }
}

/// Typed 4-tuple for approval suspension. The worker_sdk path binds
/// its own waitpoint via `ClaimedTask::suspend(...)`, which mints the
/// `WaitpointBinding::Fresh` internally; the resume condition reads
/// the waitpoint_key from `primary().waitpoint_key()` at the Lua
/// layer, so we cannot materialise the `Single { waitpoint_key }`
/// shape here without knowing the binding. Keep the worker helper on
/// the composite `Count { DistinctWaitpoints }` shape it has used
/// since CG-b — the worker-side `ClaimedTask::suspend` converts this
/// into the v0.10 bindings internally (see flowfabric::sdk::task).
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
pub(crate) fn typed_subagent(child_task_id: &str, deadline_ms: Option<u64>) -> TypedSuspendArgs {
    let safe_id = sanitize_signal_component(child_task_id);
    let waitpoint = format!("child_completed:{safe_id}");
    let cond = ResumeCondition::Single {
        waitpoint_key: waitpoint.clone(),
        matcher: SignalMatcher::ByName(waitpoint),
    };
    (
        SuspensionReasonCode::WaitingForSignal,
        cond,
        compute_timeout(deadline_ms, TimeoutBehavior::Fail),
        ResumePolicy::normal(),
    )
}

/// Typed 4-tuple for tool-result suspension: `tool_result:<id>`
/// resumes.
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
#[allow(dead_code)] // Wired up when worker-side signal patterns land.
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

// ─── Service-side SuspendArgs / LeaseFence builders (CG-c / FF#322) ─────

/// Every service-layer suspend call site — the three inside
/// [`FabricRunService::pause`] / [`FabricRunService::enter_waiting_approval`]
/// / [`FabricTaskService::pause`] — expresses its intent as one of
/// these typed variants. [`build_suspend_args`] turns the variant into
/// a full [`SuspendArgs`] the FF trait accepts.
///
/// Each variant captures the semantic (reason code, resume condition,
/// timeout behaviour), not a wire-level signal-set shape. No
/// `resume_condition_json` or `resume_policy_json` string is built on
/// cairn's side.
pub(crate) enum SuspendCase<'a> {
    /// Operator-initiated pause. Only an explicit operator resume
    /// closes the waitpoint. `TimeoutBehavior::Escalate` preserves
    /// cairn's pre-0.10 "unbounded hold, escalate if deadline trips"
    /// semantic (operator-hold never sets a deadline in practice, but
    /// the behaviour field is recorded for parity).
    OperatorPause,
    /// Tool requested suspension — worker called
    /// `runtime.suspend_for_tool_result`, or the orchestrator decided
    /// to externalise a tool invocation. Resume on
    /// `tool_result:<invocation_id>` signal.
    ToolRequestedSuspension {
        invocation_id: &'a str,
        resume_after_ms: Option<u64>,
    },
    /// Worker-driven generic signal suspension. Resume on
    /// `<signal_name>` (already sanitized by the caller or, when not,
    /// defence-in-depth re-sanitized here).
    RuntimeSuspension {
        signal_name: &'a str,
        resume_after_ms: Option<u64>,
    },
    /// Policy gate held the execution. Resume on
    /// `policy_resolved:<detail>`.
    PolicyHold {
        detail: &'a str,
        resume_after_ms: Option<u64>,
    },
    /// Service-layer approval suspension (pre-claim gate). Resume on
    /// any signal delivered to the bound waitpoint — see
    /// [`approval_resume_condition`] for the Wildcard rationale.
    /// `approval_id` is the scope identifier (run_id or task_id)
    /// used to construct the correlated `approval_granted:<id>` /
    /// `approval_rejected:<id>` signal names on the delivery side.
    /// Currently unused inside the typed resume condition (the
    /// condition targets the bound waitpoint_key, not a signal name)
    /// but kept on the variant for parity with the caller-side shape
    /// and for debug/trace output.
    WaitingForApproval {
        #[allow(dead_code)] // see doc-comment above
        approval_id: &'a str,
        timeout_ms: Option<u64>,
    },
}

impl SuspendCase<'_> {
    /// The `SuspensionReasonCode` stamped on the execution — read back
    /// by projection paths to render the pause reason to operators.
    fn reason_code(&self) -> SuspensionReasonCode {
        match self {
            Self::OperatorPause => SuspensionReasonCode::ManualPause,
            Self::ToolRequestedSuspension { .. } => SuspensionReasonCode::WaitingForToolResult,
            Self::RuntimeSuspension { .. } => SuspensionReasonCode::WaitingForSignal,
            Self::PolicyHold { .. } => SuspensionReasonCode::PausedByPolicy,
            Self::WaitingForApproval { .. } => SuspensionReasonCode::WaitingForApproval,
        }
    }

    /// The typed resume condition FF evaluates against delivered
    /// signals. Takes the bound waitpoint's key because approval
    /// conditions (which resume on an unconstrained signal set)
    /// reference the waitpoint directly rather than a signal name.
    fn resume_condition(&self, waitpoint_key: &str) -> ResumeCondition {
        match self {
            Self::OperatorPause => ResumeCondition::OperatorOnly,
            Self::ToolRequestedSuspension { invocation_id, .. } => {
                let safe = sanitize_signal_component(invocation_id);
                let signal = format!("tool_result:{safe}");
                ResumeCondition::Single {
                    waitpoint_key: waitpoint_key.to_owned(),
                    matcher: SignalMatcher::ByName(signal),
                }
            }
            Self::RuntimeSuspension { signal_name, .. } => {
                let safe = sanitize_signal_component(signal_name);
                ResumeCondition::Single {
                    waitpoint_key: waitpoint_key.to_owned(),
                    matcher: SignalMatcher::ByName(safe),
                }
            }
            Self::PolicyHold { detail, .. } => {
                let safe = sanitize_signal_component(detail);
                let signal = format!("policy_resolved:{safe}");
                ResumeCondition::Single {
                    waitpoint_key: waitpoint_key.to_owned(),
                    matcher: SignalMatcher::ByName(signal),
                }
            }
            Self::WaitingForApproval { .. } => approval_resume_condition(waitpoint_key),
        }
    }

    /// Timeout + behaviour pair. `None` means "no deadline — runs
    /// indefinitely".
    fn timeout(&self) -> Option<(TimestampMs, TimeoutBehavior)> {
        let (ms, behavior) = match self {
            // Operator holds don't expire; an operator must act.
            Self::OperatorPause => (None, TimeoutBehavior::Escalate),
            // Tool suspensions silently expire — the orchestrator's
            // next turn decides what to do with a timed-out invocation.
            Self::ToolRequestedSuspension {
                resume_after_ms, ..
            } => (*resume_after_ms, TimeoutBehavior::Expire),
            // Generic signal suspensions fail the run if the signal
            // never arrives before the deadline.
            Self::RuntimeSuspension {
                resume_after_ms, ..
            } => (*resume_after_ms, TimeoutBehavior::Fail),
            // Policy holds are escalated to operator review — they
            // typically need human adjudication, not auto-fail.
            Self::PolicyHold {
                resume_after_ms, ..
            } => (*resume_after_ms, TimeoutBehavior::Fail),
            // Approval timeouts escalate to operator review.
            Self::WaitingForApproval { timeout_ms, .. } => (*timeout_ms, TimeoutBehavior::Escalate),
        };
        compute_timeout(ms, behavior)
    }
}

/// Build a complete [`SuspendArgs`] for a service-layer suspension.
///
/// Every service-layer call site goes through this builder + a
/// [`build_lease_fence`] call, so the `SuspensionRequester::Operator`
/// stamp + `WaitpointBinding::fresh()` choice are expressed in one
/// place. Worker-originated suspensions continue to ride the SDK
/// `task.suspend(…)` path with `SuspensionRequester::Worker`.
pub(crate) fn build_suspend_args(case: SuspendCase<'_>) -> SuspendArgs {
    // Mint the waitpoint_id / waitpoint_key up front so the typed
    // ResumeCondition can reference the exact key the binding will
    // carry. `WaitpointBinding::fresh()` would generate its own
    // uuid — we replicate its shape manually to keep the binding and
    // the condition in sync.
    let waitpoint_id = WaitpointId::new();
    let waitpoint_key = format!("wpk:{waitpoint_id}");
    let binding = WaitpointBinding::Fresh {
        waitpoint_id,
        waitpoint_key: waitpoint_key.clone(),
    };

    let resume_condition = case.resume_condition(&waitpoint_key);
    let reason_code = case.reason_code();
    let timeout = case.timeout();

    let mut args = SuspendArgs::new(
        SuspensionId::new(),
        binding,
        resume_condition,
        ResumePolicy::normal(),
        reason_code,
        TimestampMs::now(),
    )
    .with_requester(SuspensionRequester::Operator);
    if let Some((at, behavior)) = timeout {
        args = args.with_timeout(at, behavior);
    }
    args
}

/// Promote cairn's string-form [`ExecutionLeaseContext`] to a typed
/// [`LeaseFence`] triple for [`EngineBackend::suspend_by_triple`].
///
/// The legacy Lua-glue path accepted an unfenced
/// (`source = "operator_override"`, all three tokens empty)
/// [`ExecutionLeaseContext`]. The typed FF 0.10 trait surface has no
/// such override — a suspension without an active lease is a
/// contract violation, and we surface that as
/// [`FabricError::Validation`] rather than silently reaching a Lua
/// branch that bypasses the fence gate. Callers that hit this are
/// asking FF to suspend an execution whose current lease is gone
/// (never claimed, terminal, or reclaimed); the right thing is to
/// read-and-retry or reject at the API layer.
pub(crate) fn build_lease_fence(
    lease: &ExecutionLeaseContext,
) -> Result<LeaseFence, FabricError> {
    if lease.source == "operator_override"
        || lease.lease_id.is_empty()
        || lease.lease_epoch.is_empty()
        || lease.attempt_id.is_empty()
    {
        // Shape the error message as `ff_suspend_execution rejected:
        // fence_required` so the cairn-app `fabric_adapter`
        // `is_suspend_state_conflict` classifier maps it to
        // `RuntimeError::InvalidTransition` → HTTP 409 (not 500, not
        // 422). This is the operator-visible state conflict a pending-
        // run pause hits: the run has no lease yet, so there is no
        // fence triple to build.
        //
        // FF 0.10's `suspend_by_triple` would return the same
        // `fence_required` code if we passed a nil lease triple into
        // the FCALL; we short-circuit here (no round-trip to Valkey)
        // because the missing-lease check is cheap and local. Keeping
        // the wire-string shape identical means the HTTP layer
        // classifier does not need a second code path.
        return Err(FabricError::Internal(
            "ff_suspend_execution rejected: fence_required".to_owned(),
        ));
    }
    let lease_id = LeaseId::parse(&lease.lease_id).map_err(|e| FabricError::Validation {
        reason: format!("suspend_by_triple: malformed lease_id {:?}: {e}", lease.lease_id),
    })?;
    let lease_epoch_u64: u64 =
        lease.lease_epoch.parse().map_err(|e| FabricError::Validation {
            reason: format!(
                "suspend_by_triple: malformed lease_epoch {:?}: {e}",
                lease.lease_epoch
            ),
        })?;
    let attempt_id = AttemptId::parse(&lease.attempt_id).map_err(|e| FabricError::Validation {
        reason: format!(
            "suspend_by_triple: malformed attempt_id {:?}: {e}",
            lease.attempt_id
        ),
    })?;
    Ok(LeaseFence {
        lease_id,
        lease_epoch: LeaseEpoch::new(lease_epoch_u64),
        attempt_id,
    })
}

/// Invoke `EngineBackend::suspend_by_triple` and map its error +
/// outcome to cairn's layer. Service call sites discard the
/// `SuspendOutcome` (they re-read the execution snapshot to recover
/// the post-commit state — the outcome's `handle` is not lease-bearing
/// from cairn's perspective because cairn does not cache handles at
/// the service layer).
pub(crate) async fn suspend_by_triple(
    backend: &std::sync::Arc<dyn flowfabric::core::engine_backend::EngineBackend>,
    exec_id: ExecutionId,
    fence: LeaseFence,
    args: SuspendArgs,
) -> Result<(), FabricError> {
    let _outcome: flowfabric::core::contracts::SuspendOutcome = backend
        .suspend_by_triple(exec_id, fence, args)
        .await
        .map_err(|e| FabricError::Engine(Box::new(e)))?;
    // Dropping the `SuspendOutcome::handle` is intentional: the
    // service-layer caller reads the post-suspend snapshot to build
    // its `RunRecord` / `TaskRecord`, so the handle has no ongoing
    // consumer at this layer. Handle retention at the service layer
    // would imply cairn owns the lease, which is the worker-sdk path
    // (CG-b), not this one.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_approval_maps_reason_and_composite() {
        let (reason, cond, timeout, _policy) = typed_approval("appr_7", Some(45_000));
        assert_eq!(reason, SuspensionReasonCode::WaitingForApproval);
        match cond {
            ResumeCondition::Composite(body) => {
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
                assert_eq!(waitpoint_key, "custom_signal_x");
            }
            other => panic!("expected Single for generic signal, got {other:?}"),
        }
    }

    #[test]
    fn typed_timeout_saturates_on_u64_max() {
        let (_, _, timeout, _) = typed_tool_result("inv_big", Some(u64::MAX));
        let (at, _behavior) = timeout.expect("timeout_ms supplied");
        assert!(at.0 > 0, "saturating add must not wrap negative");
    }

    // ─── SuspendCase / build_suspend_args tests (new in CG-c) ────────────

    #[test]
    fn service_operator_pause_is_operator_only() {
        let args = build_suspend_args(SuspendCase::OperatorPause);
        assert_eq!(args.reason_code, SuspensionReasonCode::ManualPause);
        assert!(matches!(args.resume_condition, ResumeCondition::OperatorOnly));
        assert!(args.timeout_at.is_none(), "operator holds never expire");
        assert_eq!(args.requested_by, SuspensionRequester::Operator);
    }

    /// Helper: pull the bound waitpoint_key from a built SuspendArgs
    /// so we can cross-check the ResumeCondition references the same
    /// string.
    fn bound_wp_key(args: &SuspendArgs) -> String {
        match args.primary() {
            WaitpointBinding::Fresh { waitpoint_key, .. } => waitpoint_key.clone(),
            other => panic!("expected Fresh binding, got {other:?}"),
        }
    }

    #[test]
    fn service_tool_requested_suspension_targets_bound_waitpoint() {
        let args = build_suspend_args(SuspendCase::ToolRequestedSuspension {
            invocation_id: "inv_7",
            resume_after_ms: Some(30_000),
        });
        assert_eq!(args.reason_code, SuspensionReasonCode::WaitingForToolResult);
        let wp = bound_wp_key(&args);
        match &args.resume_condition {
            ResumeCondition::Single { waitpoint_key, matcher } => {
                assert_eq!(waitpoint_key, &wp);
                match matcher {
                    SignalMatcher::ByName(name) => assert_eq!(name, "tool_result:inv_7"),
                    other => panic!("expected ByName matcher, got {other:?}"),
                }
            }
            other => panic!("expected Single, got {other:?}"),
        }
        assert_eq!(args.timeout_behavior, TimeoutBehavior::Expire);
    }

    #[test]
    fn service_runtime_suspension_sanitizes_signal_name() {
        let args = build_suspend_args(SuspendCase::RuntimeSuspension {
            signal_name: "my_signal",
            resume_after_ms: None,
        });
        assert_eq!(args.reason_code, SuspensionReasonCode::WaitingForSignal);
        let wp = bound_wp_key(&args);
        match &args.resume_condition {
            ResumeCondition::Single { waitpoint_key, matcher } => {
                assert_eq!(waitpoint_key, &wp);
                assert!(matches!(matcher, SignalMatcher::ByName(n) if n == "my_signal"));
            }
            other => panic!("expected Single, got {other:?}"),
        }
        assert!(args.timeout_at.is_none());
    }

    #[test]
    fn service_policy_hold_uses_policy_resolved_prefix() {
        let args = build_suspend_args(SuspendCase::PolicyHold {
            detail: "budget",
            resume_after_ms: None,
        });
        assert_eq!(args.reason_code, SuspensionReasonCode::PausedByPolicy);
        let wp = bound_wp_key(&args);
        match &args.resume_condition {
            ResumeCondition::Single { waitpoint_key, matcher } => {
                assert_eq!(waitpoint_key, &wp);
                assert!(matches!(matcher, SignalMatcher::ByName(n) if n == "policy_resolved:budget"));
            }
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn service_waiting_for_approval_wildcard_single() {
        let args = build_suspend_args(SuspendCase::WaitingForApproval {
            approval_id: "run_7",
            timeout_ms: Some(60_000),
        });
        assert_eq!(args.reason_code, SuspensionReasonCode::WaitingForApproval);
        let wp = bound_wp_key(&args);
        match &args.resume_condition {
            ResumeCondition::Single { waitpoint_key, matcher } => {
                assert_eq!(waitpoint_key, &wp);
                assert!(matches!(matcher, SignalMatcher::Wildcard));
            }
            other => panic!("expected Single, got {other:?}"),
        }
    }

    #[test]
    fn lease_fence_rejects_operator_override_as_fence_required() {
        // Unfenced context (no live lease) must surface a `fence_required`
        // error in the shape the cairn-app fabric_adapter classifier
        // recognises, so pause-on-pending-run returns HTTP 409 not 500.
        let ctx = ExecutionLeaseContext::unfenced(
            flowfabric::core::types::LaneId::new("cairn"),
            flowfabric::core::types::AttemptIndex::new(0),
        );
        let err = build_lease_fence(&ctx).unwrap_err();
        match err {
            FabricError::Internal(msg) => {
                assert!(
                    msg.starts_with("ff_suspend_execution rejected: "),
                    "expected ff_suspend_execution prefix: {msg}"
                );
                assert!(
                    msg.ends_with(": fence_required"),
                    "expected fence_required code: {msg}"
                );
            }
            other => panic!("expected Internal(fence_required), got {other:?}"),
        }
    }

    #[test]
    fn lease_fence_round_trip_from_populated_context() {
        let lease_id = flowfabric::core::types::LeaseId::new();
        let attempt_id = flowfabric::core::types::AttemptId::new();
        let ctx = ExecutionLeaseContext {
            lane_id: flowfabric::core::types::LaneId::new("cairn"),
            attempt_index: flowfabric::core::types::AttemptIndex::new(3),
            lease_id: lease_id.to_string(),
            lease_epoch: "7".to_owned(),
            attempt_id: attempt_id.to_string(),
            worker_instance_id: flowfabric::core::types::WorkerInstanceId::new("instance-a"),
            source: String::new(),
        };
        let fence = build_lease_fence(&ctx).expect("populated context yields LeaseFence");
        assert_eq!(fence.lease_id, lease_id);
        assert_eq!(fence.lease_epoch.0, 7);
        assert_eq!(fence.attempt_id, attempt_id);
    }

    #[test]
    fn lease_fence_rejects_malformed_epoch() {
        let ctx = ExecutionLeaseContext {
            lane_id: flowfabric::core::types::LaneId::new("cairn"),
            attempt_index: flowfabric::core::types::AttemptIndex::new(0),
            lease_id: flowfabric::core::types::LeaseId::new().to_string(),
            lease_epoch: "not-a-number".to_owned(),
            attempt_id: flowfabric::core::types::AttemptId::new().to_string(),
            worker_instance_id: flowfabric::core::types::WorkerInstanceId::new("w"),
            source: String::new(),
        };
        let err = build_lease_fence(&ctx).unwrap_err();
        assert!(matches!(err, FabricError::Validation { .. }));
    }
}
