//! RFC 002 event log retention window tests.
//!
//! Proves the event log's retention and replay contracts:
//! - 50-event dataset with correct count, head_position, and order.
//! - Events older than a timestamp cutoff are identifiable via stored_at.
//! - apply_retention prunes per-entity history beyond max_events_per_entity
//!   while leaving entity-scoped reads intact for surviving events.
//! - The 72-hour SSE replay window contract: position-based read_stream after
//!   any cursor returns all events after that position, enabling reconnect
//!   replay within the defined window.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RetentionPolicySet, RunCreated, RunId,
    RuntimeEvent, SessionCreated, SessionId, TenantId,
};
use cairn_store::{
    event_log::{EntityRef, EventPosition},
    projections::{RetentionMaintenance, RetentionPolicyReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(n: u32) -> ProjectKey {
    ProjectKey::new("tenant_ret", "ws_ret", format!("proj_{n}"))
}

fn tenant() -> TenantId {
    TenantId::new("tenant_ret")
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

/// Append 50 heterogeneous events: 10 sessions + 20 runs (2 per session) + 20 tasks.
/// Returns the head position after all appends.
async fn append_50_events(store: &Arc<InMemoryStore>) -> EventPosition {
    for i in 0..10u32 {
        let sess = SessionId::new(format!("sess_ret_{i}"));
        let mut batch = vec![ev(
            &format!("sess_{i}"),
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(i),
                session_id: sess.clone(),
            }),
        )];
        for j in 0..2u32 {
            let n = i * 2 + j;
            batch.push(ev(
                &format!("run_{n}"),
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(i),
                    session_id: sess.clone(),
                    run_id: RunId::new(format!("run_ret_{n}")),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ));
            batch.push(ev(
                &format!("task_{n}"),
                RuntimeEvent::SessionCreated(SessionCreated {
                    // Use SessionCreated as a lightweight 50th-event filler.
                    project: project(i),
                    session_id: SessionId::new(format!("sess_extra_{n}")),
                }),
            ));
        }
        store.append(&batch).await.unwrap();
    }
    EventLog::head_position(store.as_ref())
        .await
        .unwrap()
        .unwrap()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): 50 events appended; read_stream with various limits
/// returns exactly the expected number of events.
#[tokio::test]
async fn fifty_events_read_stream_correct_count() {
    let store = Arc::new(InMemoryStore::new());
    append_50_events(&store).await;

    // All 50.
    let all = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(
        all.len(),
        50,
        "read_stream(limit=100) must return all 50 events"
    );

    // Exactly 10.
    let ten = EventLog::read_stream(store.as_ref(), None, 10)
        .await
        .unwrap();
    assert_eq!(
        ten.len(),
        10,
        "read_stream(limit=10) must return exactly 10 events"
    );

    // Exactly 1.
    let one = EventLog::read_stream(store.as_ref(), None, 1)
        .await
        .unwrap();
    assert_eq!(one.len(), 1);

    // After position 40 → 10 remaining.
    let tail = EventLog::read_stream(store.as_ref(), Some(EventPosition(40)), 100)
        .await
        .unwrap();
    assert_eq!(tail.len(), 10, "10 events remain after position 40");

    // After last position → empty.
    let empty = EventLog::read_stream(store.as_ref(), Some(EventPosition(50)), 100)
        .await
        .unwrap();
    assert!(empty.is_empty(), "nothing after the last event");
}

/// (3): head_position = 50 after exactly 50 appends.
#[tokio::test]
async fn head_position_equals_fifty_after_fifty_appends() {
    let store = Arc::new(InMemoryStore::new());
    let head = append_50_events(&store).await;

    assert_eq!(
        head,
        EventPosition(50),
        "head_position must be 50 after 50 appends"
    );

    // head_position always reflects the latest append.
    store
        .append(&[ev(
            "extra",
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(99),
                session_id: SessionId::new("sess_extra"),
            }),
        )])
        .await
        .unwrap();
    let head51 = EventLog::head_position(store.as_ref())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        head51,
        EventPosition(51),
        "head_position advances to 51 after one more append"
    );
}

/// (4): Events older than a cutoff timestamp are identifiable via stored_at.
///
/// All 50 events in this test are appended at (roughly) the same wall-clock
/// millisecond, so the cutoff tests use the boundary cases:
/// - stored_at ≥ 0 → all 50 events qualify as "within retention window"
/// - stored_at > far_future → no events qualify as "stale"
#[tokio::test]
async fn events_older_than_cutoff_are_identifiable() {
    let store = Arc::new(InMemoryStore::new());
    append_50_events(&store).await;

    let events = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(events.len(), 50);

    // All events have a stored_at timestamp set at append time.
    let min_stored_at = events.iter().map(|e| e.stored_at).min().unwrap();
    let max_stored_at = events.iter().map(|e| e.stored_at).max().unwrap();

    // Cutoff = max_stored_at + 1: all events are "before the cutoff" (potentially retainable).
    let cutoff_future = max_stored_at + 1;
    let older_than_future: Vec<_> = events
        .iter()
        .filter(|e| e.stored_at < cutoff_future)
        .collect();
    assert_eq!(
        older_than_future.len(), 50,
        "all 50 events must be older than cutoff_future (recently appended events are within any future window)"
    );

    // Cutoff = min_stored_at: events at or after the earliest timestamp.
    // At least 1 event (the first) has stored_at == min_stored_at.
    let at_earliest: Vec<_> = events
        .iter()
        .filter(|e| e.stored_at >= min_stored_at)
        .collect();
    assert_eq!(
        at_earliest.len(),
        50,
        "all events are at or after the earliest stored_at"
    );

    // Cutoff = min_stored_at - 1: all events are "younger than" that past cutoff.
    let before_all = min_stored_at.saturating_sub(1);
    let stale_count = events.iter().filter(|e| e.stored_at <= before_all).count();
    assert_eq!(
        stale_count, 0,
        "no events should be stale relative to a timestamp before all appends"
    );

    // Simulate 72-hour window: any event appended less than 72 hours ago is retained.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let window_72h_ms = 72 * 60 * 60 * 1_000; // 72 hours in ms
    let replay_cutoff = now_ms.saturating_sub(window_72h_ms);

    let within_window: Vec<_> = events
        .iter()
        .filter(|e| e.stored_at >= replay_cutoff)
        .collect();
    assert_eq!(
        within_window.len(),
        50,
        "all 50 recently-appended events must fall within the 72-hour SSE replay window"
    );
}

/// (5): apply_retention prunes per-entity history beyond max_events_per_entity,
/// but entity-scoped reads for surviving events remain fully intact.
#[tokio::test]
async fn retention_doesnt_break_entity_scoped_reads() {
    let store = Arc::new(InMemoryStore::new());

    // Append 5 SessionCreated events for the same session — simulates
    // an entity that accumulates many events over its lifetime.
    let sess = SessionId::new("sess_prune");
    for i in 0..5u32 {
        store
            .append(&[ev(
                &format!("sess_prune_{i}"),
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(0),
                    session_id: sess.clone(),
                }),
            )])
            .await
            .unwrap();
    }

    // Also add one unrelated session that should survive retention.
    store
        .append(&[ev(
            "unrelated",
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(1),
                session_id: SessionId::new("sess_unrelated"),
            }),
        )])
        .await
        .unwrap();

    // Set a retention policy: keep at most 2 events per entity.
    store
        .append(&[ev(
            "policy_set",
            RuntimeEvent::RetentionPolicySet(RetentionPolicySet {
                tenant_id: tenant(),
                policy_id: "policy_ret_1".to_owned(),
                full_history_days: 30,
                current_state_days: 7,
                max_events_per_entity: Some(2),
            }),
        )])
        .await
        .unwrap();

    let before = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap()
        .len();
    assert_eq!(
        before, 7,
        "7 events before retention: 5 duplicates + 1 unrelated + 1 policy"
    );

    // Apply retention for the tenant.
    let result = RetentionMaintenance::apply_retention(store.as_ref(), &tenant())
        .await
        .unwrap();

    // 5 events for sess_prune, keep 2 → prune 3.
    // 1 event for sess_unrelated (below threshold) → no prune.
    // Policy event has no entity ref → not counted.
    assert!(
        result.events_pruned >= 3,
        "at least 3 events must be pruned (5 - 2 = 3)"
    );

    // After retention: total event count must drop.
    let after = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert!(
        after.len() < before,
        "event log must shrink after apply_retention: {before} → {}",
        after.len()
    );

    // Entity-scoped reads for the unrelated session remain intact.
    let unrelated_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(SessionId::new("sess_unrelated")),
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(
        unrelated_events.len(),
        1,
        "unrelated session event must survive retention (not over the per-entity limit)"
    );

    // The surviving sess_prune events are still entity-readable.
    let prune_events =
        EventLog::read_by_entity(store.as_ref(), &EntityRef::Session(sess), None, 100)
            .await
            .unwrap();
    assert!(
        !prune_events.is_empty(),
        "at least one sess_prune event must survive (max_events_per_entity=2)"
    );
    assert!(
        prune_events.len() <= 2,
        "at most 2 sess_prune events must survive: got {}",
        prune_events.len()
    );
}

/// (6): The 72-hour SSE replay window contract: position-based read_stream
/// after any valid cursor returns all events after that position.
///
/// RFC 002 requires that clients reconnecting with `Last-Event-ID` from up
/// to 72 hours ago receive a complete replay. This test proves the mechanism:
/// - Events appended at positions 1–50 are all replayable by position.
/// - Reconnecting at any position P returns exactly (50 - P) events.
/// - The monotonic position ordering means no events are skipped.
#[tokio::test]
async fn seventy_two_hour_sse_replay_window_contract() {
    let store = Arc::new(InMemoryStore::new());
    append_50_events(&store).await;

    let all = EventLog::read_stream(store.as_ref(), None, 100)
        .await
        .unwrap();
    assert_eq!(all.len(), 50);

    // Verify the complete position sequence: 1, 2, ..., 50 with no gaps.
    for (i, event) in all.iter().enumerate() {
        assert_eq!(
            event.position.0,
            (i + 1) as u64,
            "position must be sequential: expected {} got {}",
            i + 1,
            event.position.0
        );
    }

    // RFC 002 replay contract: reconnect at position P → receive (50 - P) events.
    for reconnect_at in [0u64, 1, 10, 25, 40, 49] {
        let replay = EventLog::read_stream(store.as_ref(), Some(EventPosition(reconnect_at)), 100)
            .await
            .unwrap();

        let expected = 50 - reconnect_at as usize;
        assert_eq!(
            replay.len(),
            expected,
            "reconnecting at position {reconnect_at} must replay {expected} events, got {}",
            replay.len()
        );

        // All replayed events must have positions > reconnect_at.
        assert!(
            replay.iter().all(|e| e.position.0 > reconnect_at),
            "all replayed events must be strictly after position {reconnect_at}"
        );

        // First replayed event is immediately after the cursor.
        if !replay.is_empty() {
            assert_eq!(
                replay[0].position.0,
                reconnect_at + 1,
                "first replayed event at reconnect position {reconnect_at} must be {}",
                reconnect_at + 1
            );
        }
    }

    // Edge case: reconnect at position 50 → empty replay (already at head).
    let at_head = EventLog::read_stream(store.as_ref(), Some(EventPosition(50)), 100)
        .await
        .unwrap();
    assert!(
        at_head.is_empty(),
        "reconnecting at head position must return empty replay (already caught up)"
    );

    // The stored_at timestamps of all events in the log are within the last
    // 72 hours, confirming they are in the active replay window.
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let window_start = now_ms.saturating_sub(72 * 60 * 60 * 1_000);

    let events_in_window = all.iter().filter(|e| e.stored_at >= window_start).count();
    assert_eq!(
        events_in_window, 50,
        "all 50 events must be within the 72-hour replay window (stored_at >= window_start)"
    );
}
