//! SSE replay window integration tests (RFC 002).
//!
//! The SSE stream must support reconnect-with-replay: when a client reconnects
//! it sends its last seen event position and the server replays all events after
//! that position.  These tests validate that `read_stream` honours the
//! exclusive-after-position contract that the frontend depends on.
//!
//! Contract under test:
//!   read_stream(None, N)               → all events (SSE initial connect)
//!   read_stream(Some(EventPosition(k)), N) → only events at positions > k
//!   read_stream(Some(head), N)         → empty  (client is fully caught up)
//!   head_position()                    → exact position of the last stored event

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, SessionCreated,
    SessionId, TenantId, WorkspaceId,
};
use cairn_store::{EventLog, EventPosition, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_sse"),
        workspace_id: WorkspaceId::new("w_sse"),
        project_id: ProjectId::new("p_sse"),
    }
}

/// Build a `SessionCreated` envelope with a deterministic ID based on `n`.
fn make_event(n: u32) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_{n:03}")),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            session_id: SessionId::new(format!("sess_{n:03}")),
            project: test_project(),
        }),
    )
}

/// Append `count` events one at a time and return the assigned positions.
async fn append_n(store: &InMemoryStore, count: u32) -> Vec<EventPosition> {
    let mut positions = Vec::with_capacity(count as usize);
    for i in 1..=count {
        let mut batch = store.append(&[make_event(i)]).await.unwrap();
        positions.append(&mut batch);
    }
    positions
}

// ── 1. Baseline: 10 events are stored at sequential positions ─────────────────

#[tokio::test]
async fn ten_events_assigned_sequential_positions() {
    let store = InMemoryStore::new();
    let positions = append_n(&store, 10).await;

    assert_eq!(positions.len(), 10);
    for (i, pos) in positions.iter().enumerate() {
        assert_eq!(
            pos.0,
            (i + 1) as u64,
            "position should be 1-indexed and sequential"
        );
    }
}

// ── 2. read_stream(None) returns the full log ─────────────────────────────────

#[tokio::test]
async fn read_stream_none_returns_all_events() {
    let store = InMemoryStore::new();
    append_n(&store, 10).await;

    let events = store.read_stream(None, 100).await.unwrap();

    assert_eq!(events.len(), 10, "None reads from the very beginning");
    assert_eq!(events[0].position.0, 1);
    assert_eq!(events[9].position.0, 10);
}

// ── 3. read_stream(position 0) is equivalent to None ─────────────────────────

#[tokio::test]
async fn read_stream_position_zero_returns_all_events() {
    let store = InMemoryStore::new();
    append_n(&store, 10).await;

    // EventPosition(0) is the "before the first event" sentinel used when
    // SSE clients connect without a Last-Event-ID header.
    let events = store
        .read_stream(Some(EventPosition(0)), 100)
        .await
        .unwrap();

    assert_eq!(events.len(), 10, "position 0 is before all events");
    assert_eq!(events[0].position.0, 1);
    assert_eq!(events[9].position.0, 10);
}

// ── 4. read_stream(position 5) returns only events 6-10 ──────────────────────

#[tokio::test]
async fn read_stream_from_mid_position_returns_tail() {
    let store = InMemoryStore::new();
    append_n(&store, 10).await;

    // SSE client sends Last-Event-ID: 5 → replay events after position 5.
    let events = store
        .read_stream(Some(EventPosition(5)), 100)
        .await
        .unwrap();

    assert_eq!(events.len(), 5, "positions 6-10 = 5 events");
    assert_eq!(
        events[0].position.0, 6,
        "first replayed event is position 6"
    );
    assert_eq!(
        events[4].position.0, 10,
        "last replayed event is position 10"
    );

    // Each replayed event's session ID should match the expected ordinal.
    for (i, stored) in events.iter().enumerate() {
        match &stored.envelope.payload {
            RuntimeEvent::SessionCreated(e) => {
                let expected_n = i as u32 + 6;
                assert_eq!(
                    e.session_id.as_str(),
                    format!("sess_{expected_n:03}"),
                    "session ID mismatch at replay index {i}",
                );
            }
            other => panic!("unexpected event payload: {other:?}"),
        }
    }
}

// ── 5. read_stream(head) returns empty — client is fully caught up ────────────

#[tokio::test]
async fn read_stream_at_head_returns_empty() {
    let store = InMemoryStore::new();
    append_n(&store, 10).await;

    let head = store.head_position().await.unwrap().unwrap();

    // SSE client whose Last-Event-ID equals the current head has nothing to replay.
    let events = store.read_stream(Some(head), 100).await.unwrap();

    assert!(events.is_empty(), "no events after head");
}

// ── 6. head_position() reflects the last stored event ────────────────────────

#[tokio::test]
async fn head_position_tracks_append() {
    let store = Arc::new(InMemoryStore::new());

    assert!(
        store.head_position().await.unwrap().is_none(),
        "empty store has no head"
    );

    store.append(&[make_event(1)]).await.unwrap();
    assert_eq!(store.head_position().await.unwrap().unwrap().0, 1);

    store.append(&[make_event(2)]).await.unwrap();
    assert_eq!(store.head_position().await.unwrap().unwrap().0, 2);

    append_n(&store, 8).await; // appends events 1..=8 with new IDs but positions 3..=10
    assert_eq!(store.head_position().await.unwrap().unwrap().0, 10);
}

// ── 7. limit is respected — read_stream never over-delivers ──────────────────

#[tokio::test]
async fn read_stream_respects_limit() {
    let store = InMemoryStore::new();
    append_n(&store, 10).await;

    let events = store.read_stream(None, 3).await.unwrap();
    assert_eq!(events.len(), 3, "limit=3 should cap the result");
    assert_eq!(events[0].position.0, 1);
    assert_eq!(events[2].position.0, 3);
}

// ── 8. Replay window: batch-appended events behave identically ────────────────

#[tokio::test]
async fn batch_append_produces_same_replay_semantics() {
    let store = InMemoryStore::new();

    // Append all 10 in one batch instead of one at a time.
    let batch: Vec<_> = (1..=10).map(make_event).collect();
    let positions = store.append(&batch).await.unwrap();

    assert_eq!(positions.len(), 10);
    assert_eq!(positions[0].0, 1);
    assert_eq!(positions[9].0, 10);

    // Replay semantics are the same regardless of append granularity.
    let tail = store
        .read_stream(Some(EventPosition(5)), 100)
        .await
        .unwrap();
    assert_eq!(tail.len(), 5);
    assert_eq!(tail[0].position.0, 6);
}
