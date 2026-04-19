//! Regression guards for the registry-less terminal emission contract.
//!
//! The bug these tests pin:
//! `FabricTaskService::complete` / `fail` / `cancel` used to gate
//! `BridgeEvent::TaskStateChanged` emission on `ActiveTaskRegistry`
//! membership. Tasks claimed via any path that did not populate the
//! registry (external API callers, `CairnWorker::claim_next` under
//! `insecure-direct-claim`, or a cairn process restart between claim
//! and completion) silently skipped emission — and the cairn-store
//! `TaskReadModel` projection drifted from FF's exec_core truth.
//!
//! The fix:
//! 1. Removed the `was_registered` gate in all three terminal methods.
//! 2. Deleted `ActiveTaskRegistry` entirely — FF owns every field it
//!    cached (`lease_id`, `lease_epoch`, `attempt_index`), and the
//!    `Option<ClaimedTask>` slot was already carried inside `CairnTask`.
//!
//! Each test exercises a DIFFERENT claim path, completes or fails or
//! cancels, and asserts the projection sees the terminal transition.
//! If the gate ever comes back (or the emission is otherwise dropped),
//! exactly one of these tests fails with a specific, actionable message.

use std::time::Duration;

use cairn_domain::lifecycle::{FailureClass, RunState, TaskState};
use cairn_domain::RuntimeEvent;
use cairn_fabric::services::FabricTaskService;
use cairn_store::event_log::EventLog;

use crate::TestHarness;

/// Wait for a projection condition to hold, polling every 50ms up to
/// `deadline`. Returns Err on timeout with the last observed event count.
///
/// Bridge events are asynchronous — `FabricTaskService::complete` emits
/// into a tokio mpsc and returns before the `EventBridge` consumer has
/// appended to the `InMemoryStore`. A hard sleep is flaky; this waits
/// exactly as long as needed.
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

/// Happy-path baseline: claim via `FabricTaskService::claim`, complete,
/// assert projection sees `RuntimeEvent::TaskStateChanged{to: Completed}`.
///
/// Pre-fix this passed (registry had the entry). Post-fix it still passes.
/// Keeps the baseline covered so a regression that breaks ALL emission —
/// not just the registry-less path — is caught too.
#[tokio::test]
async fn task_complete_emits_state_changed_after_claim() {
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
    let record = h
        .fabric
        .tasks
        .complete(&h.project, &task_id)
        .await
        .expect("complete failed");
    assert_eq!(record.state, TaskState::Completed);

    let expected_task = task_id.clone();
    wait_for_event(&h, Duration::from_secs(2), move |event| {
        matches!(event, RuntimeEvent::TaskStateChanged(e)
            if e.task_id == expected_task && e.transition.to == TaskState::Completed)
    })
    .await
    .expect("TaskStateChanged{Completed} not emitted — the terminal emission path is broken");

    h.teardown().await;
}

/// The regression this commit fixes: complete via a DIFFERENT
/// `FabricTaskService` instance than the one that claimed. Before the
/// fix, the registry-less second instance dropped the `TaskStateChanged`
/// emission silently; the projection stayed stuck at Leased even though
/// FF moved the exec_core to Completed.
///
/// Post-fix: complete emits unconditionally. This test FAILS on the
/// pre-fix code with a `TaskStateChanged{Completed} not emitted` error.
#[tokio::test]
async fn task_complete_emits_when_claim_and_complete_use_distinct_services() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    // 1. Submit + claim via the harness's FabricTaskService (registry-populating path).
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

    // 2. Simulate "process restart between claim and complete" — a
    //    second `FabricTaskService` sharing the same runtime + bridge
    //    but with NO registry entry (registry is gone entirely post-fix;
    //    pre-fix this instance would have had an empty registry).
    let fresh_tasks = FabricTaskService::new(h.fabric.runtime.clone(), h.fabric.bridge.clone());

    // 3. Complete through the fresh instance. FF sees the transition;
    //    the bridge event MUST still fire or the projection drifts.
    let record = fresh_tasks
        .complete(&h.project, &task_id)
        .await
        .expect("complete via fresh service failed");
    assert_eq!(record.state, TaskState::Completed);

    let expected_task = task_id.clone();
    wait_for_event(&h, Duration::from_secs(2), move |event| {
        matches!(event, RuntimeEvent::TaskStateChanged(e)
            if e.task_id == expected_task && e.transition.to == TaskState::Completed)
    })
    .await
    .expect(
        "TaskStateChanged{Completed} missing from the projection — the emission gate regressed, \
         a task claimed outside this FabricTaskService instance will not land in the cairn-store \
         projection.",
    );

    h.teardown().await;
}

/// Cancel path: same registry-less concern as complete. Cancel a task
/// through a fresh `FabricTaskService` and assert the projection sees
/// `TaskStateChanged{Canceled}`.
#[tokio::test]
async fn task_cancel_emits_when_called_from_registry_less_service() {
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

    let fresh_tasks = FabricTaskService::new(h.fabric.runtime.clone(), h.fabric.bridge.clone());
    let record = fresh_tasks
        .cancel(&h.project, &task_id)
        .await
        .expect("cancel via fresh service failed");
    assert_eq!(record.state, TaskState::Canceled);

    let expected_task = task_id.clone();
    wait_for_event(&h, Duration::from_secs(2), move |event| {
        matches!(event, RuntimeEvent::TaskStateChanged(e)
            if e.task_id == expected_task && e.transition.to == TaskState::Canceled)
    })
    .await
    .expect("TaskStateChanged{Canceled} missing from the projection — cancel emission regressed");

    h.teardown().await;
}

/// Fail path: terminal fail (retries exhausted) via a fresh service.
/// Walks attempts 1..=3 through the same fresh service to drain the
/// retry budget, then asserts the projection sees
/// `TaskStateChanged{Failed}` after the final attempt.
#[tokio::test]
async fn task_terminal_fail_emits_when_called_from_registry_less_service() {
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

    // max_retries=2 → three attempts to reach terminal Failed.
    // Use a fresh FabricTaskService for EVERY attempt to simulate the
    // worst case: a different cairn process handling each retry.
    let max_attempts = 3;
    for attempt in 1..=max_attempts {
        // Wait for claim eligibility after the previous fail (FF's
        // delayed_promoter scanner moves the task from delayed back to
        // eligible after the backoff elapses). Uses 200ms poll + 10s
        // deadline, same as test_run_lifecycle.rs pattern.
        wait_until_claim_eligible(&h, &task_id, Duration::from_secs(10))
            .await
            .unwrap_or_else(|_| {
                panic!("attempt {attempt}: task not eligible within 10s — delayed_promoter down?")
            });

        let fresh_tasks = FabricTaskService::new(h.fabric.runtime.clone(), h.fabric.bridge.clone());
        fresh_tasks
            .claim(&h.project, &task_id, "test-worker".into(), 30_000)
            .await
            .unwrap_or_else(|e| panic!("attempt {attempt}: claim via fresh service: {e}"));

        let fresh_fail = FabricTaskService::new(h.fabric.runtime.clone(), h.fabric.bridge.clone());
        let record = fresh_fail
            .fail(&h.project, &task_id, FailureClass::ExecutionError)
            .await
            .unwrap_or_else(|e| panic!("attempt {attempt}: fail via fresh service: {e}"));

        if attempt == max_attempts {
            assert_eq!(record.state, TaskState::Failed);
        }
    }

    let expected_task = task_id.clone();
    wait_for_event(&h, Duration::from_secs(5), move |event| {
        matches!(event, RuntimeEvent::TaskStateChanged(e)
            if e.task_id == expected_task && e.transition.to == TaskState::Failed)
    })
    .await
    .expect(
        "TaskStateChanged{Failed} missing from the projection after terminal fail via fresh \
         FabricTaskService — the emission gate regressed on the fail path",
    );

    h.teardown().await;
}

/// Silence the dead-code warning on `RunState` import while still
/// keeping the cross-module contract visible in imports above — the
/// test assertions match on `transition.to: TaskState`, not RunState.
#[allow(dead_code)]
fn _runstate_imported_for_symmetry(_: RunState) {}

/// Poll `tasks.get` until the task reaches a claim-eligible state
/// (`Queued`). Used between `fail` calls to wait for FF's
/// delayed_promoter to drain the retry backoff.
async fn wait_until_claim_eligible(
    h: &TestHarness,
    task_id: &cairn_domain::TaskId,
    deadline: Duration,
) -> Result<(), Duration> {
    let start = std::time::Instant::now();
    loop {
        let record = match h.fabric.tasks.get(&h.project, task_id).await {
            Ok(Some(r)) => r,
            _ => {
                if start.elapsed() >= deadline {
                    return Err(start.elapsed());
                }
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        if matches!(record.state, TaskState::Queued) {
            return Ok(());
        }
        if start.elapsed() >= deadline {
            return Err(start.elapsed());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}
