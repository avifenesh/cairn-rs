//! Task state machine integration tests (RFC 002).
//!
//! Validates the full task lifecycle using `InMemoryStore` + `EventLog::append`.
//! Tasks are the leased work units inside a run — they carry approval gates,
//! retry counts, and dependency tracking.
//!
//! TaskState machine (RFC 005 transitions):
//!   Queued → Leased → Running → WaitingApproval ↔ Running → Completed
//!                             ↘ Failed | Canceled | RetryableFailed → Queued
//!
//! Note on "WaitingInput": the manager specified this state, but the domain
//! uses WaitingApproval (human approval gate) and WaitingDependency (task
//! depends on another task) — both map to the "waiting for input" concept.
//! Tests use WaitingApproval as the canonical "WaitingInput" gate.
//!
//! list_by_run is implemented via TaskReadModel::list_by_parent_run.

use cairn_domain::lifecycle::can_transition_task_state;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, FailureClass, ProjectId, ProjectKey, RunCreated, RunId,
    RuntimeEvent, SessionCreated, SessionId, StateTransition, TaskCreated, TaskId, TaskState,
    TaskStateChanged, TenantId, WorkspaceId,
};
use cairn_store::{projections::TaskReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_task"),
        workspace_id: WorkspaceId::new("w_task"),
        project_id: ProjectId::new("p_task"),
    }
}

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

fn task_transition(
    evt_id: &str,
    task_id: &str,
    from: Option<TaskState>,
    to: TaskState,
    failure_class: Option<FailureClass>,
) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: project(),
            task_id: TaskId::new(task_id),
            transition: StateTransition { from, to },
            failure_class,
            pause_reason: None,
            resume_trigger: None,
        }),
    )
}

/// Build session + run + one task in a single vec.
fn setup(run_id: &str, task_id: &str) -> Vec<EventEnvelope<RuntimeEvent>> {
    let sess = format!("sess_{run_id}");
    vec![
        evt(
            "e_s",
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(),
                session_id: SessionId::new(&sess),
            }),
        ),
        evt(
            "e_r",
            RuntimeEvent::RunCreated(RunCreated {
                project: project(),
                session_id: SessionId::new(&sess),
                run_id: RunId::new(run_id),
                parent_run_id: None,
                prompt_release_id: None,
                agent_role_id: None,
            }),
        ),
        evt(
            "e_t",
            RuntimeEvent::TaskCreated(TaskCreated {
                project: project(),
                task_id: TaskId::new(task_id),
                parent_run_id: Some(RunId::new(run_id)),
                parent_task_id: None,
                prompt_release_id: None,
            }),
        ),
    ]
}

// ── 1. TaskCreated → state = Queued, version = 1 ────────────────────────────

#[tokio::test]
async fn task_created_has_queued_state() {
    let store = InMemoryStore::new();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    store.append(&setup("run_q", "task_q")).await.unwrap();

    let record = TaskReadModel::get(&store, &TaskId::new("task_q"))
        .await
        .unwrap()
        .expect("TaskRecord must exist after TaskCreated");

    assert_eq!(record.state, TaskState::Queued);
    assert_eq!(record.task_id.as_str(), "task_q");
    assert_eq!(record.project, project());
    assert_eq!(record.parent_run_id, Some(RunId::new("run_q")));
    assert_eq!(record.version, 1, "initial version is 1");
    assert!(record.failure_class.is_none());
    assert_eq!(record.retry_count, 0);
    assert!(record.created_at >= ts);
    assert_eq!(record.created_at, record.updated_at);
}

// ── 2. Queued → Leased → Running ─────────────────────────────────────────────

#[tokio::test]
async fn task_transitions_queued_to_leased_to_running() {
    let store = InMemoryStore::new();
    store.append(&setup("run_qlr", "task_qlr")).await.unwrap();

    store
        .append(&[task_transition(
            "t1",
            "task_qlr",
            Some(TaskState::Queued),
            TaskState::Leased,
            None,
        )])
        .await
        .unwrap();
    let leased = TaskReadModel::get(&store, &TaskId::new("task_qlr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(leased.state, TaskState::Leased);
    assert_eq!(leased.version, 2);
    assert!(!leased.state.is_terminal());

    store
        .append(&[task_transition(
            "t2",
            "task_qlr",
            Some(TaskState::Leased),
            TaskState::Running,
            None,
        )])
        .await
        .unwrap();
    let running = TaskReadModel::get(&store, &TaskId::new("task_qlr"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(running.state, TaskState::Running);
    assert_eq!(running.version, 3);
    assert!(!running.state.is_terminal());
}

// ── 3. Running → WaitingApproval ("WaitingInput") ────────────────────────────

#[tokio::test]
async fn task_transitions_running_to_waiting_approval() {
    let store = InMemoryStore::new();
    store.append(&setup("run_wa", "task_wa")).await.unwrap();
    store
        .append(&[
            task_transition(
                "t1",
                "task_wa",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "t2",
                "task_wa",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            task_transition(
                "t3",
                "task_wa",
                Some(TaskState::Running),
                TaskState::WaitingApproval,
                None,
            ),
        ])
        .await
        .unwrap();

    let record = TaskReadModel::get(&store, &TaskId::new("task_wa"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        record.state,
        TaskState::WaitingApproval,
        "Running→WaitingApproval is the human-input gate"
    );
    assert_eq!(record.version, 4);
    assert!(
        !record.state.is_terminal(),
        "WaitingApproval is non-terminal"
    );
}

// ── 4. WaitingApproval → Running → Completed (full happy path) ───────────────

#[tokio::test]
async fn task_completes_after_approval() {
    let store = InMemoryStore::new();
    store.append(&setup("run_done", "task_done")).await.unwrap();
    store
        .append(&[
            task_transition(
                "t1",
                "task_done",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "t2",
                "task_done",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            task_transition(
                "t3",
                "task_done",
                Some(TaskState::Running),
                TaskState::WaitingApproval,
                None,
            ),
            task_transition(
                "t4",
                "task_done",
                Some(TaskState::WaitingApproval),
                TaskState::Running,
                None,
            ),
            task_transition(
                "t5",
                "task_done",
                Some(TaskState::Running),
                TaskState::Completed,
                None,
            ),
        ])
        .await
        .unwrap();

    let record = TaskReadModel::get(&store, &TaskId::new("task_done"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.state, TaskState::Completed);
    assert!(record.state.is_terminal(), "Completed is terminal");
    assert_eq!(record.version, 6, "create(1) + 5 transitions = version 6");
    assert!(record.failure_class.is_none());
}

// ── 5. is_terminal() contract for all terminal states ─────────────────────────

#[test]
fn terminal_states_are_completed_failed_canceled_deadlettered() {
    assert!(TaskState::Completed.is_terminal());
    assert!(TaskState::Failed.is_terminal());
    assert!(TaskState::Canceled.is_terminal());
    assert!(TaskState::DeadLettered.is_terminal());
    assert!(!TaskState::Queued.is_terminal());
    assert!(!TaskState::Leased.is_terminal());
    assert!(!TaskState::Running.is_terminal());
    assert!(!TaskState::WaitingApproval.is_terminal());
    assert!(!TaskState::WaitingDependency.is_terminal());
    assert!(!TaskState::Paused.is_terminal());
    assert!(!TaskState::RetryableFailed.is_terminal());
}

// ── 6. Failure path: Running → Failed with failure_class ─────────────────────

#[tokio::test]
async fn task_fails_with_execution_error() {
    let store = InMemoryStore::new();
    store.append(&setup("run_fail", "task_fail")).await.unwrap();
    store
        .append(&[
            task_transition(
                "t1",
                "task_fail",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "t2",
                "task_fail",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            evt(
                "t3",
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: TaskId::new("task_fail"),
                    transition: StateTransition {
                        from: Some(TaskState::Running),
                        to: TaskState::Failed,
                    },
                    failure_class: Some(FailureClass::ExecutionError),
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let record = TaskReadModel::get(&store, &TaskId::new("task_fail"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.state, TaskState::Failed);
    assert!(record.state.is_terminal());
    assert_eq!(record.failure_class, Some(FailureClass::ExecutionError));
    assert_eq!(record.version, 4);
}

// ── 7. Failure path: WaitingApproval → Failed (approval rejected) ─────────────

#[tokio::test]
async fn task_fails_when_approval_rejected() {
    let store = InMemoryStore::new();
    store.append(&setup("run_rej", "task_rej")).await.unwrap();
    store
        .append(&[
            task_transition(
                "t1",
                "task_rej",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "t2",
                "task_rej",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            task_transition(
                "t3",
                "task_rej",
                Some(TaskState::Running),
                TaskState::WaitingApproval,
                None,
            ),
            evt(
                "t4",
                RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project(),
                    task_id: TaskId::new("task_rej"),
                    transition: StateTransition {
                        from: Some(TaskState::WaitingApproval),
                        to: TaskState::Failed,
                    },
                    failure_class: Some(FailureClass::ApprovalRejected),
                    pause_reason: None,
                    resume_trigger: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let record = TaskReadModel::get(&store, &TaskId::new("task_rej"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(record.state, TaskState::Failed);
    assert_eq!(record.failure_class, Some(FailureClass::ApprovalRejected));
}

// ── 8. RetryableFailed increments retry_count ────────────────────────────────

#[tokio::test]
async fn retryable_failure_increments_retry_count() {
    let store = InMemoryStore::new();
    store
        .append(&setup("run_retry", "task_retry"))
        .await
        .unwrap();

    // First attempt: fails with retryable error.
    store
        .append(&[
            task_transition(
                "t1",
                "task_retry",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "t2",
                "task_retry",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            task_transition(
                "t3",
                "task_retry",
                Some(TaskState::Running),
                TaskState::RetryableFailed,
                None,
            ),
        ])
        .await
        .unwrap();

    let after_first_fail = TaskReadModel::get(&store, &TaskId::new("task_retry"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after_first_fail.state, TaskState::RetryableFailed);
    assert_eq!(
        after_first_fail.retry_count, 1,
        "first retryable failure = retry_count 1"
    );
    assert!(
        !after_first_fail.state.is_terminal(),
        "RetryableFailed is non-terminal"
    );

    // Re-queue and retry.
    store
        .append(&[
            task_transition(
                "t4",
                "task_retry",
                Some(TaskState::RetryableFailed),
                TaskState::Queued,
                None,
            ),
            task_transition(
                "t5",
                "task_retry",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "t6",
                "task_retry",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            task_transition(
                "t7",
                "task_retry",
                Some(TaskState::Running),
                TaskState::RetryableFailed,
                None,
            ),
        ])
        .await
        .unwrap();

    let after_second_fail = TaskReadModel::get(&store, &TaskId::new("task_retry"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after_second_fail.retry_count, 2,
        "second retryable failure = retry_count 2"
    );
}

// ── 9. list_by_parent_run returns tasks with correct states ────────────────────
//      (TaskReadModel's "list_by_run" is list_by_parent_run)

#[tokio::test]
async fn list_by_parent_run_returns_tasks_with_correct_states() {
    let store = InMemoryStore::new();

    // Create run with 3 tasks in different states.
    let sess = "sess_list";
    store
        .append(&[
            evt(
                "e_s",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new(sess),
                }),
            ),
            evt(
                "e_r",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new(sess),
                    run_id: RunId::new("run_list"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            // Task A: will complete.
            evt(
                "e_ta",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("task_lst_a"),
                    parent_run_id: Some(RunId::new("run_list")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            // Task B: will fail.
            evt(
                "e_tb",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("task_lst_b"),
                    parent_run_id: Some(RunId::new("run_list")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            // Task C: still queued.
            evt(
                "e_tc",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("task_lst_c"),
                    parent_run_id: Some(RunId::new("run_list")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Transition A to Completed.
    store
        .append(&[
            task_transition(
                "a1",
                "task_lst_a",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "a2",
                "task_lst_a",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            task_transition(
                "a3",
                "task_lst_a",
                Some(TaskState::Running),
                TaskState::Completed,
                None,
            ),
        ])
        .await
        .unwrap();

    // Transition B to Failed.
    store
        .append(&[
            task_transition(
                "b1",
                "task_lst_b",
                Some(TaskState::Queued),
                TaskState::Leased,
                None,
            ),
            task_transition(
                "b2",
                "task_lst_b",
                Some(TaskState::Leased),
                TaskState::Running,
                None,
            ),
            task_transition(
                "b3",
                "task_lst_b",
                Some(TaskState::Running),
                TaskState::Failed,
                Some(FailureClass::ExecutionError),
            ),
        ])
        .await
        .unwrap();

    let tasks = TaskReadModel::list_by_parent_run(&store, &RunId::new("run_list"), 10)
        .await
        .unwrap();

    assert_eq!(tasks.len(), 3, "all 3 tasks returned for run_list");
    assert!(tasks
        .iter()
        .all(|t| t.parent_run_id == Some(RunId::new("run_list"))));

    let find = |id: &str| tasks.iter().find(|t| t.task_id.as_str() == id).unwrap();
    assert_eq!(find("task_lst_a").state, TaskState::Completed);
    assert_eq!(find("task_lst_b").state, TaskState::Failed);
    assert_eq!(
        find("task_lst_b").failure_class,
        Some(FailureClass::ExecutionError)
    );
    assert_eq!(
        find("task_lst_c").state,
        TaskState::Queued,
        "C was never transitioned"
    );
}

// ── 10. list_by_parent_run cross-run isolation ────────────────────────────────

#[tokio::test]
async fn list_by_parent_run_scoped_to_run() {
    let store = InMemoryStore::new();
    let sess = "sess_iso";

    store
        .append(&[
            evt(
                "e_s",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project(),
                    session_id: SessionId::new(sess),
                }),
            ),
            evt(
                "e_r1",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new(sess),
                    run_id: RunId::new("run_iso_1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            evt(
                "e_r2",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    session_id: SessionId::new(sess),
                    run_id: RunId::new("run_iso_2"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            evt(
                "e_t1",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("task_iso_1"),
                    parent_run_id: Some(RunId::new("run_iso_1")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            evt(
                "e_t2",
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project(),
                    task_id: TaskId::new("task_iso_2"),
                    parent_run_id: Some(RunId::new("run_iso_2")),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let run1_tasks = TaskReadModel::list_by_parent_run(&store, &RunId::new("run_iso_1"), 10)
        .await
        .unwrap();
    assert_eq!(run1_tasks.len(), 1);
    assert_eq!(run1_tasks[0].task_id.as_str(), "task_iso_1");

    let run2_tasks = TaskReadModel::list_by_parent_run(&store, &RunId::new("run_iso_2"), 10)
        .await
        .unwrap();
    assert_eq!(run2_tasks.len(), 1);
    assert_eq!(run2_tasks[0].task_id.as_str(), "task_iso_2");
}

// ── 11. Version increments on each transition ─────────────────────────────────

#[tokio::test]
async fn version_increments_on_every_transition() {
    let store = InMemoryStore::new();
    store.append(&setup("run_ver", "task_ver")).await.unwrap();

    macro_rules! check {
        ($store:expr, $ver:expr, $state:expr) => {{
            let r = TaskReadModel::get($store, &TaskId::new("task_ver"))
                .await
                .unwrap()
                .unwrap();
            assert_eq!(r.version, $ver, "version must be {}", $ver);
            assert_eq!(r.state, $state);
        }};
    }

    store
        .append(&[task_transition(
            "t1",
            "task_ver",
            Some(TaskState::Queued),
            TaskState::Leased,
            None,
        )])
        .await
        .unwrap();
    check!(&store, 2, TaskState::Leased);

    store
        .append(&[task_transition(
            "t2",
            "task_ver",
            Some(TaskState::Leased),
            TaskState::Running,
            None,
        )])
        .await
        .unwrap();
    check!(&store, 3, TaskState::Running);

    store
        .append(&[task_transition(
            "t3",
            "task_ver",
            Some(TaskState::Running),
            TaskState::WaitingApproval,
            None,
        )])
        .await
        .unwrap();
    check!(&store, 4, TaskState::WaitingApproval);

    store
        .append(&[task_transition(
            "t4",
            "task_ver",
            Some(TaskState::WaitingApproval),
            TaskState::Running,
            None,
        )])
        .await
        .unwrap();
    check!(&store, 5, TaskState::Running);

    store
        .append(&[task_transition(
            "t5",
            "task_ver",
            Some(TaskState::Running),
            TaskState::Completed,
            None,
        )])
        .await
        .unwrap();
    check!(&store, 6, TaskState::Completed);
}

// ── 12. Task transition domain contract ───────────────────────────────────────

#[test]
fn valid_task_transitions_per_domain() {
    // Core happy path.
    assert!(can_transition_task_state(
        TaskState::Queued,
        TaskState::Leased
    ));
    assert!(can_transition_task_state(
        TaskState::Leased,
        TaskState::Running
    ));
    assert!(can_transition_task_state(
        TaskState::Running,
        TaskState::WaitingApproval
    ));
    assert!(can_transition_task_state(
        TaskState::WaitingApproval,
        TaskState::Running
    ));
    assert!(can_transition_task_state(
        TaskState::Running,
        TaskState::Completed
    ));

    // Failure paths.
    assert!(can_transition_task_state(
        TaskState::Running,
        TaskState::Failed
    ));
    assert!(can_transition_task_state(
        TaskState::WaitingApproval,
        TaskState::Failed
    ));
    assert!(can_transition_task_state(
        TaskState::Running,
        TaskState::RetryableFailed
    ));
    assert!(can_transition_task_state(
        TaskState::RetryableFailed,
        TaskState::Queued
    ));

    // Invalid transitions.
    assert!(!can_transition_task_state(
        TaskState::Completed,
        TaskState::Running
    ));
    assert!(!can_transition_task_state(
        TaskState::Failed,
        TaskState::Running
    ));
    assert!(
        !can_transition_task_state(TaskState::Queued, TaskState::Running),
        "must go via Leased"
    );
    assert!(!can_transition_task_state(
        TaskState::Queued,
        TaskState::Completed
    ));
}
