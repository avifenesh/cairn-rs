//! RFC 002 event log compaction readiness tests.
//!
//! Proves the event log scales to large event sets:
//! - head_position advances monotonically with each append.
//! - read_stream pagination (limit+after) works correctly.
//! - read_by_entity scoped pagination returns only matching events.
//! - find_by_causation_id works across large event sets.
//! - All 50 positions are strictly monotonically increasing.

use std::sync::Arc;

use cairn_domain::{
    CommandId, EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId,
    RuntimeEvent, SessionCreated, SessionId, TaskCreated, TaskId,
};
use cairn_store::{
    event_log::{EntityRef, EventPosition},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_compact", "ws_compact", "proj_compact")
}

fn ev(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(id),
        EventSource::Runtime,
        payload,
    )
}

fn ev_with_causation(id: &str, cmd: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(id),
        EventSource::Runtime,
        payload,
    )
    .with_causation_id(CommandId::new(cmd))
}

fn session_event(n: u32) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_sess_{n}"),
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: SessionId::new(format!("sess_{n}")),
        }),
    )
}

fn run_event(n: u32) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_run_{n}"),
        RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: SessionId::new(format!("sess_{n}")),
            run_id: RunId::new(format!("run_{n}")),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
    )
}

fn task_event(n: u32) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_task_{n}"),
        RuntimeEvent::TaskCreated(TaskCreated {
            project: project(),
            task_id: TaskId::new(format!("task_{n}")),
            parent_run_id: Some(RunId::new(format!("run_{n}"))),
            parent_task_id: None,
            prompt_release_id: None,
        }),
    )
}

/// Append exactly 50 events: 10 sessions, 20 runs (2 per session), 20 tasks (2 per run).
/// Every 5th event is tagged with a causation_id for the causation test.
async fn seed_50_events(store: &Arc<InMemoryStore>) {
    for i in 0..10u32 {
        let mut batch = vec![session_event(i)];

        for j in 0..2u32 {
            let n = i * 2 + j;
            let run_ev = if n % 5 == 0 {
                ev_with_causation(
                    &format!("evt_run_{n}"),
                    &format!("cmd_{n}"),
                    RuntimeEvent::RunCreated(RunCreated {
                        project: project(),
                        session_id: SessionId::new(format!("sess_{i}")),
                        run_id: RunId::new(format!("run_{n}")),
                        parent_run_id: None,
                        prompt_release_id: None,
                        agent_role_id: None,
                    }),
                )
            } else {
                run_event(n)
            };
            batch.push(run_ev);
            batch.push(task_event(n));
        }

        store.append(&batch).await.unwrap();
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): Append 50 events; head_position must equal 50.
#[tokio::test]
async fn head_position_equals_event_count() {
    let store = Arc::new(InMemoryStore::new());

    // Confirm empty on start.
    assert!(
        EventLog::head_position(store.as_ref()).await.unwrap().is_none(),
        "empty store must have no head position"
    );

    seed_50_events(&store).await;

    let head = EventLog::head_position(store.as_ref())
        .await
        .unwrap()
        .expect("head_position must be set after 50 appends");

    assert_eq!(
        head,
        EventPosition(50),
        "head_position must be 50 after appending 50 events"
    );
}

/// (3): read_stream with limit=10 returns exactly 10 events; cursor-based
/// pagination correctly pages through all 50 events in 5 pages of 10.
#[tokio::test]
async fn read_stream_pagination_pages_through_50_events() {
    let store = Arc::new(InMemoryStore::new());
    seed_50_events(&store).await;

    // Page 1: events 1–10.
    let page1 = EventLog::read_stream(store.as_ref(), None, 10).await.unwrap();
    assert_eq!(page1.len(), 10, "page 1 must return exactly 10 events");
    assert_eq!(page1[0].position, EventPosition(1));
    assert_eq!(page1[9].position, EventPosition(10));

    // Page 2: events 11–20.
    let cursor1 = page1.last().unwrap().position;
    let page2 = EventLog::read_stream(store.as_ref(), Some(cursor1), 10).await.unwrap();
    assert_eq!(page2.len(), 10, "page 2 must return 10 events after cursor");
    assert_eq!(page2[0].position, EventPosition(11));

    // Pages 3, 4, 5 — collect all remaining.
    let cursor2 = page2.last().unwrap().position;
    let page3 = EventLog::read_stream(store.as_ref(), Some(cursor2), 10).await.unwrap();
    let cursor3 = page3.last().unwrap().position;
    let page4 = EventLog::read_stream(store.as_ref(), Some(cursor3), 10).await.unwrap();
    let cursor4 = page4.last().unwrap().position;
    let page5 = EventLog::read_stream(store.as_ref(), Some(cursor4), 10).await.unwrap();

    assert_eq!(page3.len(), 10);
    assert_eq!(page4.len(), 10);
    assert_eq!(page5.len(), 10, "page 5 must cover the final 10 events (41–50)");
    assert_eq!(page5.last().unwrap().position, EventPosition(50));

    // No overlap: all cursors are strictly increasing.
    let all_positions: Vec<_> = [&page1, &page2, &page3, &page4, &page5]
        .iter()
        .flat_map(|p| p.iter().map(|e| e.position))
        .collect();
    assert_eq!(all_positions.len(), 50, "all 5 pages must cover all 50 events exactly once");

    // Page after the last event must be empty.
    let empty = EventLog::read_stream(store.as_ref(), Some(EventPosition(50)), 10)
        .await
        .unwrap();
    assert!(empty.is_empty(), "read after last event must return empty");
}

/// (4): read_by_entity with limit=5 returns only events for that entity.
/// With 10 sessions seeded, each session has exactly 1 SessionCreated event.
#[tokio::test]
async fn read_by_entity_scoped_pagination() {
    let store = Arc::new(InMemoryStore::new());
    seed_50_events(&store).await;

    // Each session has exactly 1 SessionCreated event.
    let sess_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(SessionId::new("sess_3")),
        None,
        5,
    )
    .await
    .unwrap();

    assert_eq!(sess_events.len(), 1, "sess_3 must have exactly 1 entity-scoped event");
    assert!(
        matches!(
            &sess_events[0].envelope.payload,
            RuntimeEvent::SessionCreated(e) if e.session_id == SessionId::new("sess_3")
        ),
        "event must be the SessionCreated for sess_3"
    );

    // run_0 has 1 RunCreated + 1 TaskCreated under it = 2 events.
    // But TaskCreated is scoped to the Task entity, not Run. Run sees only RunCreated.
    let run0_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Run(RunId::new("run_0")),
        None,
        5,
    )
    .await
    .unwrap();
    assert_eq!(run0_events.len(), 1, "run_0 must have exactly 1 entity-scoped event");

    // task_1 has 1 TaskCreated event.
    let task1_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Task(TaskId::new("task_1")),
        None,
        5,
    )
    .await
    .unwrap();
    assert_eq!(task1_events.len(), 1, "task_1 must have 1 entity-scoped event");

    // limit=5 applied on a session that doesn't exist returns empty.
    let missing = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(SessionId::new("sess_999")),
        None,
        5,
    )
    .await
    .unwrap();
    assert!(missing.is_empty(), "non-existent entity must return empty");

    // Cursor-based scoped read: read after first event for a session returns empty.
    let cursor = sess_events[0].position;
    let after = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::Session(SessionId::new("sess_3")),
        Some(cursor),
        5,
    )
    .await
    .unwrap();
    assert!(
        after.is_empty(),
        "after the only session event the scoped stream must be empty"
    );
}

/// (5): find_by_causation_id works correctly when searching across 50 events.
/// Events at positions divisible by 5 (runs 0, 5, 10, 15) are tagged with causation IDs.
#[tokio::test]
async fn find_by_causation_id_across_large_event_set() {
    let store = Arc::new(InMemoryStore::new());
    seed_50_events(&store).await;

    // cmd_0 tags the RunCreated for run_0 — must be findable.
    let found_0 = EventLog::find_by_causation_id(store.as_ref(), "cmd_0")
        .await
        .unwrap();
    assert!(
        found_0.is_some(),
        "cmd_0 must be found among 50 events"
    );

    // cmd_5 tags run_5's RunCreated.
    let found_5 = EventLog::find_by_causation_id(store.as_ref(), "cmd_5")
        .await
        .unwrap();
    assert!(found_5.is_some(), "cmd_5 must be found");

    // cmd_10, cmd_15 also exist.
    for cmd in ["cmd_10", "cmd_15"] {
        let found = EventLog::find_by_causation_id(store.as_ref(), cmd).await.unwrap();
        assert!(found.is_some(), "{cmd} must be found in large event set");
    }

    // cmd_0 must appear BEFORE cmd_5 in the log (positions are monotonic).
    let pos_0 = found_0.unwrap();
    let pos_5 = found_5.unwrap();
    assert!(
        pos_0 < pos_5,
        "cmd_0 ({pos_0:?}) must appear before cmd_5 ({pos_5:?}) in the log"
    );

    // A causation_id that was never used returns None.
    let missing = EventLog::find_by_causation_id(store.as_ref(), "cmd_never_used")
        .await
        .unwrap();
    assert!(
        missing.is_none(),
        "non-existent causation_id must return None in large event set"
    );
}

/// (6): All 50 event positions are strictly monotonically increasing.
/// No gaps, no duplicates, no out-of-order positions.
#[tokio::test]
async fn positions_are_strictly_monotonically_increasing() {
    let store = Arc::new(InMemoryStore::new());
    seed_50_events(&store).await;

    let all = EventLog::read_stream(store.as_ref(), None, 100).await.unwrap();
    assert_eq!(all.len(), 50, "must have exactly 50 events");

    // Strict monotonic increase: each position must be exactly prev + 1.
    for window in all.windows(2) {
        let prev = window[0].position;
        let next = window[1].position;
        assert!(
            next > prev,
            "positions must be strictly increasing: {:?} must be < {:?}",
            prev, next
        );
        assert_eq!(
            next.0,
            prev.0 + 1,
            "positions must be sequential (no gaps): expected {:?}+1={:?} but got {:?}",
            prev, prev.0 + 1, next
        );
    }

    // First position is 1, last is 50.
    assert_eq!(all.first().unwrap().position, EventPosition(1));
    assert_eq!(all.last().unwrap().position, EventPosition(50));

    // No duplicate positions.
    let unique: std::collections::HashSet<_> = all.iter().map(|e| e.position).collect();
    assert_eq!(
        unique.len(),
        50,
        "all 50 positions must be unique"
    );
}

/// Bulk-append variant: all 50 events in a single append call still get
/// sequential positions and correct head_position.
#[tokio::test]
async fn bulk_append_preserves_sequential_positions() {
    let store = Arc::new(InMemoryStore::new());

    // Build 50 events in one batch.
    let batch: Vec<EventEnvelope<RuntimeEvent>> = (0u32..50)
        .map(|n| {
            ev(
                &format!("bulk_{n}"),
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new(format!("bulk_sess_{n}")),
                }),
            )
        })
        .collect();

    let positions = EventLog::append(store.as_ref(), &batch).await.unwrap();

    assert_eq!(positions.len(), 50, "append must return 50 positions");
    assert_eq!(positions[0], EventPosition(1));
    assert_eq!(positions[49], EventPosition(50));

    // All returned positions are sequential.
    for (i, pos) in positions.iter().enumerate() {
        assert_eq!(pos.0, (i + 1) as u64,
            "position {i} must be {}", i + 1);
    }

    let head = EventLog::head_position(store.as_ref()).await.unwrap().unwrap();
    assert_eq!(head, EventPosition(50));
}
