//! RFC 002 — SQLite event log backend parity tests.
//!
//! Verifies that `SqliteEventLog` implements the `EventLog` contract with
//! identical semantics to `InMemoryStore`, using an in-memory SQLite database
//! so tests are self-contained and run without an external service.
//!
//! Run with:
//!   cargo test -p cairn-store --test sqlite_event_log --features sqlite

#![cfg(feature = "sqlite")]

use cairn_domain::{
    CommandId, EventEnvelope, EventId, EventSource, OperatorId, ProjectId, ProjectKey,
    RunId, RuntimeEvent, SessionId, TenantId, WorkspaceId,
    events::{RunCreated, SessionCreated},
    tenancy::OwnershipKey,
};
use cairn_store::{
    sqlite::{SqliteAdapter, SqliteEventLog},
    EntityRef, EventLog, EventPosition,
};

// ── Test harness ──────────────────────────────────────────────────────────────

/// Open an in-memory SQLite database and return a `SqliteEventLog` backed by it.
async fn open() -> SqliteEventLog {
    let adapter = SqliteAdapter::in_memory().await.expect("in-memory SQLite must open");
    SqliteEventLog::new(adapter.pool().clone())
}

// ── Fixture helpers ───────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("tenant_sqlite"),
        workspace_id: WorkspaceId::new("ws_sqlite"),
        project_id: ProjectId::new("proj_sqlite"),
    }
}

fn ownership() -> OwnershipKey {
    OwnershipKey::Project(project())
}

fn session_envelope(id: &str, session_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::new(
        EventId::new(id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: SessionId::new(session_id),
        }),
    )
}

fn run_envelope(id: &str, session_id: &str, run_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::new(
        EventId::new(id),
        EventSource::Runtime,
        ownership(),
        RuntimeEvent::RunCreated(RunCreated {
            project: project(),
            session_id: SessionId::new(session_id),
            run_id: RunId::new(run_id),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
    )
}

// ── 1. append + read_stream round-trip ────────────────────────────────────────

#[tokio::test]
async fn append_single_event_round_trips() {
    let log = open().await;
    let envelope = session_envelope("evt_rt_1", "sess_rt_1");

    let positions = log.append(&[envelope.clone()]).await.unwrap();
    assert_eq!(positions.len(), 1);
    assert!(positions[0].0 >= 1, "SQLite positions start at 1 (AUTOINCREMENT)");

    let all = log.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 1);

    let stored = &all[0];
    assert_eq!(stored.position, positions[0]);
    assert!(stored.stored_at > 0, "stored_at must be a non-zero timestamp");
    assert_eq!(stored.envelope.event_id, envelope.event_id);
    assert_eq!(stored.envelope.source,   envelope.source);
    assert_eq!(stored.envelope.ownership, envelope.ownership);
    assert_eq!(stored.envelope.payload,  envelope.payload);
}

#[tokio::test]
async fn append_empty_batch_returns_empty_positions() {
    let log = open().await;
    let positions = log.append(&[]).await.unwrap();
    assert!(positions.is_empty());
}

#[tokio::test]
async fn append_batch_of_ten_returns_ten_positions() {
    let log = open().await;

    let batch: Vec<_> = (0..10u32)
        .map(|i| session_envelope(&format!("e{i}"), &format!("s{i}")))
        .collect();

    let positions = log.append(&batch).await.unwrap();
    assert_eq!(positions.len(), 10);
}

#[tokio::test]
async fn read_stream_returns_all_events_when_after_is_none() {
    let log = open().await;

    for i in 0..5u32 {
        log.append(&[session_envelope(&format!("e{i}"), &format!("s{i}"))])
            .await.unwrap();
    }

    let all = log.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 5);
}

#[tokio::test]
async fn read_stream_respects_after_cursor() {
    let log = open().await;

    let mut last_pos = EventPosition(0);
    for i in 0..5u32 {
        let pos = log.append(&[session_envelope(&format!("e{i}"), &format!("s{i}"))])
            .await.unwrap();
        last_pos = pos[0];
    }

    // Positions start at 1; after=pos1 means we skip position 1.
    let first_pos = log.read_stream(None, 1).await.unwrap()[0].position;
    let after_first = log.read_stream(Some(first_pos), 100).await.unwrap();
    assert_eq!(after_first.len(), 4, "after first position must return 4 events");
    assert!(after_first.iter().all(|e| e.position > first_pos));
}

#[tokio::test]
async fn read_stream_respects_limit() {
    let log = open().await;

    for i in 0..10u32 {
        log.append(&[session_envelope(&format!("e{i}"), &format!("s{i}"))])
            .await.unwrap();
    }

    let three = log.read_stream(None, 3).await.unwrap();
    assert_eq!(three.len(), 3);
}

#[tokio::test]
async fn read_stream_empty_store_returns_empty() {
    let log = open().await;
    let all = log.read_stream(None, 100).await.unwrap();
    assert!(all.is_empty());
}

// ── 2. find_by_causation_id ───────────────────────────────────────────────────

#[tokio::test]
async fn find_by_causation_id_returns_position_when_found() {
    let log = open().await;

    let causation = "cmd_sqlite_42";
    let env = session_envelope("evt_caus", "sess_caus")
        .with_causation_id(CommandId::new(causation));

    let positions = log.append(&[env]).await.unwrap();
    let expected = positions[0];

    let found = log.find_by_causation_id(causation).await.unwrap();
    assert_eq!(found, Some(expected), "must return the exact position");
}

#[tokio::test]
async fn find_by_causation_id_returns_none_when_absent() {
    let log = open().await;
    log.append(&[session_envelope("e1", "s1")]).await.unwrap();

    let result = log.find_by_causation_id("cmd_ghost").await.unwrap();
    assert_eq!(result, None);
}

#[tokio::test]
async fn find_by_causation_id_returns_first_position_when_duplicate_command() {
    let log = open().await;

    let causation = "cmd_dupe";
    let env1 = session_envelope("evt_d1", "sess_d1")
        .with_causation_id(CommandId::new(causation));
    let env2 = session_envelope("evt_d2", "sess_d2")
        .with_causation_id(CommandId::new(causation));

    let pos1 = log.append(&[env1]).await.unwrap()[0];
    log.append(&[env2]).await.unwrap();

    let found = log.find_by_causation_id(causation).await.unwrap();
    // SQLite returns MIN(position) — the first event with this causation_id.
    assert_eq!(found, Some(pos1), "must return the first (minimum) position");
}

#[tokio::test]
async fn event_without_causation_id_does_not_match_find() {
    let log = open().await;
    log.append(&[session_envelope("e_no_caus", "s_nc")]).await.unwrap();

    // Find with any string must not match an event with NULL causation_id.
    let result = log.find_by_causation_id("").await.unwrap();
    assert_eq!(result, None);
}

// ── 3. head_position ──────────────────────────────────────────────────────────

#[tokio::test]
async fn head_position_none_on_empty_store() {
    let log = open().await;
    let head = log.head_position().await.unwrap();
    assert_eq!(head, None, "fresh SQLite store must have no head position");
}

#[tokio::test]
async fn head_position_equals_last_appended_position() {
    let log = open().await;

    let batch: Vec<_> = (0..5u32)
        .map(|i| session_envelope(&format!("e{i}"), &format!("s{i}")))
        .collect();

    let positions = log.append(&batch).await.unwrap();
    let last = *positions.last().unwrap();

    let head = log.head_position().await.unwrap();
    assert_eq!(head, Some(last), "head_position must equal last assigned position");
}

#[tokio::test]
async fn head_position_advances_after_each_append() {
    let log = open().await;

    for i in 0..3u32 {
        log.append(&[session_envelope(&format!("e{i}"), &format!("s{i}"))])
            .await.unwrap();
        let head = log.head_position().await.unwrap();
        assert!(head.is_some(), "head must be set after append {i}");
    }
}

// ── 4. read_by_entity ─────────────────────────────────────────────────────────

#[tokio::test]
async fn read_by_entity_session_returns_only_its_events() {
    let log = open().await;

    let sess_a = SessionId::new("sess_ent_a");
    let sess_b = SessionId::new("sess_ent_b");

    // Two events for sess_a, one for sess_b.
    log.append(&[session_envelope("e_a1", sess_a.as_str())]).await.unwrap();
    log.append(&[session_envelope("e_a2", sess_a.as_str())]).await.unwrap();
    log.append(&[session_envelope("e_b1", sess_b.as_str())]).await.unwrap();

    let a_events = log
        .read_by_entity(&EntityRef::Session(sess_a.clone()), None, 100)
        .await.unwrap();
    let b_events = log
        .read_by_entity(&EntityRef::Session(sess_b.clone()), None, 100)
        .await.unwrap();

    assert_eq!(a_events.len(), 2, "sess_a must have 2 events");
    assert_eq!(b_events.len(), 1, "sess_b must have 1 event");
}

#[tokio::test]
async fn read_by_entity_run_returns_run_events_only() {
    let log = open().await;

    let run_id = RunId::new("run_ent_1");

    log.append(&[session_envelope("e_sess", "sess_re")]).await.unwrap();
    log.append(&[run_envelope("e_run", "sess_re", run_id.as_str())]).await.unwrap();

    let run_events = log
        .read_by_entity(&EntityRef::Run(run_id.clone()), None, 100)
        .await.unwrap();

    assert_eq!(run_events.len(), 1, "only the RunCreated event must be returned");
    assert!(matches!(
        &run_events[0].envelope.payload,
        RuntimeEvent::RunCreated(e) if e.run_id == run_id
    ));
}

#[tokio::test]
async fn read_by_entity_respects_after_cursor() {
    let log = open().await;

    let sess = SessionId::new("sess_cursor");

    let p1 = log.append(&[session_envelope("e_c1", sess.as_str())]).await.unwrap()[0];
    log.append(&[session_envelope("e_c2", sess.as_str())]).await.unwrap();
    log.append(&[session_envelope("e_c3", sess.as_str())]).await.unwrap();

    let after_first = log
        .read_by_entity(&EntityRef::Session(sess.clone()), Some(p1), 100)
        .await.unwrap();

    assert_eq!(after_first.len(), 2, "after position 1 must return 2 events");
    assert!(after_first.iter().all(|e| e.position > p1));
}

#[tokio::test]
async fn read_by_entity_returns_empty_for_unknown_entity() {
    let log = open().await;
    log.append(&[session_envelope("e1", "s1")]).await.unwrap();

    let result = log
        .read_by_entity(&EntityRef::Session(SessionId::new("no_such_sess")), None, 100)
        .await.unwrap();
    assert!(result.is_empty());
}

// ── 5. Position monotonicity ──────────────────────────────────────────────────

#[tokio::test]
async fn positions_are_strictly_monotonically_increasing() {
    let log = open().await;

    let batch: Vec<_> = (0..10u32)
        .map(|i| session_envelope(&format!("e{i}"), &format!("s{i}")))
        .collect();

    let positions = log.append(&batch).await.unwrap();
    for w in positions.windows(2) {
        assert!(w[0] < w[1], "positions must be strictly increasing: {:?} < {:?}", w[0], w[1]);
    }
}

#[tokio::test]
async fn positions_never_repeat_across_separate_appends() {
    let log = open().await;

    let mut all_positions = Vec::new();
    for i in 0..5u32 {
        let pos = log
            .append(&[session_envelope(&format!("e{i}"), &format!("s{i}"))])
            .await.unwrap();
        all_positions.push(pos[0]);
    }

    let unique: std::collections::HashSet<_> = all_positions.iter().collect();
    assert_eq!(unique.len(), 5, "all 5 positions must be distinct");

    for w in all_positions.windows(2) {
        assert!(w[0] < w[1], "positions must be strictly increasing across separate appends");
    }
}

#[tokio::test]
async fn read_stream_returns_events_in_position_order() {
    let log = open().await;

    for i in 0..5u32 {
        log.append(&[session_envelope(&format!("e{i}"), &format!("s{i}"))])
            .await.unwrap();
    }

    let all = log.read_stream(None, 100).await.unwrap();
    for w in all.windows(2) {
        assert!(w[0].position < w[1].position,
            "events must be returned in ascending position order");
    }
}

// ── 6. EventEnvelope fields survive round-trip ───────────────────────────────

#[tokio::test]
async fn event_source_variants_survive_sqlite_round_trip() {
    let log = open().await;

    let sources = vec![
        EventSource::Runtime,
        EventSource::Scheduler,
        EventSource::System,
        EventSource::Operator { operator_id: OperatorId::new("op_sqlite") },
        EventSource::ExternalWorker { worker: "worker_sq".to_owned() },
    ];

    for (i, source) in sources.iter().enumerate() {
        let env = EventEnvelope::new(
            EventId::new(format!("evt_src_{i}")),
            source.clone(),
            ownership(),
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new(format!("sess_src_{i}")),
            }),
        );
        log.append(&[env]).await.unwrap();
    }

    let all = log.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), sources.len());

    for (stored, expected) in all.iter().zip(sources.iter()) {
        assert_eq!(&stored.envelope.source, expected,
            "EventSource {:?} must survive SQLite round-trip", expected);
    }
}

#[tokio::test]
async fn causation_and_correlation_ids_survive_round_trip() {
    let log = open().await;

    let env = session_envelope("evt_ids", "sess_ids")
        .with_causation_id(CommandId::new("cmd_sql_1"))
        .with_correlation_id("corr_sql_1");

    log.append(&[env]).await.unwrap();

    let all = log.read_stream(None, 10).await.unwrap();
    let stored = &all[0];

    assert_eq!(
        stored.envelope.causation_id.as_ref().map(|c| c.as_str()),
        Some("cmd_sql_1")
    );
    assert_eq!(
        stored.envelope.correlation_id.as_deref(),
        Some("corr_sql_1")
    );
}
