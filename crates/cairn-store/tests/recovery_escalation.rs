//! RFC 002 recovery escalation integration tests.
//!
//! Recovery events (RecoveryAttempted, RecoveryCompleted, RecoveryEscalated)
//! are durable event log entries. They are no-op projections — the recovery
//! pipeline reads them directly from the global event log via read_stream
//! and filters by run_id. (They are not mapped in event_matches_entity so
//! read_by_entity does not surface them.)
//!
//! Validates:
//! - RecoveryAttempted is stored with the run context and reason.
//! - RecoveryCompleted(recovered=true) marks successful recovery.
//! - RecoveryCompleted(recovered=false) marks a failed attempt.
//! - RecoveryEscalated records the escalation with attempt_count and reason.
//! - The full escalation chain (attempt → failed → escalated) is queryable
//!   in causal (log-position) order.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RecoveryAttempted, RecoveryCompleted,
    RunCreated, RunId, RuntimeEvent, SessionCreated, SessionId, TaskId,
};
use cairn_domain::events::RecoveryEscalated;
use cairn_store::{EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_rec", "ws_rec", "proj_rec")
}

fn run_id(n: &str)  -> RunId     { RunId::new(format!("run_rec_{n}")) }
fn task_id(n: &str) -> TaskId    { TaskId::new(format!("task_rec_{n}")) }
fn sess_id(n: &str) -> SessionId { SessionId::new(format!("sess_rec_{n}")) }

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

async fn seed_run(store: &Arc<InMemoryStore>, n: &str) {
    store.append(&[
        ev(&format!("sess_{n}"), RuntimeEvent::SessionCreated(SessionCreated {
            project: project(), session_id: sess_id(n),
        })),
        ev(&format!("run_{n}"), RuntimeEvent::RunCreated(RunCreated {
            project: project(), session_id: sess_id(n), run_id: run_id(n),
            parent_run_id: None, prompt_release_id: None, agent_role_id: None,
        })),
    ]).await.unwrap();
}

/// Read all events from the global log filtered to a specific run_id.
async fn recovery_events_for_run(store: &Arc<InMemoryStore>, rid: &RunId)
    -> Vec<cairn_store::event_log::StoredEvent>
{
    EventLog::read_stream(store.as_ref(), None, 200)
        .await.unwrap()
        .into_iter()
        .filter(|e| match &e.envelope.payload {
            RuntimeEvent::RecoveryAttempted(r)  => r.run_id.as_ref() == Some(rid),
            RuntimeEvent::RecoveryCompleted(c)  => c.run_id.as_ref() == Some(rid),
            RuntimeEvent::RecoveryEscalated(esc) => esc.run_id.as_ref() == Some(rid),
            _ => false,
        })
        .collect()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2) + (3): Session+run seeded; RecoveryAttempted appended;
/// record is stored in the event log with run_id and reason preserved.
#[tokio::test]
async fn recovery_attempted_is_stored_in_event_log() {
    let store = Arc::new(InMemoryStore::new());
    seed_run(&store, "1").await;

    // (2) Append RecoveryAttempted.
    store.append(&[ev("evt_attempt", RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
        project: project(),
        run_id: Some(run_id("1")),
        task_id: None,
        reason: "run stalled for 30 s — initiating recovery sweep".to_owned(),
    }))]).await.unwrap();

    // (3) Verify stored via filtered global log.
    let rec_events = recovery_events_for_run(&store, &run_id("1")).await;
    assert_eq!(rec_events.len(), 1, "one recovery event must be stored for the run");

    if let RuntimeEvent::RecoveryAttempted(r) = &rec_events[0].envelope.payload {
        assert_eq!(r.run_id, Some(run_id("1")), "run_id must match");
        assert!(r.reason.contains("stalled"), "reason must be preserved");
        assert!(r.task_id.is_none(), "no task involved in this recovery");
    } else {
        panic!("expected RecoveryAttempted");
    }
}

/// (4) + (5): RecoveryCompleted(recovered=true) marks successful recovery;
/// both attempt and completion are stored in causal order.
#[tokio::test]
async fn recovery_completed_success_marks_recovered() {
    let store = Arc::new(InMemoryStore::new());
    seed_run(&store, "2").await;

    store.append(&[
        ev("evt_attempt2", RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
            project: project(),
            run_id: Some(run_id("2")),
            task_id: None,
            reason: "task lease expired — re-queuing".to_owned(),
        })),
        ev("evt_complete2", RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
            project: project(),
            run_id: Some(run_id("2")),
            task_id: None,
            recovered: true,
        })),
    ]).await.unwrap();

    let rec_events = recovery_events_for_run(&store, &run_id("2")).await;

    // (5) Both attempt and completion in log.
    assert_eq!(rec_events.len(), 2, "RecoveryAttempted + RecoveryCompleted must both be stored");

    // Verify completion is marked as successful.
    let completed = rec_events.iter().find(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RecoveryCompleted(c) if c.recovered)
    });
    assert!(completed.is_some(), "RecoveryCompleted(recovered=true) must be stored");

    // Causal order: attempt position < completion position.
    assert!(
        rec_events[0].position < rec_events[1].position,
        "RecoveryAttempted must appear before RecoveryCompleted in the log"
    );
}

/// (6): RecoveryEscalated event records failed recovery with attempt_count,
/// last_error, and reason. All fields survive the event log round-trip.
#[tokio::test]
async fn recovery_escalated_stored_with_all_fields() {
    let store = Arc::new(InMemoryStore::new());
    seed_run(&store, "3").await;

    // Three failed attempts before escalation.
    for i in 1u32..=3 {
        store.append(&[
            ev(&format!("attempt_3_{i}"), RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                project: project(),
                run_id: Some(run_id("3")),
                task_id: Some(task_id("3")),
                reason: format!("attempt {i}: checkpoint restore failed"),
            })),
            ev(&format!("complete_3_{i}"), RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
                project: project(),
                run_id: Some(run_id("3")),
                task_id: Some(task_id("3")),
                recovered: false,
            })),
        ]).await.unwrap();
    }

    // Escalate after 3 failed attempts.
    store.append(&[ev("evt_escalated_3", RuntimeEvent::RecoveryEscalated(RecoveryEscalated {
        task_id: task_id("3"),
        run_id: Some(run_id("3")),
        reason: "exceeded 3 recovery attempts — operator intervention required".to_owned(),
        escalated_at_ms: 99_000,
        last_error: Some("checkpoint restore failed on attempt 3".to_owned()),
        attempt_count: 3,
    }))]).await.unwrap();

    let rec_events = recovery_events_for_run(&store, &run_id("3")).await;

    // 3 attempts + 3 failed completions + 1 escalation = 7 events.
    assert_eq!(rec_events.len(), 7, "3 attempts + 3 failed completions + 1 escalation = 7 events");

    // Verify the escalation's fields.
    let escalation = rec_events.iter().find(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RecoveryEscalated(_))
    }).expect("RecoveryEscalated must be present");

    if let RuntimeEvent::RecoveryEscalated(esc) = &escalation.envelope.payload {
        assert_eq!(esc.attempt_count, 3, "attempt_count must be 3");
        assert_eq!(esc.escalated_at_ms, 99_000, "escalated_at_ms must be preserved");
        assert!(esc.reason.contains("operator intervention"), "reason must explain the escalation");
        assert!(esc.last_error.is_some(), "last_error must be set");
        assert_eq!(esc.run_id, Some(run_id("3")));
    }

    // The 3 failed completions are distinct from the escalation.
    let failed_count = rec_events.iter().filter(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RecoveryCompleted(c) if !c.recovered)
    }).count();
    assert_eq!(failed_count, 3, "exactly 3 RecoveryCompleted(recovered=false) events");
}

/// (7): The full escalation chain (attempt → failed_complete → escalated)
/// is queryable in causal (log-position) order.
#[tokio::test]
async fn escalation_chain_is_queryable_in_causal_order() {
    let store = Arc::new(InMemoryStore::new());
    seed_run(&store, "chain").await;

    store.append(&[
        ev("evt_attempt_chain", RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
            project: project(),
            run_id: Some(run_id("chain")),
            task_id: None,
            reason: "run stalled — recovery initiated".to_owned(),
        })),
        ev("evt_fail_chain", RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
            project: project(),
            run_id: Some(run_id("chain")),
            task_id: None,
            recovered: false,
        })),
        ev("evt_escalate_chain", RuntimeEvent::RecoveryEscalated(RecoveryEscalated {
            task_id: task_id("chain"),
            run_id: Some(run_id("chain")),
            reason: "recovery failed — escalating to operator".to_owned(),
            escalated_at_ms: 50_000,
            last_error: Some("run could not be restored from checkpoint".to_owned()),
            attempt_count: 1,
        })),
    ]).await.unwrap();

    let rec_events = recovery_events_for_run(&store, &run_id("chain")).await;
    assert_eq!(rec_events.len(), 3, "attempt + failed_complete + escalated = 3 events");

    // (7) Events must appear in strict causal (position) order.
    assert!(matches!(&rec_events[0].envelope.payload, RuntimeEvent::RecoveryAttempted(_)),
        "first event must be RecoveryAttempted");
    assert!(matches!(&rec_events[1].envelope.payload, RuntimeEvent::RecoveryCompleted(_)),
        "second event must be RecoveryCompleted");
    assert!(matches!(&rec_events[2].envelope.payload, RuntimeEvent::RecoveryEscalated(_)),
        "third event must be RecoveryEscalated");

    assert!(rec_events[0].position < rec_events[1].position,
        "attempt must precede failed completion");
    assert!(rec_events[1].position < rec_events[2].position,
        "failed completion must precede escalation");

    // The recovered=false flag is explicitly set on the completion.
    if let RuntimeEvent::RecoveryCompleted(c) = &rec_events[1].envelope.payload {
        assert!(!c.recovered, "failed completion must have recovered=false");
    }
}

/// RecoveryCompleted(recovered=false) is distinguishable from success.
#[tokio::test]
async fn recovery_completed_false_is_distinct_from_success() {
    let store = Arc::new(InMemoryStore::new());
    seed_run(&store, "fail").await;

    store.append(&[
        ev("attempt_fail", RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
            project: project(),
            run_id: Some(run_id("fail")),
            task_id: None,
            reason: "recovery initiated".to_owned(),
        })),
        ev("complete_fail", RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
            project: project(),
            run_id: Some(run_id("fail")),
            task_id: None,
            recovered: false,
        })),
    ]).await.unwrap();

    let rec_events = recovery_events_for_run(&store, &run_id("fail")).await;

    let has_failure = rec_events.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RecoveryCompleted(c) if !c.recovered)
    });
    let has_success = rec_events.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RecoveryCompleted(c) if c.recovered)
    });

    assert!(has_failure, "failed completion must be present");
    assert!(!has_success, "no successful completion must exist for this run");
}

/// Different runs' recovery events are isolated by run_id filtering.
#[tokio::test]
async fn recovery_events_are_isolated_by_run() {
    let store = Arc::new(InMemoryStore::new());
    seed_run(&store, "iso_a").await;
    seed_run(&store, "iso_b").await;

    store.append(&[
        ev("attempt_a", RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
            project: project(), run_id: Some(run_id("iso_a")),
            task_id: None, reason: "run A recovery".to_owned(),
        })),
        ev("attempt_b", RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
            project: project(), run_id: Some(run_id("iso_b")),
            task_id: None, reason: "run B recovery".to_owned(),
        })),
        ev("complete_b", RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
            project: project(), run_id: Some(run_id("iso_b")),
            task_id: None, recovered: true,
        })),
    ]).await.unwrap();

    let a_events = recovery_events_for_run(&store, &run_id("iso_a")).await;
    let b_events = recovery_events_for_run(&store, &run_id("iso_b")).await;

    assert_eq!(a_events.len(), 1, "run_a must see only its 1 recovery event");
    assert_eq!(b_events.len(), 2, "run_b must see its 2 recovery events");

    // run_a sees no completion; run_b's successful completion is isolated.
    assert!(!a_events.iter().any(|e| matches!(&e.envelope.payload, RuntimeEvent::RecoveryCompleted(_))),
        "run_a must not see run_b's completion");
}
