//! RFC 002 event log idempotency integration tests.
//!
//! RFC 002 requires that command handlers be idempotent: re-delivering a
//! command must not produce duplicate side-effects. The contract is:
//!
//!   1. Before appending, call `find_by_causation_id(cmd_id)`.
//!   2. If `Some(position)` is returned, the command was already applied —
//!      return that position without re-appending (idempotent rejection).
//!   3. If `None` is returned, append the event with `causation_id` set.
//!
//! These tests validate that `find_by_causation_id` is the correct seam for
//! this check, and that the overall pattern preserves exactly-once semantics.

use std::sync::Arc;

use cairn_domain::{
    CommandId, EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId,
};
use cairn_store::event_log::EventPosition;
use cairn_store::{EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_idem", "ws_idem", "proj_idem")
}

fn session_id() -> SessionId {
    SessionId::new("sess_idem_1")
}

/// Build a SessionCreated event optionally tagged with a causation_id.
fn session_event(evt_id: &str, causation: Option<&str>) -> EventEnvelope<RuntimeEvent> {
    let env = EventEnvelope::for_runtime_event(
        EventId::new(evt_id),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: session_id(),
        }),
    );
    match causation {
        Some(cmd) => env.with_causation_id(CommandId::new(cmd)),
        None => env,
    }
}

/// Build a RunCreated event optionally tagged with a causation_id.
fn run_event(evt_id: &str, run: &str, causation: Option<&str>) -> EventEnvelope<RuntimeEvent> {
    let env = EventEnvelope::for_runtime_event(
        EventId::new(evt_id),
        EventSource::Runtime,
        RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: session_id(),
            run_id: RunId::new(run),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
    );
    match causation {
        Some(cmd) => env.with_causation_id(CommandId::new(cmd)),
        None => env,
    }
}

/// Idempotent-append helper — mirrors the RFC 002 command handler pattern.
///
/// Returns `(position, was_duplicate)`:
/// - `was_duplicate = true` when the causation_id was already present.
/// - `was_duplicate = false` when the event was freshly appended.
async fn idempotent_append(
    store: &Arc<InMemoryStore>,
    event: EventEnvelope<RuntimeEvent>,
    causation_id: &str,
) -> (EventPosition, bool) {
    // RFC 002 step 1: check for prior application.
    if let Some(pos) = EventLog::find_by_causation_id(store.as_ref(), causation_id)
        .await
        .unwrap()
    {
        return (pos, true); // already applied — idempotent rejection
    }

    // RFC 002 step 2: first application — append.
    let positions = EventLog::append(store.as_ref(), &[event]).await.unwrap();
    (positions[0], false)
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Append event with causation_id='cmd_1'.
/// (2) Verify find_by_causation_id returns the position.
#[tokio::test]
async fn causation_id_lookup_returns_correct_position() {
    let store = Arc::new(InMemoryStore::new());

    // Append an event tagged with causation_id='cmd_1'.
    let positions = EventLog::append(store.as_ref(), &[session_event("evt_1", Some("cmd_1"))])
        .await
        .unwrap();

    let appended_pos = positions[0];

    // find_by_causation_id must return the same position.
    let found = EventLog::find_by_causation_id(store.as_ref(), "cmd_1")
        .await
        .unwrap();

    assert!(found.is_some(), "find_by_causation_id must find 'cmd_1'");
    assert_eq!(
        found.unwrap(),
        appended_pos,
        "returned position must match the appended event's position"
    );
}

/// (3) Attempt to append same causation_id again — verify idempotent rejection.
///
/// RFC 002: a command handler that calls find_by_causation_id before appending
/// must detect the duplicate and return the original position without
/// re-appending, leaving the event count unchanged.
#[tokio::test]
async fn same_causation_id_is_rejected_idempotently() {
    let store = Arc::new(InMemoryStore::new());

    // First application of cmd_1.
    let (pos1, dup1) =
        idempotent_append(&store, session_event("evt_first", Some("cmd_1")), "cmd_1").await;
    assert!(!dup1, "first application must not be a duplicate");

    // Second application of cmd_1 (re-delivery simulation).
    let (pos2, dup2) = idempotent_append(
        &store,
        session_event("evt_second", Some("cmd_1")), // different evt_id, same cmd
        "cmd_1",
    )
    .await;
    assert!(
        dup2,
        "second application with same causation_id must be rejected as duplicate"
    );
    assert_eq!(
        pos1, pos2,
        "idempotent rejection must return the original event position, not a new one"
    );

    // Store must contain exactly 1 event — no double-write.
    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(
        events.len(),
        1,
        "exactly 1 event must be in the log after idempotent re-delivery"
    );
}

/// (4) Verify different causation_id succeeds and produces a new position.
#[tokio::test]
async fn different_causation_id_appends_new_event() {
    let store = Arc::new(InMemoryStore::new());

    // Apply cmd_1.
    let (pos1, _) =
        idempotent_append(&store, session_event("evt_cmd1", Some("cmd_1")), "cmd_1").await;

    // Apply cmd_2 (different causation_id).
    let (pos2, dup2) = idempotent_append(
        &store,
        run_event("evt_cmd2", "run_1", Some("cmd_2")),
        "cmd_2",
    )
    .await;

    assert!(!dup2, "cmd_2 must not be treated as a duplicate of cmd_1");
    assert_ne!(
        pos1, pos2,
        "cmd_2 must produce a distinct position from cmd_1"
    );
    assert!(
        pos2 > pos1,
        "second event must have a higher position than the first"
    );

    // Both causation_ids must be individually findable.
    let found1 = EventLog::find_by_causation_id(store.as_ref(), "cmd_1")
        .await
        .unwrap();
    let found2 = EventLog::find_by_causation_id(store.as_ref(), "cmd_2")
        .await
        .unwrap();

    assert_eq!(
        found1.unwrap(),
        pos1,
        "cmd_1 must resolve to its original position"
    );
    assert_eq!(
        found2.unwrap(),
        pos2,
        "cmd_2 must resolve to its own position"
    );

    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 2, "log must contain exactly 2 events");
}

/// (5) Events with None causation_id are always accepted without idempotency gating.
///
/// RFC 002: only causation_id-tagged commands are idempotency-checked.
/// Un-tagged events (causation_id=None) are always accepted and do not interfere
/// with causation_id lookups.
#[tokio::test]
async fn none_causation_id_events_always_accepted() {
    let store = Arc::new(InMemoryStore::new());

    // Append three events with no causation_id.
    EventLog::append(
        store.as_ref(),
        &[
            session_event("evt_nc_1", None),
            run_event("evt_nc_2", "run_a", None),
            run_event("evt_nc_3", "run_b", None),
        ],
    )
    .await
    .unwrap();

    // All three must land in the log.
    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(
        events.len(),
        3,
        "all events with None causation_id must be accepted (no idempotency gating)"
    );

    // find_by_causation_id for any string must return None (no tagged events).
    let not_found = EventLog::find_by_causation_id(store.as_ref(), "anything")
        .await
        .unwrap();
    assert!(
        not_found.is_none(),
        "find_by_causation_id must return None when no event has that causation_id"
    );
}

/// Mixing tagged and untagged events: tagged events are independently findable
/// while untagged events do not interfere with causation_id lookups.
#[tokio::test]
async fn tagged_and_untagged_events_coexist() {
    let store = Arc::new(InMemoryStore::new());

    // One tagged event sandwiched by two untagged events.
    let positions = EventLog::append(
        store.as_ref(),
        &[
            session_event("evt_1", None),                    // no causation_id
            run_event("evt_2", "run_a", Some("cmd_tagged")), // tagged
            run_event("evt_3", "run_b", None),               // no causation_id
        ],
    )
    .await
    .unwrap();

    // The tagged event is at position index 1 (middle of the three).
    let tagged_pos = positions[1];

    let found = EventLog::find_by_causation_id(store.as_ref(), "cmd_tagged")
        .await
        .unwrap();
    assert_eq!(
        found.unwrap(),
        tagged_pos,
        "find_by_causation_id must find the tagged event among untagged neighbours"
    );

    // Untagged events do not match any causation_id lookup.
    let not_found = EventLog::find_by_causation_id(store.as_ref(), "evt_1")
        .await
        .unwrap();
    assert!(
        not_found.is_none(),
        "event_id 'evt_1' is not a causation_id — must not be found"
    );
}

/// RFC 002 exactly-once guarantee: replaying the same command 10 times must
/// produce exactly 1 persisted event and always return the same position.
#[tokio::test]
async fn exactly_once_guarantee_under_repeated_redelivery() {
    let store = Arc::new(InMemoryStore::new());
    const CMD_ID: &str = "cmd_exactly_once";

    let mut returned_positions = Vec::new();

    // Simulate 10 re-deliveries of the same command.
    for i in 0..10 {
        let (pos, _) = idempotent_append(
            &store,
            session_event(&format!("evt_attempt_{i}"), Some(CMD_ID)),
            CMD_ID,
        )
        .await;
        returned_positions.push(pos);
    }

    // Every attempt must return the same position (first application).
    let first_pos = returned_positions[0];
    for (i, pos) in returned_positions.iter().enumerate() {
        assert_eq!(
            *pos, first_pos,
            "attempt {i} must return the original position, not a new one"
        );
    }

    // Exactly 1 event in the log — no duplicates.
    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(
        events.len(),
        1,
        "exactly 1 event must be persisted after 10 redeliveries"
    );
}

/// find_by_causation_id returns the FIRST occurrence when searching.
/// (The store must not silently discard a causation_id if append is called
/// twice with the same value without the idempotency guard.)
#[tokio::test]
async fn find_by_causation_id_returns_first_match() {
    let store = Arc::new(InMemoryStore::new());

    // Bypass the guard and force-append two events with the same causation_id
    // to verify find_by_causation_id returns the first one.
    let positions = EventLog::append(
        store.as_ref(),
        &[
            session_event("evt_first", Some("cmd_dup")),
            run_event("evt_second", "run_dup", Some("cmd_dup")),
        ],
    )
    .await
    .unwrap();

    let first_pos = positions[0];

    let found = EventLog::find_by_causation_id(store.as_ref(), "cmd_dup")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        found, first_pos,
        "find_by_causation_id must return the position of the FIRST matching event"
    );
}
