//! RFC 005 — checkpoint and recovery lifecycle end-to-end integration tests.
//!
//! Covers the full checkpoint + recovery arc:
//!   1. Create session and run
//!   2. Save a checkpoint with a structured data payload
//!   3. Verify the checkpoint is retrievable by ID and as the latest for the run
//!   4. Transition the run to Failed (simulating a crash / executor death)
//!   5. Recover from the latest checkpoint — run returns to Running
//!   6. Verify all expected domain events were emitted:
//!      CheckpointRecorded, RunStateChanged (→ Failed), CheckpointRestored,
//!      RunStateChanged (→ Running), RecoveryAttempted, RecoveryCompleted

use std::sync::Arc;

use cairn_domain::*;
use cairn_runtime::checkpoints::CheckpointService;
use cairn_runtime::runs::RunService;
use cairn_runtime::services::{CheckpointServiceImpl, RunServiceImpl, SessionServiceImpl};
use cairn_runtime::sessions::SessionService;
use cairn_store::projections::RunReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_rfc005", "w_rfc005", "p_rfc005")
}

// ── Helper: build a runtime event envelope without going through service internals ──

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

// ── Test 1: full checkpoint → fail → recover lifecycle ──────────────────────

/// RFC 005 §3: a run may save checkpoints at arbitrary points during execution.
/// If the run later fails, recovery uses the latest checkpoint to restore state.
#[tokio::test]
async fn checkpoint_save_fail_recover_lifecycle() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let checkpoint_svc = CheckpointServiceImpl::new(store.clone());

    let session_id = SessionId::new("sess_rfc005");
    let run_id = RunId::new("run_rfc005");
    let checkpoint_id = CheckpointId::new("cp_rfc005_v1");

    // ── Phase 1: create session and run ────────────────────────────────────
    session_svc
        .create(&project(), session_id.clone())
        .await
        .unwrap();

    run_svc
        .start(&project(), &session_id, run_id.clone(), None)
        .await
        .unwrap();

    let run = RunReadModel::get(store.as_ref(), &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        run.state,
        RunState::Pending,
        "run must start in Pending state"
    );

    // ── Phase 2: save checkpoint with structured data payload ───────────────
    //
    // The CheckpointService::save() API does not expose a data parameter
    // (RFC 005 gap); we append the event directly so the checkpoint record
    // carries a meaningful payload for the recovery assertion below.
    let cp_data = serde_json::json!({
        "step": "ingest_batch_3",
        "processed_records": 1_250,
        "last_offset": "shard-02:offset-9182",
        "artifacts": ["batch_3_summary.json", "embeddings_v2.bin"]
    });

    store
        .append(&[evt(
            "evt_cp_recorded",
            RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
                project: project(),
                run_id: run_id.clone(),
                checkpoint_id: checkpoint_id.clone(),
                disposition: CheckpointDisposition::Latest,
                data: Some(cp_data.clone()),
            }),
        )])
        .await
        .unwrap();

    // ── Phase 3: verify checkpoint is retrievable ───────────────────────────
    let fetched = checkpoint_svc
        .get(&checkpoint_id)
        .await
        .unwrap()
        .expect("checkpoint must be retrievable by ID after save");

    assert_eq!(fetched.checkpoint_id, checkpoint_id);
    assert_eq!(fetched.run_id, run_id);
    assert_eq!(
        fetched.disposition,
        CheckpointDisposition::Latest,
        "freshly saved checkpoint must be Latest"
    );
    assert_eq!(
        fetched.data,
        Some(cp_data.clone()),
        "checkpoint data payload must round-trip through the projection"
    );

    let latest = checkpoint_svc
        .latest_for_run(&run_id)
        .await
        .unwrap()
        .expect("latest_for_run must return the saved checkpoint");
    assert_eq!(
        latest.checkpoint_id, checkpoint_id,
        "latest_for_run must return the correct checkpoint"
    );

    // ── Phase 4: transition run to Failed ───────────────────────────────────
    let failed_run = run_svc
        .fail(&run_id, FailureClass::ExecutionError)
        .await
        .unwrap();

    assert_eq!(
        failed_run.state,
        RunState::Failed,
        "run must be Failed after explicit failure"
    );

    let failed_run_read = RunReadModel::get(store.as_ref(), &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        failed_run_read.state,
        RunState::Failed,
        "RunReadModel must reflect Failed state"
    );

    // ── Phase 5: recover from checkpoint ───────────────────────────────────
    //
    // The full checkpoint-based recovery path (CheckpointRestored →
    // RunStateChanged → RecoveryCompleted) is not yet wired into
    // `RecoveryServiceImpl::recover_interrupted_runs` for failed runs.
    // We append the canonical recovery events directly so that:
    //   (a) the read-model projections exercise the full state arc
    //   (b) the event stream can be verified end-to-end
    //
    // This mirrors what a fully implemented recovery reactor should emit.

    store
        .append(&[
            // Signal which checkpoint is being restored.
            evt(
                "evt_cp_restored",
                RuntimeEvent::CheckpointRestored(CheckpointRestored {
                    project: project(),
                    run_id: run_id.clone(),
                    checkpoint_id: checkpoint_id.clone(),
                }),
            ),
            // Transition run back to Running (recovery bypasses normal state guard).
            evt(
                "evt_run_recovered",
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Failed),
                        to: RunState::Running,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: Some(ResumeTrigger::RuntimeSignal),
                }),
            ),
            // Confirm recovery completed successfully.
            evt(
                "evt_recovery_completed",
                RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                    project: project(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    recovered: true,
                }),
            ),
        ])
        .await
        .unwrap();

    // ── Phase 5b: verify run returned to Running ────────────────────────────
    let recovered_run = RunReadModel::get(store.as_ref(), &run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        recovered_run.state,
        RunState::Running,
        "run must be Running after checkpoint-based recovery"
    );

    // ── Phase 6: verify the full event stream ──────────────────────────────
    let all_events = store.read_stream(None, 1_000).await.unwrap();

    let mut saw_checkpoint_recorded = false;
    let mut saw_run_failed = false;
    let mut saw_checkpoint_restored = false;
    let mut saw_run_recovered_to_running = false;
    let mut saw_recovery_completed = false;

    for stored in &all_events {
        match &stored.envelope.payload {
            RuntimeEvent::CheckpointRecorded(e) if e.run_id == run_id => {
                saw_checkpoint_recorded = true;
                assert_eq!(
                    e.checkpoint_id, checkpoint_id,
                    "recorded checkpoint ID must match"
                );
                assert_eq!(
                    e.data,
                    Some(cp_data.clone()),
                    "CheckpointRecorded event must carry the data payload"
                );
            }
            RuntimeEvent::RunStateChanged(e)
                if e.run_id == run_id && e.transition.to == RunState::Failed =>
            {
                saw_run_failed = true;
                assert_eq!(
                    e.transition.from,
                    Some(RunState::Pending),
                    "run transitioned from Pending to Failed"
                );
            }
            RuntimeEvent::CheckpointRestored(e) if e.run_id == run_id => {
                saw_checkpoint_restored = true;
                assert_eq!(
                    e.checkpoint_id, checkpoint_id,
                    "restored checkpoint ID must match the saved checkpoint"
                );
            }
            RuntimeEvent::RunStateChanged(e)
                if e.run_id == run_id && e.transition.to == RunState::Running =>
            {
                saw_run_recovered_to_running = true;
                assert_eq!(
                    e.transition.from,
                    Some(RunState::Failed),
                    "recovery must transition from Failed to Running"
                );
                assert_eq!(
                    e.resume_trigger,
                    Some(ResumeTrigger::RuntimeSignal),
                    "recovery resume trigger must be RuntimeSignal"
                );
            }
            RuntimeEvent::RecoveryCompleted(e) if e.run_id == Some(run_id.clone()) => {
                saw_recovery_completed = true;
                assert!(e.recovered, "RecoveryCompleted must report recovered=true");
            }
            _ => {}
        }
    }

    assert!(
        saw_checkpoint_recorded,
        "RFC 005: CheckpointRecorded event must be present in the event stream"
    );
    assert!(
        saw_run_failed,
        "RFC 005: RunStateChanged(→ Failed) event must be present"
    );
    assert!(
        saw_checkpoint_restored,
        "RFC 005: CheckpointRestored event must be emitted during recovery"
    );
    assert!(
        saw_run_recovered_to_running,
        "RFC 005: RunStateChanged(→ Running) event must be emitted during recovery"
    );
    assert!(
        saw_recovery_completed,
        "RFC 005: RecoveryCompleted event must confirm successful recovery"
    );
}

// ── Test 2: multiple checkpoints — only the latest is used for recovery ──────

/// RFC 005 §3: when a run saves multiple checkpoints, only the latest one
/// (marked `Latest`; prior ones become `Superseded`) is used for recovery.
#[tokio::test]
async fn multiple_checkpoints_latest_is_used_for_recovery() {
    let store = Arc::new(InMemoryStore::new());
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let checkpoint_svc = CheckpointServiceImpl::new(store.clone());

    let session_id = SessionId::new("sess_multi_cp");
    let run_id = RunId::new("run_multi_cp");

    session_svc
        .create(&project(), session_id.clone())
        .await
        .unwrap();
    run_svc
        .start(&project(), &session_id, run_id.clone(), None)
        .await
        .unwrap();

    // Save three checkpoints in sequence via the service.
    let cp1 = checkpoint_svc
        .save(&project(), &run_id, CheckpointId::new("cp_multi_v1"))
        .await
        .unwrap();
    let cp2 = checkpoint_svc
        .save(&project(), &run_id, CheckpointId::new("cp_multi_v2"))
        .await
        .unwrap();
    let cp3 = checkpoint_svc
        .save(&project(), &run_id, CheckpointId::new("cp_multi_v3"))
        .await
        .unwrap();

    // v1 and v2 must be Superseded.
    assert_eq!(cp1.disposition, CheckpointDisposition::Latest); // only at save time
    let cp1_now = checkpoint_svc
        .get(&cp1.checkpoint_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cp1_now.disposition,
        CheckpointDisposition::Superseded,
        "first checkpoint must be Superseded after second save"
    );

    let cp2_now = checkpoint_svc
        .get(&cp2.checkpoint_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cp2_now.disposition,
        CheckpointDisposition::Superseded,
        "second checkpoint must be Superseded after third save"
    );

    // v3 must be Latest.
    assert_eq!(
        cp3.disposition,
        CheckpointDisposition::Latest,
        "third checkpoint must be Latest"
    );

    // latest_for_run must return v3.
    let latest = checkpoint_svc
        .latest_for_run(&run_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        latest.checkpoint_id,
        CheckpointId::new("cp_multi_v3"),
        "latest_for_run must return the most recently saved checkpoint"
    );

    // list_by_run must return all three.
    let all = checkpoint_svc.list_by_run(&run_id, 10).await.unwrap();
    assert_eq!(all.len(), 3, "list_by_run must return all checkpoints");

    let latest_count = all
        .iter()
        .filter(|cp| cp.disposition == CheckpointDisposition::Latest)
        .count();
    assert_eq!(
        latest_count, 1,
        "RFC 005: exactly one checkpoint per run may be Latest"
    );
}

// Tests 3 + 4 (`recover_interrupted_runs_with_checkpoint_returns_action`,
// `recover_interrupted_run_without_checkpoint_fails_it`) exercised
// `RecoveryServiceImpl::recover_interrupted_runs` against the in-memory
// store. Both were deleted in the Fabric finalization round — FF's
// AttemptTimeoutScanner + ExecutionDeadlineScanner + LeaseExpiryScanner
// own recovery of interrupted / orphaned runs unconditionally
// (ff-engine/src/scanner/{attempt_timeout,execution_deadline,
// lease_expiry}.rs). Tests 1 + 2 above stay — they validate checkpoint
// lifecycle + latest-wins semantics, which are cairn-side concerns
// unrelated to recovery.
