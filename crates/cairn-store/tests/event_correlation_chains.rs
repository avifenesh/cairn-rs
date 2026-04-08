//! Event correlation chain tests (RFC 002).
//!
//! Validates the RFC 002 event tracing contract: every event may carry a
//! `correlation_id` (logical trace group) and a `causation_id` (the command
//! that produced this event).  Together they form the full audit chain:
//!
//!   correlation_id — groups all events that belong to the same user request
//!                    or logical workflow, across entity boundaries.
//!   causation_id   — identifies the specific command/event that caused this
//!                    event (direct parent in the causal graph).
//!
//! Query paths:
//!   Correlation  — read_stream(None) + filter by envelope.correlation_id
//!   Causation    — EventLog::find_by_causation_id(cmd_id) → EventPosition
//!   Entity scope — EventLog::read_by_entity(EntityRef::*) — correlation
//!                  metadata survives entity-scoped reads

use cairn_domain::{
    CommandId, EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RunCreated, RunId,
    RuntimeEvent, SessionCreated, SessionId, TaskCreated, TaskId, TenantId, WorkspaceId,
};
use cairn_store::{EntityRef, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_corr"),
        workspace_id: WorkspaceId::new("w_corr"),
        project_id: ProjectId::new("p_corr"),
    }
}

fn session_evt(event_id: &str, session_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(event_id),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: SessionId::new(session_id),
        }),
    )
}

fn run_evt(event_id: &str, session_id: &str, run_id: &str) -> EventEnvelope<RuntimeEvent> {
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

fn task_evt(event_id: &str, run_id: &str, task_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(event_id),
        EventSource::Runtime,
        RuntimeEvent::TaskCreated(TaskCreated {
            project: project(),
            task_id: TaskId::new(task_id),
            parent_run_id: Some(RunId::new(run_id)),
            parent_task_id: None,
            prompt_release_id: None,
        }),
    )
}

// ── 1. Correlation chain: 3 events share correlation_id='chain_1' ─────────────

#[tokio::test]
async fn correlation_chain_links_session_run_task() {
    let store = InMemoryStore::new();

    let events = vec![
        session_evt("evt_s1", "sess_c1").with_correlation_id("chain_1"),
        run_evt("evt_r1", "sess_c1", "run_c1").with_correlation_id("chain_1"),
        task_evt("evt_t1", "run_c1", "task_c1").with_correlation_id("chain_1"),
    ];
    store.append(&events).await.unwrap();

    // Read all events and filter by correlation_id.
    let all = store.read_stream(None, 100).await.unwrap();
    let chain1: Vec<_> = all
        .iter()
        .filter(|e| e.envelope.correlation_id.as_deref() == Some("chain_1"))
        .collect();

    assert_eq!(chain1.len(), 3, "chain_1 must link exactly 3 events");

    // All three entity types are in the chain.
    assert!(chain1
        .iter()
        .any(|e| matches!(&e.envelope.payload, RuntimeEvent::SessionCreated(_))));
    assert!(chain1
        .iter()
        .any(|e| matches!(&e.envelope.payload, RuntimeEvent::RunCreated(_))));
    assert!(chain1
        .iter()
        .any(|e| matches!(&e.envelope.payload, RuntimeEvent::TaskCreated(_))));
}

// ── 2. Different correlation_ids don't cross-contaminate ──────────────────────

#[tokio::test]
async fn different_correlation_ids_are_isolated() {
    let store = InMemoryStore::new();

    store
        .append(&[
            // chain_1: session + run
            session_evt("e_s1", "sess_chain1").with_correlation_id("chain_1"),
            run_evt("e_r1", "sess_chain1", "run_chain1").with_correlation_id("chain_1"),
            // chain_2: session + run + task
            session_evt("e_s2", "sess_chain2").with_correlation_id("chain_2"),
            run_evt("e_r2", "sess_chain2", "run_chain2").with_correlation_id("chain_2"),
            task_evt("e_t2", "run_chain2", "task_chain2").with_correlation_id("chain_2"),
            // No correlation_id: uncorrelated event
            session_evt("e_s3", "sess_none"),
        ])
        .await
        .unwrap();

    let all = store.read_stream(None, 100).await.unwrap();

    let chain1: Vec<_> = all
        .iter()
        .filter(|e| e.envelope.correlation_id.as_deref() == Some("chain_1"))
        .collect();
    let chain2: Vec<_> = all
        .iter()
        .filter(|e| e.envelope.correlation_id.as_deref() == Some("chain_2"))
        .collect();
    let uncorrelated: Vec<_> = all
        .iter()
        .filter(|e| e.envelope.correlation_id.is_none())
        .collect();

    assert_eq!(chain1.len(), 2, "chain_1 has 2 events");
    assert_eq!(chain2.len(), 3, "chain_2 has 3 events");
    assert_eq!(uncorrelated.len(), 1, "1 event has no correlation_id");

    // No chain_1 event has correlation_id "chain_2" and vice versa.
    assert!(chain1
        .iter()
        .all(|e| e.envelope.correlation_id.as_deref() == Some("chain_1")));
    assert!(chain2
        .iter()
        .all(|e| e.envelope.correlation_id.as_deref() == Some("chain_2")));
}

// ── 3. Causation chain: event B caused by command that produced event A ────────

#[tokio::test]
async fn causation_id_links_cause_to_effect() {
    let store = InMemoryStore::new();

    // Event A: the root cause (no causation_id).
    let evt_a = session_evt("evt_cause_a", "sess_cause");
    // Event B: caused by the command that produced A (uses A's event_id as causation).
    let evt_b = run_evt("evt_effect_b", "sess_cause", "run_cause")
        .with_causation_id(CommandId::new("cmd_root_001"));

    store.append(&[evt_a, evt_b]).await.unwrap();

    // find_by_causation_id("cmd_root_001") should return B's position.
    let pos = store
        .find_by_causation_id("cmd_root_001")
        .await
        .unwrap()
        .expect("event B must be findable by its causation_id");

    // Position 2 = event B (A was position 1).
    assert_eq!(pos.0, 2, "event B is at position 2");

    // Event A has no causation_id.
    let all = store.read_stream(None, 100).await.unwrap();
    let evt_a_stored = all
        .iter()
        .find(|e| e.envelope.event_id.as_str() == "evt_cause_a")
        .unwrap();
    assert!(
        evt_a_stored.envelope.causation_id.is_none(),
        "root event has no causation_id"
    );

    // Event B carries the causation_id.
    let evt_b_stored = all
        .iter()
        .find(|e| e.envelope.event_id.as_str() == "evt_effect_b")
        .unwrap();
    assert_eq!(
        evt_b_stored
            .envelope
            .causation_id
            .as_ref()
            .map(|c| c.as_str()),
        Some("cmd_root_001"),
        "effect event must carry the causation_id"
    );
}

// ── 4. Multi-hop causation chain ──────────────────────────────────────────────

#[tokio::test]
async fn multi_hop_causation_chain() {
    let store = InMemoryStore::new();

    // cmd_001 → SessionCreated
    // cmd_002 → RunCreated  (caused by cmd_001's downstream action)
    // cmd_003 → TaskCreated (caused by cmd_002's downstream action)
    store
        .append(&[
            session_evt("e_hop_1", "sess_hop").with_causation_id(CommandId::new("cmd_hop_001")),
            run_evt("e_hop_2", "sess_hop", "run_hop")
                .with_causation_id(CommandId::new("cmd_hop_002")),
            task_evt("e_hop_3", "run_hop", "task_hop")
                .with_causation_id(CommandId::new("cmd_hop_003")),
        ])
        .await
        .unwrap();

    // Each causation_id maps to a different event.
    let pos1 = store
        .find_by_causation_id("cmd_hop_001")
        .await
        .unwrap()
        .unwrap();
    let pos2 = store
        .find_by_causation_id("cmd_hop_002")
        .await
        .unwrap()
        .unwrap();
    let pos3 = store
        .find_by_causation_id("cmd_hop_003")
        .await
        .unwrap()
        .unwrap();

    assert_eq!(pos1.0, 1);
    assert_eq!(pos2.0, 2);
    assert_eq!(pos3.0, 3);

    // Unknown causation_id returns None.
    let none = store.find_by_causation_id("cmd_nonexistent").await.unwrap();
    assert!(none.is_none());
}

// ── 5. Both correlation_id and causation_id on the same event ─────────────────

#[tokio::test]
async fn event_can_carry_both_correlation_and_causation() {
    let store = InMemoryStore::new();

    let evt = session_evt("evt_both", "sess_both")
        .with_correlation_id("workflow_abc")
        .with_causation_id(CommandId::new("cmd_trigger_001"));
    store.append(&[evt]).await.unwrap();

    let stored = store.read_stream(None, 10).await.unwrap();
    assert_eq!(stored.len(), 1);
    let e = &stored[0];

    assert_eq!(e.envelope.correlation_id.as_deref(), Some("workflow_abc"));
    assert_eq!(
        e.envelope.causation_id.as_ref().map(|c| c.as_str()),
        Some("cmd_trigger_001")
    );

    // Both query paths work independently.
    let by_causation = store.find_by_causation_id("cmd_trigger_001").await.unwrap();
    assert_eq!(by_causation.unwrap().0, 1);
}

// ── 6. find_by_causation_id returns the first matching event position ──────────

#[tokio::test]
async fn find_by_causation_id_returns_first_match() {
    let store = InMemoryStore::new();

    // Two events with different causation_ids.
    store
        .append(&[
            session_evt("e1", "sess_fbc1").with_causation_id(CommandId::new("cmd_fbc_a")),
            run_evt("e2", "sess_fbc1", "run_fbc").with_causation_id(CommandId::new("cmd_fbc_b")),
        ])
        .await
        .unwrap();

    // cmd_fbc_a → position 1 (SessionCreated).
    let pos_a = store
        .find_by_causation_id("cmd_fbc_a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pos_a.0, 1);

    // cmd_fbc_b → position 2 (RunCreated).
    let pos_b = store
        .find_by_causation_id("cmd_fbc_b")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pos_b.0, 2);

    assert_ne!(
        pos_a, pos_b,
        "different causation_ids map to different positions"
    );
}

// ── 7. Correlation persists through read_by_entity ────────────────────────────

#[tokio::test]
async fn correlation_id_persists_through_read_by_entity() {
    let store = InMemoryStore::new();

    let sess_id = SessionId::new("sess_entity_corr");

    store
        .append(&[
            session_evt("e_s", "sess_entity_corr").with_correlation_id("entity_chain_xyz"),
            run_evt("e_r", "sess_entity_corr", "run_entity_corr")
                .with_correlation_id("entity_chain_xyz"),
        ])
        .await
        .unwrap();

    // read_by_entity returns events scoped to this session.
    let sess_events = store
        .read_by_entity(&EntityRef::Session(sess_id.clone()), None, 10)
        .await
        .unwrap();

    assert_eq!(sess_events.len(), 1, "one SessionCreated for this session");
    assert_eq!(
        sess_events[0].envelope.correlation_id.as_deref(),
        Some("entity_chain_xyz"),
        "correlation_id must survive read_by_entity"
    );

    // Run events are not included in session entity filter.
    let run_events = store
        .read_by_entity(&EntityRef::Run(RunId::new("run_entity_corr")), None, 10)
        .await
        .unwrap();
    assert_eq!(run_events.len(), 1);
    assert_eq!(
        run_events[0].envelope.correlation_id.as_deref(),
        Some("entity_chain_xyz"),
        "correlation_id persists on run event too"
    );
}

// ── 8. Correlation chain respects SSE replay position ─────────────────────────

#[tokio::test]
async fn correlation_chain_visible_in_partial_replay() {
    let store = InMemoryStore::new();

    // Append chain events at positions 1-5, mixed with uncorrelated events.
    store
        .append(&[
            session_evt("e1", "sess_replay1"), // pos 1, no corr
            session_evt("e2", "sess_replay2").with_correlation_id("replay_chain"), // pos 2
            run_evt("e3", "sess_replay2", "run_replay").with_correlation_id("replay_chain"), // pos 3
            session_evt("e4", "sess_replay3"), // pos 4, no corr
            task_evt("e5", "run_replay", "task_replay").with_correlation_id("replay_chain"), // pos 5
        ])
        .await
        .unwrap();

    // Read from position 2 onwards — chain_start + chain events only.
    use cairn_store::EventPosition;
    let partial = store
        .read_stream(Some(EventPosition(1)), 100)
        .await
        .unwrap();
    assert_eq!(partial.len(), 4, "events at positions 2-5");

    let chain: Vec<_> = partial
        .iter()
        .filter(|e| e.envelope.correlation_id.as_deref() == Some("replay_chain"))
        .collect();
    assert_eq!(
        chain.len(),
        3,
        "replay from pos 1 includes all 3 chain events"
    );
}

// ── 9. Events without correlation_id have None (not empty string) ──────────────

#[tokio::test]
async fn uncorrelated_event_has_none_not_empty_string() {
    let store = InMemoryStore::new();

    store
        .append(&[session_evt("e1", "sess_no_corr")])
        .await
        .unwrap();

    let events = store.read_stream(None, 10).await.unwrap();
    assert_eq!(events.len(), 1);
    assert!(
        events[0].envelope.correlation_id.is_none(),
        "no correlation → None, not Some(\"\")"
    );
    assert!(
        events[0].envelope.causation_id.is_none(),
        "no causation → None"
    );
}
