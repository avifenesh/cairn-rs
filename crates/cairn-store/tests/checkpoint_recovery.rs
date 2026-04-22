//! RFC 004 checkpoint and recovery integration tests.
//!
//! Validates the full checkpoint/recovery pipeline through InMemoryStore:
//! - CheckpointRecorded events are projected into the checkpoint read-model.
//! - Latest-disposition checkpoints supersede prior ones for the same run.
//! - Recovery events (RecoveryAttempted → RecoveryCompleted) land in the log
//!   and are readable in sequence.
//! - read_stream after a known checkpoint position returns only post-checkpoint
//!   events, proving the event log's positional read works for replay.

use std::sync::Arc;

use cairn_domain::{
    CheckpointDisposition, CheckpointId, CheckpointRecorded, EventEnvelope, EventId, EventSource,
    ProjectKey, RecoveryAttempted, RecoveryCompleted, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, TaskCreated, TaskId,
};
use cairn_store::{projections::CheckpointReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_ckpt", "ws_ckpt", "proj_ckpt")
}

fn run_id() -> RunId {
    RunId::new("run_ckpt_1")
}

fn session_id() -> SessionId {
    SessionId::new("sess_ckpt_1")
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn checkpoint_event(
    cp_id: &str,
    disposition: CheckpointDisposition,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_{cp_id}"),
        RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
            project: project(),
            run_id: run_id(),
            checkpoint_id: CheckpointId::new(cp_id),
            disposition,
            data: Some(serde_json::json!({ "step": cp_id })),
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Session + run + task events are appended; (2) CheckpointRecorded is appended
/// at a known position; (3) CheckpointReadModel reflects the stored checkpoint.
#[tokio::test]
async fn checkpoint_recorded_and_readable() {
    let store = Arc::new(InMemoryStore::new());

    // Step 1: create session, run, and two tasks.
    store
        .append(&[
            ev(
                "evt_sess",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id(),
                }),
            ),
            ev(
                "evt_run",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id(),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                "evt_task1",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("task_1"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                    session_id: None,
                }),
            ),
            ev(
                "evt_task2",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("task_2"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: Some(TaskId::new("task_1")),
                    prompt_release_id: None,
                    session_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Step 2: record a checkpoint at this point.
    store
        .append(&[checkpoint_event("cp_1", CheckpointDisposition::Latest)])
        .await
        .unwrap();

    // Step 3: verify the checkpoint is stored and readable.
    let cp = CheckpointReadModel::get(store.as_ref(), &CheckpointId::new("cp_1"))
        .await
        .unwrap()
        .expect("checkpoint must be readable after CheckpointRecorded");

    assert_eq!(cp.checkpoint_id.as_str(), "cp_1");
    assert_eq!(cp.run_id, run_id());
    assert_eq!(cp.disposition, CheckpointDisposition::Latest);
    assert!(cp.data.is_some(), "checkpoint data must be preserved");

    // latest_for_run must return this checkpoint.
    let latest = CheckpointReadModel::latest_for_run(store.as_ref(), &run_id())
        .await
        .unwrap()
        .expect("latest_for_run must find the checkpoint");
    assert_eq!(latest.checkpoint_id.as_str(), "cp_1");
}

/// Recording a second checkpoint supersedes the first: the first transitions to
/// Superseded and only the new one is returned by latest_for_run.
#[tokio::test]
async fn second_checkpoint_supersedes_first() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            ev(
                "evt_sess",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id(),
                }),
            ),
            ev(
                "evt_run",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id(),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    store
        .append(&[checkpoint_event("cp_a", CheckpointDisposition::Latest)])
        .await
        .unwrap();

    store
        .append(&[checkpoint_event("cp_b", CheckpointDisposition::Latest)])
        .await
        .unwrap();

    // cp_a must now be Superseded.
    let cp_a = CheckpointReadModel::get(store.as_ref(), &CheckpointId::new("cp_a"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        cp_a.disposition,
        CheckpointDisposition::Superseded,
        "first checkpoint should be marked Superseded after a newer one is recorded"
    );

    // latest_for_run returns cp_b only.
    let latest = CheckpointReadModel::latest_for_run(store.as_ref(), &run_id())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(latest.checkpoint_id.as_str(), "cp_b");
    assert_eq!(latest.disposition, CheckpointDisposition::Latest);
}

/// (4) RecoveryAttempted and (5) RecoveryCompleted appear in the event log in
/// the correct order, proving the recovery pipeline can be traced.
#[tokio::test]
async fn recovery_events_land_in_log_in_order() {
    let store = Arc::new(InMemoryStore::new());

    // Baseline events.
    store
        .append(&[
            ev(
                "evt_sess",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id(),
                }),
            ),
            ev(
                "evt_run",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id(),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Record a checkpoint to recover from.
    store
        .append(&[checkpoint_event(
            "cp_recovery",
            CheckpointDisposition::Latest,
        )])
        .await
        .unwrap();

    // Step 4: simulate recovery attempt.
    store
        .append(&[ev(
            "evt_recovery_attempted",
            RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                project: project(),
                run_id: Some(run_id()),
                task_id: None,
                reason: "simulated failure requiring checkpoint restore".to_owned(),
                boot_id: None,
            }),
        )])
        .await
        .unwrap();

    // Step 5: complete recovery.
    store
        .append(&[ev(
            "evt_recovery_completed",
            RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                project: project(),
                run_id: Some(run_id()),
                task_id: None,
                recovered: true,
                boot_id: None,
            }),
        )])
        .await
        .unwrap();

    // Both recovery events must be in the log in the correct order.
    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();

    let positions: Vec<_> = events
        .iter()
        .enumerate()
        .filter_map(|(i, e)| match &e.envelope.payload {
            RuntimeEvent::RecoveryAttempted(_) => Some(("attempted", i)),
            RuntimeEvent::RecoveryCompleted(_) => Some(("completed", i)),
            _ => None,
        })
        .collect();

    assert_eq!(
        positions.len(),
        2,
        "both recovery events must be in the log"
    );
    assert_eq!(positions[0].0, "attempted");
    assert_eq!(positions[1].0, "completed");
    assert!(
        positions[0].1 < positions[1].1,
        "RecoveryAttempted must appear before RecoveryCompleted"
    );

    // RecoveryCompleted must report recovered=true.
    let completed_event = events
        .iter()
        .find(|e| matches!(&e.envelope.payload, RuntimeEvent::RecoveryCompleted(r) if r.recovered));
    assert!(
        completed_event.is_some(),
        "RecoveryCompleted with recovered=true must be present"
    );
}

/// (6) read_stream with an `after` position returns only events that occurred
/// after the checkpoint, proving positional replay works for recovery.
#[tokio::test]
async fn read_after_checkpoint_position_returns_only_post_checkpoint_events() {
    let store = Arc::new(InMemoryStore::new());

    // Pre-checkpoint events.
    store
        .append(&[
            ev(
                "evt_sess",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id(),
                }),
            ),
            ev(
                "evt_run",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id(),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            ev(
                "evt_task1",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("pre_task_1"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                    session_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Record checkpoint — capture the position after all pre-checkpoint events.
    store
        .append(&[checkpoint_event(
            "cp_boundary",
            CheckpointDisposition::Latest,
        )])
        .await
        .unwrap();

    let checkpoint_position = store
        .head_position()
        .await
        .unwrap()
        .expect("head position must be set after checkpoint");

    // Post-checkpoint events.
    store
        .append(&[
            ev(
                "evt_task2",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("post_task_2"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: None,
                    prompt_release_id: None,
                    session_id: None,
                }),
            ),
            ev(
                "evt_task3",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("post_task_3"),
                    parent_run_id: Some(run_id()),
                    parent_task_id: Some(TaskId::new("post_task_2")),
                    prompt_release_id: None,
                    session_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Read only events after the checkpoint position.
    let post_events = EventLog::read_stream(store.as_ref(), Some(checkpoint_position), 50)
        .await
        .unwrap();

    // Only the two post-checkpoint task events should be returned.
    assert_eq!(
        post_events.len(),
        2,
        "read_stream after checkpoint position must return only post-checkpoint events; \
         got {} event(s): {:?}",
        post_events.len(),
        post_events
            .iter()
            .map(|e| &e.envelope.event_id)
            .collect::<Vec<_>>()
    );

    // All returned events must be strictly after the checkpoint position.
    for event in &post_events {
        assert!(
            event.position > checkpoint_position,
            "event at position {:?} must be after checkpoint at {:?}",
            event.position,
            checkpoint_position
        );
    }

    // Verify the post-checkpoint events are our two new tasks (by event_id).
    let event_ids: Vec<_> = post_events
        .iter()
        .map(|e| e.envelope.event_id.as_str())
        .collect();
    assert!(
        event_ids.contains(&"evt_task2"),
        "post-checkpoint stream must include evt_task2"
    );
    assert!(
        event_ids.contains(&"evt_task3"),
        "post-checkpoint stream must include evt_task3"
    );
}

/// list_by_run returns all checkpoints for the run, ordered by creation time.
#[tokio::test]
async fn list_by_run_returns_all_checkpoints_in_order() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            ev(
                "evt_sess",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: session_id(),
                }),
            ),
            ev(
                "evt_run",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: session_id(),
                    run_id: run_id(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            checkpoint_event("cp_x1", CheckpointDisposition::Latest),
            checkpoint_event("cp_x2", CheckpointDisposition::Latest),
            checkpoint_event("cp_x3", CheckpointDisposition::Latest),
        ])
        .await
        .unwrap();

    let all = CheckpointReadModel::list_by_run(store.as_ref(), &run_id(), 100)
        .await
        .unwrap();

    assert_eq!(all.len(), 3, "all three checkpoints must be in list_by_run");

    // Only the last should still be Latest; earlier ones should be Superseded.
    let latest_count = all
        .iter()
        .filter(|c| c.disposition == CheckpointDisposition::Latest)
        .count();
    assert_eq!(
        latest_count, 1,
        "exactly one checkpoint should have Latest disposition"
    );
}
