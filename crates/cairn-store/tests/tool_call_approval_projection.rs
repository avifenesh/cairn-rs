//! PR BP-2: `tool_call_approvals` projection state machine.
//!
//! Covers all four `ToolCall*` events against the in-memory backend
//! (the canonical reference backend for local-mode). Cross-backend
//! parity with SQLite is exercised separately in
//! `cross_backend_parity.rs`.

use std::sync::atomic::{AtomicU64, Ordering};

use cairn_domain::{
    ApprovalMatchPolicy, ApprovalScope, EventEnvelope, EventId, EventSource, OperatorId,
    ProjectKey, RunId, RuntimeEvent, SessionId, ToolCallAmended, ToolCallApproved, ToolCallId,
    ToolCallProposed, ToolCallRejected,
};
use cairn_store::event_log::EventLog;
use cairn_store::in_memory::InMemoryStore;
use cairn_store::projections::{ToolCallApprovalReadModel, ToolCallApprovalState};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn project() -> ProjectKey {
    ProjectKey::new("tenant", "workspace", "project")
}

fn make_envelope(event: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_{n}")),
        EventSource::Runtime,
        event,
    )
}

fn proposed(
    call_id: &str,
    session: &str,
    run: &str,
    proposed_at_ms: u64,
) -> EventEnvelope<RuntimeEvent> {
    make_envelope(RuntimeEvent::ToolCallProposed(ToolCallProposed {
        project: project(),
        call_id: ToolCallId::new(call_id),
        session_id: SessionId::new(session),
        run_id: RunId::new(run),
        tool_name: "read_file".to_owned(),
        tool_args: serde_json::json!({ "path": "/workspaces/cairn/Cargo.toml" }),
        display_summary: format!("Read file for {call_id}"),
        match_policy: ApprovalMatchPolicy::ExactPath {
            path: "/workspaces/cairn/Cargo.toml".to_owned(),
        },
        proposed_at_ms,
    }))
}

fn approved(
    call_id: &str,
    session: &str,
    operator: &str,
    approved_at_ms: u64,
    approved_args: Option<serde_json::Value>,
) -> EventEnvelope<RuntimeEvent> {
    make_envelope(RuntimeEvent::ToolCallApproved(ToolCallApproved {
        project: project(),
        call_id: ToolCallId::new(call_id),
        session_id: SessionId::new(session),
        operator_id: OperatorId::new(operator),
        scope: ApprovalScope::Once,
        approved_tool_args: approved_args,
        approved_at_ms,
    }))
}

fn rejected(
    call_id: &str,
    session: &str,
    operator: &str,
    reason: Option<&str>,
    rejected_at_ms: u64,
) -> EventEnvelope<RuntimeEvent> {
    make_envelope(RuntimeEvent::ToolCallRejected(ToolCallRejected {
        project: project(),
        call_id: ToolCallId::new(call_id),
        session_id: SessionId::new(session),
        operator_id: OperatorId::new(operator),
        reason: reason.map(str::to_owned),
        rejected_at_ms,
    }))
}

fn amended(
    call_id: &str,
    session: &str,
    operator: &str,
    new_args: serde_json::Value,
    amended_at_ms: u64,
) -> EventEnvelope<RuntimeEvent> {
    make_envelope(RuntimeEvent::ToolCallAmended(ToolCallAmended {
        project: project(),
        call_id: ToolCallId::new(call_id),
        session_id: SessionId::new(session),
        operator_id: OperatorId::new(operator),
        new_tool_args: new_args,
        amended_at_ms,
    }))
}

#[tokio::test]
async fn proposed_inserts_pending_record() {
    let store = InMemoryStore::new();
    store
        .append(&[proposed("tc_1", "sess_1", "run_1", 1_000)])
        .await
        .unwrap();

    let rec = store
        .get(&ToolCallId::new("tc_1"))
        .await
        .unwrap()
        .expect("record must be populated on ToolCallProposed");
    assert_eq!(rec.state, ToolCallApprovalState::Pending);
    assert_eq!(rec.session_id, SessionId::new("sess_1"));
    assert_eq!(rec.run_id, RunId::new("run_1"));
    assert_eq!(rec.tool_name, "read_file");
    assert!(rec.amended_tool_args.is_none());
    assert!(rec.approved_tool_args.is_none());
    assert!(rec.operator_id.is_none());
    assert!(rec.scope.is_none());
    assert_eq!(rec.proposed_at_ms, 1_000);
    assert!(rec.approved_at_ms.is_none());
    assert_eq!(rec.version, 1);
    // Non-empty display_summary is preserved.
    assert_eq!(rec.display_summary.as_deref(), Some("Read file for tc_1"));
}

#[tokio::test]
async fn amendment_updates_args_without_changing_state() {
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_2", "sess_1", "run_1", 1_000),
            amended(
                "tc_2",
                "sess_1",
                "op_1",
                serde_json::json!({ "path": "/workspaces/cairn/README.md" }),
                1_100,
            ),
        ])
        .await
        .unwrap();

    let rec = store.get(&ToolCallId::new("tc_2")).await.unwrap().unwrap();
    assert_eq!(rec.state, ToolCallApprovalState::Pending);
    assert_eq!(
        rec.amended_tool_args,
        Some(serde_json::json!({ "path": "/workspaces/cairn/README.md" }))
    );
    assert_eq!(rec.last_amended_at_ms, Some(1_100));
    // Operator not yet set on the record — amendments don't commit a
    // resolver identity.
    assert!(rec.operator_id.is_none());
    assert!(rec.approved_at_ms.is_none());
    // Version bumped once from the amendment.
    assert_eq!(rec.version, 2);
}

#[tokio::test]
async fn approval_transitions_state_and_records_operator_and_override() {
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_3", "sess_1", "run_1", 1_000),
            approved(
                "tc_3",
                "sess_1",
                "op_1",
                1_500,
                Some(serde_json::json!({ "path": "/workspaces/cairn/overridden.md" })),
            ),
        ])
        .await
        .unwrap();

    let rec = store.get(&ToolCallId::new("tc_3")).await.unwrap().unwrap();
    assert_eq!(rec.state, ToolCallApprovalState::Approved);
    assert_eq!(rec.operator_id.as_ref().unwrap().as_str(), "op_1");
    assert_eq!(
        rec.approved_tool_args,
        Some(serde_json::json!({ "path": "/workspaces/cairn/overridden.md" }))
    );
    assert_eq!(rec.approved_at_ms, Some(1_500));
    assert!(matches!(rec.scope, Some(ApprovalScope::Once)));
    assert!(rec.rejected_at_ms.is_none());
    assert_eq!(rec.version, 2);
}

#[tokio::test]
async fn approval_without_override_keeps_amended_args_as_source_of_truth() {
    // Replay invariant: after an amendment, an Approved-with-None keeps
    // the amended args as the "effective" executed args; the projection
    // just records the state transition.
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_4", "sess_1", "run_1", 1_000),
            amended(
                "tc_4",
                "sess_1",
                "op_1",
                serde_json::json!({ "path": "/edited.md" }),
                1_100,
            ),
            approved("tc_4", "sess_1", "op_1", 1_500, None),
        ])
        .await
        .unwrap();

    let rec = store.get(&ToolCallId::new("tc_4")).await.unwrap().unwrap();
    assert_eq!(rec.state, ToolCallApprovalState::Approved);
    assert_eq!(
        rec.amended_tool_args,
        Some(serde_json::json!({ "path": "/edited.md" }))
    );
    assert!(
        rec.approved_tool_args.is_none(),
        "Approved-with-None must NOT populate approved_tool_args"
    );
}

#[tokio::test]
async fn rejection_transitions_state_and_captures_reason() {
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_5", "sess_1", "run_1", 1_000),
            rejected("tc_5", "sess_1", "op_2", Some("looks unsafe"), 1_600),
        ])
        .await
        .unwrap();

    let rec = store.get(&ToolCallId::new("tc_5")).await.unwrap().unwrap();
    assert_eq!(rec.state, ToolCallApprovalState::Rejected);
    assert_eq!(rec.operator_id.as_ref().unwrap().as_str(), "op_2");
    assert_eq!(rec.reason.as_deref(), Some("looks unsafe"));
    assert_eq!(rec.rejected_at_ms, Some(1_600));
    assert!(rec.approved_at_ms.is_none());
}

#[tokio::test]
async fn amend_then_reject_records_both() {
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_6", "sess_1", "run_1", 1_000),
            amended(
                "tc_6",
                "sess_1",
                "op_1",
                serde_json::json!({ "path": "/tmp/x" }),
                1_100,
            ),
            rejected("tc_6", "sess_1", "op_1", None, 1_200),
        ])
        .await
        .unwrap();

    let rec = store.get(&ToolCallId::new("tc_6")).await.unwrap().unwrap();
    assert_eq!(rec.state, ToolCallApprovalState::Rejected);
    assert_eq!(
        rec.amended_tool_args,
        Some(serde_json::json!({ "path": "/tmp/x" }))
    );
    assert_eq!(rec.last_amended_at_ms, Some(1_100));
    assert_eq!(rec.rejected_at_ms, Some(1_200));
    assert!(rec.reason.is_none());
}

#[tokio::test]
async fn list_for_run_returns_oldest_first() {
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_b", "sess_1", "run_1", 2_000),
            proposed("tc_a", "sess_1", "run_1", 1_000),
            proposed("tc_c", "sess_1", "run_1", 3_000),
            // Different run: should NOT show up.
            proposed("tc_other", "sess_1", "run_other", 1_500),
        ])
        .await
        .unwrap();

    let list = store
        .list_for_run(&RunId::new("run_1"))
        .await
        .unwrap();
    let ids: Vec<&str> = list.iter().map(|r| r.call_id.as_str()).collect();
    assert_eq!(ids, vec!["tc_a", "tc_b", "tc_c"]);
}

#[tokio::test]
async fn list_for_session_returns_oldest_first() {
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_y", "sess_1", "run_1", 2_000),
            proposed("tc_x", "sess_1", "run_2", 1_000),
            proposed("tc_other_sess", "sess_2", "run_9", 1_500),
        ])
        .await
        .unwrap();

    let list = store
        .list_for_session(&SessionId::new("sess_1"))
        .await
        .unwrap();
    let ids: Vec<&str> = list.iter().map(|r| r.call_id.as_str()).collect();
    assert_eq!(ids, vec!["tc_x", "tc_y"]);
}

#[tokio::test]
async fn list_pending_for_project_excludes_resolved_and_applies_limit_offset() {
    let store = InMemoryStore::new();
    store
        .append(&[
            proposed("tc_p1", "sess_1", "run_1", 1_000),
            proposed("tc_p2", "sess_1", "run_1", 2_000),
            proposed("tc_p3", "sess_1", "run_1", 3_000),
            proposed("tc_p4", "sess_1", "run_1", 4_000),
            // tc_p2 resolved; should drop out of the pending list.
            approved("tc_p2", "sess_1", "op_1", 2_100, None),
            // tc_p3 rejected; should drop out of the pending list.
            rejected("tc_p3", "sess_1", "op_1", None, 3_100),
        ])
        .await
        .unwrap();

    let all_pending = store
        .list_pending_for_project(&project(), 100, 0)
        .await
        .unwrap();
    let ids: Vec<&str> = all_pending.iter().map(|r| r.call_id.as_str()).collect();
    assert_eq!(ids, vec!["tc_p1", "tc_p4"]);

    let page = store
        .list_pending_for_project(&project(), 1, 1)
        .await
        .unwrap();
    let ids: Vec<&str> = page.iter().map(|r| r.call_id.as_str()).collect();
    assert_eq!(ids, vec!["tc_p4"]);
}

#[tokio::test]
async fn amend_on_unknown_call_id_is_noop() {
    let store = InMemoryStore::new();
    // An amendment for a call that was never proposed must not panic or
    // leak a partial row into the projection.
    store
        .append(&[amended(
            "tc_ghost",
            "sess_1",
            "op_1",
            serde_json::json!({}),
            1_000,
        )])
        .await
        .unwrap();

    let rec = store.get(&ToolCallId::new("tc_ghost")).await.unwrap();
    assert!(rec.is_none());
}

#[tokio::test]
async fn empty_display_summary_stored_as_none() {
    let store = InMemoryStore::new();
    let mut envelope = proposed("tc_empty", "sess_1", "run_1", 1_000);
    if let RuntimeEvent::ToolCallProposed(p) = &mut envelope.payload {
        p.display_summary.clear();
    }
    store.append(&[envelope]).await.unwrap();

    let rec = store
        .get(&ToolCallId::new("tc_empty"))
        .await
        .unwrap()
        .unwrap();
    assert!(
        rec.display_summary.is_none(),
        "empty display_summary must round-trip as None"
    );
}
