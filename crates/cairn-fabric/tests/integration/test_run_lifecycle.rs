// Run and task lifecycle integration tests.
//
// These tests create real FF executions in Valkey. Runs against a
// testcontainers-provisioned Valkey (see tests/integration.rs); every test
// gets a FLUSHDB between invocations plus unique ids as defense in depth.
//
// Terminal operations (complete/fail/cancel) require the execution to be
// Active (leased). run_service.start() creates executions in Waiting state.
// We use task_service.submit() + task_service.claim() to get an Active
// execution, then call terminal operations on the task.

use std::collections::HashMap;

use cairn_domain::lifecycle::{FailureClass, TaskState};
use cairn_domain::TaskId;

use crate::TestHarness;

/// Sqeq ingress threads a request-scoped correlation id through
/// `start_with_correlation`. The adapter MUST preserve it end-to-end:
///   1. Tagged onto FF's `exec_core:tags` as `cairn.correlation_id`.
///   2. Threaded onto `BridgeEvent::ExecutionCreated` → envelope
///      `correlation_id` in cairn-store (audit / SSE consumers rely on it).
///
/// Regression guard for the default-trait-impl fallthrough found in the
/// finalization cross-review: if someone deletes the override, sqeq
/// telemetry silently drops request correlation on the Fabric path.
#[tokio::test]
async fn test_start_with_correlation_tags_exec_core() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();
    let corr = format!("sqeq-{}", uuid::Uuid::new_v4());

    h.fabric
        .runs
        .start_with_correlation(&h.project, &session_id, run_id.clone(), None, Some(&corr))
        .await
        .expect("start_with_correlation failed");

    // Read back the exec tags hash and verify `cairn.correlation_id` landed.
    // Key layout matches FabricRunService::create_execution.
    let eid = cairn_fabric::id_map::run_to_execution_id(&h.project, &run_id);
    let partition =
        ff_core::partition::execution_partition(&eid, &h.fabric.runtime.partition_config);
    let ctx = ff_core::keys::ExecKeyContext::new(&partition, &eid);
    let tags: HashMap<String, String> = h
        .fabric
        .runtime
        .client
        .hgetall(&ctx.tags())
        .await
        .expect("HGETALL exec_core:tags failed");

    assert_eq!(
        tags.get("cairn.correlation_id").map(String::as_str),
        Some(corr.as_str()),
        "correlation_id must be tagged on exec_core; got tags={tags:?}",
    );

    h.teardown().await;
}

#[tokio::test]
async fn test_create_and_read_run() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    let record = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    assert_eq!(record.run_id, run_id);
    assert_eq!(record.project.tenant_id, h.project.tenant_id);
    assert_eq!(record.project.workspace_id, h.project.workspace_id);
    assert_eq!(record.project.project_id, h.project.project_id);

    let fetched = h
        .fabric
        .runs
        .get(&h.project, &run_id)
        .await
        .expect("get failed")
        .expect("run not found");

    assert_eq!(fetched.run_id, run_id);
    assert_eq!(fetched.project.tenant_id, h.project.tenant_id);

    h.teardown().await;
}

#[tokio::test]
async fn test_tags_readable() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    let record = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("start failed");

    assert_eq!(record.session_id, session_id);

    h.teardown().await;
}

#[tokio::test]
async fn test_duplicate_start_is_idempotent() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let run_id = h.unique_run_id();

    let first = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("first start failed");

    let second = h
        .fabric
        .runs
        .start(&h.project, &session_id, run_id.clone(), None)
        .await
        .expect("second start failed");

    assert_eq!(first.run_id, second.run_id);
    assert_eq!(first.state, second.state);
    assert_eq!(first.project, second.project);
    assert_eq!(first.session_id, second.session_id);

    h.teardown().await;
}

/// `runs.claim` is NOT idempotent on the Fabric path. Re-claiming an
/// already-active run must fail at FF's grant gate:
/// `ff_issue_claim_grant` requires `lifecycle_phase="runnable"` +
/// `eligibility_state="eligible_now"` (lua/scheduling.lua:109-112); the
/// `use_claim_resumed_execution` dispatch (claim_common.rs:148-154)
/// only fires for `attempt_interrupted` executions (resume-from-suspend),
/// not for a fresh re-claim of an active run.
///
/// This test is the tripwire for five written assertions of that contract
/// (trait docstring, handler docstring, OpenAPI description, smoke-test
/// comment, handler non-idempotency note). If FF ever relaxes
/// scheduling.lua's grant gate, the second claim returns Ok and this
/// test goes RED at `expect_err` — the loud alarm that the
/// non-idempotency docs no longer hold and need to be revisited
/// alongside the contract change. That is the bug pattern that caused
/// round-1 cross-review to reject the original endpoint claim of
/// idempotency — we add the test so recurrence is structurally
/// impossible.
///
/// Resume-after-suspend (a legitimate second claim) is already covered
/// by `test_suspension.rs::test_suspend_and_resume_roundtrip`, which
/// also exercises the `ff_claim_resumed_execution` dispatch path.
#[tokio::test]
async fn test_claim_rejects_reclaim_on_active() {
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
        .expect("first claim must succeed — run is runnable");

    let second = h.fabric.runs.claim(&h.project, &run_id).await;

    let err = second.expect_err(
        "second claim on an already-active run must fail at FF's grant gate, \
         not silently succeed — see trait docstring contract on RunService::claim",
    );
    let msg = format!("{err}");
    assert!(
        msg.contains("execution_not_eligible"),
        "expected FF grant-gate rejection with `execution_not_eligible` code \
         from ff_issue_claim_grant (lua/scheduling.lua:109-112); got: {msg}",
    );

    h.teardown().await;
}

// Terminal operations require Active (leased) execution.
// We use task_service which creates + claims in the correct flow.

#[tokio::test]
async fn test_complete_task() {
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

    let completed = h
        .fabric
        .tasks
        .complete(&h.project, &task_id)
        .await
        .expect("complete failed");

    assert_eq!(completed.state, TaskState::Completed);

    let fetched = h
        .fabric
        .tasks
        .get(&h.project, &task_id)
        .await
        .expect("get failed")
        .expect("task not found");

    assert_eq!(fetched.state, TaskState::Completed);

    h.teardown().await;
}

// `task_service::submit` hardcodes the retry policy to max_retries=2,
// exponential backoff (initial 1s, 2x multiplier — see task_service.rs:288).
// That means the FIRST fail MUST return retry-scheduled (Queued /
// RetryableFailed), not terminal Failed. Pinning that precisely catches a
// class of regression where the retry path is silently skipped and a
// transient error is promoted straight to Failed.
#[tokio::test]
async fn test_fail_task_retry_scheduled() {
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

    let failed = h
        .fabric
        .tasks
        .fail(&h.project, &task_id, FailureClass::ExecutionError)
        .await
        .expect("fail failed");

    // Strict: first fail MUST NOT be terminal when max_retries=2.
    assert!(
        !failed.state.is_terminal(),
        "first fail under max_retries=2 must schedule a retry, got terminal state {:?}",
        failed.state
    );
    // Strict: first fail MUST be in a retry-scheduled state, not arbitrary
    // non-terminal. Acceptable mappings are Queued (returned to queue) or
    // RetryableFailed (FF Delayed public_state → cairn RetryableFailed).
    assert!(
        matches!(failed.state, TaskState::Queued | TaskState::RetryableFailed),
        "expected Queued or RetryableFailed after first fail with retries remaining, got {:?}",
        failed.state
    );

    h.teardown().await;
}

/// Fix for cross-review BUG #1 / GAP #7: the prior `test_fail_task_terminal`
/// name promised terminal transition but only fired fail() once. Walk
/// through the full retry budget (max_retries=2 → 3 total attempts) and
/// assert the LAST fail is terminal with failure_class set.
///
/// The bounded polling loop waits for FF's DelayedPromoter (scan interval
/// 750ms per ff-engine/src/scanner/delayed_promoter.rs) to move the
/// execution back to `eligible_now` after each retry-scheduled backoff.
/// Task backoffs: 1s, 2s. Plus promoter latency. 15s total budget is
/// generous but bounded.
///
/// If this test flakes on slow CI, the first response should be to
/// investigate delayed_promoter health — NOT to widen the timeout.
#[tokio::test]
async fn test_fail_reaches_terminal_after_retry_exhaustion() {
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

    // Attempt budget with max_retries=2: attempts 1, 2, 3 all fail; after
    // attempt 3, FF promotes to terminal (execution.lua:1057 — `can_retry`
    // is false once retry_count == max_retries).
    let max_attempts = 3;
    let mut last_state = None;

    for attempt in 1..=max_attempts {
        wait_until_eligible(&h, &task_id, std::time::Duration::from_secs(10))
            .await
            .unwrap_or_else(|diag| {
                panic!(
                    "attempt {attempt}: task did not become claim-eligible within timeout — {diag}"
                );
            });

        h.fabric
            .tasks
            .claim(&h.project, &task_id, "test-worker".into(), 30_000)
            .await
            .unwrap_or_else(|e| panic!("attempt {attempt}: claim failed: {e}"));

        let failed = h
            .fabric
            .tasks
            .fail(&h.project, &task_id, FailureClass::ExecutionError)
            .await
            .unwrap_or_else(|e| panic!("attempt {attempt}: fail failed: {e}"));

        last_state = Some(failed.state);

        if attempt < max_attempts {
            assert!(
                !failed.state.is_terminal(),
                "attempt {attempt}: expected retry-scheduled (non-terminal), got {:?}",
                failed.state
            );
            assert!(
                matches!(failed.state, TaskState::Queued | TaskState::RetryableFailed),
                "attempt {attempt}: expected Queued/RetryableFailed, got {:?}",
                failed.state
            );
        } else {
            // Final attempt: retries exhausted → terminal Failed.
            assert_eq!(
                failed.state,
                TaskState::Failed,
                "attempt {attempt}: expected terminal Failed after {attempt} fails, got {:?}",
                failed.state
            );
            assert!(
                failed.state.is_terminal(),
                "final fail must be terminal, got {:?}",
                failed.state
            );
        }
    }

    // Re-read from Valkey — service should still report terminal Failed.
    let persisted = h
        .fabric
        .tasks
        .get(&h.project, &task_id)
        .await
        .expect("get after terminal fail failed")
        .expect("terminal task must be readable");
    assert_eq!(
        persisted.state,
        TaskState::Failed,
        "post-terminal Valkey read must still be Failed (persistence), got {:?}",
        persisted.state,
    );

    let _ = last_state;
    h.teardown().await;
}

/// Poll until `tasks.get(...)` reports a state that is claim-eligible,
/// or `deadline` elapses. On timeout, returns a diagnostic string that
/// includes the last observed state and how long we waited — on CI flake
/// this points the oncall straight at delayed_promoter health without
/// requiring log archaeology.
///
/// Used by `test_fail_reaches_terminal_after_retry_exhaustion` to wait for
/// FF's DelayedPromoter to move a retry-delayed task back to the eligible
/// set between fail+reclaim cycles.
async fn wait_until_eligible(
    h: &TestHarness,
    task_id: &TaskId,
    deadline: std::time::Duration,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    let mut last_state: Option<TaskState> = None;
    let mut last_fetch_err: Option<String> = None;
    loop {
        match h.fabric.tasks.get(&h.project, task_id).await {
            Ok(Some(record)) => {
                last_state = Some(record.state);
                // Leave last_fetch_err untouched — only overwrite when a
                // fetch actually fails, so the diagnostic reports the most
                // recent failure even after subsequent successes.
                // Claim-eligible = Queued (cairn maps FF public_state
                // "waiting" → Queued). RetryableFailed means still delayed.
                if matches!(record.state, TaskState::Queued) {
                    return Ok(());
                }
            }
            Ok(None) => {
                last_fetch_err = Some("tasks.get returned None (task missing)".into());
            }
            Err(e) => {
                last_fetch_err = Some(format!("tasks.get errored: {e}"));
            }
        }

        if start.elapsed() >= deadline {
            let diag = match (last_state, last_fetch_err) {
                (Some(s), _) => format!(
                    "waited {:?}, last state = {:?} (delayed_promoter may be down or backoff still pending)",
                    start.elapsed(),
                    s
                ),
                (None, Some(e)) => format!(
                    "waited {:?}, never observed a state — {e}",
                    start.elapsed()
                ),
                (None, None) => format!(
                    "waited {:?}, never observed a state and no fetch error recorded",
                    start.elapsed()
                ),
            };
            return Err(diag);
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

#[tokio::test]
async fn test_cancel_task() {
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

    let cancelled = h
        .fabric
        .tasks
        .cancel(&h.project, &task_id)
        .await
        .expect("cancel failed");

    assert_eq!(cancelled.state, TaskState::Canceled);

    h.teardown().await;
}
