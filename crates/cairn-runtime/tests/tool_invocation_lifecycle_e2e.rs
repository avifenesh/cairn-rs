//! RFC 002 — tool invocation lifecycle end-to-end integration tests.
//!
//! Tests the complete tool invocation arc:
//!   1. Create a tool invocation with tool_name and target
//!   2. Verify it starts in Started state with correct fields
//!   3. Update progress percentage (via direct event append)
//!   4. Complete the invocation with outcome Success
//!   5. Verify terminal Completed state
//!   6. Cancel a second invocation and verify Canceled state

use std::sync::Arc;

use cairn_domain::policy::ExecutionClass;
use cairn_domain::tool_invocation::{
    ToolInvocationOutcomeKind, ToolInvocationState, ToolInvocationTarget,
};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunId, RuntimeEvent, SessionId,
    TaskId, ToolInvocationId, ToolInvocationProgressUpdated,
};
use cairn_runtime::services::{ToolInvocationService, ToolInvocationServiceImpl};
use cairn_store::projections::ToolInvocationReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_tool", "w_tool", "p_tool")
}

fn invid(id: &str) -> ToolInvocationId {
    ToolInvocationId::new(id)
}

// ── Tests 1–5: create, verify Started, progress, complete, verify Completed ──

/// RFC 002: tool invocations must be recorded durably with a stable lifecycle.
#[tokio::test]
async fn tool_invocation_start_progress_complete() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    let inv_id = invid("inv_complete_1");
    let run_id = RunId::new("run_tool_1");
    let session_id = SessionId::new("sess_tool_1");
    let task_id = TaskId::new("task_tool_1");

    // ── (1) Create a tool invocation with tool_name and target ────────────
    svc.record_start(
        &project(),
        inv_id.clone(),
        Some(session_id.clone()),
        Some(run_id.clone()),
        Some(task_id.clone()),
        ToolInvocationTarget::Builtin {
            tool_name: "fs.read".to_owned(),
        },
        ExecutionClass::SupervisedProcess,
    )
    .await
    .unwrap();

    // ── (2) Verify Started state with correct fields ───────────────────────
    let record = ToolInvocationReadModel::get(store.as_ref(), &inv_id)
        .await
        .unwrap()
        .expect("record must exist after record_start");

    assert_eq!(record.invocation_id, inv_id);
    assert_eq!(
        record.state,
        ToolInvocationState::Started,
        "invocation must be in Started state after record_start"
    );
    assert_eq!(record.project, project());
    assert_eq!(record.session_id.as_ref().unwrap(), &session_id);
    assert_eq!(record.run_id.as_ref().unwrap(), &run_id);
    assert_eq!(record.task_id.as_ref().unwrap(), &task_id);
    assert!(
        record.started_at_ms.is_some(),
        "started_at_ms must be set in Started state"
    );
    assert!(
        record.finished_at_ms.is_none(),
        "finished_at_ms must be None until completion"
    );
    assert!(record.outcome.is_none(), "outcome must be None until terminal");

    // Verify target contains the tool_name.
    match &record.target {
        ToolInvocationTarget::Builtin { tool_name } => {
            assert_eq!(tool_name, "fs.read", "tool_name must round-trip");
        }
        other => panic!("expected Builtin target; got: {other:?}"),
    }

    // ── (3) Update progress percentage ────────────────────────────────────
    // ToolInvocationService has no record_progress method; progress is
    // communicated via direct event append (same pattern used in production).
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_tool_progress_50"),
            EventSource::Runtime,
            RuntimeEvent::ToolInvocationProgressUpdated(ToolInvocationProgressUpdated {
                invocation_id: inv_id.clone(),
                progress_pct: 50,
                message: Some("Halfway through file read".to_owned()),
                updated_at_ms: 1_700_000_001_000,
            }),
        )])
        .await
        .unwrap();

    // State must remain Started — progress does not change the lifecycle state.
    let mid = ToolInvocationReadModel::get(store.as_ref(), &inv_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        mid.state,
        ToolInvocationState::Started,
        "progress update must not change state from Started"
    );

    // Second progress update — 90%.
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_tool_progress_90"),
            EventSource::Runtime,
            RuntimeEvent::ToolInvocationProgressUpdated(ToolInvocationProgressUpdated {
                invocation_id: inv_id.clone(),
                progress_pct: 90,
                message: Some("Almost done".to_owned()),
                updated_at_ms: 1_700_000_002_000,
            }),
        )])
        .await
        .unwrap();

    // ── (4) Complete with outcome Success ─────────────────────────────────
    svc.record_completed(
        &project(),
        inv_id.clone(),
        Some(task_id.clone()),
        "fs.read".to_owned(),
    )
    .await
    .unwrap();

    // ── (5) Verify terminal Completed state ───────────────────────────────
    let completed = ToolInvocationReadModel::get(store.as_ref(), &inv_id)
        .await
        .unwrap()
        .expect("record must still exist after completion");

    assert_eq!(
        completed.state,
        ToolInvocationState::Completed,
        "RFC 002: state must be Completed after record_completed"
    );
    assert_eq!(
        completed.outcome,
        Some(ToolInvocationOutcomeKind::Success),
        "RFC 002: outcome must be Success"
    );
    assert!(
        completed.finished_at_ms.is_some(),
        "finished_at_ms must be set after completion"
    );
    assert!(
        completed.error_message.is_none(),
        "success completion must not carry an error_message"
    );
    assert!(
        completed.state.is_terminal(),
        "Completed must be a terminal state"
    );
}

// ── Test 6: cancel a second invocation ────────────────────────────────────────

/// RFC 002: canceling an invocation via record_failed(Canceled) must
/// transition it to Canceled state with the cancellation reason recorded.
#[tokio::test]
async fn tool_invocation_cancel_records_canceled_state() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    let inv_id = invid("inv_canceled_1");

    // Start the invocation.
    svc.record_start(
        &project(),
        inv_id.clone(),
        None,
        Some(RunId::new("run_cancel_1")),
        None,
        ToolInvocationTarget::Plugin {
            plugin_id: "com.example.code_exec".to_owned(),
            tool_name: "code_exec.run".to_owned(),
        },
        ExecutionClass::SandboxedProcess,
    )
    .await
    .unwrap();

    // Confirm Started.
    let before = ToolInvocationReadModel::get(store.as_ref(), &inv_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.state, ToolInvocationState::Started);

    // ── (6) Cancel the invocation ─────────────────────────────────────────
    svc.record_failed(
        &project(),
        inv_id.clone(),
        None,
        "code_exec.run".to_owned(),
        ToolInvocationOutcomeKind::Canceled,
        Some("operator canceled before completion".to_owned()),
    )
    .await
    .unwrap();

    // Verify Canceled state.
    let canceled = ToolInvocationReadModel::get(store.as_ref(), &inv_id)
        .await
        .unwrap()
        .expect("record must exist after cancellation");

    assert_eq!(
        canceled.state,
        ToolInvocationState::Canceled,
        "RFC 002: state must be Canceled after record_failed(Canceled)"
    );
    assert_eq!(
        canceled.outcome,
        Some(ToolInvocationOutcomeKind::Canceled),
        "outcome must be Canceled"
    );
    assert!(
        canceled.finished_at_ms.is_some(),
        "finished_at_ms must be set after cancellation"
    );
    assert_eq!(
        canceled.error_message.as_deref(),
        Some("operator canceled before completion"),
        "cancellation reason must be preserved"
    );
    assert!(
        canceled.state.is_terminal(),
        "Canceled must be a terminal state"
    );

    // Plugin target fields must be preserved.
    match &canceled.target {
        ToolInvocationTarget::Plugin { plugin_id, tool_name } => {
            assert_eq!(plugin_id, "com.example.code_exec");
            assert_eq!(tool_name, "code_exec.run");
        }
        other => panic!("expected Plugin target; got: {other:?}"),
    }
}

// ── Failed invocation records error_message ──────────────────────────────────

/// RFC 002: a failed invocation must record the failure class and error_message
/// for operator visibility.
#[tokio::test]
async fn tool_invocation_permanent_failure_records_error() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    let inv_id = invid("inv_failed_1");

    svc.record_start(
        &project(),
        inv_id.clone(),
        None,
        Some(RunId::new("run_fail_1")),
        None,
        ToolInvocationTarget::Builtin { tool_name: "http.get".to_owned() },
        ExecutionClass::SupervisedProcess,
    )
    .await
    .unwrap();

    svc.record_failed(
        &project(),
        inv_id.clone(),
        None,
        "http.get".to_owned(),
        ToolInvocationOutcomeKind::PermanentFailure,
        Some("connection refused: target host unreachable".to_owned()),
    )
    .await
    .unwrap();

    let failed = ToolInvocationReadModel::get(store.as_ref(), &inv_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(failed.state, ToolInvocationState::Failed);
    assert_eq!(failed.outcome, Some(ToolInvocationOutcomeKind::PermanentFailure));
    assert_eq!(
        failed.error_message.as_deref(),
        Some("connection refused: target host unreachable")
    );
    assert!(failed.state.is_terminal());
}

// ── list_by_run returns all invocations for a run ─────────────────────────────

/// RFC 002: list_by_run must return all invocations scoped to the queried run.
#[tokio::test]
async fn list_by_run_returns_all_invocations() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    let run_id = RunId::new("run_list_1");
    let other_run = RunId::new("run_other");

    // Three invocations for the main run.
    for i in 0u32..3 {
        svc.record_start(
            &project(),
            invid(&format!("inv_list_{i}")),
            None,
            Some(run_id.clone()),
            None,
            ToolInvocationTarget::Builtin { tool_name: format!("tool_{i}") },
            ExecutionClass::SupervisedProcess,
        )
        .await
        .unwrap();
    }

    // One for another run.
    svc.record_start(
        &project(),
        invid("inv_other"),
        None,
        Some(other_run.clone()),
        None,
        ToolInvocationTarget::Builtin { tool_name: "other.tool".to_owned() },
        ExecutionClass::SupervisedProcess,
    )
    .await
    .unwrap();

    let run_invocations = ToolInvocationReadModel::list_by_run(store.as_ref(), &run_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(
        run_invocations.len(),
        3,
        "list_by_run must return all 3 invocations for the queried run"
    );

    let other_invocations = ToolInvocationReadModel::list_by_run(store.as_ref(), &other_run, 10, 0)
        .await
        .unwrap();
    assert_eq!(other_invocations.len(), 1, "other run must see only its 1 invocation");
}

// ── Multiple terminal outcomes all set finished_at_ms ─────────────────────────

#[tokio::test]
async fn all_terminal_outcomes_set_finished_at_ms() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ToolInvocationServiceImpl::new(store.clone());

    let run_id = RunId::new("run_terminal");
    let cases = [
        ("inv_term_success",  ToolInvocationOutcomeKind::Success,          None),
        ("inv_term_canceled", ToolInvocationOutcomeKind::Canceled,         Some("canceled")),
        ("inv_term_timeout",  ToolInvocationOutcomeKind::Timeout,          Some("timed out")),
        ("inv_term_perm",     ToolInvocationOutcomeKind::PermanentFailure, Some("perm fail")),
    ];

    for (id, outcome, err_msg) in &cases {
        let inv = invid(id);
        svc.record_start(
            &project(), inv.clone(), None, Some(run_id.clone()), None,
            ToolInvocationTarget::Builtin { tool_name: "t".to_owned() },
            ExecutionClass::SupervisedProcess,
        )
        .await
        .unwrap();

        if *outcome == ToolInvocationOutcomeKind::Success {
            svc.record_completed(&project(), inv.clone(), None, "t".to_owned()).await.unwrap();
        } else {
            svc.record_failed(
                &project(), inv.clone(), None, "t".to_owned(),
                *outcome,
                err_msg.map(|s| s.to_owned()),
            )
            .await
            .unwrap();
        }

        let rec = ToolInvocationReadModel::get(store.as_ref(), &inv).await.unwrap().unwrap();
        assert!(
            rec.state.is_terminal(),
            "outcome {outcome:?} must produce a terminal state; got: {:?}", rec.state
        );
        assert!(
            rec.finished_at_ms.is_some(),
            "finished_at_ms must be set for terminal outcome {outcome:?}"
        );
        assert_eq!(rec.outcome, Some(*outcome));
    }
}
