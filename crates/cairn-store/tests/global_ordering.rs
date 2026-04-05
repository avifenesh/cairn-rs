//! Event log global ordering tests (RFC 002).
//!
//! The RFC 002 event log contract requires that every appended event receives
//! a globally unique, strictly monotonically increasing position, regardless
//! of which entity type or session the event belongs to.
//!
//! Guarantees under test:
//!   1. All positions are strictly monotonic (pos[n+1] > pos[n])
//!   2. No position gaps (pos[0]=1, pos[n]=n+1 for all n)
//!   3. Interleaved entity types share the same position sequence
//!   4. Batch appends assign sequential positions within the batch
//!   5. head_position() always reflects the last appended event

use cairn_domain::{
    ApprovalId, ApprovalRequested, EventEnvelope, EventId, EventSource, MailboxMessageAppended,
    MailboxMessageId, ProjectId, ProjectKey, RunCreated, RunId, RuntimeEvent, SessionCreated,
    SessionId, SignalId, SignalIngested, TaskCreated, TaskId, TenantId, WorkspaceId,
};
use cairn_domain::policy::ApprovalRequirement;
use cairn_store::{EventLog, EventPosition, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id:    TenantId::new("t_ord"),
        workspace_id: WorkspaceId::new("w_ord"),
        project_id:   ProjectId::new("p_ord"),
    }
}

fn session_evt(n: u32) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("sess_evt_{n:04}")),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(), session_id: SessionId::new(format!("sess_{n:04}")),
        }),
    )
}

fn run_evt(n: u32) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("run_evt_{n:04}")),
        EventSource::Runtime,
        RuntimeEvent::RunCreated(RunCreated {
            project:           project(),
            session_id:        SessionId::new(format!("sess_{n:04}")),
            run_id:            RunId::new(format!("run_{n:04}")),
            parent_run_id:     None,
            prompt_release_id: None,
            agent_role_id:     None,
        }),
    )
}

fn task_evt(n: u32) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("task_evt_{n:04}")),
        EventSource::Runtime,
        RuntimeEvent::TaskCreated(TaskCreated {
            project:           project(),
            task_id:           TaskId::new(format!("task_{n:04}")),
            parent_run_id:     Some(RunId::new(format!("run_{n:04}"))),
            parent_task_id:    None,
            prompt_release_id: None,
        }),
    )
}

fn approval_evt(n: u32) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("appr_evt_{n:04}")),
        EventSource::Runtime,
        RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project:     project(),
            approval_id: ApprovalId::new(format!("appr_{n:04}")),
            run_id:      Some(RunId::new(format!("run_{n:04}"))),
            task_id:     None,
            requirement: ApprovalRequirement::Required,
        }),
    )
}

fn signal_evt(n: u32) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("sig_evt_{n:04}")),
        EventSource::Runtime,
        RuntimeEvent::SignalIngested(SignalIngested {
            project:      project(),
            signal_id:    SignalId::new(format!("sig_{n:04}")),
            source:       "timer".to_owned(),
            payload:      serde_json::json!({ "n": n }),
            timestamp_ms: n as u64 * 10,
        }),
    )
}

fn mailbox_evt(n: u32) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("mbx_evt_{n:04}")),
        EventSource::Runtime,
        RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
            project:    project(),
            message_id: MailboxMessageId::new(format!("msg_{n:04}")),
            run_id:     None,
            task_id:    None,
            content:    format!("message {n}"),
            from_run_id:  None,
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

/// Build 100 events interleaved across 5 entity types (20 of each).
fn build_100_interleaved() -> Vec<EventEnvelope<RuntimeEvent>> {
    // Round-robin: session, run, task, approval, signal for each i
    (0u32..20)
        .flat_map(|i| {
            vec![
                session_evt(i),
                run_evt(i),
                task_evt(i),
                approval_evt(i),
                signal_evt(i),
            ]
        })
        .collect()
}

// ── 1. Strict monotonicity over 100 interleaved events ────────────────────────

#[tokio::test]
async fn positions_are_strictly_monotonic() {
    let store = InMemoryStore::new();

    // Append all 100 events in one batch.
    let positions = store.append(&build_100_interleaved()).await.unwrap();
    assert_eq!(positions.len(), 100);

    // Every position must be strictly greater than the previous.
    for window in positions.windows(2) {
        assert!(
            window[1].0 > window[0].0,
            "position {} must be > {} (strict monotonicity violated)",
            window[1].0, window[0].0
        );
    }
}

// ── 2. No position gaps ───────────────────────────────────────────────────────

#[tokio::test]
async fn no_position_gaps_over_100_events() {
    let store = InMemoryStore::new();
    let positions = store.append(&build_100_interleaved()).await.unwrap();

    assert_eq!(positions.len(), 100);

    // Positions must be contiguous starting at 1.
    for (i, pos) in positions.iter().enumerate() {
        let expected = (i + 1) as u64;
        assert_eq!(
            pos.0, expected,
            "position at index {i} must be {expected}, got {}",
            pos.0
        );
    }
}

// ── 3. read_stream returns all 100 in order with no gaps ──────────────────────

#[tokio::test]
async fn read_stream_returns_all_100_in_order() {
    let store = InMemoryStore::new();
    store.append(&build_100_interleaved()).await.unwrap();

    let events = store.read_stream(None, 200).await.unwrap();
    assert_eq!(events.len(), 100, "read_stream must return all 100 events");

    // Positions start at 1 and are contiguous.
    for (i, stored) in events.iter().enumerate() {
        let expected = (i + 1) as u64;
        assert_eq!(
            stored.position.0, expected,
            "event at index {i}: position must be {expected}"
        );
    }

    // Consecutive positions are strictly increasing.
    for window in events.windows(2) {
        assert!(
            window[1].position.0 > window[0].position.0,
            "read_stream: positions must be strictly monotonic"
        );
    }
}

// ── 4. Interleaved entity types share the same position sequence ──────────────

#[tokio::test]
async fn interleaved_entity_types_share_global_sequence() {
    let store = InMemoryStore::new();
    store.append(&build_100_interleaved()).await.unwrap();

    let events = store.read_stream(None, 200).await.unwrap();

    // Collect which positions are occupied by each entity type.
    let mut session_positions  = vec![];
    let mut run_positions      = vec![];
    let mut task_positions     = vec![];
    let mut approval_positions = vec![];
    let mut signal_positions   = vec![];

    for e in &events {
        match &e.envelope.payload {
            RuntimeEvent::SessionCreated(_)  => session_positions.push(e.position.0),
            RuntimeEvent::RunCreated(_)      => run_positions.push(e.position.0),
            RuntimeEvent::TaskCreated(_)     => task_positions.push(e.position.0),
            RuntimeEvent::ApprovalRequested(_) => approval_positions.push(e.position.0),
            RuntimeEvent::SignalIngested(_)  => signal_positions.push(e.position.0),
            _ => {}
        }
    }

    // Each entity type contributed exactly 20 events.
    assert_eq!(session_positions.len(),  20, "20 session events");
    assert_eq!(run_positions.len(),      20, "20 run events");
    assert_eq!(task_positions.len(),     20, "20 task events");
    assert_eq!(approval_positions.len(), 20, "20 approval events");
    assert_eq!(signal_positions.len(),   20, "20 signal events");

    // All 100 positions are represented — no entity type monopolises the sequence.
    let mut all: Vec<u64> = session_positions.iter()
        .chain(run_positions.iter())
        .chain(task_positions.iter())
        .chain(approval_positions.iter())
        .chain(signal_positions.iter())
        .copied()
        .collect();
    all.sort_unstable();
    for (i, &pos) in all.iter().enumerate() {
        assert_eq!(pos, (i + 1) as u64, "combined positions must cover 1..=100");
    }
}

// ── 5. Batch appends preserve ordering within and across batches ──────────────

#[tokio::test]
async fn batch_appends_preserve_global_ordering() {
    let store = InMemoryStore::new();

    // Append 10 batches of 10 events, one batch at a time.
    for batch in 0u32..10 {
        let events: Vec<_> = (0u32..10)
            .map(|i| mailbox_evt(batch * 10 + i))
            .collect();
        store.append(&events).await.unwrap();
    }

    let all_events = store.read_stream(None, 200).await.unwrap();
    assert_eq!(all_events.len(), 100);

    // All 100 positions are strictly monotonic with no gaps.
    for (i, e) in all_events.iter().enumerate() {
        assert_eq!(e.position.0, (i + 1) as u64,
            "batch append: position at index {i} must be {}", i + 1);
    }

    for window in all_events.windows(2) {
        assert!(window[1].position.0 == window[0].position.0 + 1,
            "consecutive positions must differ by exactly 1");
    }
}

// ── 6. head_position = 100 after all appends ──────────────────────────────────

#[tokio::test]
async fn head_position_equals_100_after_100_appends() {
    let store = InMemoryStore::new();
    store.append(&build_100_interleaved()).await.unwrap();

    let head = store.head_position().await.unwrap()
        .expect("head_position must be Some after 100 appends");

    assert_eq!(head.0, 100, "head_position must be 100 after 100 events");
}

// ── 7. head_position tracks incremental appends exactly ───────────────────────

#[tokio::test]
async fn head_position_tracks_each_append() {
    let store = InMemoryStore::new();

    assert!(store.head_position().await.unwrap().is_none(), "empty log has no head");

    for n in 1u32..=100 {
        store.append(&[session_evt(n)]).await.unwrap();
        let head = store.head_position().await.unwrap().unwrap();
        assert_eq!(head.0, n as u64, "after append {n}: head must be {n}");
    }
}

// ── 8. Positions from interleaved single-event appends are gapless ─────────────

#[tokio::test]
async fn single_event_interleaved_appends_are_gapless() {
    let store = InMemoryStore::new();

    // Alternate between 5 entity types, one event at a time.
    for i in 0u32..20 {
        store.append(&[session_evt(i)]).await.unwrap();
        store.append(&[run_evt(i)]).await.unwrap();
        store.append(&[task_evt(i)]).await.unwrap();
        store.append(&[approval_evt(i)]).await.unwrap();
        store.append(&[signal_evt(i)]).await.unwrap();
    }

    let head = store.head_position().await.unwrap().unwrap();
    assert_eq!(head.0, 100, "100 single-event appends = head at 100");

    let events = store.read_stream(None, 200).await.unwrap();
    assert_eq!(events.len(), 100);

    // No gaps anywhere.
    for (i, e) in events.iter().enumerate() {
        assert_eq!(e.position.0, (i + 1) as u64);
    }
}

// ── 9. read_stream cursor correctly skips events before the cursor ─────────────

#[tokio::test]
async fn read_stream_cursor_skips_and_reads_remainder() {
    let store = InMemoryStore::new();
    store.append(&build_100_interleaved()).await.unwrap();

    // Read the last 50 events (after position 50).
    let tail = store.read_stream(Some(EventPosition(50)), 200).await.unwrap();
    assert_eq!(tail.len(), 50, "50 events after position 50");
    assert_eq!(tail[0].position.0,  51, "first event after cursor is position 51");
    assert_eq!(tail[49].position.0, 100, "last event is position 100");

    // They are still strictly monotonic.
    for window in tail.windows(2) {
        assert!(window[1].position.0 > window[0].position.0);
    }
}

// ── 10. Derived events don't break the ordering guarantee ─────────────────────

#[tokio::test]
async fn derived_events_do_not_break_monotonicity() {
    // ProviderCallCompleted triggers derived RunCostUpdated and SessionCostUpdated
    // events in the projection. These get their own positions in the log.
    // The global ordering guarantee must hold even with derived events.
    use cairn_domain::{
        ProviderBindingId, ProviderCallCompleted, ProviderCallId, ProviderConnectionId,
        ProviderModelId, RouteAttemptId, RouteDecisionId, SessionCreated, SessionId, RunCreated, RunId,
    };
    use cairn_domain::providers::{OperationKind, ProviderCallStatus, RouteDecisionStatus};

    let store = InMemoryStore::new();

    // Append a session + run to enable cost derivation.
    store.append(&[
        session_evt(0),
        run_evt(0),
    ]).await.unwrap();

    // Append a ProviderCallCompleted that will trigger derived cost events.
    store.append(&[EventEnvelope::for_runtime_event(
        EventId::new("pc_1"),
        EventSource::Runtime,
        RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
            project:                project(),
            provider_call_id:       ProviderCallId::new("pc_1"),
            route_decision_id:      RouteDecisionId::new("rd_1"),
            route_attempt_id:       RouteAttemptId::new("ra_1"),
            provider_binding_id:    ProviderBindingId::new("pb_1"),
            provider_connection_id: ProviderConnectionId::new("conn_1"),
            provider_model_id:      ProviderModelId::new("gpt-4o"),
            operation_kind:         OperationKind::Generate,
            status:                 ProviderCallStatus::Succeeded,
            latency_ms:             Some(100),
            input_tokens:           Some(200),
            output_tokens:          Some(100),
            cost_micros:            Some(5_000),
            completed_at:           1_000_000,
            session_id:             None,
            run_id:                 Some(RunId::new("run_0000")),
            error_class:            None,
            raw_error_message:      None,
            retry_count:            0,
        }),
    )]).await.unwrap();

    // Read the full log — positions must be monotonic even if derived events
    // were inserted by the projection.
    let events = store.read_stream(None, 200).await.unwrap();
    assert!(!events.is_empty());

    for window in events.windows(2) {
        assert!(
            window[1].position.0 > window[0].position.0,
            "derived events must not break position monotonicity: {} vs {}",
            window[0].position.0, window[1].position.0
        );
    }

    // head_position must equal the last event's position.
    let head = store.head_position().await.unwrap().unwrap();
    assert_eq!(head.0, events.last().unwrap().position.0,
        "head_position must equal the last event's position");
}
