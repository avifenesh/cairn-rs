//! Tool invocation lifecycle integration tests.
//!
//! Validates the full tool execution audit trail using `InMemoryStore` +
//! `EventLog::append`. Every tool call the agent makes is durably recorded
//! with enough state to reconstruct the timeline, audit decisions, and
//! surface failures to operators.
//!
//! Pipeline under test:
//!   ToolInvocationStarted    → state = Started  (Requested→Started in one projection step)
//!     → ToolInvocationCompleted → state = Completed, outcome = Success
//!   ToolInvocationStarted (2nd) → state = Started
//!     → ToolInvocationFailed    → state = Failed, outcome + error_message set
//!
//! Also validates:
//!   - Plugin-backed target round-trips through the projection
//!   - list_by_run scopes correctly to the run
//!   - Different failure outcomes (RetryableFailure, PermanentFailure, Timeout)

use cairn_domain::tool_invocation::{
    ToolInvocationOutcomeKind, ToolInvocationState, ToolInvocationTarget,
};
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ExecutionClass, ProjectId, ProjectKey, RunCreated, RunId,
    RuntimeEvent, SessionCreated, SessionId, TenantId, ToolInvocationCompleted,
    ToolInvocationFailed, ToolInvocationId, ToolInvocationStarted, WorkspaceId,
};
use cairn_store::{projections::ToolInvocationReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_tool"),
        workspace_id: WorkspaceId::new("w_tool"),
        project_id: ProjectId::new("p_tool"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Append a session + run, returning the run_id. Avoids boilerplate in every test.
async fn setup_run(store: &InMemoryStore, suffix: &str) -> RunId {
    let run_id = RunId::new(format!("run_{suffix}"));
    store
        .append(&[
            evt(
                &format!("e_sess_{suffix}"),
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new(format!("sess_{suffix}")),
                }),
            ),
            evt(
                &format!("e_run_{suffix}"),
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new(format!("sess_{suffix}")),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();
    run_id
}

// ── 1. ToolInvocationStarted → state = Started ───────────────────────────────

#[tokio::test]
async fn tool_invocation_started_shows_started_state() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = setup_run(&store, "1").await;
    let inv_id = ToolInvocationId::new("inv_1");

    store
        .append(&[evt(
            "e_inv_1",
            RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project(),
                invocation_id: inv_id.clone(),
                session_id: Some(SessionId::new("sess_1")),
                run_id: Some(run_id.clone()),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "read_file".to_owned(),
                },
                execution_class: ExecutionClass::SandboxedProcess,
                prompt_release_id: None,
                requested_at_ms: ts,
                started_at_ms: ts + 1,
                args_json: None
            }),
        )])
        .await
        .unwrap();

    let record = ToolInvocationReadModel::get(&store, &inv_id)
        .await
        .unwrap()
        .expect("record must exist after ToolInvocationStarted");

    // The projection applies Requested→Started atomically on ToolInvocationStarted.
    assert_eq!(record.state, ToolInvocationState::Started);
    assert_eq!(record.invocation_id, inv_id);
    assert_eq!(record.run_id, Some(run_id));
    assert_eq!(record.execution_class, ExecutionClass::SandboxedProcess);
    assert!(
        matches!(&record.target, ToolInvocationTarget::Builtin { tool_name } if tool_name == "read_file")
    );
    assert_eq!(record.version, 2, "Requested(v1)→Started(v2)");
    assert!(record.outcome.is_none(), "no outcome yet");
    assert!(record.finished_at_ms.is_none());
    assert_eq!(record.started_at_ms, Some(ts + 1));
}

// ── 2. ToolInvocationCompleted → state = Completed, outcome = Success ─────────

#[tokio::test]
async fn tool_invocation_completed_transitions_to_success() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = setup_run(&store, "2").await;
    let inv_id = ToolInvocationId::new("inv_2");

    store
        .append(&[evt(
            "e_inv_2",
            RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project(),
                invocation_id: inv_id.clone(),
                session_id: Some(SessionId::new("sess_2")),
                run_id: Some(run_id.clone()),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "write_file".to_owned(),
                },
                execution_class: ExecutionClass::SupervisedProcess,
                prompt_release_id: None,
                requested_at_ms: ts,
                started_at_ms: ts + 1,
                args_json: None
            }),
        )])
        .await
        .unwrap();

    // Verify started state before completion.
    let started = ToolInvocationReadModel::get(&store, &inv_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(started.state, ToolInvocationState::Started);

    store
        .append(&[evt(
            "e_inv_2_done",
            RuntimeEvent::ToolInvocationCompleted(ToolInvocationCompleted {
                project: project(),
                invocation_id: inv_id.clone(),
                task_id: None,
                tool_name: "write_file".to_owned(),
                finished_at_ms: ts + 50,
                outcome: ToolInvocationOutcomeKind::Success,
                tool_call_id: None,
                result_json: None,
                output_preview: None
            }),
        )])
        .await
        .unwrap();

    let completed = ToolInvocationReadModel::get(&store, &inv_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(completed.state, ToolInvocationState::Completed);
    assert_eq!(completed.outcome, Some(ToolInvocationOutcomeKind::Success));
    assert_eq!(completed.finished_at_ms, Some(ts + 50));
    assert!(
        completed.error_message.is_none(),
        "success has no error message"
    );
    assert_eq!(completed.version, 3, "Requested(1)→Started(2)→Completed(3)");
}

// ── 3. ToolInvocationFailed with retryable error ──────────────────────────────

#[tokio::test]
async fn tool_invocation_failed_with_retryable_error() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = setup_run(&store, "3").await;
    let inv_id = ToolInvocationId::new("inv_3");

    store
        .append(&[
            evt(
                "e_inv_3",
                RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project(),
                    invocation_id: inv_id.clone(),
                    session_id: Some(SessionId::new("sess_3")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "call_api".to_owned(),
                    },
                    execution_class: ExecutionClass::SandboxedProcess,
                    prompt_release_id: None,
                    requested_at_ms: ts,
                    started_at_ms: ts + 1,
                    args_json: None
                }),
            ),
            evt(
                "e_inv_3_fail",
                RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                    project: project(),
                    invocation_id: inv_id.clone(),
                    task_id: None,
                    tool_name: "call_api".to_owned(),
                    finished_at_ms: ts + 100,
                    outcome: ToolInvocationOutcomeKind::RetryableFailure,
                    error_message: Some("upstream returned 503, retry eligible".to_owned()),
                    output_preview: None
                }),
            ),
        ])
        .await
        .unwrap();

    let record = ToolInvocationReadModel::get(&store, &inv_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, ToolInvocationState::Failed);
    assert_eq!(
        record.outcome,
        Some(ToolInvocationOutcomeKind::RetryableFailure)
    );
    assert_eq!(
        record.error_message.as_deref(),
        Some("upstream returned 503, retry eligible")
    );
    assert_eq!(record.finished_at_ms, Some(ts + 100));
    assert_eq!(record.version, 3);
}

// ── 4. ToolInvocationFailed with permanent error ──────────────────────────────

#[tokio::test]
async fn tool_invocation_failed_with_permanent_error() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = setup_run(&store, "4").await;
    let inv_id = ToolInvocationId::new("inv_4");

    store
        .append(&[
            evt(
                "e_inv_4",
                RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project(),
                    invocation_id: inv_id.clone(),
                    session_id: Some(SessionId::new("sess_4")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Plugin {
                        plugin_id: "com.example.db".to_owned(),
                        tool_name: "db.execute".to_owned(),
                    },
                    execution_class: ExecutionClass::SupervisedProcess,
                    prompt_release_id: None,
                    requested_at_ms: ts,
                    started_at_ms: ts + 2,
                    args_json: None
                }),
            ),
            evt(
                "e_inv_4_fail",
                RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                    project: project(),
                    invocation_id: inv_id.clone(),
                    task_id: None,
                    tool_name: "db.execute".to_owned(),
                    finished_at_ms: ts + 15,
                    outcome: ToolInvocationOutcomeKind::PermanentFailure,
                    error_message: Some("permission denied: insufficient privileges".to_owned()),
                    output_preview: None
                }),
            ),
        ])
        .await
        .unwrap();

    let record = ToolInvocationReadModel::get(&store, &inv_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.state, ToolInvocationState::Failed);
    assert_eq!(
        record.outcome,
        Some(ToolInvocationOutcomeKind::PermanentFailure)
    );
    assert!(record
        .error_message
        .as_deref()
        .unwrap()
        .contains("permission denied"));
    // Plugin target is preserved through the projection.
    assert!(matches!(&record.target,
        ToolInvocationTarget::Plugin { plugin_id, tool_name }
        if plugin_id == "com.example.db" && tool_name == "db.execute"
    ));
}

// ── 5. ToolInvocationFailed with timeout ─────────────────────────────────────

#[tokio::test]
async fn tool_invocation_failed_with_timeout() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = setup_run(&store, "5").await;
    let inv_id = ToolInvocationId::new("inv_5");

    store
        .append(&[
            evt(
                "e_inv_5",
                RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project(),
                    invocation_id: inv_id.clone(),
                    session_id: Some(SessionId::new("sess_5")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "fetch_url".to_owned(),
                    },
                    execution_class: ExecutionClass::SandboxedProcess,
                    prompt_release_id: None,
                    requested_at_ms: ts,
                    started_at_ms: ts + 1,
                    args_json: None
                }),
            ),
            evt(
                "e_inv_5_timeout",
                RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                    project: project(),
                    invocation_id: inv_id.clone(),
                    task_id: None,
                    tool_name: "fetch_url".to_owned(),
                    finished_at_ms: ts + 30_000,
                    outcome: ToolInvocationOutcomeKind::Timeout,
                    error_message: Some("request timed out after 30s".to_owned()),
                    output_preview: None
                }),
            ),
        ])
        .await
        .unwrap();

    let record = ToolInvocationReadModel::get(&store, &inv_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.state, ToolInvocationState::Failed);
    assert_eq!(record.outcome, Some(ToolInvocationOutcomeKind::Timeout));
    assert_eq!(record.finished_at_ms, Some(ts + 30_000));
}

// ── 6. list_by_run scopes correctly ──────────────────────────────────────────

#[tokio::test]
async fn list_by_run_returns_all_invocations_for_run() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_a = setup_run(&store, "6a").await;
    let run_b = setup_run(&store, "6b").await;

    let inv_a1 = ToolInvocationId::new("inv_6a1");
    let inv_a2 = ToolInvocationId::new("inv_6a2");
    let inv_b1 = ToolInvocationId::new("inv_6b1");

    // Two invocations in run_a.
    store
        .append(&[
            evt(
                "ea1",
                RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project(),
                    invocation_id: inv_a1.clone(),
                    session_id: Some(SessionId::new("sess_6a")),
                    run_id: Some(run_a.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "read_file".to_owned(),
                    },
                    execution_class: ExecutionClass::SandboxedProcess,
                    prompt_release_id: None,
                    requested_at_ms: ts,
                    started_at_ms: ts + 1,
                    args_json: None
                }),
            ),
            evt(
                "ea2",
                RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project(),
                    invocation_id: inv_a2.clone(),
                    session_id: Some(SessionId::new("sess_6a")),
                    run_id: Some(run_a.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "write_file".to_owned(),
                    },
                    execution_class: ExecutionClass::SupervisedProcess,
                    prompt_release_id: None,
                    requested_at_ms: ts + 10,
                    started_at_ms: ts + 11,
                    args_json: None
                }),
            ),
        ])
        .await
        .unwrap();

    // One invocation in run_b.
    store
        .append(&[evt(
            "eb1",
            RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project(),
                invocation_id: inv_b1.clone(),
                session_id: Some(SessionId::new("sess_6b")),
                run_id: Some(run_b.clone()),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "list_dir".to_owned(),
                },
                execution_class: ExecutionClass::SandboxedProcess,
                prompt_release_id: None,
                requested_at_ms: ts + 20,
                started_at_ms: ts + 21,
                args_json: None
            }),
        )])
        .await
        .unwrap();

    let run_a_invocations = ToolInvocationReadModel::list_by_run(&store, &run_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(run_a_invocations.len(), 2, "run_a has 2 invocations");
    let ids: Vec<_> = run_a_invocations
        .iter()
        .map(|r| r.invocation_id.as_str())
        .collect();
    assert!(ids.contains(&"inv_6a1"));
    assert!(ids.contains(&"inv_6a2"));
    assert!(
        !ids.contains(&"inv_6b1"),
        "run_b invocation must not appear in run_a list"
    );

    let run_b_invocations = ToolInvocationReadModel::list_by_run(&store, &run_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(run_b_invocations.len(), 1);
    assert_eq!(run_b_invocations[0].invocation_id, inv_b1);
}

// ── 7. Plugin-backed tool target round-trips through projection ───────────────

#[tokio::test]
async fn plugin_tool_target_preserved_through_projection() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = setup_run(&store, "7").await;
    let inv_id = ToolInvocationId::new("inv_7");

    store
        .append(&[evt(
            "e_inv_7",
            RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project(),
                invocation_id: inv_id.clone(),
                session_id: Some(SessionId::new("sess_7")),
                run_id: Some(run_id.clone()),
                task_id: None,
                target: ToolInvocationTarget::Plugin {
                    plugin_id: "com.acme.git".to_owned(),
                    tool_name: "git.commit".to_owned(),
                },
                execution_class: ExecutionClass::SupervisedProcess,
                prompt_release_id: None,
                requested_at_ms: ts,
                started_at_ms: ts + 5,
                args_json: None
            }),
        )])
        .await
        .unwrap();

    let record = ToolInvocationReadModel::get(&store, &inv_id)
        .await
        .unwrap()
        .unwrap();
    match &record.target {
        ToolInvocationTarget::Plugin {
            plugin_id,
            tool_name,
        } => {
            assert_eq!(plugin_id, "com.acme.git");
            assert_eq!(tool_name, "git.commit");
        }
        other => panic!("expected Plugin target, got {other:?}"),
    }
    assert_eq!(record.execution_class, ExecutionClass::SupervisedProcess);
}

// ── 8. Consecutive success + failure in same run builds full audit trail ───────

#[tokio::test]
async fn run_audit_trail_captures_mixed_outcomes() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let run_id = setup_run(&store, "8").await;
    let inv_ok = ToolInvocationId::new("inv_8_ok");
    let inv_fail = ToolInvocationId::new("inv_8_fail");

    store
        .append(&[
            // First call: succeeds.
            evt(
                "e8a",
                RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project(),
                    invocation_id: inv_ok.clone(),
                    session_id: Some(SessionId::new("sess_8")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "search".to_owned(),
                    },
                    execution_class: ExecutionClass::SandboxedProcess,
                    prompt_release_id: None,
                    requested_at_ms: ts,
                    started_at_ms: ts + 1,
                    args_json: None
                }),
            ),
            evt(
                "e8a_done",
                RuntimeEvent::ToolInvocationCompleted(ToolInvocationCompleted {
                    project: project(),
                    invocation_id: inv_ok.clone(),
                    task_id: None,
                    tool_name: "search".to_owned(),
                    finished_at_ms: ts + 20,
                    outcome: ToolInvocationOutcomeKind::Success,
                    tool_call_id: None,
                    result_json: None,
                    output_preview: None
                }),
            ),
            // Second call: fails with protocol violation.
            evt(
                "e8b",
                RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project(),
                    invocation_id: inv_fail.clone(),
                    session_id: Some(SessionId::new("sess_8")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Plugin {
                        plugin_id: "com.acme.legacy".to_owned(),
                        tool_name: "legacy.call".to_owned(),
                    },
                    execution_class: ExecutionClass::SandboxedProcess,
                    prompt_release_id: None,
                    requested_at_ms: ts + 25,
                    started_at_ms: ts + 26,
                    args_json: None
                }),
            ),
            evt(
                "e8b_fail",
                RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                    project: project(),
                    invocation_id: inv_fail.clone(),
                    task_id: None,
                    tool_name: "legacy.call".to_owned(),
                    finished_at_ms: ts + 30,
                    outcome: ToolInvocationOutcomeKind::ProtocolViolation,
                    error_message: Some("plugin returned malformed JSON response".to_owned()),
                    output_preview: None
                }),
            ),
        ])
        .await
        .unwrap();

    // Both appear in the run's audit trail.
    let trail = ToolInvocationReadModel::list_by_run(&store, &run_id, 10, 0)
        .await
        .unwrap();
    assert_eq!(trail.len(), 2);

    let ok = ToolInvocationReadModel::get(&store, &inv_ok)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ok.state, ToolInvocationState::Completed);
    assert_eq!(ok.outcome, Some(ToolInvocationOutcomeKind::Success));
    assert!(ok.error_message.is_none());

    let fail = ToolInvocationReadModel::get(&store, &inv_fail)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(fail.state, ToolInvocationState::Failed);
    assert_eq!(
        fail.outcome,
        Some(ToolInvocationOutcomeKind::ProtocolViolation)
    );
    assert!(fail
        .error_message
        .as_deref()
        .unwrap()
        .contains("malformed JSON"));
}
