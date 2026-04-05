//! RFC 002 mailbox messaging end-to-end integration test.
//!
//! Validates the full inter-agent mailbox pipeline:
//!   (1) append a message with sender, recipient, body fields
//!   (2) retrieve the message and verify all fields
//!   (3) list messages by run
//!   (4) mark a message as delivered (re-append with delivery_status)
//!   (5) verify the delivery_status changed
//!   (6) list_pending returns deferred messages due for delivery
//!   (7) send task-to-task message via MailboxService::send

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, MailboxMessageAppended, MailboxMessageId,
    ProjectKey, RunId, RuntimeEvent, SessionId, TaskId,
};
use cairn_runtime::{MailboxService, MailboxServiceImpl, RunService, RunServiceImpl,
    SessionService, SessionServiceImpl};
use cairn_store::projections::MailboxReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_mbox", "ws_mbox", "proj_mbox")
}

fn services() -> (
    Arc<InMemoryStore>,
    SessionServiceImpl<InMemoryStore>,
    RunServiceImpl<InMemoryStore>,
    MailboxServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    (
        store.clone(),
        SessionServiceImpl::new(store.clone()),
        RunServiceImpl::new(store.clone()),
        MailboxServiceImpl::new(store),
    )
}

/// Helper: append a MailboxMessageAppended event directly to the store,
/// exposing all RFC 002 fields that MailboxService::append() does not.
async fn append_rich_message(
    store: &Arc<InMemoryStore>,
    message_id: &str,
    run_id: Option<RunId>,
    sender: Option<&str>,
    recipient: Option<&str>,
    body: Option<&str>,
    sent_at: Option<u64>,
    delivery_status: Option<&str>,
) {
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_{message_id}")),
            EventSource::Runtime,
            RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                project: project(),
                message_id: MailboxMessageId::new(message_id),
                run_id,
                task_id: None,
                from_task_id: None,
                from_run_id: None,
                content: body.unwrap_or("").to_owned(),
                deliver_at_ms: 0,
                sender: sender.map(str::to_owned),
                recipient: recipient.map(str::to_owned),
                body: body.map(str::to_owned),
                sent_at,
                delivery_status: delivery_status.map(str::to_owned),
            }),
        )])
        .await
        .unwrap();
}

// ── (1) + (2) Append with RFC 002 fields, retrieve and verify ────────────

#[tokio::test]
async fn append_and_retrieve_message_with_all_fields() {
    let (store, sessions, runs, mailbox) = services();

    sessions
        .create(&project(), SessionId::new("sess_mbox_1"))
        .await
        .unwrap();
    runs.start(
        &project(),
        &SessionId::new("sess_mbox_1"),
        RunId::new("run_mbox_1"),
        None,
    )
    .await
    .unwrap();

    let run_id = RunId::new("run_mbox_1");
    append_rich_message(
        &store,
        "msg_full_1",
        Some(run_id.clone()),
        Some("agent:orchestrator"),
        Some("agent:worker-1"),
        Some("Please summarise the document at /docs/spec.md"),
        Some(1_700_000_000_000),
        Some("pending"),
    )
    .await;

    let record = mailbox
        .get(&MailboxMessageId::new("msg_full_1"))
        .await
        .unwrap()
        .expect("message must exist after append");

    assert_eq!(record.message_id, MailboxMessageId::new("msg_full_1"));
    assert_eq!(record.run_id, Some(run_id));
    assert_eq!(record.sender.as_deref(), Some("agent:orchestrator"));
    assert_eq!(record.recipient.as_deref(), Some("agent:worker-1"));
    assert_eq!(
        record.body.as_deref(),
        Some("Please summarise the document at /docs/spec.md")
    );
    assert_eq!(record.sent_at, Some(1_700_000_000_000));
    assert_eq!(record.delivery_status.as_deref(), Some("pending"));
}

// ── (3) List messages by run ──────────────────────────────────────────────

#[tokio::test]
async fn list_by_run_returns_all_messages_for_run() {
    let (store, sessions, runs, mailbox) = services();

    sessions
        .create(&project(), SessionId::new("sess_list"))
        .await
        .unwrap();
    runs.start(
        &project(),
        &SessionId::new("sess_list"),
        RunId::new("run_list"),
        None,
    )
    .await
    .unwrap();

    let run_id = RunId::new("run_list");

    for i in 1u32..=3 {
        append_rich_message(
            &store,
            &format!("msg_list_{i}"),
            Some(run_id.clone()),
            Some("agent:sender"),
            Some("agent:receiver"),
            Some(&format!("Message {i}")),
            Some(1_000 * i as u64),
            None,
        )
        .await;
    }

    let messages = mailbox.list_by_run(&run_id, 10, 0).await.unwrap();
    assert_eq!(messages.len(), 3, "all 3 messages must be listed for the run");

    // All messages belong to the expected run.
    assert!(messages.iter().all(|m| m.run_id == Some(run_id.clone())));
}

// ── (4) + (5) Mark as delivered — delivery_status changes ─────────────────

#[tokio::test]
async fn mark_message_delivered_updates_delivery_status() {
    let (store, sessions, runs, mailbox) = services();

    sessions
        .create(&project(), SessionId::new("sess_deliver"))
        .await
        .unwrap();
    runs.start(
        &project(),
        &SessionId::new("sess_deliver"),
        RunId::new("run_deliver"),
        None,
    )
    .await
    .unwrap();

    let run_id = RunId::new("run_deliver");

    // Step 1: append with delivery_status "pending".
    append_rich_message(
        &store,
        "msg_deliver",
        Some(run_id.clone()),
        Some("agent:A"),
        Some("agent:B"),
        Some("Task assignment"),
        Some(2_000_000),
        Some("pending"),
    )
    .await;

    let before = mailbox
        .get(&MailboxMessageId::new("msg_deliver"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(before.delivery_status.as_deref(), Some("pending"));

    // Step 2: re-append same message ID with delivery_status "delivered".
    // The projection upserts by message_id, so this overwrites the record.
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_msg_deliver_ack"),
            EventSource::Runtime,
            RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                project: project(),
                message_id: MailboxMessageId::new("msg_deliver"),
                run_id: Some(run_id),
                task_id: None,
                from_task_id: None,
                from_run_id: None,
                content: "Task assignment".to_owned(),
                deliver_at_ms: 0,
                sender: Some("agent:A".to_owned()),
                recipient: Some("agent:B".to_owned()),
                body: Some("Task assignment".to_owned()),
                sent_at: Some(2_000_000),
                delivery_status: Some("delivered".to_owned()),
            }),
        )])
        .await
        .unwrap();

    let after = mailbox
        .get(&MailboxMessageId::new("msg_deliver"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        after.delivery_status.as_deref(),
        Some("delivered"),
        "delivery_status must change to 'delivered' after the update event"
    );
}

// ── (6) list_pending returns deferred messages ────────────────────────────

#[tokio::test]
async fn deferred_message_appears_in_list_pending() {
    let (store, sessions, runs, _) = services();

    sessions
        .create(&project(), SessionId::new("sess_pending"))
        .await
        .unwrap();
    runs.start(
        &project(),
        &SessionId::new("sess_pending"),
        RunId::new("run_pending"),
        None,
    )
    .await
    .unwrap();

    let deliver_at = 9_999_999_999_000_u64; // far future
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_deferred"),
            EventSource::Runtime,
            RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
                project: project(),
                message_id: MailboxMessageId::new("msg_deferred"),
                run_id: Some(RunId::new("run_pending")),
                task_id: None,
                from_task_id: None,
                from_run_id: None,
                content: "deferred payload".to_owned(),
                deliver_at_ms: deliver_at,
                sender: None,
                recipient: None,
                body: None,
                sent_at: None,
                delivery_status: Some("scheduled".to_owned()),
            }),
        )])
        .await
        .unwrap();

    // Before the delivery time: list_pending at now=1 should return nothing.
    let pending_now = MailboxReadModel::list_pending(store.as_ref(), 1, 10)
        .await
        .unwrap();
    assert!(
        pending_now.is_empty(),
        "message not yet due must not appear in list_pending"
    );

    // After the delivery time: list_pending at now=far_future+1 should return it.
    let pending_future = MailboxReadModel::list_pending(
        store.as_ref(),
        deliver_at + 1,
        10,
    )
    .await
    .unwrap();
    assert_eq!(
        pending_future.len(),
        1,
        "deferred message must appear in list_pending once its time arrives"
    );
    assert_eq!(pending_future[0].message_id, MailboxMessageId::new("msg_deferred"));
}

// ── (7) Task-to-task send via MailboxService::send ────────────────────────

#[tokio::test]
async fn send_task_to_task_creates_mailbox_record() {
    let (store, _, _, mailbox) = services();

    let from_task = TaskId::new("task_sender");
    let to_task = TaskId::new("task_receiver");
    let message = "Here is the result of my analysis.";

    let record = mailbox
        .send(&project(), from_task.clone(), to_task.clone(), message.to_owned())
        .await
        .unwrap();

    assert_eq!(record.task_id, Some(to_task.clone()));
    assert_eq!(record.from_task_id, Some(from_task));
    assert!(
        record.content.contains("Here is the result"),
        "message content must be preserved"
    );

    // Verify persisted in store.
    let fetched = mailbox.get(&record.message_id).await.unwrap();
    assert!(fetched.is_some(), "sent message must be retrievable by ID");

    // Retrievable via list_by_task.
    let inbox = mailbox.list_by_task(&to_task, 10, 0).await.unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].message_id, record.message_id);

    // Event logged for the message.
    let events = store.read_stream(None, 20).await.unwrap();
    let logged = events.iter().any(|e| {
        matches!(
            &e.envelope.payload,
            RuntimeEvent::MailboxMessageAppended(ev)
                if ev.message_id == record.message_id
        )
    });
    assert!(logged, "MailboxMessageAppended must be in the event log");
}
