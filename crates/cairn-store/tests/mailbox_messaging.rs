//! Mailbox / inter-agent messaging integration tests (RFC 012).
//!
//! Validates the mailbox pipeline using `InMemoryStore` + `EventLog::append`.
//! The mailbox is the inter-agent communication channel: a run appends a
//! message addressed to another run (or task), and the recipient polls via
//! the `MailboxReadModel`.
//!
//! Contract under test:
//!   MailboxMessageAppended → MailboxRecord stored with all fields intact
//!   list_by_run            → all messages for a run, sorted by message_id
//!   list_by_task           → messages scoped to a task, sorted by created_at
//!   list_pending           → deferred messages due for delivery
//!   Cross-run isolation    → run A's messages never appear in run B's list

use cairn_domain::{
    EventEnvelope, EventId, EventSource, MailboxMessageAppended, MailboxMessageId, ProjectId,
    ProjectKey, RunId, RuntimeEvent, SessionCreated, SessionId, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::{MailboxReadModel, MAX_MESSAGE_CONTENT_LEN},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_mailbox"),
        workspace_id: WorkspaceId::new("w_mailbox"),
        project_id: ProjectId::new("p_mailbox"),
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

fn msg(evt_id: &str, msg_id: &str, run_id: &str, content: &str) -> EventEnvelope<RuntimeEvent> {
    evt(
        evt_id,
        RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
            project: project(),
            message_id: MailboxMessageId::new(msg_id),
            run_id: Some(RunId::new(run_id)),
            task_id: None,
            content: content.to_owned(),
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

// ── 1. Single message stored with all fields intact ───────────────────────────

#[tokio::test]
async fn message_appended_stores_all_fields() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let msg_id = MailboxMessageId::new("msg_001");
    let run_id = RunId::new("run_mb_1");
    let from_run = RunId::new("run_sender");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                project: project(),
                message_id: msg_id.clone(),
                run_id: Some(run_id.clone()),
                task_id: None,
                content: "Hello from run_sender".to_owned(),
                from_run_id: Some(from_run.clone()),
                from_task_id: None,
                deliver_at_ms: 0,
                sender: None,
                recipient: None,
                body: None,
                sent_at: None,
                delivery_status: None,
            }),
        )])
        .await
        .unwrap();

    let record = MailboxReadModel::get(&store, &msg_id)
        .await
        .unwrap()
        .expect("MailboxRecord must exist after MailboxMessageAppended");

    assert_eq!(record.message_id, msg_id);
    assert_eq!(record.run_id, Some(run_id));
    assert_eq!(record.content, "Hello from run_sender");
    assert_eq!(record.from_run_id, Some(from_run));
    assert_eq!(record.deliver_at_ms, 0);
    assert_eq!(record.version, 1);
    assert_eq!(record.project, project());
    assert!(record.created_at >= ts);
}

// ── 2. list_by_run returns all messages for the run ───────────────────────────

#[tokio::test]
async fn list_by_run_returns_all_messages() {
    let store = InMemoryStore::new();

    store
        .append(&[
            msg("e1", "msg_a", "run_multi", "first"),
            msg("e2", "msg_b", "run_multi", "second"),
            msg("e3", "msg_c", "run_multi", "third"),
        ])
        .await
        .unwrap();

    let messages = MailboxReadModel::list_by_run(&store, &RunId::new("run_multi"), 10, 0)
        .await
        .unwrap();

    assert_eq!(messages.len(), 3);
    // Sorted by message_id lexicographically.
    assert_eq!(messages[0].message_id.as_str(), "msg_a");
    assert_eq!(messages[1].message_id.as_str(), "msg_b");
    assert_eq!(messages[2].message_id.as_str(), "msg_c");

    // Content is preserved.
    assert_eq!(messages[0].content, "first");
    assert_eq!(messages[2].content, "third");
}

// ── 3. list_by_run ordering follows message_id lexicographic sort ─────────────

#[tokio::test]
async fn list_by_run_order_is_lexicographic_by_message_id() {
    let store = InMemoryStore::new();

    // Append in reverse lexicographic order to prove ordering comes from message_id.
    store
        .append(&[
            msg("e1", "msg_z", "run_order", "last alpha"),
            msg("e2", "msg_a", "run_order", "first alpha"),
            msg("e3", "msg_m", "run_order", "middle alpha"),
        ])
        .await
        .unwrap();

    let messages = MailboxReadModel::list_by_run(&store, &RunId::new("run_order"), 10, 0)
        .await
        .unwrap();

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].message_id.as_str(), "msg_a");
    assert_eq!(messages[1].message_id.as_str(), "msg_m");
    assert_eq!(messages[2].message_id.as_str(), "msg_z");
}

// ── 4. list_by_run respects limit and offset ──────────────────────────────────

#[tokio::test]
async fn list_by_run_respects_limit_and_offset() {
    let store = InMemoryStore::new();

    store
        .append(&[
            msg("e1", "msg_1a", "run_page", "one"),
            msg("e2", "msg_1b", "run_page", "two"),
            msg("e3", "msg_1c", "run_page", "three"),
            msg("e4", "msg_1d", "run_page", "four"),
        ])
        .await
        .unwrap();

    // First page: limit 2.
    let page1 = MailboxReadModel::list_by_run(&store, &RunId::new("run_page"), 2, 0)
        .await
        .unwrap();
    assert_eq!(page1.len(), 2);
    assert_eq!(page1[0].message_id.as_str(), "msg_1a");
    assert_eq!(page1[1].message_id.as_str(), "msg_1b");

    // Second page: offset 2.
    let page2 = MailboxReadModel::list_by_run(&store, &RunId::new("run_page"), 2, 2)
        .await
        .unwrap();
    assert_eq!(page2.len(), 2);
    assert_eq!(page2[0].message_id.as_str(), "msg_1c");
    assert_eq!(page2[1].message_id.as_str(), "msg_1d");
}

// ── 5. Cross-run isolation ────────────────────────────────────────────────────

#[tokio::test]
async fn messages_are_isolated_between_runs() {
    let store = InMemoryStore::new();

    store
        .append(&[
            msg("e1", "msg_run_a_1", "run_iso_a", "for A"),
            msg("e2", "msg_run_a_2", "run_iso_a", "also for A"),
            msg("e3", "msg_run_b_1", "run_iso_b", "for B"),
        ])
        .await
        .unwrap();

    let for_a = MailboxReadModel::list_by_run(&store, &RunId::new("run_iso_a"), 10, 0)
        .await
        .unwrap();
    assert_eq!(for_a.len(), 2);
    assert!(for_a
        .iter()
        .all(|m| m.run_id == Some(RunId::new("run_iso_a"))));

    let for_b = MailboxReadModel::list_by_run(&store, &RunId::new("run_iso_b"), 10, 0)
        .await
        .unwrap();
    assert_eq!(for_b.len(), 1);
    assert_eq!(for_b[0].message_id.as_str(), "msg_run_b_1");

    // Run with no messages returns empty.
    let for_c = MailboxReadModel::list_by_run(&store, &RunId::new("run_iso_c"), 10, 0)
        .await
        .unwrap();
    assert!(for_c.is_empty());
}

// ── 6. from_run_id and from_task_id (inter-agent routing fields) ──────────────

#[tokio::test]
async fn inter_agent_routing_fields_are_preserved() {
    use cairn_domain::TaskId;

    let store = InMemoryStore::new();
    let msg_id = MailboxMessageId::new("msg_ipc");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                project: project(),
                message_id: msg_id.clone(),
                run_id: Some(RunId::new("run_recipient")),
                task_id: Some(TaskId::new("task_recipient")),
                content: "tool result payload".to_owned(),
                from_run_id: Some(RunId::new("run_orchestrator")),
                from_task_id: Some(TaskId::new("task_orchestrator")),
                deliver_at_ms: 0,
                sender: None,
                recipient: None,
                body: None,
                sent_at: None,
                delivery_status: None,
            }),
        )])
        .await
        .unwrap();

    let record = MailboxReadModel::get(&store, &msg_id)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(record.from_run_id, Some(RunId::new("run_orchestrator")));
    assert_eq!(record.from_task_id, Some(TaskId::new("task_orchestrator")));
    assert_eq!(record.task_id, Some(TaskId::new("task_recipient")));
}

// ── 7. list_by_task returns task-scoped messages ──────────────────────────────

#[tokio::test]
async fn list_by_task_returns_task_scoped_messages() {
    use cairn_domain::TaskId;

    let store = InMemoryStore::new();
    let task_id = TaskId::new("task_inbox");

    store
        .append(&[
            // Two messages addressed to task_inbox.
            evt(
                "e1",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: project(),
                    message_id: MailboxMessageId::new("msg_t1"),
                    run_id: None,
                    task_id: Some(task_id.clone()),
                    content: "step result A".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: project(),
                    message_id: MailboxMessageId::new("msg_t2"),
                    run_id: None,
                    task_id: Some(task_id.clone()),
                    content: "step result B".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                }),
            ),
            // Message addressed to a different task — must not appear.
            evt(
                "e3",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: project(),
                    message_id: MailboxMessageId::new("msg_other_task"),
                    run_id: None,
                    task_id: Some(TaskId::new("task_other")),
                    content: "for other task".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let messages = MailboxReadModel::list_by_task(&store, &task_id, 10, 0)
        .await
        .unwrap();

    assert_eq!(messages.len(), 2, "only messages for task_inbox");
    let contents: Vec<_> = messages.iter().map(|m| m.content.as_str()).collect();
    assert!(contents.contains(&"step result A"));
    assert!(contents.contains(&"step result B"));
    assert!(!contents.contains(&"for other task"));
}

// ── 8. Deferred delivery: list_pending returns due messages only ──────────────

#[tokio::test]
async fn list_pending_returns_only_due_messages() {
    let store = InMemoryStore::new();
    let now = now_ms();

    store
        .append(&[
            // Already due (in the past).
            evt(
                "e1",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: project(),
                    message_id: MailboxMessageId::new("msg_due"),
                    run_id: Some(RunId::new("run_deferred")),
                    task_id: None,
                    content: "I am due".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: now - 1_000,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None, // 1 second ago
                }),
            ),
            // Future delivery (not yet due).
            evt(
                "e2",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: project(),
                    message_id: MailboxMessageId::new("msg_future"),
                    run_id: Some(RunId::new("run_deferred")),
                    task_id: None,
                    content: "not yet".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: now + 60_000,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None, // 1 minute from now
                }),
            ),
            // Immediate delivery (deliver_at_ms == 0) — not included in pending.
            evt(
                "e3",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: project(),
                    message_id: MailboxMessageId::new("msg_immediate"),
                    run_id: Some(RunId::new("run_deferred")),
                    task_id: None,
                    content: "immediate".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                }),
            ),
        ])
        .await
        .unwrap();

    let pending = MailboxReadModel::list_pending(&store, now, 10)
        .await
        .unwrap();

    assert_eq!(pending.len(), 1, "only the past-due message is pending");
    assert_eq!(pending[0].message_id.as_str(), "msg_due");
    assert_eq!(pending[0].content, "I am due");
}

// ── 9. MAX_MESSAGE_CONTENT_LEN constant is exported ──────────────────────────

#[test]
fn max_message_content_len_constant_has_expected_value() {
    // Mirrors the Go implementation's maxMessageContentLen = 4000.
    assert_eq!(MAX_MESSAGE_CONTENT_LEN, 4000);
}

// ── 10. Multiple messages across projects are correctly isolated ──────────────

#[tokio::test]
async fn messages_are_isolated_across_projects() {
    let store = InMemoryStore::new();

    let proj_a = ProjectKey {
        tenant_id: TenantId::new("t_a"),
        workspace_id: WorkspaceId::new("w_a"),
        project_id: ProjectId::new("p_a"),
    };
    let proj_b = ProjectKey {
        tenant_id: TenantId::new("t_b"),
        workspace_id: WorkspaceId::new("w_b"),
        project_id: ProjectId::new("p_b"),
    };

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: proj_a.clone(),
                    message_id: MailboxMessageId::new("msg_proj_a"),
                    run_id: Some(RunId::new("run_shared_id")),
                    task_id: None,
                    content: "from project A".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                    project: proj_b.clone(),
                    message_id: MailboxMessageId::new("msg_proj_b"),
                    run_id: Some(RunId::new("run_shared_id")), // same run_id string, different project
                    task_id: None,
                    content: "from project B".to_owned(),
                    from_run_id: None,
                    from_task_id: None,
                    deliver_at_ms: 0,
                    sender: None,
                    recipient: None,
                    body: None,
                    sent_at: None,
                    delivery_status: None,
                }),
            ),
        ])
        .await
        .unwrap();

    // Direct get shows correct project on each message.
    let rec_a = MailboxReadModel::get(&store, &MailboxMessageId::new("msg_proj_a"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rec_a.project, proj_a);
    assert_eq!(rec_a.content, "from project A");

    let rec_b = MailboxReadModel::get(&store, &MailboxMessageId::new("msg_proj_b"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(rec_b.project, proj_b);
    assert_eq!(rec_b.content, "from project B");
}
