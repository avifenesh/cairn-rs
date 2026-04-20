// End-to-end integration tests for FF flow-edge-backed task
// dependencies.
//
// Covers:
//   - happy path: declare → check (blocked) → complete prerequisite
//     → push listener fires → check (empty) → dependent eligible.
//   - auto-skip: fail prerequisite → FF cascades Skipped to dependent.
//   - cycle detection: back-edge rejected with Validation.
//   - cross-session rejection (both at the fabric service layer,
//     since the adapter lives in cairn-app; this test suite only has
//     the raw service surface).
//   - self-dependency rejection.
//   - idempotent re-declare.
//
// Runs against the shared Valkey container (see `tests/integration.rs`).
// `completion_listener: Some(_)` is enabled by `FabricRuntime::start` as
// of commit B, so resolution is push-based; we poll on eligibility with a
// 5s budget.

use cairn_domain::lifecycle::TaskState;
use cairn_domain::FailureClass;

/// Submit-task policy has `max_retries = 2`, so driving a task to
/// Failed via `fail()` takes 3 full claim/fail cycles (attempts 1 and
/// 2 reschedule into RetryableFailed / Queued, attempt 3 terminates).
/// Each retry carries an exponential backoff starting at 1000ms —
/// this helper waits for Queued before each subsequent claim.
async fn drive_task_to_terminal_failure(
    h: &TestHarness,
    session_id: &cairn_domain::SessionId,
    task_id: &cairn_domain::TaskId,
) {
    for attempt in 1..=3 {
        // Wait for Queued state before claiming (retry backoff).
        let state = wait_for_task_state(h, session_id, task_id, TaskState::Queued, 10_000).await;
        assert_eq!(
            state,
            TaskState::Queued,
            "attempt {attempt}: expected Queued, got {state:?}"
        );
        h.fabric
            .tasks
            .claim(
                &h.project,
                Some(session_id),
                task_id,
                "test-worker".into(),
                30_000,
            )
            .await
            .unwrap_or_else(|e| panic!("attempt {attempt}: claim failed: {e}"));
        h.fabric
            .tasks
            .fail(
                &h.project,
                Some(session_id),
                task_id,
                FailureClass::ExecutionError,
            )
            .await
            .unwrap_or_else(|e| panic!("attempt {attempt}: fail failed: {e}"));
    }
}

use crate::TestHarness;

/// Poll `check_dependencies` until the returned slice is empty or the
/// budget expires. Returns true if the list drained inside the budget.
async fn wait_until_no_blockers(
    h: &TestHarness,
    session_id: &cairn_domain::SessionId,
    task_id: &cairn_domain::TaskId,
    budget_ms: u64,
) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(budget_ms);
    loop {
        match h
            .fabric
            .tasks
            .check_dependencies(&h.project, session_id, task_id)
            .await
        {
            Ok(v) if v.is_empty() => return true,
            Ok(_) => {}
            Err(_) => {}
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Poll the task state until it reaches `target` (or a terminal state
/// sibling) or the budget expires. Returns the observed state.
async fn wait_for_task_state(
    h: &TestHarness,
    session_id: &cairn_domain::SessionId,
    task_id: &cairn_domain::TaskId,
    target: TaskState,
    budget_ms: u64,
) -> TaskState {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(budget_ms);
    let mut last = TaskState::Queued;
    loop {
        if let Ok(Some(rec)) = h
            .fabric
            .tasks
            .get(&h.project, Some(session_id), task_id)
            .await
        {
            last = rec.state;
            if rec.state == target {
                return rec.state;
            }
        }
        if std::time::Instant::now() >= deadline {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Happy path: B depends on A, A completes, push listener drains the
/// dep, B transitions out of blocked.
#[tokio::test]
async fn declare_then_complete_prereq_unblocks_dependent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();
    let task_b = h.unique_task_id();

    // Submit both tasks. ff_add_execution_to_flow runs on each submit
    // so both are flow members before the edge is declared.
    h.fabric
        .tasks
        .submit(&h.project, task_a.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit A");
    h.fabric
        .tasks
        .submit(&h.project, task_b.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit B");

    // Declare B → A.
    h.fabric
        .tasks
        .declare_dependency(&h.project, &session_id, &task_b, &task_a)
        .await
        .expect("declare_dependency");

    // Before A completes: B has exactly one blocking dep (A).
    let blockers = h
        .fabric
        .tasks
        .check_dependencies(&h.project, &session_id, &task_b)
        .await
        .expect("check_dependencies while blocked");
    assert_eq!(
        blockers.len(),
        1,
        "expected exactly one blocker, got {blockers:?}"
    );
    assert_eq!(
        blockers[0].dependency.depends_on_task_id, task_a,
        "blocker should name A as prerequisite"
    );
    assert!(
        blockers[0].resolved_at_ms.is_none(),
        "blocking dep should have resolved_at_ms = None"
    );

    // Complete A.
    h.fabric
        .tasks
        .claim(
            &h.project,
            Some(&session_id),
            &task_a,
            "test-worker".into(),
            30_000,
        )
        .await
        .expect("claim A");
    h.fabric
        .tasks
        .complete(&h.project, Some(&session_id), &task_a)
        .await
        .expect("complete A");

    // Push listener should drain the edge. 5s budget tolerates
    // cold-connection overhead on the dedicated RESP3 client.
    assert!(
        wait_until_no_blockers(&h, &session_id, &task_b, 5_000).await,
        "check_dependencies should have returned empty within 5s"
    );

    h.teardown().await;
}

/// Auto-skip: A fails (terminally), B is marked Skipped.
///
/// FF's `ff_resolve_dependency` receives `outcome=failed`, marks the
/// edge impossible, and skips the child. The push listener dispatches
/// within ~RTT.
#[tokio::test]
async fn failed_prereq_auto_skips_dependent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();
    let task_b = h.unique_task_id();

    h.fabric
        .tasks
        .submit(&h.project, task_a.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit A");
    h.fabric
        .tasks
        .submit(&h.project, task_b.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit B");
    h.fabric
        .tasks
        .declare_dependency(&h.project, &session_id, &task_b, &task_a)
        .await
        .expect("declare B → A");

    // Drive A to terminal Failed by exhausting its retry budget
    // (max_retries=2 → 3 fail cycles). Each retry has a 1s backoff.
    drive_task_to_terminal_failure(&h, &session_id, &task_a).await;

    // Confirm A actually reached terminal Failed — a stuck-in-retry
    // A would not trigger the push listener, so this assert surfaces
    // a different bug than the push-latency one we're actually
    // testing.
    let rec_a = h
        .fabric
        .tasks
        .get(&h.project, Some(&session_id), &task_a)
        .await
        .expect("get A")
        .expect("A not found");
    assert_eq!(
        rec_a.state,
        TaskState::Failed,
        "A should be terminal Failed"
    );

    // B should transition to Failed/DependencyFailed (FF's "skipped"
    // public state maps to this shape via state_map, since cairn
    // TaskState has no dedicated Skipped variant). Push listener
    // dispatches synchronously under normal operation; the 20s
    // budget is generous enough to also tolerate a reconciler-
    // fallback path (15s interval default) on CI machines where
    // pubsub subscription setup can lag on cold RESP3 connections.
    let final_b = wait_for_task_state(&h, &session_id, &task_b, TaskState::Failed, 20_000).await;
    assert_eq!(
        final_b,
        TaskState::Failed,
        "expected B Failed (skipped via dep), got {final_b:?}"
    );
    let rec_b = h
        .fabric
        .tasks
        .get(&h.project, Some(&session_id), &task_b)
        .await
        .expect("get B")
        .expect("B not found");
    assert_eq!(
        rec_b.failure_class,
        Some(FailureClass::DependencyFailed),
        "expected B's failure_class = DependencyFailed"
    );

    h.teardown().await;
}

/// Cycle detection: declare B→A, then A→B. Second call should be
/// rejected with a Validation error carrying the FF cycle_detected
/// code, mapped cleanly at the fabric layer.
#[tokio::test]
async fn cycle_is_rejected_with_validation() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();
    let task_b = h.unique_task_id();

    h.fabric
        .tasks
        .submit(&h.project, task_a.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit A");
    h.fabric
        .tasks
        .submit(&h.project, task_b.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit B");

    // First edge: B → A. OK.
    h.fabric
        .tasks
        .declare_dependency(&h.project, &session_id, &task_b, &task_a)
        .await
        .expect("first declare");

    // Second edge: A → B would close a cycle (A → B → A).
    let err = h
        .fabric
        .tasks
        .declare_dependency(&h.project, &session_id, &task_a, &task_b)
        .await
        .expect_err("cycle-closing declare should fail");

    match err {
        cairn_fabric::FabricError::Validation { reason } => {
            assert!(
                reason.contains("cycle"),
                "expected cycle message, got: {reason}"
            );
        }
        other => panic!("expected Validation error, got {other:?}"),
    }

    h.teardown().await;
}

/// Self-dependency: declaring a task depends on itself is rejected
/// client-side before any FCALL (FF also rejects, but cairn's guard
/// avoids the round-trip and produces a clearer message).
#[tokio::test]
async fn self_dependency_is_rejected() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();

    h.fabric
        .tasks
        .submit(&h.project, task_a.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit A");

    let err = h
        .fabric
        .tasks
        .declare_dependency(&h.project, &session_id, &task_a, &task_a)
        .await
        .expect_err("self-dependency should fail");

    match err {
        cairn_fabric::FabricError::Validation { reason } => {
            assert!(
                reason.contains("itself"),
                "expected self-ref message, got: {reason}"
            );
        }
        other => panic!("expected Validation, got {other:?}"),
    }

    h.teardown().await;
}

/// Idempotent declare: declaring the same edge twice returns success
/// the second time. FF's `dependency_already_exists` is mapped to
/// success — callers can declare defensively without tracking state.
#[tokio::test]
async fn duplicate_declare_is_idempotent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_a = h.unique_task_id();
    let task_b = h.unique_task_id();

    h.fabric
        .tasks
        .submit(&h.project, task_a.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit A");
    h.fabric
        .tasks
        .submit(&h.project, task_b.clone(), None, None, 0, Some(&session_id))
        .await
        .expect("submit B");

    h.fabric
        .tasks
        .declare_dependency(&h.project, &session_id, &task_b, &task_a)
        .await
        .expect("first declare");

    // Second declare should succeed, not error.
    let rec = h
        .fabric
        .tasks
        .declare_dependency(&h.project, &session_id, &task_b, &task_a)
        .await
        .expect("second declare");

    assert_eq!(rec.dependency.dependent_task_id, task_b);
    assert_eq!(rec.dependency.depends_on_task_id, task_a);

    h.teardown().await;
}
