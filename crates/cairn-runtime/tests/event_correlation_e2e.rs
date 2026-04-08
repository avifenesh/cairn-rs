//! RFC 002 event correlation system end-to-end integration test.
//!
//! Validates the causation chain contract:
//!   (1) append events with causation_id links forming A→B→C chain
//!   (2) query forward through the chain via find_by_causation_id
//!   (3) idempotency: check before append; duplicate causation_id is skipped
//!   (4) find_by_causation_id returns the correct event position
//!   (5) events without causation_id are chain roots (no parent)
//!   (6) correlation_id groups unrelated events into a session/trace
//!   (7) multi-chain isolation: two independent chains don't cross-link

use std::sync::Arc;

use cairn_domain::{
    CommandId, EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, TenantId,
};
use cairn_store::{event_log::EventPosition, EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_corr", "ws_corr", "proj_corr")
}

fn session_event(event_id: &str, session_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(event_id),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: SessionId::new(session_id),
        }),
    )
}

fn run_event(event_id: &str, run_id: &str, session_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(event_id),
        EventSource::Runtime,
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

// ── (1)+(2) Append A→B→C chain, traverse via causation_id ────────────────

#[tokio::test]
async fn causation_chain_links_three_events_in_sequence() {
    let store = Arc::new(InMemoryStore::new());

    // Event A: root (no causation).
    let evt_a = session_event("evt_a", "sess_chain_a");
    let positions_a = store.append(&[evt_a]).await.unwrap();
    let pos_a = positions_a[0];

    // Event B: caused by A.
    let evt_b =
        run_event("evt_b", "run_b", "sess_chain_a").with_causation_id(CommandId::new("evt_a"));
    let positions_b = store.append(&[evt_b]).await.unwrap();
    let pos_b = positions_b[0];

    // Event C: caused by B.
    let evt_c =
        run_event("evt_c", "run_c", "sess_chain_a").with_causation_id(CommandId::new("evt_b"));
    let positions_c = store.append(&[evt_c]).await.unwrap();
    let pos_c = positions_c[0];

    // Positions must be strictly increasing.
    assert!(pos_a < pos_b, "A must precede B in the log");
    assert!(pos_b < pos_c, "B must precede C in the log");

    // Traverse A→B: find_by_causation_id("evt_a") returns B's position.
    let b_pos = store
        .find_by_causation_id("evt_a")
        .await
        .unwrap()
        .expect("B must be found via causation_id=evt_a");
    assert_eq!(b_pos, pos_b, "causation evt_a must point to B's position");

    // Traverse B→C: find_by_causation_id("evt_b") returns C's position.
    let c_pos = store
        .find_by_causation_id("evt_b")
        .await
        .unwrap()
        .expect("C must be found via causation_id=evt_b");
    assert_eq!(c_pos, pos_c, "causation evt_b must point to C's position");
}

// ── (3) Idempotency: skip duplicate causation_id ──────────────────────────

#[tokio::test]
async fn idempotent_append_skips_duplicate_causation_id() {
    let store = Arc::new(InMemoryStore::new());

    let evt_root = session_event("evt_root", "sess_idem");
    store.append(&[evt_root]).await.unwrap();

    let evt_child = run_event("evt_child", "run_idem", "sess_idem")
        .with_causation_id(CommandId::new("evt_root"));
    store.append(&[evt_child]).await.unwrap();

    // Idempotency check: caller inspects find_by_causation_id before re-appending.
    let existing = store.find_by_causation_id("evt_root").await.unwrap();
    assert!(
        existing.is_some(),
        "find_by_causation_id must return Some when event already exists"
    );

    // Simulate idempotent caller: only append if not already present.
    let should_append = existing.is_none();
    assert!(
        !should_append,
        "caller must skip the append when causation_id is already registered"
    );

    // Log must still have exactly one event with causation_id=evt_root.
    let all = store.read_stream(None, 50).await.unwrap();
    let count = all
        .iter()
        .filter(|s| s.envelope.causation_id.as_ref().map(|id| id.as_str()) == Some("evt_root"))
        .count();
    assert_eq!(
        count, 1,
        "exactly one event must carry causation_id=evt_root"
    );
}

// ── (4) find_by_causation_id returns correct position ────────────────────

#[tokio::test]
async fn find_by_causation_id_returns_correct_position() {
    let store = Arc::new(InMemoryStore::new());

    // Append several events.
    store
        .append(&[session_event("e1", "sess_1")])
        .await
        .unwrap();
    store
        .append(&[session_event("e2", "sess_2")])
        .await
        .unwrap();

    let evt_with_causation =
        session_event("e3", "sess_3").with_causation_id(CommandId::new("cmd_xyz"));
    let positions = store.append(&[evt_with_causation]).await.unwrap();
    let expected_pos = positions[0];

    store
        .append(&[session_event("e4", "sess_4")])
        .await
        .unwrap();

    let found = store
        .find_by_causation_id("cmd_xyz")
        .await
        .unwrap()
        .expect("event with causation_id=cmd_xyz must be found");

    assert_eq!(
        found, expected_pos,
        "must return the exact position of the event carrying cmd_xyz"
    );

    // Non-existent causation_id returns None.
    let missing = store.find_by_causation_id("cmd_no_such").await.unwrap();
    assert!(missing.is_none(), "unknown causation_id must return None");
}

// ── (5) Root events have no causation_id ─────────────────────────────────

#[tokio::test]
async fn root_events_have_no_causation_id() {
    let store = Arc::new(InMemoryStore::new());

    let root = session_event("root_evt", "sess_root");
    // Confirm no causation_id before append.
    assert!(
        root.causation_id.is_none(),
        "root event must have no causation_id"
    );

    store.append(&[root]).await.unwrap();

    let all = store.read_stream(None, 10).await.unwrap();
    // AuditLogEntryRecorded or other derived events may appear; find the SessionCreated one.
    let session_events: Vec<_> = all
        .iter()
        .filter(|s| matches!(&s.envelope.payload, RuntimeEvent::SessionCreated(e) if e.session_id == SessionId::new("sess_root")))
        .collect();

    assert_eq!(session_events.len(), 1);
    assert!(
        session_events[0].envelope.causation_id.is_none(),
        "persisted root event must have no causation_id"
    );
}

// ── (6) correlation_id groups events into a trace ─────────────────────────

#[tokio::test]
async fn correlation_id_groups_events_into_a_session_trace() {
    let store = Arc::new(InMemoryStore::new());
    let trace = "trace_abc_123";

    // Three events in the same trace, two in a different one.
    let e1 = session_event("corr_e1", "sess_corr_1").with_correlation_id(trace);
    let e2 = run_event("corr_e2", "run_corr_1", "sess_corr_1").with_correlation_id(trace);
    let e3 = run_event("corr_e3", "run_corr_2", "sess_corr_1").with_correlation_id(trace);
    let e_other = session_event("corr_other", "sess_other").with_correlation_id("trace_other");

    store.append(&[e1, e2, e3, e_other]).await.unwrap();

    let all = store.read_stream(None, 20).await.unwrap();
    let in_trace: Vec<_> = all
        .iter()
        .filter(|s| s.envelope.correlation_id.as_deref() == Some(trace))
        .collect();

    assert_eq!(
        in_trace.len(),
        3,
        "exactly 3 events must share the trace correlation_id"
    );

    let other_trace: Vec<_> = all
        .iter()
        .filter(|s| s.envelope.correlation_id.as_deref() == Some("trace_other"))
        .collect();
    assert_eq!(
        other_trace.len(),
        1,
        "the unrelated event must have a different correlation_id"
    );
}

// ── (7) Two independent chains don't cross-link ───────────────────────────

#[tokio::test]
async fn independent_chains_do_not_cross_link() {
    let store = Arc::new(InMemoryStore::new());

    // Chain 1: X→Y
    let x = session_event("chain1_x", "sess_x");
    store.append(&[x]).await.unwrap();
    let y = run_event("chain1_y", "run_y", "sess_x").with_causation_id(CommandId::new("chain1_x"));
    store.append(&[y]).await.unwrap();

    // Chain 2: P→Q (independent)
    let p = session_event("chain2_p", "sess_p");
    store.append(&[p]).await.unwrap();
    let q = run_event("chain2_q", "run_q", "sess_p").with_causation_id(CommandId::new("chain2_p"));
    store.append(&[q]).await.unwrap();

    // Chain 1: find Y via X.
    let y_pos = store
        .find_by_causation_id("chain1_x")
        .await
        .unwrap()
        .unwrap();
    // Chain 2: find Q via P.
    let q_pos = store
        .find_by_causation_id("chain2_p")
        .await
        .unwrap()
        .unwrap();

    assert_ne!(y_pos, q_pos, "Y and Q must be at different positions");

    // Cross-link must return None: X does not cause Q, P does not cause Y.
    assert!(
        store
            .find_by_causation_id("chain1_x")
            .await
            .unwrap()
            .unwrap()
            != q_pos,
        "chain1_x must not point to Q"
    );
    assert!(
        store
            .find_by_causation_id("chain2_p")
            .await
            .unwrap()
            .unwrap()
            != y_pos,
        "chain2_p must not point to Y"
    );
}

// ── head_position advances with each append ───────────────────────────────

#[tokio::test]
async fn head_position_advances_monotonically() {
    let store = Arc::new(InMemoryStore::new());

    assert!(
        store.head_position().await.unwrap().is_none(),
        "empty log must have no head position"
    );

    store
        .append(&[session_event("h1", "sess_h1")])
        .await
        .unwrap();
    let head1 = store.head_position().await.unwrap().unwrap();

    store
        .append(&[session_event("h2", "sess_h2")])
        .await
        .unwrap();
    let head2 = store.head_position().await.unwrap().unwrap();

    assert!(head2 > head1, "head must advance after each append");
}
