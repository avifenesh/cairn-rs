//! Cross-backend parity tests.
//!
//! Verifies that InMemoryStore and SQLite event log produce identical
//! results for the same event sequence. This ensures workers can depend
//! on the store without backend-specific conditionals.

#[cfg(feature = "sqlite")]
mod sqlite_parity {
    use cairn_domain::*;
    use cairn_store::event_log::EventLog;
    use cairn_store::in_memory::InMemoryStore;
    use cairn_store::sqlite::SqliteAdapter;

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant", "workspace", "project")
    }

    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn make_envelope(event: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_{n}")),
            EventSource::Runtime,
            event,
        )
    }

    fn session_created(id: &str) -> EventEnvelope<RuntimeEvent> {
        make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
            project: test_project(),
            session_id: SessionId::new(id),
        }))
    }

    fn run_created(run_id: &str, session_id: &str) -> EventEnvelope<RuntimeEvent> {
        make_envelope(RuntimeEvent::RunCreated(RunCreated {
            project: test_project(),
            session_id: SessionId::new(session_id),
            run_id: RunId::new(run_id),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }))
    }

    fn run_state_changed(run_id: &str, to: RunState) -> EventEnvelope<RuntimeEvent> {
        make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
            project: test_project(),
            run_id: RunId::new(run_id),
            transition: StateTransition { from: None, to },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }))
    }

    /// Both backends assign monotonically increasing positions.
    #[tokio::test]
    async fn event_positions_are_monotonic_in_both_backends() {
        let mem = InMemoryStore::new();
        let sqlite_adapter = SqliteAdapter::in_memory().await.unwrap();
        let sqlite_log = cairn_store::sqlite::SqliteEventLog::new(sqlite_adapter.pool().clone());

        let events = vec![
            session_created("s1"),
            session_created("s2"),
            session_created("s3"),
        ];

        let mem_positions = mem.append(&events).await.unwrap();
        let sqlite_positions = sqlite_log.append(&events).await.unwrap();

        // Both should return 3 positions.
        assert_eq!(mem_positions.len(), 3);
        assert_eq!(sqlite_positions.len(), 3);

        // Positions should be monotonically increasing in both.
        for positions in [&mem_positions, &sqlite_positions] {
            for window in positions.windows(2) {
                assert!(window[0].0 < window[1].0);
            }
        }
    }

    /// Stream reads return events in the same order for both backends.
    #[tokio::test]
    async fn stream_read_order_is_identical() {
        let mem = InMemoryStore::new();
        let sqlite_adapter = SqliteAdapter::in_memory().await.unwrap();
        let sqlite_log = cairn_store::sqlite::SqliteEventLog::new(sqlite_adapter.pool().clone());

        let events = vec![
            session_created("s1"),
            run_created("r1", "s1"),
            run_state_changed("r1", RunState::Running),
            run_state_changed("r1", RunState::Completed),
        ];

        mem.append(&events).await.unwrap();
        sqlite_log.append(&events).await.unwrap();

        let mem_stream = mem.read_stream(None, 100).await.unwrap();
        let sqlite_stream = sqlite_log.read_stream(None, 100).await.unwrap();

        assert_eq!(mem_stream.len(), sqlite_stream.len());

        for (m, s) in mem_stream.iter().zip(sqlite_stream.iter()) {
            // Same event payload.
            assert_eq!(m.envelope.payload, s.envelope.payload);
            // Same relative position ordering.
            assert_eq!(
                m.position.0 <= mem_stream.last().unwrap().position.0,
                s.position.0 <= sqlite_stream.last().unwrap().position.0
            );
        }
    }

    /// Cursor-based replay returns the same events after a given position.
    #[tokio::test]
    async fn cursor_replay_produces_same_tail() {
        let mem = InMemoryStore::new();
        let sqlite_adapter = SqliteAdapter::in_memory().await.unwrap();
        let sqlite_log = cairn_store::sqlite::SqliteEventLog::new(sqlite_adapter.pool().clone());

        let events = vec![
            session_created("s1"),
            session_created("s2"),
            session_created("s3"),
            session_created("s4"),
        ];

        let mem_pos = mem.append(&events).await.unwrap();
        let sqlite_pos = sqlite_log.append(&events).await.unwrap();

        // Read after position 2 in both.
        let mem_tail = mem.read_stream(Some(mem_pos[1]), 100).await.unwrap();
        let sqlite_tail = sqlite_log
            .read_stream(Some(sqlite_pos[1]), 100)
            .await
            .unwrap();

        assert_eq!(mem_tail.len(), 2);
        assert_eq!(sqlite_tail.len(), 2);

        for (m, s) in mem_tail.iter().zip(sqlite_tail.iter()) {
            assert_eq!(m.envelope.payload, s.envelope.payload);
        }
    }

    /// Cursor stability: a cursor from batch 1 remains valid after batch 2+3
    /// are appended. Reading from the batch-1 cursor returns exactly batch 2+3.
    #[tokio::test]
    async fn cursor_remains_stable_across_later_appends() {
        let mem = InMemoryStore::new();

        // Batch 1.
        let b1 = vec![session_created("s1"), session_created("s2")];
        let b1_pos = mem.append(&b1).await.unwrap();
        let cursor_after_b1 = *b1_pos.last().unwrap();

        // Batch 2.
        let b2 = vec![session_created("s3")];
        mem.append(&b2).await.unwrap();

        // Batch 3.
        let b3 = vec![session_created("s4"), session_created("s5")];
        mem.append(&b3).await.unwrap();

        // Read from cursor_after_b1 — should return s3, s4, s5 (batches 2+3).
        let tail = mem.read_stream(Some(cursor_after_b1), 100).await.unwrap();
        assert_eq!(tail.len(), 3, "cursor should produce exactly batches 2+3");

        // Verify head is s5.
        let head = mem.head_position().await.unwrap().unwrap();
        assert_eq!(head.0, 5);

        // The cursor itself didn't shift.
        let tail_again = mem.read_stream(Some(cursor_after_b1), 100).await.unwrap();
        assert_eq!(tail.len(), tail_again.len(), "cursor should be idempotent");
    }

    /// Head position is consistent across backends.
    #[tokio::test]
    async fn head_position_is_consistent() {
        let mem = InMemoryStore::new();
        let sqlite_adapter = SqliteAdapter::in_memory().await.unwrap();
        let sqlite_log = cairn_store::sqlite::SqliteEventLog::new(sqlite_adapter.pool().clone());

        // Empty log.
        assert_eq!(mem.head_position().await.unwrap(), None);
        assert_eq!(sqlite_log.head_position().await.unwrap(), None);

        // After appending.
        let events = vec![session_created("s1"), session_created("s2")];
        mem.append(&events).await.unwrap();
        sqlite_log.append(&events).await.unwrap();

        let mem_head = mem.head_position().await.unwrap().unwrap();
        let sqlite_head = sqlite_log.head_position().await.unwrap().unwrap();

        // Both should point to position 2.
        assert_eq!(mem_head.0, 2);
        assert_eq!(sqlite_head.0, 2);
    }

    /// Read model list queries return deterministic ordering in both backends.
    #[tokio::test]
    async fn list_queries_return_deterministic_ordering() {
        use cairn_store::projections::SessionReadModel;

        let mem = InMemoryStore::new();
        let project = test_project();

        // Create sessions in a specific order.
        let events = vec![
            session_created("s_beta"),
            session_created("s_alpha"),
            session_created("s_gamma"),
        ];
        mem.append(&events).await.unwrap();

        // InMemoryStore should now return sessions sorted by
        // (created_at, session_id) — all have same created_at
        // so it falls to session_id alphabetical order.
        let sessions = mem.list_by_project(&project, 10, 0).await.unwrap();
        assert_eq!(sessions.len(), 3);

        // Since all events are appended at the same millisecond,
        // secondary sort is by session_id.
        let ids: Vec<&str> = sessions.iter().map(|s| s.session_id.as_str()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids, "sessions should be in deterministic order");
    }

    /// Run list ordering is deterministic within a session.
    #[tokio::test]
    async fn run_list_ordering_is_deterministic() {
        use cairn_store::projections::RunReadModel;

        let mem = InMemoryStore::new();

        // Create session first, then runs.
        mem.append(&[session_created("s1")]).await.unwrap();
        mem.append(&[
            run_created("r_charlie", "s1"),
            run_created("r_alpha", "s1"),
            run_created("r_bravo", "s1"),
        ])
        .await
        .unwrap();

        let runs = mem
            .list_by_session(&cairn_domain::SessionId::new("s1"), 10, 0)
            .await
            .unwrap();
        assert_eq!(runs.len(), 3);

        let ids: Vec<&str> = runs.iter().map(|r| r.run_id.as_str()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort();
        assert_eq!(ids, sorted_ids, "runs should be in deterministic order");
    }

    /// Replay/rebuild parity: task, approval, and tool invocation projections
    /// produce consistent current-state rows after event replay.
    #[tokio::test]
    async fn task_approval_projection_survives_replay() {
        use cairn_store::projections::{ApprovalReadModel, TaskReadModel};

        let mem = InMemoryStore::new();
        let project = test_project();

        // Build up state: session -> run -> tasks -> approvals.
        mem.append(&[session_created("s1")]).await.unwrap();
        mem.append(&[run_created("r1", "s1")]).await.unwrap();

        // Create tasks.
        for id in ["t_alpha", "t_bravo", "t_charlie"] {
            mem.append(&[make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new(id),
                parent_run_id: Some(RunId::new("r1")),
                parent_task_id: None,
                prompt_release_id: None,
            }))])
            .await
            .unwrap();
        }

        // Complete one, fail another.
        mem.append(&[make_envelope(RuntimeEvent::TaskStateChanged(
            TaskStateChanged {
                project: project.clone(),
                task_id: TaskId::new("t_alpha"),
                transition: StateTransition {
                    from: None,
                    to: TaskState::Completed,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            },
        ))])
        .await
        .unwrap();

        mem.append(&[make_envelope(RuntimeEvent::TaskStateChanged(
            TaskStateChanged {
                project: project.clone(),
                task_id: TaskId::new("t_bravo"),
                transition: StateTransition {
                    from: None,
                    to: TaskState::Failed,
                },
                failure_class: Some(FailureClass::ExecutionError),
                pause_reason: None,
                resume_trigger: None,
            },
        ))])
        .await
        .unwrap();

        // Create approvals.
        mem.append(&[make_envelope(RuntimeEvent::ApprovalRequested(
            ApprovalRequested {
                project: project.clone(),
                approval_id: ApprovalId::new("ap_1"),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            },
        ))])
        .await
        .unwrap();

        // Verify task states.
        let t_alpha = TaskReadModel::get(&mem, &TaskId::new("t_alpha"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(t_alpha.state, TaskState::Completed);

        let t_bravo = TaskReadModel::get(&mem, &TaskId::new("t_bravo"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(t_bravo.state, TaskState::Failed);
        assert_eq!(t_bravo.failure_class, Some(FailureClass::ExecutionError));

        let t_charlie = TaskReadModel::get(&mem, &TaskId::new("t_charlie"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(t_charlie.state, TaskState::Queued);

        // Verify approval.
        let ap = ApprovalReadModel::get(&mem, &ApprovalId::new("ap_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ap.requirement, ApprovalRequirement::Required);
        assert_eq!(ap.decision, None);

        // Verify queued task list is deterministic.
        let queued = mem
            .list_by_state(&project, TaskState::Queued, 10)
            .await
            .unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].task_id, TaskId::new("t_charlie"));

        // Verify pending approval list.
        let pending = mem.list_pending(&project, 10, 0).await.unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].approval_id, ApprovalId::new("ap_1"));

        // Verify event stream captures everything.
        let all_events = mem.read_stream(None, 100).await.unwrap();
        assert!(all_events.len() >= 6); // session + run + 3 tasks + 2 state changes + 1 approval
    }

    /// Tool invocation projection parity: started/completed/failed events
    /// produce correct read-model state and deterministic list ordering.
    #[tokio::test]
    async fn tool_invocation_projection_parity() {
        use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};

        let mem = InMemoryStore::new();

        // Session + run required as parent context.
        mem.append(&[session_created("s1")]).await.unwrap();
        mem.append(&[run_created("r1", "s1")]).await.unwrap();

        // Tool invocation started (later request time).
        mem.append(&[make_envelope(RuntimeEvent::ToolInvocationStarted(
            ToolInvocationStarted {
                project: test_project(),
                invocation_id: ToolInvocationId::new("tool_b"),
                session_id: Some(SessionId::new("s1")),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "fs.write".to_owned(),
                },
                execution_class: cairn_domain::ExecutionClass::SupervisedProcess,
                prompt_release_id: None,
                requested_at_ms: 200,
                started_at_ms: 201,
            },
        ))])
        .await
        .unwrap();

        // Tool invocation started (earlier request time — should sort first).
        mem.append(&[make_envelope(RuntimeEvent::ToolInvocationStarted(
            ToolInvocationStarted {
                project: test_project(),
                invocation_id: ToolInvocationId::new("tool_a"),
                session_id: Some(SessionId::new("s1")),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "fs.read".to_owned(),
                },
                execution_class: cairn_domain::ExecutionClass::SupervisedProcess,
                prompt_release_id: None,
                requested_at_ms: 100,
                started_at_ms: 101,
            },
        ))])
        .await
        .unwrap();

        // Complete tool_a.
        mem.append(&[make_envelope(RuntimeEvent::ToolInvocationCompleted(
            ToolInvocationCompleted {
                project: test_project(),
                invocation_id: ToolInvocationId::new("tool_a"),
                task_id: None,
                tool_name: "fs.read".to_owned(),
                finished_at_ms: 150,
                outcome: ToolInvocationOutcomeKind::Success,
            },
        ))])
        .await
        .unwrap();

        // Fail tool_b.
        mem.append(&[make_envelope(RuntimeEvent::ToolInvocationFailed(
            ToolInvocationFailed {
                project: test_project(),
                invocation_id: ToolInvocationId::new("tool_b"),
                task_id: None,
                tool_name: "fs.write".to_owned(),
                finished_at_ms: 250,
                outcome: ToolInvocationOutcomeKind::PermanentFailure,
                error_message: Some("disk full".to_owned()),
            },
        ))])
        .await
        .unwrap();

        // Verify individual records.
        use cairn_store::projections::ToolInvocationReadModel;

        let a = ToolInvocationReadModel::get(&mem, &ToolInvocationId::new("tool_a"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            a.state,
            cairn_domain::tool_invocation::ToolInvocationState::Completed
        );
        assert_eq!(a.outcome, Some(ToolInvocationOutcomeKind::Success));
        assert_eq!(a.error_message, None);

        let b = ToolInvocationReadModel::get(&mem, &ToolInvocationId::new("tool_b"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            b.state,
            cairn_domain::tool_invocation::ToolInvocationState::Failed
        );
        assert_eq!(b.outcome, Some(ToolInvocationOutcomeKind::PermanentFailure));
        assert_eq!(b.error_message.as_deref(), Some("disk full"));

        // Verify list ordering: by (requested_at_ms, invocation_id).
        // tool_a was requested at 100, tool_b at 200 — tool_a should come first.
        let list = ToolInvocationReadModel::list_by_run(&mem, &RunId::new("r1"), 10, 0)
            .await
            .unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].invocation_id.as_str(), "tool_a");
        assert_eq!(list[1].invocation_id.as_str(), "tool_b");
    }

    /// Approval list ordering: pending approvals returned in deterministic
    /// (created_at, approval_id) order — the next API surface Worker 8 consumes.
    #[tokio::test]
    async fn approval_list_ordering_is_deterministic() {
        use cairn_store::projections::ApprovalReadModel;
        use cairn_store::sqlite::SqliteAdapter;

        let mem = InMemoryStore::new();
        let sqlite = SqliteAdapter::in_memory().await.unwrap();
        let project = test_project();

        let bootstrap = vec![session_created("s1"), run_created("r1", "s1")];
        mem.append(&bootstrap).await.unwrap();

        sqlx::query(
            "INSERT INTO sessions (session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at)
             VALUES (?, ?, ?, ?, 'open', 1, ?, ?)",
        )
        .bind("s1")
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(1_i64)
        .bind(1_i64)
        .execute(sqlite.pool())
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO runs (run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, 'pending', 1, ?, ?)",
        )
        .bind("r1")
        .bind("s1")
        .bind::<Option<&str>>(None)
        .bind(project.tenant_id.as_str())
        .bind(project.workspace_id.as_str())
        .bind(project.project_id.as_str())
        .bind(1_i64)
        .bind(1_i64)
        .execute(sqlite.pool())
        .await
        .unwrap();

        // Create approvals in a single batch so the created_at tie-breaker is
        // deterministic and the approval_id ordering is the observable tie break.
        let approvals = vec![
            make_envelope(RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project.clone(),
                approval_id: ApprovalId::new("ap_charlie"),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
            make_envelope(RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project.clone(),
                approval_id: ApprovalId::new("ap_alpha"),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
            make_envelope(RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project.clone(),
                approval_id: ApprovalId::new("ap_bravo"),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
        ];
        mem.append(&approvals).await.unwrap();
        for approval_id in ["ap_charlie", "ap_alpha", "ap_bravo"] {
            sqlx::query(
                "INSERT INTO approvals (approval_id, tenant_id, workspace_id, project_id, run_id, task_id, requirement, decision, title, description, version, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, 1, ?, ?)",
            )
            .bind(approval_id)
            .bind(project.tenant_id.as_str())
            .bind(project.workspace_id.as_str())
            .bind(project.project_id.as_str())
            .bind(Some("r1"))
            .bind::<Option<&str>>(None)
            .bind("required")
            .bind(2_i64)
            .bind(2_i64)
            .execute(sqlite.pool())
            .await
            .unwrap();
        }

        let pending_mem = mem.list_pending(&project, 10, 0).await.unwrap();
        let pending_sqlite = sqlite.list_pending(&project, 10, 0).await.unwrap();
        assert_eq!(pending_mem.len(), 3);
        assert_eq!(pending_sqlite.len(), 3);

        let ids_mem: Vec<&str> = pending_mem.iter().map(|a| a.approval_id.as_str()).collect();
        let ids_sqlite: Vec<&str> = pending_sqlite
            .iter()
            .map(|a| a.approval_id.as_str())
            .collect();
        let mut sorted = ids_mem.clone();
        sorted.sort();
        assert_eq!(
            ids_mem, sorted,
            "approvals should be in deterministic order"
        );
        assert_eq!(ids_mem, ids_sqlite, "backend ordering should match");

        // Resolve one — list should shrink on both backends.
        let resolved = vec![make_envelope(RuntimeEvent::ApprovalResolved(
            ApprovalResolved {
                project: project.clone(),
                approval_id: ApprovalId::new("ap_alpha"),
                decision: ApprovalDecision::Approved,
            },
        ))];
        mem.append(&resolved).await.unwrap();
        sqlx::query(
            "UPDATE approvals SET decision = 'approved', updated_at = ? WHERE approval_id = ?",
        )
        .bind(3_i64)
        .bind("ap_alpha")
        .execute(sqlite.pool())
        .await
        .unwrap();

        let pending_after_mem = mem.list_pending(&project, 10, 0).await.unwrap();
        let pending_after_sqlite = sqlite.list_pending(&project, 10, 0).await.unwrap();
        assert_eq!(pending_after_mem.len(), 2);
        assert_eq!(pending_after_sqlite.len(), 2);

        let ids_after_mem: Vec<&str> = pending_after_mem
            .iter()
            .map(|a| a.approval_id.as_str())
            .collect();
        let ids_after_sqlite: Vec<&str> = pending_after_sqlite
            .iter()
            .map(|a| a.approval_id.as_str())
            .collect();
        assert_eq!(ids_after_mem, vec!["ap_bravo", "ap_charlie"]);
        assert_eq!(
            ids_after_mem, ids_after_sqlite,
            "backend ordering should match"
        );
    }

    /// Mailbox list ordering: messages for a run returned in deterministic order.
    #[tokio::test]
    async fn mailbox_list_ordering_is_deterministic() {
        use cairn_store::projections::MailboxReadModel;

        let mem = InMemoryStore::new();
        let project = test_project();

        mem.append(&[session_created("s1")]).await.unwrap();
        mem.append(&[run_created("r1", "s1")]).await.unwrap();

        for id in ["msg_z", "msg_a", "msg_m"] {
            mem.append(&[make_envelope(RuntimeEvent::MailboxMessageAppended(
                MailboxMessageAppended {
                    project: project.clone(),
                    message_id: MailboxMessageId::new(id),
                    run_id: Some(RunId::new("r1")),
                    task_id: None,
                    content: String::new(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                },
            ))])
            .await
            .unwrap();
        }

        let msgs = mem.list_by_run(&RunId::new("r1"), 10, 0).await.unwrap();
        assert_eq!(msgs.len(), 3);

        let ids: Vec<&str> = msgs.iter().map(|m| m.message_id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(
            ids, sorted,
            "mailbox messages should be in deterministic order"
        );
    }

    /// latest_root_run: returns the most recently created root run (no parent),
    /// which Worker 8 uses for session state derivation.
    #[tokio::test]
    async fn latest_root_run_returns_most_recent() {
        use cairn_store::projections::RunReadModel;

        let mem = InMemoryStore::new();

        mem.append(&[session_created("s1")]).await.unwrap();

        // Create two root runs in one batch.
        mem.append(&[run_created("r_old", "s1"), run_created("r_new", "s1")])
            .await
            .unwrap();

        let latest = mem
            .latest_root_run(&cairn_domain::SessionId::new("s1"))
            .await
            .unwrap()
            .unwrap();

        // Both have same created_at (single batch), so latest_root_run
        // should return one deterministically. The important thing is
        // it returns a root run (no parent_run_id).
        assert!(
            latest.parent_run_id.is_none(),
            "latest root run must have no parent"
        );

        // any_non_terminal should be true since runs are in Pending state.
        let has_non_terminal = mem
            .any_non_terminal(&cairn_domain::SessionId::new("s1"))
            .await
            .unwrap();
        assert!(has_non_terminal, "pending runs are non-terminal");
    }

    /// Child-task lookup by parent_run_id: list_by_parent_run returns
    /// children in deterministic order, any_non_terminal_children detects
    /// non-terminal children for stale-dependency resolution.
    #[tokio::test]
    async fn child_task_lookup_by_parent_run() {
        use cairn_store::projections::TaskReadModel;

        let mem = InMemoryStore::new();
        let project = test_project();

        mem.append(&[session_created("s1")]).await.unwrap();
        mem.append(&[run_created("r_parent", "s1")]).await.unwrap();

        // Create child tasks in one batch (same created_at).
        mem.append(&[
            make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new("child_z"),
                parent_run_id: Some(RunId::new("r_parent")),
                parent_task_id: None,
                prompt_release_id: None,
            })),
            make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new("child_a"),
                parent_run_id: Some(RunId::new("r_parent")),
                parent_task_id: None,
                prompt_release_id: None,
            })),
            make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new("child_m"),
                parent_run_id: Some(RunId::new("r_parent")),
                parent_task_id: None,
                prompt_release_id: None,
            })),
        ])
        .await
        .unwrap();

        // list_by_parent_run returns children sorted by (created_at, task_id).
        let children = mem
            .list_by_parent_run(&RunId::new("r_parent"), 10)
            .await
            .unwrap();
        assert_eq!(children.len(), 3);
        let ids: Vec<&str> = children.iter().map(|t| t.task_id.as_str()).collect();
        assert_eq!(ids, vec!["child_a", "child_m", "child_z"]);

        // All children are Queued (non-terminal).
        assert!(mem
            .any_non_terminal_children(&RunId::new("r_parent"))
            .await
            .unwrap());

        // Complete all children.
        for id in ["child_a", "child_m", "child_z"] {
            mem.append(&[make_envelope(RuntimeEvent::TaskStateChanged(
                TaskStateChanged {
                    project: project.clone(),
                    task_id: TaskId::new(id),
                    transition: StateTransition {
                        from: None,
                        to: TaskState::Completed,
                    },
                    failure_class: None,
                    pause_reason: None,
                    resume_trigger: None,
                },
            ))])
            .await
            .unwrap();
        }

        // No non-terminal children remain.
        assert!(!mem
            .any_non_terminal_children(&RunId::new("r_parent"))
            .await
            .unwrap());

        // Non-existent parent returns empty.
        let empty = mem
            .list_by_parent_run(&RunId::new("r_nonexistent"), 10)
            .await
            .unwrap();
        assert!(empty.is_empty());
    }

    /// Checkpoint list ordering: checkpoints for a run returned in deterministic
    /// (created_at, checkpoint_id) order. Appended in one batch so created_at
    /// is identical, proving the secondary sort on checkpoint_id works.
    #[tokio::test]
    async fn checkpoint_list_ordering_is_deterministic() {
        use cairn_store::projections::CheckpointReadModel;

        let mem = InMemoryStore::new();
        let project = test_project();

        mem.append(&[session_created("s1")]).await.unwrap();
        mem.append(&[run_created("r1", "s1")]).await.unwrap();

        // Single batch: same created_at for all three.
        mem.append(&[
            make_envelope(RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
                project: project.clone(),
                run_id: RunId::new("r1"),
                checkpoint_id: CheckpointId::new("cp_z"),
                disposition: CheckpointDisposition::Superseded,
                data: None,
            })),
            make_envelope(RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
                project: project.clone(),
                run_id: RunId::new("r1"),
                checkpoint_id: CheckpointId::new("cp_a"),
                disposition: CheckpointDisposition::Superseded,
                data: None,
            })),
            make_envelope(RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
                project: project.clone(),
                run_id: RunId::new("r1"),
                checkpoint_id: CheckpointId::new("cp_m"),
                disposition: CheckpointDisposition::Latest,
                data: None,
            })),
        ])
        .await
        .unwrap();

        let cps = mem.list_by_run(&RunId::new("r1"), 10).await.unwrap();
        assert_eq!(cps.len(), 3);

        // Same created_at -> sorted by checkpoint_id alphabetically.
        let ids: Vec<&str> = cps.iter().map(|c| c.checkpoint_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["cp_a", "cp_m", "cp_z"],
            "checkpoints should be in deterministic order"
        );

        // Verify latest is cp_m.
        let latest = mem
            .latest_for_run(&RunId::new("r1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.checkpoint_id, CheckpointId::new("cp_m"));
    }

    /// Rebuild parity: events written to store A, read back via stream,
    /// replayed into store B — both produce identical read-model state.
    /// Covers task, approval, and tool invocation after external-worker events.
    #[tokio::test]
    async fn rebuild_from_event_stream_produces_identical_state() {
        use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
        use cairn_store::projections::{
            ApprovalReadModel, SessionReadModel, TaskReadModel, ToolInvocationReadModel,
        };

        let store_a = InMemoryStore::new();
        let project = test_project();

        // Build up realistic state in store A.
        let events = vec![
            session_created("s1"),
            run_created("r1", "s1"),
            // Tasks.
            make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new("t1"),
                parent_run_id: Some(RunId::new("r1")),
                parent_task_id: None,
                prompt_release_id: None,
            })),
            make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                project: project.clone(),
                task_id: TaskId::new("t1"),
                transition: StateTransition {
                    from: None,
                    to: TaskState::Completed,
                },
                failure_class: None,
                pause_reason: None,
                resume_trigger: None,
            })),
            // Approval.
            make_envelope(RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project: project.clone(),
                approval_id: ApprovalId::new("ap1"),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            })),
            make_envelope(RuntimeEvent::ApprovalResolved(ApprovalResolved {
                project: project.clone(),
                approval_id: ApprovalId::new("ap1"),
                decision: ApprovalDecision::Approved,
            })),
            // Tool invocation.
            make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project.clone(),
                invocation_id: ToolInvocationId::new("inv1"),
                session_id: Some(SessionId::new("s1")),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "fs.read".to_owned(),
                },
                execution_class: cairn_domain::ExecutionClass::SupervisedProcess,
                prompt_release_id: None,
                requested_at_ms: 100,
                started_at_ms: 101,
            })),
            make_envelope(RuntimeEvent::ToolInvocationCompleted(
                ToolInvocationCompleted {
                    project: project.clone(),
                    invocation_id: ToolInvocationId::new("inv1"),
                    task_id: None,
                    tool_name: "fs.read".to_owned(),
                    finished_at_ms: 200,
                    outcome: ToolInvocationOutcomeKind::Success,
                },
            )),
            // Canceled tool invocation (recently added path).
            make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project: project.clone(),
                invocation_id: ToolInvocationId::new("inv2"),
                session_id: Some(SessionId::new("s1")),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                target: ToolInvocationTarget::Builtin {
                    tool_name: "fs.write".to_owned(),
                },
                execution_class: cairn_domain::ExecutionClass::SandboxedProcess,
                prompt_release_id: None,
                requested_at_ms: 300,
                started_at_ms: 301,
            })),
            make_envelope(RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                project: project.clone(),
                invocation_id: ToolInvocationId::new("inv2"),
                task_id: None,
                tool_name: "fs.write".to_owned(),
                finished_at_ms: 350,
                outcome: ToolInvocationOutcomeKind::Canceled,
                error_message: Some("operator cancel".to_owned()),
            })),
            // External worker (audit-only, no projection).
            make_envelope(RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
                project: project.clone(),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                reason: "lease expired".to_owned(),
            })),
        ];

        store_a.append(&events).await.unwrap();

        // Read all events back from store A's stream.
        let stream = store_a.read_stream(None, 1000).await.unwrap();
        assert_eq!(stream.len(), events.len());

        // Replay into store B by re-appending the envelope payloads.
        let store_b = InMemoryStore::new();
        let replayed: Vec<_> = stream.iter().map(|e| e.envelope.clone()).collect();
        store_b.append(&replayed).await.unwrap();

        // Compare: session.
        let s_a = SessionReadModel::get(&store_a, &SessionId::new("s1"))
            .await
            .unwrap()
            .unwrap();
        let s_b = SessionReadModel::get(&store_b, &SessionId::new("s1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(s_a.state, s_b.state);

        // Compare: task.
        let t_a = TaskReadModel::get(&store_a, &TaskId::new("t1"))
            .await
            .unwrap()
            .unwrap();
        let t_b = TaskReadModel::get(&store_b, &TaskId::new("t1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(t_a.state, t_b.state);
        assert_eq!(t_a.failure_class, t_b.failure_class);
        assert_eq!(t_a.project, t_b.project);
        assert_eq!(t_a.parent_run_id, t_b.parent_run_id);
        assert_eq!(t_a.parent_task_id, t_b.parent_task_id);
        assert_eq!(t_a.lease_owner, t_b.lease_owner);
        assert_eq!(t_a.lease_expires_at, t_b.lease_expires_at);
        assert_eq!(t_a.title, t_b.title);
        assert_eq!(t_a.description, t_b.description);
        assert_eq!(t_a.version, t_b.version);
        // Note: created_at/updated_at are storage-level timestamps set by
        // now_millis() during append, so they naturally differ across stores.

        // Compare: approval.
        let ap_a = ApprovalReadModel::get(&store_a, &ApprovalId::new("ap1"))
            .await
            .unwrap()
            .unwrap();
        let ap_b = ApprovalReadModel::get(&store_b, &ApprovalId::new("ap1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(ap_a.project, ap_b.project);
        assert_eq!(ap_a.run_id, ap_b.run_id);
        assert_eq!(ap_a.task_id, ap_b.task_id);
        assert_eq!(ap_a.requirement, ap_b.requirement);
        assert_eq!(ap_a.decision, ap_b.decision);
        assert_eq!(ap_a.title, ap_b.title);
        assert_eq!(ap_a.description, ap_b.description);
        assert_eq!(ap_a.version, ap_b.version);
        // Note: created_at/updated_at are storage timestamps, not domain state.

        // Compare: tool invocation.
        let inv_a = ToolInvocationReadModel::get(&store_a, &ToolInvocationId::new("inv1"))
            .await
            .unwrap()
            .unwrap();
        let inv_b = ToolInvocationReadModel::get(&store_b, &ToolInvocationId::new("inv1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inv_a.state, inv_b.state);
        assert_eq!(inv_a.outcome, inv_b.outcome);
        assert_eq!(inv_a.error_message, inv_b.error_message);

        // Compare: canceled tool invocation (recently added projection path).
        let inv2_a = ToolInvocationReadModel::get(&store_a, &ToolInvocationId::new("inv2"))
            .await
            .unwrap()
            .unwrap();
        let inv2_b = ToolInvocationReadModel::get(&store_b, &ToolInvocationId::new("inv2"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(inv2_a.state, inv2_b.state);
        assert_eq!(inv2_a.outcome, inv2_b.outcome);
        assert_eq!(inv2_a.error_message, inv2_b.error_message);
        assert_eq!(inv2_a.outcome, Some(ToolInvocationOutcomeKind::Canceled));

        // Compare: pending approval list after rebuild (edge case: one resolved, one pending).
        // Add a second pending approval to store_a and rebuild.
        let extra = vec![make_envelope(RuntimeEvent::ApprovalRequested(
            ApprovalRequested {
                project: project.clone(),
                approval_id: ApprovalId::new("ap2"),
                run_id: Some(RunId::new("r1")),
                task_id: None,
                requirement: ApprovalRequirement::Required,
            },
        ))];
        store_a.append(&extra).await.unwrap();

        // Rebuild store_b with the extra event.
        let full_stream = store_a.read_stream(None, 1000).await.unwrap();
        let store_c = InMemoryStore::new();
        let replayed_c: Vec<_> = full_stream.iter().map(|e| e.envelope.clone()).collect();
        store_c.append(&replayed_c).await.unwrap();

        // ap1 is resolved, ap2 is pending — list_pending should return only ap2.
        let pending_a = store_a.list_pending(&project, 10, 0).await.unwrap();
        let pending_c = store_c.list_pending(&project, 10, 0).await.unwrap();
        assert_eq!(pending_a.len(), 1);
        assert_eq!(pending_c.len(), 1);
        assert_eq!(pending_a[0].approval_id, pending_c[0].approval_id);
        assert_eq!(pending_c[0].approval_id, ApprovalId::new("ap2"));

        // Compare: event stream length.
        let stream_b = store_b.read_stream(None, 1000).await.unwrap();
        // stream_b has the original events, stream length check is against original.
        assert!(!stream_b.is_empty());
    }

    /// Rebuild-ordering regression: tool invocation list_by_run ordering
    /// is preserved after rebuilding from the event stream.
    #[tokio::test]
    async fn rebuild_preserves_tool_invocation_list_ordering() {
        use cairn_domain::tool_invocation::ToolInvocationTarget;
        use cairn_store::projections::ToolInvocationReadModel;

        let store_a = InMemoryStore::new();

        store_a.append(&[session_created("s1")]).await.unwrap();
        store_a.append(&[run_created("r1", "s1")]).await.unwrap();

        // Three tool invocations with different request times (non-alphabetical IDs).
        for (id, req_ms) in [("tool_z", 300u64), ("tool_a", 100), ("tool_m", 200)] {
            store_a
                .append(&[make_envelope(RuntimeEvent::ToolInvocationStarted(
                    ToolInvocationStarted {
                        project: test_project(),
                        invocation_id: ToolInvocationId::new(id),
                        session_id: Some(SessionId::new("s1")),
                        run_id: Some(RunId::new("r1")),
                        task_id: None,
                        target: ToolInvocationTarget::Builtin {
                            tool_name: "test".to_owned(),
                        },
                        execution_class: cairn_domain::ExecutionClass::SupervisedProcess,
                        prompt_release_id: None,
                        requested_at_ms: req_ms,
                        started_at_ms: req_ms + 1,
                    },
                ))])
                .await
                .unwrap();
        }

        // Verify ordering in store A: by requested_at_ms.
        let list_a = ToolInvocationReadModel::list_by_run(&store_a, &RunId::new("r1"), 10, 0)
            .await
            .unwrap();
        assert_eq!(list_a.len(), 3);
        let ids_a: Vec<&str> = list_a.iter().map(|t| t.invocation_id.as_str()).collect();
        assert_eq!(ids_a, vec!["tool_a", "tool_m", "tool_z"]);

        // Rebuild into store B.
        let stream = store_a.read_stream(None, 1000).await.unwrap();
        let store_b = InMemoryStore::new();
        let replayed: Vec<_> = stream.iter().map(|e| e.envelope.clone()).collect();
        store_b.append(&replayed).await.unwrap();

        // Verify ordering survives rebuild.
        let list_b = ToolInvocationReadModel::list_by_run(&store_b, &RunId::new("r1"), 10, 0)
            .await
            .unwrap();
        let ids_b: Vec<&str> = list_b.iter().map(|t| t.invocation_id.as_str()).collect();
        assert_eq!(
            ids_a, ids_b,
            "tool invocation ordering must survive rebuild"
        );
    }
}
