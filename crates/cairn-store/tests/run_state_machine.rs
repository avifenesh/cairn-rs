//! Run state machine integration tests (RFC 002).
//!
//! Validates the run lifecycle using `InMemoryStore` + `EventLog::append`.
//! Runs are the unit of execution within a session — they carry tool calls,
//! approval gates, and can spawn child runs (subagent spawning via parent_run_id).
//!
//! RunState: Pending | Running | WaitingApproval | WaitingDependency |
//!           Paused | Completed | Failed | Canceled
//!   is_terminal() = Completed | Failed | Canceled
//!
//! Projection contract:
//!   RunCreated       → state = Pending, version = 1, failure_class = None
//!   RunStateChanged  → state updated, version bumped, failure_class/pause_reason set

use cairn_domain::{
    EventEnvelope, EventId, EventSource, FailureClass, ProjectId, ProjectKey, RunCreated, RunId,
    RunState, RunStateChanged, RuntimeEvent, SessionCreated, SessionId, StateTransition, TenantId,
    WorkspaceId,
};
use cairn_store::{projections::RunReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_run"),
        workspace_id: WorkspaceId::new("w_run"),
        project_id: ProjectId::new("p_run"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

/// Create session + run events in one shot.
fn setup(
    sess_evt: &str,
    sess_id: &str,
    run_evt: &str,
    run_id: &str,
) -> Vec<EventEnvelope<RuntimeEvent>> {
    vec![
        evt(
            sess_evt,
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new(sess_id),
            }),
        ),
        evt(
            run_evt,
            RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new(sess_id),
                run_id: RunId::new(run_id),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
        ),
    ]
}

/// Emit a RunStateChanged event.
fn run_transition(
    evt_id: &str,
    run_id: &str,
    from: Option<RunState>,
    to: RunState,
    failure_class: Option<FailureClass>,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::RunStateChanged(RunStateChanged {
            project: project(),
            run_id: RunId::new(run_id),
            transition: StateTransition { from, to },
            failure_class,
            pause_reason: None,
            resume_trigger: None,
        }),
    )
}

// ── 1. RunCreated → state = Pending ──────────────────────────────────────────

#[tokio::test]
async fn run_created_has_pending_state() {
    let store = InMemoryStore::new();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    store
        .append(&setup("e_s", "sess_p", "e_r", "run_pending"))
        .await
        .unwrap();

    let record = RunReadModel::get(&store, &RunId::new("run_pending"))
        .await
        .unwrap()
        .expect("RunRecord must exist after RunCreated");

    assert_eq!(record.state, RunState::Pending);
    assert_eq!(record.run_id.as_str(), "run_pending");
    assert_eq!(record.session_id.as_str(), "sess_p");
    assert_eq!(record.project, project());
    assert_eq!(record.version, 1);
    assert!(record.failure_class.is_none());
    assert!(record.parent_run_id.is_none());
    assert!(record.created_at >= ts);
    assert_eq!(record.created_at, record.updated_at);
}

// ── 2. Pending → Running ──────────────────────────────────────────────────────

#[tokio::test]
async fn run_transitions_pending_to_running() {
    let store = InMemoryStore::new();

    store
        .append(&setup("e_s", "sess_r", "e_r", "run_run"))
        .await
        .unwrap();
    store
        .append(&[run_transition(
            "e_t",
            "run_run",
            Some(RunState::Pending),
            RunState::Running,
            None,
        )])
        .await
        .unwrap();

    let record = RunReadModel::get(&store, &RunId::new("run_run"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, RunState::Running);
    assert_eq!(record.version, 2, "version bumped by transition");
    assert!(!record.state.is_terminal(), "Running is non-terminal");
    assert!(record.updated_at >= record.created_at);
}

// ── 3. Running → WaitingApproval ──────────────────────────────────────────────

#[tokio::test]
async fn run_transitions_running_to_waiting_approval() {
    let store = InMemoryStore::new();

    store
        .append(&setup("e_s", "sess_wa", "e_r", "run_wa"))
        .await
        .unwrap();
    store
        .append(&[
            run_transition(
                "e_t1",
                "run_wa",
                Some(RunState::Pending),
                RunState::Running,
                None,
            ),
            run_transition(
                "e_t2",
                "run_wa",
                Some(RunState::Running),
                RunState::WaitingApproval,
                None,
            ),
        ])
        .await
        .unwrap();

    let record = RunReadModel::get(&store, &RunId::new("run_wa"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, RunState::WaitingApproval);
    assert!(
        !record.state.is_terminal(),
        "WaitingApproval is non-terminal"
    );
    assert_eq!(record.version, 3);
}

// ── 4. WaitingApproval → Running → Completed (full happy path) ───────────────

#[tokio::test]
async fn run_completes_after_approval() {
    let store = InMemoryStore::new();

    store
        .append(&setup("e_s", "sess_done", "e_r", "run_done"))
        .await
        .unwrap();
    store
        .append(&[
            run_transition(
                "e1",
                "run_done",
                Some(RunState::Pending),
                RunState::Running,
                None,
            ),
            run_transition(
                "e2",
                "run_done",
                Some(RunState::Running),
                RunState::WaitingApproval,
                None,
            ),
            run_transition(
                "e3",
                "run_done",
                Some(RunState::WaitingApproval),
                RunState::Running,
                None,
            ),
            run_transition(
                "e4",
                "run_done",
                Some(RunState::Running),
                RunState::Completed,
                None,
            ),
        ])
        .await
        .unwrap();

    let record = RunReadModel::get(&store, &RunId::new("run_done"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, RunState::Completed);
    assert!(record.state.is_terminal(), "Completed is terminal");
    assert_eq!(record.version, 5, "4 transitions + initial = version 5");
    assert!(
        record.failure_class.is_none(),
        "successful run has no failure class"
    );
}

// ── 5. is_terminal() contracts ────────────────────────────────────────────────

#[test]
fn terminal_states_are_completed_failed_canceled() {
    assert!(RunState::Completed.is_terminal());
    assert!(RunState::Failed.is_terminal());
    assert!(RunState::Canceled.is_terminal());
    assert!(!RunState::Pending.is_terminal());
    assert!(!RunState::Running.is_terminal());
    assert!(!RunState::WaitingApproval.is_terminal());
    assert!(!RunState::WaitingDependency.is_terminal());
    assert!(!RunState::Paused.is_terminal());
}

// ── 6. Failure path: Running → Failed with failure_class ─────────────────────

#[tokio::test]
async fn run_fails_with_execution_error_class() {
    let store = InMemoryStore::new();

    store
        .append(&setup("e_s", "sess_fail", "e_r", "run_fail"))
        .await
        .unwrap();
    store
        .append(&[
            run_transition(
                "e1",
                "run_fail",
                Some(RunState::Pending),
                RunState::Running,
                None,
            ),
            evt(
                "e2",
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project(),
                    run_id: RunId::new("run_fail"),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::Failed,
                    },
                    failure_class: Some(FailureClass::ExecutionError),
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let record = RunReadModel::get(&store, &RunId::new("run_fail"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, RunState::Failed);
    assert!(record.state.is_terminal());
    assert_eq!(record.failure_class, Some(FailureClass::ExecutionError));
    assert_eq!(record.version, 3);
}

// ── 7. Approval-rejected failure class ───────────────────────────────────────

#[tokio::test]
async fn run_fails_with_approval_rejected_class() {
    let store = InMemoryStore::new();

    store
        .append(&setup("e_s", "sess_rej", "e_r", "run_rej"))
        .await
        .unwrap();
    store
        .append(&[
            run_transition(
                "e1",
                "run_rej",
                Some(RunState::Pending),
                RunState::Running,
                None,
            ),
            run_transition(
                "e2",
                "run_rej",
                Some(RunState::Running),
                RunState::WaitingApproval,
                None,
            ),
            evt(
                "e3",
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project(),
                    run_id: RunId::new("run_rej"),
                    transition: StateTransition {
                        from: Some(RunState::WaitingApproval),
                        to: RunState::Failed,
                    },
                    failure_class: Some(FailureClass::ApprovalRejected),
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let record = RunReadModel::get(&store, &RunId::new("run_rej"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, RunState::Failed);
    assert_eq!(record.failure_class, Some(FailureClass::ApprovalRejected));
}

// ── 8. Canceled path ─────────────────────────────────────────────────────────

#[tokio::test]
async fn run_can_be_canceled_by_operator() {
    let store = InMemoryStore::new();

    store
        .append(&setup("e_s", "sess_cancel", "e_r", "run_cancel"))
        .await
        .unwrap();
    store
        .append(&[
            run_transition(
                "e1",
                "run_cancel",
                Some(RunState::Pending),
                RunState::Running,
                None,
            ),
            evt(
                "e2",
                RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project(),
                    run_id: RunId::new("run_cancel"),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::Canceled,
                    },
                    failure_class: Some(FailureClass::CanceledByOperator),
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let record = RunReadModel::get(&store, &RunId::new("run_cancel"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.state, RunState::Canceled);
    assert!(record.state.is_terminal());
    assert_eq!(record.failure_class, Some(FailureClass::CanceledByOperator));
}

// ── 9. parent_run_id for subagent spawning ────────────────────────────────────

#[tokio::test]
async fn child_run_carries_parent_run_id() {
    let store = InMemoryStore::new();

    store
        .append(&[
            evt(
                "e_s",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_tree"),
                }),
            ),
            // Root run (orchestrator).
            evt(
                "e_root",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_tree"),
                    run_id: RunId::new("run_root"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: Some("orchestrator".to_owned()),
                }),
            ),
            // Child run (subagent) spawned by root.
            evt(
                "e_child",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_tree"),
                    run_id: RunId::new("run_child"),
                    parent_run_id: Some(RunId::new("run_root")),
                    prompt_release_id: None,
                    agent_role_id: Some("researcher".to_owned()),
                }),
            ),
        ])
        .await
        .unwrap();

    let root = RunReadModel::get(&store, &RunId::new("run_root"))
        .await
        .unwrap()
        .unwrap();
    assert!(root.parent_run_id.is_none(), "root run has no parent");
    assert_eq!(root.agent_role_id.as_deref(), Some("orchestrator"));

    let child = RunReadModel::get(&store, &RunId::new("run_child"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(child.parent_run_id, Some(RunId::new("run_root")));
    assert_eq!(
        child.session_id.as_str(),
        "sess_tree",
        "child shares session with root"
    );
    assert_eq!(child.agent_role_id.as_deref(), Some("researcher"));
}

// ── 10. list_by_session returns all runs for a session ────────────────────────

#[tokio::test]
async fn list_by_session_returns_all_session_runs() {
    let store = InMemoryStore::new();

    store
        .append(&[
            evt(
                "e_s",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_list"),
                }),
            ),
            evt(
                "e_r1",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_list"),
                    run_id: RunId::new("run_list_1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            evt(
                "e_r2",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_list"),
                    run_id: RunId::new("run_list_2"),
                    parent_run_id: Some(RunId::new("run_list_1")),
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            evt(
                "e_r3",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_list"),
                    run_id: RunId::new("run_list_3"),
                    parent_run_id: Some(RunId::new("run_list_1")),
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let runs = RunReadModel::list_by_session(&store, &SessionId::new("sess_list"), 10, 0)
        .await
        .unwrap();

    assert_eq!(runs.len(), 3, "all three runs returned");
    assert!(runs.iter().all(|r| r.session_id.as_str() == "sess_list"));

    // list_by_session returns root run first (sorted by created_at).
    assert_eq!(runs[0].run_id.as_str(), "run_list_1");
    assert!(runs[1..]
        .iter()
        .all(|r| r.parent_run_id == Some(RunId::new("run_list_1"))));
}

// ── 11. list_by_session is scoped — cross-session isolation ───────────────────

#[tokio::test]
async fn list_by_session_excludes_other_sessions() {
    let store = InMemoryStore::new();

    store
        .append(&[
            evt(
                "e_s1",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_iso_a"),
                }),
            ),
            evt(
                "e_s2",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_iso_b"),
                }),
            ),
            evt(
                "e_ra",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_iso_a"),
                    run_id: RunId::new("run_iso_a"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            evt(
                "e_rb",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_iso_b"),
                    run_id: RunId::new("run_iso_b"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let for_a = RunReadModel::list_by_session(&store, &SessionId::new("sess_iso_a"), 10, 0)
        .await
        .unwrap();
    assert_eq!(for_a.len(), 1);
    assert_eq!(for_a[0].run_id.as_str(), "run_iso_a");

    let for_b = RunReadModel::list_by_session(&store, &SessionId::new("sess_iso_b"), 10, 0)
        .await
        .unwrap();
    assert_eq!(for_b.len(), 1);
    assert_eq!(for_b[0].run_id.as_str(), "run_iso_b");
}

// ── 12. any_non_terminal reflects live run state ──────────────────────────────

#[tokio::test]
async fn any_non_terminal_reflects_active_runs() {
    let store = InMemoryStore::new();

    store
        .append(&setup("e_s", "sess_nt", "e_r", "run_nt"))
        .await
        .unwrap();

    // One active run → any_non_terminal = true.
    assert!(
        RunReadModel::any_non_terminal(&store, &SessionId::new("sess_nt"))
            .await
            .unwrap(),
        "Pending run is non-terminal"
    );

    store
        .append(&[
            run_transition(
                "e1",
                "run_nt",
                Some(RunState::Pending),
                RunState::Running,
                None,
            ),
            run_transition(
                "e2",
                "run_nt",
                Some(RunState::Running),
                RunState::Completed,
                None,
            ),
        ])
        .await
        .unwrap();

    // Run completed → any_non_terminal = false.
    assert!(
        !RunReadModel::any_non_terminal(&store, &SessionId::new("sess_nt"))
            .await
            .unwrap(),
        "Completed run — session should report no non-terminal runs"
    );
}

// ── 13. latest_root_run returns most-recently-created root run ────────────────

#[tokio::test]
async fn latest_root_run_returns_most_recent_root() {
    let store = InMemoryStore::new();

    store
        .append(&[
            evt(
                "e_s",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_root"),
                }),
            ),
            // First root run.
            evt(
                "e_r1",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_root"),
                    run_id: RunId::new("root_1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Second root run in a separate append (gets a later created_at).
    store
        .append(&[evt(
            "e_r2",
            RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_root"),
                run_id: RunId::new("root_2"),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
        )])
        .await
        .unwrap();

    // Child of root_1 — must not be returned as a root run.
    store
        .append(&[evt(
            "e_r3",
            RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new("sess_root"),
                run_id: RunId::new("child_of_root_1"),
                parent_run_id: Some(RunId::new("root_1")),
                prompt_release_id: None,
                agent_role_id: None,
            }),
        )])
        .await
        .unwrap();

    let latest = RunReadModel::latest_root_run(&store, &SessionId::new("sess_root"))
        .await
        .unwrap()
        .expect("at least one root run must exist");

    assert_eq!(
        latest.run_id.as_str(),
        "root_2",
        "most recently created root run is root_2"
    );
    assert!(latest.parent_run_id.is_none());
}

// ── 14. list_active_by_project excludes terminal runs ────────────────────────

#[tokio::test]
async fn list_active_by_project_excludes_terminal_runs() {
    let store = InMemoryStore::new();

    store
        .append(&[
            evt(
                "e_s",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new("sess_active"),
                }),
            ),
            evt(
                "e_r1",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_active"),
                    run_id: RunId::new("run_active"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            evt(
                "e_r2",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new("sess_active"),
                    run_id: RunId::new("run_terminal"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Complete run_terminal.
    store
        .append(&[
            run_transition(
                "e1",
                "run_terminal",
                Some(RunState::Pending),
                RunState::Running,
                None,
            ),
            run_transition(
                "e2",
                "run_terminal",
                Some(RunState::Running),
                RunState::Completed,
                None,
            ),
        ])
        .await
        .unwrap();

    let active = RunReadModel::list_active_by_project(&store, &project(), 10)
        .await
        .unwrap();

    assert_eq!(active.len(), 1, "only one non-terminal run");
    assert_eq!(active[0].run_id.as_str(), "run_active");
    assert!(!active.iter().any(|r| r.run_id.as_str() == "run_terminal"));
}
