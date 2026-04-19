// Suspension / resume / signal-delivery integration tests.
//
// Covers FCALLs previously uncovered by the integration suite:
//   - ff_suspend_execution   (via tasks.pause, runs.enter_waiting_approval)
//   - ff_resume_execution    (via tasks.resume)
//   - ff_deliver_signal      (via runs.resolve_approval, signals.deliver_*)
//
// Runs against a testcontainers-provisioned Valkey (see tests/integration.rs).
// `TestHarness::setup()` issues FLUSHDB between tests so index zsets do not
// leak across runs — the earlier uuid-scoped ids stay only as defense in
// depth.

use std::collections::HashMap;
use std::time::Duration;

use cairn_domain::lifecycle::{
    PauseReason, PauseReasonKind, ResumeTrigger, TaskResumeTarget, TaskState,
};
use cairn_domain::policy::ApprovalDecision;
use cairn_domain::RuntimeEvent;
use cairn_store::event_log::EventLog;
use ff_core::keys::ExecKeyContext;
use ff_core::partition::execution_partition;
use ff_sdk::task::SignalOutcome;

use crate::TestHarness;

/// Wait for a `RuntimeEvent` matching `predicate` to land in the event
/// log, polling every 50ms up to `deadline`. Returns Err on timeout with
/// the last-observed event count. Mirrors the helper in
/// `test_event_emission.rs`; bridge emission is async (mpsc → consumer
/// → append), so a hard sleep is flaky.
async fn wait_for_event<F>(h: &TestHarness, deadline: Duration, predicate: F) -> Result<(), String>
where
    F: Fn(&RuntimeEvent) -> bool,
{
    let start = std::time::Instant::now();
    loop {
        let events = EventLog::read_stream(h.event_log.as_ref(), None, 10_000)
            .await
            .map_err(|e| format!("read_stream: {e}"))?;
        if events
            .iter()
            .any(|stored| predicate(&stored.envelope.payload))
        {
            return Ok(());
        }
        if start.elapsed() >= deadline {
            return Err(format!(
                "predicate not satisfied within {:?}; {} events observed",
                deadline,
                events.len()
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Read the FF `exec_core` hash for a run id directly from Valkey. Needed
/// for post-condition assertions that prove Valkey state, not just the
/// service's Rust return value. The field set comes from FF
/// `lua/suspension.lua:183-201` (ff_suspend_execution exec_core HSET) and
/// `lua/signal.lua:219-231` (ff_deliver_signal resume path).
///
/// Task execution id derivation is a private method on FabricTaskService,
/// so this helper only covers the run path. Task-side assertions go
/// through the service's `tasks.get` (which itself HGETALLs Valkey).
async fn read_exec_core_for_run(
    h: &TestHarness,
    run_id: &cairn_domain::RunId,
) -> HashMap<String, String> {
    let eid = cairn_fabric::id_map::run_to_execution_id(&h.project, run_id);
    let partition = execution_partition(&eid, &h.fabric.runtime.partition_config);
    let ctx = ExecKeyContext::new(&partition, &eid);
    let fields: HashMap<String, String> = h
        .fabric
        .runtime
        .client
        .hgetall(&ctx.core())
        .await
        .expect("HGETALL exec core failed");
    fields
}

/// Suspend + resume roundtrip: exercises `ff_suspend_execution` and
/// `ff_resume_execution` — the paired spine of every approval / timer /
/// signal-wait path. Reads the task state mid-flight to prove Valkey
/// persistence, not just the in-process return.
#[tokio::test]
async fn test_suspend_and_resume_roundtrip() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    h.fabric
        .tasks
        .claim(&h.project, &task_id, "test-worker".into(), 30_000)
        .await
        .expect("claim failed");

    let pause_reason = PauseReason {
        kind: PauseReasonKind::OperatorPause,
        detail: None,
        resume_after_ms: None,
        actor: Some("integration-test".into()),
    };

    let paused = h
        .fabric
        .tasks
        .pause(&h.project, &task_id, pause_reason)
        .await
        .expect("pause failed");

    // TODO(operator-hold-mapping): PauseReasonKind::OperatorPause maps to
    // `crate::suspension::for_operator_hold()` which uses reason_code
    // "operator_hold". FF's `map_reason_to_blocking` may route this to
    // either `operator_hold` or `waiting_for_approval`, which in turn maps
    // to TaskState::Paused vs WaitingApproval in
    // state_map::adjust_task_state_for_blocking_reason. Accepting either
    // keeps the test stable against FF-side reason_code drift. Style-only.
    assert!(
        matches!(paused.state, TaskState::Paused | TaskState::WaitingApproval),
        "expected Paused or WaitingApproval after pause, got {:?}",
        paused.state
    );

    // Verify Valkey persistence by reading the task state back through the
    // service. If Rust returned Paused but Valkey wasn't written, a
    // follow-up get would disagree.
    let mid = h
        .fabric
        .tasks
        .get(&h.project, &task_id)
        .await
        .expect("mid-flight get failed")
        .expect("task must be readable while paused");
    assert!(
        matches!(mid.state, TaskState::Paused | TaskState::WaitingApproval),
        "mid-flight get must also report Paused/WaitingApproval (Valkey persistence), got {:?}",
        mid.state
    );

    let resumed = h
        .fabric
        .tasks
        .resume(
            &h.project,
            &task_id,
            ResumeTrigger::OperatorResume,
            TaskResumeTarget::Running,
        )
        .await
        .expect("resume failed");

    // After resume the task must be in an actionable state (queued or
    // running). Accepting both because the delayed_promoter / claim
    // machinery can race the resume write.
    assert!(
        matches!(
            resumed.state,
            TaskState::Queued | TaskState::Leased | TaskState::Running
        ),
        "expected Queued/Leased/Running after resume (positive), got {:?}",
        resumed.state
    );

    h.teardown().await;
}

/// #2 from the coverage audit: `ff_deliver_signal` resumes a waiter.
///
/// **BLOCKED** — exercises `runs.enter_waiting_approval`, which calls
/// `ff_suspend_execution` on the run's execution. FF requires
/// `lifecycle_phase == "active"` (/tmp/FlowFabric/lua/suspension.lua:401),
/// but `FabricRunService` has no claim API — runs and tasks get distinct
/// execution IDs via `id_map`, so a task claim does NOT activate the run's
/// execution. Until we add `runs.claim` or change the architecture so the
/// orchestrator's task claim also activates the run's execution, this test
/// cannot pass against live FF.
///
/// Pausing via `tasks.pause(OperatorPause)` is covered by
/// `test_suspend_and_resume_roundtrip` — that exercises the same
/// `ff_suspend_execution` / `ff_resume_execution` contract.
#[tokio::test]
async fn test_signal_delivery_resumes_waiter() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    h.fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    // FF rejects suspend/signal FCALLs against a non-active execution.
    // runs.claim issues a grant + lease so lifecycle_phase flips to "active".
    h.fabric
        .runs
        .claim(&h.project, &run_id)
        .await
        .expect("runs.claim failed");

    h.fabric
        .runs
        .enter_waiting_approval(&h.project, &run_id)
        .await
        .expect("enter_waiting_approval failed");

    // Pre-condition: Valkey records the run as suspended with a waitpoint.
    // Per FF lua/suspension.lua:183-201, exec_core must have
    // public_state="suspended" and current_waitpoint_id set to a non-empty
    // value after ff_suspend_execution.
    let pre = read_exec_core_for_run(&h, &run_id).await;
    assert_eq!(
        pre.get("public_state").map(|s| s.as_str()),
        Some("suspended"),
        "after enter_waiting_approval, exec_core.public_state must be 'suspended', got {:?}",
        pre.get("public_state"),
    );
    assert!(
        pre.get("current_waitpoint_id")
            .map(|s| !s.is_empty())
            .unwrap_or(false),
        "after enter_waiting_approval, exec_core.current_waitpoint_id must be non-empty, got {:?}",
        pre.get("current_waitpoint_id"),
    );

    // Deliver the approval. ff_deliver_signal should match the
    // approval_granted:<run_id> matcher, close the waitpoint, and transition
    // the execution from suspended → runnable (see FF signal.lua:219-231).
    let resolved = h
        .fabric
        .runs
        .resolve_approval(&h.project, &run_id, ApprovalDecision::Approved)
        .await
        .expect("resolve_approval failed");
    assert_eq!(resolved.run_id, run_id);

    // Post-condition: Valkey records the run as resumed (not suspended) AND
    // current_waitpoint_id is cleared. Per FF signal.lua:228-229, the
    // resume path HSETs current_waitpoint_id="" and public_state to
    // "waiting" (resume_delay_ms=0) or "delayed" (resume_delay_ms>0).
    let post = read_exec_core_for_run(&h, &run_id).await;
    let post_state = post.get("public_state").cloned().unwrap_or_default();
    assert!(
        post_state != "suspended",
        "after resolve_approval, exec_core.public_state must NOT be 'suspended', got {:?}",
        post_state,
    );
    assert_eq!(
        post.get("current_waitpoint_id").map(|s| s.as_str()),
        Some(""),
        "after resolve_approval, exec_core.current_waitpoint_id must be cleared, got {:?}",
        post.get("current_waitpoint_id"),
    );

    h.teardown().await;
}

/// Dedup: goes directly through `SignalBridge` so the waitpoint id is
/// passed explicitly — `runs.resolve_approval` clears
/// `current_waitpoint_id` on the first call, so a second resolve would
/// error before hitting the idempotency guard.
///
/// Uses `deliver_tool_result_signal` because its signal_name
/// `tool_result:<inv_id>` does NOT match the approval waitpoint's
/// `approval_granted|rejected` matchers. The first delivery records a
/// no-op and the waitpoint stays open; the second delivery with the same
/// idempotency_key trips the SET NX in signal.lua and returns
/// `SignalOutcome::Duplicate`.
#[tokio::test]
async fn test_signal_delivery_is_idempotent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    h.fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    h.fabric
        .runs
        .claim(&h.project, &run_id)
        .await
        .expect("runs.claim failed");

    h.fabric
        .runs
        .enter_waiting_approval(&h.project, &run_id)
        .await
        .expect("enter_waiting_approval failed");

    // Read the active waitpoint id from exec_core (populated by
    // ff_suspend_execution at lua/suspension.lua:199).
    let core = read_exec_core_for_run(&h, &run_id).await;
    let wp_id_str = core
        .get("current_waitpoint_id")
        .cloned()
        .filter(|s| !s.is_empty())
        .expect("current_waitpoint_id must be set after enter_waiting_approval");
    let wp_id = ff_core::types::WaitpointId::parse(&wp_id_str).expect("waitpoint_id must parse");
    let eid = cairn_fabric::id_map::run_to_execution_id(&h.project, &run_id);

    let invocation_id = format!("inv_{}", uuid::Uuid::new_v4());

    // First delivery: records signal, no matcher hit, no_op. Waitpoint
    // stays open because `tool_result:<inv_id>` is not in the matcher set
    // for an approval waitpoint.
    let first = h
        .fabric
        .signals
        .deliver_tool_result_signal(&eid, &wp_id, &invocation_id, None)
        .await
        .expect("first tool_result signal delivery failed");
    assert!(
        matches!(first, SignalOutcome::Accepted { .. }),
        "first delivery must be Accepted (not Duplicate), got {:?}",
        first,
    );

    // Second delivery with SAME invocation_id → same idempotency_key
    // (`tool_result:<inv_id>`). FF signal.lua:117-124 reads the idem_key,
    // finds it present, returns `ok_duplicate(existing)` → parsed to
    // SignalOutcome::Duplicate by ff-sdk.
    let second = h
        .fabric
        .signals
        .deliver_tool_result_signal(&eid, &wp_id, &invocation_id, None)
        .await
        .expect("second tool_result signal delivery must return Duplicate, not error");
    assert!(
        matches!(second, SignalOutcome::Duplicate { .. }),
        "second delivery with same idempotency_key must be Duplicate, got {:?}",
        second,
    );

    h.teardown().await;
}

/// #3 from the coverage audit: probe the `already_satisfied` branch of
/// `ff_suspend_execution`.
///
/// ALREADY_SATISFIED is returned when `use_pending_waitpoint="1"` AND
/// buffered signals already satisfy the resume condition
/// (FF lua/suspension.lua:130-146). In that path, FF does NOT create a
/// new waitpoint — it CLOSES the pending one.
///
/// `enter_waiting_approval` always passes `use_pending_waitpoint=""`
/// (see run_service.rs:935, empty string = create new waitpoint). So
/// back-to-back `enter_waiting_approval` calls cannot hit ALREADY_SATISFIED
/// — the second call creates a FRESH waitpoint with a FRESH wp_id.
///
/// We therefore assert what IS observable: the second enter must succeed
/// AND produce a new current_waitpoint_id distinct from the first.
/// This catches regressions where the second suspend silently errors or
/// where FF leaks the closed waitpoint into current_waitpoint_id.
///
/// A true ALREADY_SATISFIED assertion requires pending-waitpoint
/// plumbing that cairn-fabric does not expose yet; flagged for a future
/// round alongside the pending-waitpoint builder work.
#[tokio::test]
async fn test_enter_approval_after_prior_approval_creates_fresh_waitpoint() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    h.fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    h.fabric
        .runs
        .claim(&h.project, &run_id)
        .await
        .expect("runs.claim failed");

    h.fabric
        .runs
        .enter_waiting_approval(&h.project, &run_id)
        .await
        .expect("first enter_waiting_approval failed");

    let first_wp = read_exec_core_for_run(&h, &run_id)
        .await
        .get("current_waitpoint_id")
        .cloned()
        .filter(|s| !s.is_empty())
        .expect("first enter must set current_waitpoint_id");

    h.fabric
        .runs
        .resolve_approval(&h.project, &run_id, ApprovalDecision::Approved)
        .await
        .expect("resolve_approval failed");

    // After resume, exec_core.current_waitpoint_id is cleared
    // (signal.lua:229). Confirm that before proceeding.
    let cleared = read_exec_core_for_run(&h, &run_id)
        .await
        .get("current_waitpoint_id")
        .cloned()
        .unwrap_or_default();
    assert_eq!(
        cleared, "",
        "post-resume current_waitpoint_id must be cleared, got {:?}",
        cleared,
    );

    // FF's signal-resume path (lua/signal.lua:250-262) releases the lease and
    // flips lifecycle_phase back to `runnable` / ownership_state=`unowned`.
    // ff_suspend_execution requires `active` → we must re-claim before the
    // second suspend. This is the intended FF lifecycle, not a workaround.
    h.fabric
        .runs
        .claim(&h.project, &run_id)
        .await
        .expect("runs.claim after resume failed");

    // Second enter must succeed (no FabricError::Internal propagated) and
    // must produce a FRESH waitpoint id (not the closed one).
    let second = h
        .fabric
        .runs
        .enter_waiting_approval(&h.project, &run_id)
        .await
        .expect("second enter_waiting_approval must succeed after resume");
    assert_eq!(
        second.run_id, run_id,
        "re-entered approval must return the same run record",
    );

    let second_wp = read_exec_core_for_run(&h, &run_id)
        .await
        .get("current_waitpoint_id")
        .cloned()
        .filter(|s| !s.is_empty())
        .expect("second enter must set a non-empty current_waitpoint_id");
    assert_ne!(
        first_wp, second_wp,
        "second enter_waiting_approval must create a FRESH waitpoint id; reusing a closed waitpoint would indicate FF drift",
    );

    h.teardown().await;
}

/// Regression guard for issue #40 / audit G1 + G2: task pause and resume
/// must emit `BridgeEvent::TaskStateChanged` so the cairn-store projection
/// and SSE subscribers observe the transitions.
///
/// Pre-fix: `FabricTaskService::pause` and `::resume` called the FCALL
/// successfully and returned the new state to the caller, but never
/// emitted a bridge event. FF committed the transition; cairn-store's
/// `TaskReadModel` drifted because no event reached the projection.
///
/// Post-fix: both emit unconditionally on the happy path (pause guards
/// only the `ALREADY_SATISFIED` replay branch, which is not exercised
/// here). This test fails on pre-fix code with
/// `TaskStateChanged{Paused|...} not emitted`.
///
/// Both suspend and resume are covered in one test because (a) the
/// transitions are paired on FF side, (b) the event-log is shared so
/// asserting both transitions on the same task validates that neither
/// is accidentally squelched by the other's presence.
#[tokio::test]
async fn task_pause_and_resume_emit_state_changed() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    h.fabric
        .tasks
        .claim(&h.project, &task_id, "test-worker".into(), 30_000)
        .await
        .expect("claim failed");

    // --- Pause ---
    let pause_reason = PauseReason {
        kind: PauseReasonKind::OperatorPause,
        detail: None,
        resume_after_ms: None,
        actor: Some("integration-test".into()),
    };
    let paused = h
        .fabric
        .tasks
        .pause(&h.project, &task_id, pause_reason)
        .await
        .expect("pause failed");

    // Same accept-both rationale as test_suspend_and_resume_roundtrip:
    // FF's map_reason_to_blocking may route OperatorPause to either
    // operator_hold (→ Paused) or waiting_for_approval (→ WaitingApproval).
    assert!(
        matches!(paused.state, TaskState::Paused | TaskState::WaitingApproval),
        "expected Paused or WaitingApproval after pause, got {:?}",
        paused.state
    );

    let expected_pause_state = paused.state;
    let expected_task = task_id.clone();
    wait_for_event(&h, Duration::from_secs(2), move |event| {
        matches!(event, RuntimeEvent::TaskStateChanged(e)
            if e.task_id == expected_task && e.transition.to == expected_pause_state)
    })
    .await
    .expect(
        "TaskStateChanged{Paused|WaitingApproval} not emitted on pause — G1 regressed. \
         FabricTaskService::pause must emit BridgeEvent::TaskStateChanged after FF_SUSPEND_EXECUTION \
         succeeds, otherwise the cairn-store projection stays stuck at the prior state. \
         See docs/design/bridge-event-audit.md §3.1.",
    );

    // --- Resume ---
    let resumed = h
        .fabric
        .tasks
        .resume(
            &h.project,
            &task_id,
            ResumeTrigger::OperatorResume,
            TaskResumeTarget::Running,
        )
        .await
        .expect("resume failed");

    // Same positive-assertion rationale as test_suspend_and_resume_roundtrip:
    // delayed_promoter / claim machinery can race the resume write, so we
    // accept any actionable runnable state.
    assert!(
        matches!(
            resumed.state,
            TaskState::Queued | TaskState::Leased | TaskState::Running
        ),
        "expected Queued/Leased/Running after resume (positive), got {:?}",
        resumed.state
    );

    let expected_resume_state = resumed.state;
    let expected_task = task_id.clone();
    wait_for_event(&h, Duration::from_secs(2), move |event| {
        matches!(event, RuntimeEvent::TaskStateChanged(e)
            if e.task_id == expected_task && e.transition.to == expected_resume_state)
    })
    .await
    .expect(
        "TaskStateChanged{Queued|Leased|Running} not emitted on resume — G2 regressed. \
         FabricTaskService::resume must emit BridgeEvent::TaskStateChanged after FF_RESUME_EXECUTION \
         succeeds, otherwise SSE subscribers never see task resume transitions. \
         See docs/design/bridge-event-audit.md §3.1.",
    );

    h.teardown().await;
}
