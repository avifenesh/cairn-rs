//! RFC 002 — Event persistence contract tests.
//!
//! Verifies the append-only log invariants that the rest of the system
//! depends on:
//!
//! - Every stored event carries a monotonically increasing position.
//! - `stored_at` timestamps are populated and non-zero.
//! - The full `EventEnvelope` (event_id, source, ownership, causation_id,
//!   correlation_id, payload) survives the append/read round-trip without
//!   mutation.
//! - `EventEnvelope.event_id` is unique per event.
//! - All `EventSource` variants are preserved verbatim.
//! - `OwnershipKey::Project` scoping is preserved and queryable.
//! - Correlation-ID chains (same correlation across multiple events) are
//!   preserved so callers can reconstruct workflow traces.

use std::collections::HashSet;

use cairn_domain::{
    ApprovalId, ApprovalRequirement, CheckpointDisposition, CheckpointId, EventEnvelope,
    EventId, EventSource, MailboxMessageId, OperatorId, ProjectId, ProjectKey, RunId,
    RuntimeEvent, SessionId, TaskId, TenantId, WorkspaceId,
    events::{
        ApprovalRequested, CheckpointRecorded, MailboxMessageAppended, RunCreated,
        SessionCreated, TaskCreated,
    },
    tenancy::OwnershipKey,
};
use cairn_store::{EventLog, EventPosition, InMemoryStore};

// ── Fixtures ──────────────────────────────────────────────────────────────────

fn project_a() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("tenant_a"),
        workspace_id: WorkspaceId::new("ws_a"),
        project_id: ProjectId::new("proj_a"),
    }
}

fn project_b() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("tenant_b"),
        workspace_id: WorkspaceId::new("ws_b"),
        project_id: ProjectId::new("proj_b"),
    }
}

/// Build a `SessionCreated` envelope for the given project.
fn session_event(
    id: &str,
    source: EventSource,
    project: &ProjectKey,
    session: &str,
) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(id, source, RuntimeEvent::SessionCreated(SessionCreated {
        project: project.clone(),
        session_id: SessionId::new(session),
    }))
}

/// Build a `RunCreated` envelope for the given project.
fn run_event(
    id: &str,
    source: EventSource,
    project: &ProjectKey,
    session: &str,
    run: &str,
) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(id, source, RuntimeEvent::RunCreated(RunCreated {
        project: project.clone(),
        session_id: SessionId::new(session),
        run_id: RunId::new(run),
        parent_run_id: None,
        prompt_release_id: None,
        agent_role_id: None,
    }))
}

/// Build a `TaskCreated` envelope.
fn task_event(id: &str, project: &ProjectKey, run: &str, task: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        id,
        EventSource::Runtime,
        RuntimeEvent::TaskCreated(TaskCreated {
            project: project.clone(),
            task_id: TaskId::new(task),
            parent_run_id: Some(RunId::new(run)),
            parent_task_id: None,
            prompt_release_id: None,
        }),
    )
}

/// Build an `ApprovalRequested` envelope.
fn approval_event(id: &str, project: &ProjectKey, approval: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        id,
        EventSource::Runtime,
        RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project.clone(),
            approval_id: ApprovalId::new(approval),
            run_id: None,
            task_id: None,
            requirement: ApprovalRequirement::Required,
        }),
    )
}

/// Build a `CheckpointRecorded` envelope.
fn checkpoint_event(id: &str, project: &ProjectKey, run: &str, ckpt: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        id,
        EventSource::System,
        RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
            project: project.clone(),
            run_id: RunId::new(run),
            checkpoint_id: CheckpointId::new(ckpt),
            disposition: CheckpointDisposition::Latest,
            data: None,
        }),
    )
}

/// Build a `MailboxMessageAppended` envelope.
fn mailbox_event(id: &str, project: &ProjectKey, msg: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        id,
        EventSource::Runtime,
        RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
            project: project.clone(),
            message_id: MailboxMessageId::new(msg),
            run_id: None,
            task_id: None,
            content: "hello from mailbox".to_owned(),
            from_run_id: None,
            from_task_id: None,
            deliver_at_ms: 0,
                          sender: None,
             recipient: None,
             body: None,
             sent_at: None,
             delivery_status: None,
        }),
    )
}

// ── 1. Append 10 events with mixed entity types ───────────────────────────────

#[tokio::test]
async fn append_ten_mixed_entity_events_all_stored() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let batch: Vec<EventEnvelope<RuntimeEvent>> = vec![
        session_event("evt_01", EventSource::Runtime,   &pa, "sess_01"),
        session_event("evt_02", EventSource::Scheduler, &pa, "sess_02"),
        run_event(    "evt_03", EventSource::Runtime,   &pa, "sess_01", "run_01"),
        run_event(    "evt_04", EventSource::Runtime,   &pa, "sess_02", "run_02"),
        task_event(   "evt_05", &pa, "run_01", "task_01"),
        task_event(   "evt_06", &pa, "run_01", "task_02"),
        approval_event("evt_07", &pa, "approval_01"),
        checkpoint_event("evt_08", &pa, "run_01", "ckpt_01"),
        mailbox_event("evt_09", &pa, "msg_01"),
        session_event("evt_10", EventSource::System, &pa, "sess_03"),
    ];

    let positions = store.append(&batch).await.unwrap();

    assert_eq!(positions.len(), 10, "expected 10 positions");

    // Verify all events are readable back.
    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 10);
}

// ── 2. StoredEvent carries position + stored_at + envelope ───────────────────

#[tokio::test]
async fn stored_event_fields_populated() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let envelope = session_event("evt_fields", EventSource::Runtime, &pa, "sess_fields");
    store.append(&[envelope.clone()]).await.unwrap();

    let events = store.read_stream(None, 10).await.unwrap();
    assert_eq!(events.len(), 1);

    let stored = &events[0];

    // Position is non-zero (starts at 1).
    assert!(stored.position.0 >= 1, "position must be ≥ 1");

    // stored_at is populated.
    assert!(stored.stored_at > 0, "stored_at must be a non-zero timestamp");

    // Envelope round-trips intact.
    assert_eq!(stored.envelope.event_id, envelope.event_id);
    assert_eq!(stored.envelope.source, envelope.source);
    assert_eq!(stored.envelope.ownership, envelope.ownership);
    assert_eq!(stored.envelope.payload, envelope.payload);
}

// ── 3. EventEnvelope.event_id is unique per event ────────────────────────────

#[tokio::test]
async fn event_ids_are_unique_across_ten_events() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let batch: Vec<EventEnvelope<RuntimeEvent>> = (0..10)
        .map(|i| session_event(&format!("uniq_evt_{i:02}"), EventSource::Runtime, &pa, &format!("sess_uniq_{i}")))
        .collect();

    store.append(&batch).await.unwrap();

    let all = store.read_stream(None, 100).await.unwrap();
    let ids: HashSet<String> = all
        .iter()
        .map(|e| e.envelope.event_id.as_str().to_owned())
        .collect();

    assert_eq!(ids.len(), 10, "all 10 event_ids must be distinct");
}

#[tokio::test]
async fn event_positions_are_strictly_increasing() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let batch: Vec<_> = (0..5)
        .map(|i| session_event(&format!("pos_evt_{i}"), EventSource::Runtime, &pa, &format!("sess_{i}")))
        .collect();

    let positions = store.append(&batch).await.unwrap();

    for w in positions.windows(2) {
        assert!(w[0] < w[1], "positions must be strictly increasing: {:?} < {:?}", w[0], w[1]);
    }
}

// ── 4. EventSource variants are preserved through append/read ────────────────

#[tokio::test]
async fn all_event_source_variants_survive_round_trip() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let sources = vec![
        EventSource::Runtime,
        EventSource::Scheduler,
        EventSource::System,
        EventSource::Operator {
            operator_id: OperatorId::new("op_contract_test"),
        },
        EventSource::ExternalWorker {
            worker: "worker_node_1".to_owned(),
        },
    ];

    let batch: Vec<EventEnvelope<RuntimeEvent>> = sources
        .iter()
        .enumerate()
        .map(|(i, src)| session_event(
            &format!("src_evt_{i}"),
            src.clone(),
            &pa,
            &format!("sess_src_{i}"),
        ))
        .collect();

    store.append(&batch).await.unwrap();

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), sources.len());

    for (stored, expected_src) in all.iter().zip(sources.iter()) {
        assert_eq!(
            &stored.envelope.source, expected_src,
            "EventSource {:?} did not survive round-trip",
            expected_src
        );
    }
}

#[tokio::test]
async fn operator_source_preserves_operator_id() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let source = EventSource::Operator {
        operator_id: OperatorId::new("op_alice_42"),
    };
    let envelope = session_event("evt_op_src", source.clone(), &pa, "sess_op_src");
    store.append(&[envelope]).await.unwrap();

    let all = store.read_stream(None, 10).await.unwrap();
    assert_eq!(all.len(), 1);

    match &all[0].envelope.source {
        EventSource::Operator { operator_id } => {
            assert_eq!(operator_id.as_str(), "op_alice_42");
        }
        other => panic!("expected Operator source, got {:?}", other),
    }
}

#[tokio::test]
async fn external_worker_source_preserves_worker_name() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let source = EventSource::ExternalWorker { worker: "gpu-node-7".to_owned() };
    let envelope = session_event("evt_worker_src", source, &pa, "sess_worker_src");
    store.append(&[envelope]).await.unwrap();

    let all = store.read_stream(None, 10).await.unwrap();
    match &all[0].envelope.source {
        EventSource::ExternalWorker { worker } => assert_eq!(worker, "gpu-node-7"),
        other => panic!("expected ExternalWorker, got {:?}", other),
    }
}

// ── 5. OwnershipKey.Project scoping ──────────────────────────────────────────

#[tokio::test]
async fn ownership_key_project_scoping_is_preserved() {
    let store = InMemoryStore::new();
    let pa = project_a();
    let pb = project_b();

    // Append events for two different projects.
    let batch = vec![
        session_event("evt_scope_a1", EventSource::Runtime, &pa, "sess_a1"),
        session_event("evt_scope_a2", EventSource::Runtime, &pa, "sess_a2"),
        session_event("evt_scope_b1", EventSource::Runtime, &pb, "sess_b1"),
    ];
    store.append(&batch).await.unwrap();

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 3);

    // Verify OwnershipKey.Project fields match what was submitted.
    let a_events: Vec<_> = all.iter().filter(|e| {
        matches!(&e.envelope.ownership, OwnershipKey::Project(pk) if pk == &pa)
    }).collect();
    let b_events: Vec<_> = all.iter().filter(|e| {
        matches!(&e.envelope.ownership, OwnershipKey::Project(pk) if pk == &pb)
    }).collect();

    assert_eq!(a_events.len(), 2, "project_a should have 2 events");
    assert_eq!(b_events.len(), 1, "project_b should have 1 event");
}

#[tokio::test]
async fn ownership_key_tenant_id_is_preserved() {
    let store = InMemoryStore::new();
    let pa = project_a();
    let pb = project_b();

    let batch = vec![
        run_event("evt_tenant_a", EventSource::Runtime, &pa, "sess_ta", "run_ta"),
        run_event("evt_tenant_b", EventSource::Runtime, &pb, "sess_tb", "run_tb"),
    ];
    store.append(&batch).await.unwrap();

    let all = store.read_stream(None, 100).await.unwrap();
    for stored in &all {
        if let OwnershipKey::Project(pk) = &stored.envelope.ownership {
            let event_id = stored.envelope.event_id.as_str();
            if event_id.contains("_a") {
                assert_eq!(pk.tenant_id.as_str(), "tenant_a");
            } else {
                assert_eq!(pk.tenant_id.as_str(), "tenant_b");
            }
        }
    }
}

#[tokio::test]
async fn read_by_entity_respects_project_scope() {
    use cairn_store::{EntityRef, EventLog as _};

    let store = InMemoryStore::new();
    let pa = project_a();
    let pb = project_b();

    // Two sessions in project_a, one in project_b.
    let sess_a = SessionId::new("sess_scope_a");
    let sess_b = SessionId::new("sess_scope_b");

    let batch = vec![
        session_event("evt_rbe_a", EventSource::Runtime, &pa, sess_a.as_str()),
        session_event("evt_rbe_b", EventSource::Runtime, &pb, sess_b.as_str()),
    ];
    store.append(&batch).await.unwrap();

    // Reading by project_a session should return only its event.
    let a_events = store
        .read_by_entity(&EntityRef::Session(sess_a.clone()), None, 100)
        .await
        .unwrap();
    assert_eq!(a_events.len(), 1, "project_a session should have 1 event");
    if let OwnershipKey::Project(pk) = &a_events[0].envelope.ownership {
        assert_eq!(pk.tenant_id.as_str(), "tenant_a");
    }

    // Reading by project_b session returns only its event.
    let b_events = store
        .read_by_entity(&EntityRef::Session(sess_b.clone()), None, 100)
        .await
        .unwrap();
    assert_eq!(b_events.len(), 1, "project_b session should have 1 event");
}

// ── 6. Correlation-ID chains are preserved ───────────────────────────────────

#[tokio::test]
async fn correlation_id_chain_survives_round_trip() {
    let store = InMemoryStore::new();
    let pa = project_a();
    let correlation = "corr_workflow_xyz";

    // Build a chain: session → run → task → checkpoint, all correlated.
    let events: Vec<EventEnvelope<RuntimeEvent>> = vec![
        session_event("evt_corr_1", EventSource::Runtime, &pa, "sess_corr")
            .with_correlation_id(correlation),
        run_event("evt_corr_2", EventSource::Runtime, &pa, "sess_corr", "run_corr")
            .with_correlation_id(correlation),
        task_event("evt_corr_3", &pa, "run_corr", "task_corr")
            .with_correlation_id(correlation),
        checkpoint_event("evt_corr_4", &pa, "run_corr", "ckpt_corr")
            .with_correlation_id(correlation),
    ];

    store.append(&events).await.unwrap();

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 4);

    // Every event in the chain must carry the same correlation_id.
    for stored in &all {
        assert_eq!(
            stored.envelope.correlation_id.as_deref(),
            Some(correlation),
            "event {} should carry correlation_id '{correlation}'",
            stored.envelope.event_id.as_str()
        );
    }
}

#[tokio::test]
async fn events_without_correlation_id_have_none() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let envelope = session_event("evt_no_corr", EventSource::Runtime, &pa, "sess_no_corr");
    // No with_correlation_id() call.
    store.append(&[envelope]).await.unwrap();

    let all = store.read_stream(None, 10).await.unwrap();
    assert_eq!(all[0].envelope.correlation_id, None);
}

#[tokio::test]
async fn multiple_correlation_chains_coexist_independently() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let chain_x = "corr_chain_x";
    let chain_y = "corr_chain_y";

    let batch = vec![
        session_event("evt_cx_1", EventSource::Runtime, &pa, "sess_cx_1")
            .with_correlation_id(chain_x),
        session_event("evt_cy_1", EventSource::Runtime, &pa, "sess_cy_1")
            .with_correlation_id(chain_y),
        run_event("evt_cx_2", EventSource::Runtime, &pa, "sess_cx_1", "run_cx_1")
            .with_correlation_id(chain_x),
        run_event("evt_cy_2", EventSource::Runtime, &pa, "sess_cy_1", "run_cy_1")
            .with_correlation_id(chain_y),
        // One uncorrelated event.
        session_event("evt_nc", EventSource::System, &pa, "sess_nc"),
    ];

    store.append(&batch).await.unwrap();

    let all = store.read_stream(None, 100).await.unwrap();
    let cx_chain: Vec<_> = all.iter()
        .filter(|e| e.envelope.correlation_id.as_deref() == Some(chain_x))
        .collect();
    let cy_chain: Vec<_> = all.iter()
        .filter(|e| e.envelope.correlation_id.as_deref() == Some(chain_y))
        .collect();
    let no_corr: Vec<_> = all.iter()
        .filter(|e| e.envelope.correlation_id.is_none())
        .collect();

    assert_eq!(cx_chain.len(), 2, "chain_x should have 2 events");
    assert_eq!(cy_chain.len(), 2, "chain_y should have 2 events");
    assert_eq!(no_corr.len(), 1, "1 event should have no correlation_id");
}

// ── 7. Causation-ID idempotency key is preserved ─────────────────────────────

#[tokio::test]
async fn causation_id_survives_round_trip() {
    let store = InMemoryStore::new();
    let pa = project_a();
    let causation = "cmd_bootstrap_42";

    let envelope = session_event("evt_caus", EventSource::Runtime, &pa, "sess_caus")
        .with_causation_id(causation);

    store.append(&[envelope]).await.unwrap();

    let all = store.read_stream(None, 10).await.unwrap();
    let cid = all[0].envelope.causation_id.as_ref().unwrap();
    assert_eq!(cid.as_str(), causation);
}

#[tokio::test]
async fn find_by_causation_id_returns_correct_position() {
    let store = InMemoryStore::new();
    let pa = project_a();
    let causation = "cmd_find_me";

    // Append one event without causation, then one with.
    store.append(&[session_event("evt_no_caus", EventSource::Runtime, &pa, "sess_nc")]).await.unwrap();
    let positions = store
        .append(&[session_event("evt_with_caus", EventSource::Runtime, &pa, "sess_wc")
            .with_causation_id(causation)])
        .await
        .unwrap();

    let expected_pos = positions[0];
    let found = store.find_by_causation_id(causation).await.unwrap();

    assert_eq!(found, Some(expected_pos), "find_by_causation_id must return the assigned position");
}

#[tokio::test]
async fn find_by_causation_id_returns_none_when_absent() {
    let store = InMemoryStore::new();
    let pa = project_a();

    store.append(&[session_event("evt_absent", EventSource::Runtime, &pa, "sess_absent")]).await.unwrap();

    let result = store.find_by_causation_id("cmd_ghost").await.unwrap();
    assert_eq!(result, None);
}

// ── 8. Head position tracks the last appended event ──────────────────────────

#[tokio::test]
async fn head_position_matches_last_appended_event() {
    let store = InMemoryStore::new();
    let pa = project_a();

    assert_eq!(
        store.head_position().await.unwrap(),
        None,
        "fresh store head must be None"
    );

    let batch: Vec<_> = (0..5u32)
        .map(|i| session_event(&format!("evt_head_{i}"), EventSource::Runtime, &pa, &format!("sess_h_{i}")))
        .collect();

    let positions = store.append(&batch).await.unwrap();
    let last_pos = *positions.last().unwrap();

    let head = store.head_position().await.unwrap();
    assert_eq!(head, Some(last_pos), "head_position must equal last assigned position");
}

// ── 9. Payload identity through round-trip ───────────────────────────────────

#[tokio::test]
async fn all_mixed_entity_payloads_round_trip_without_mutation() {
    let store = InMemoryStore::new();
    let pa = project_a();

    let original: Vec<EventEnvelope<RuntimeEvent>> = vec![
        session_event(   "evt_rt_01", EventSource::Runtime,   &pa, "sess_rt_1"),
        run_event(       "evt_rt_02", EventSource::Scheduler, &pa, "sess_rt_1", "run_rt_1"),
        task_event(      "evt_rt_03",                         &pa, "run_rt_1", "task_rt_1"),
        approval_event(  "evt_rt_04",                         &pa, "appr_rt_1"),
        checkpoint_event("evt_rt_05",                         &pa, "run_rt_1", "ckpt_rt_1"),
        mailbox_event(   "evt_rt_06",                         &pa, "msg_rt_1"),
        session_event(   "evt_rt_07", EventSource::System,    &pa, "sess_rt_2"),
        run_event(       "evt_rt_08", EventSource::Runtime,   &pa, "sess_rt_2", "run_rt_2"),
        task_event(      "evt_rt_09",                         &pa, "run_rt_2", "task_rt_2"),
        session_event(   "evt_rt_10", EventSource::ExternalWorker { worker: "w1".to_owned() }, &pa, "sess_rt_3"),
    ];

    store.append(&original).await.unwrap();

    let stored = store.read_stream(None, 100).await.unwrap();
    assert_eq!(stored.len(), original.len());

    for (orig, stored_ev) in original.iter().zip(stored.iter()) {
        assert_eq!(stored_ev.envelope.event_id, orig.event_id,    "event_id mismatch");
        assert_eq!(stored_ev.envelope.source,   orig.source,      "source mismatch");
        assert_eq!(stored_ev.envelope.ownership, orig.ownership,  "ownership mismatch");
        assert_eq!(stored_ev.envelope.payload,  orig.payload,     "payload mismatch");
    }
}
