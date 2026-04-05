//! Integration test: inter-agent mailbox push delivery (GAP-004).

use std::sync::Arc;
use cairn_domain::{ProjectKey, RunId, TenantId, WorkspaceId};
use cairn_store::InMemoryStore;
use cairn_runtime::{
    MailboxDeliveryService, MailboxService, MailboxServiceImpl, MailboxWatcher,
};

fn test_project() -> ProjectKey {
    ProjectKey::new(TenantId::new("t1"), WorkspaceId::new("w1"), "proj1".to_owned())
}

#[tokio::test]
async fn mailbox_delivery_run_a_to_run_b() {
    let store = Arc::new(InMemoryStore::new());
    let mailbox_svc = Arc::new(MailboxServiceImpl::new(store.clone()));
    let delivery = MailboxDeliveryService::new(mailbox_svc.clone());

    let project = test_project();
    let run_a = RunId::new("run_a");
    let run_b = RunId::new("run_b");

    // Deliver a message from run A to run B
    let record = delivery
        .deliver(
            &project,
            Some(run_a.clone()),
            Some(run_b.clone()),
            None,
            "Hello from A to B".to_owned(),
        )
        .await
        .unwrap();

    // Message must be linked to run B
    assert_eq!(record.run_id, Some(run_b.clone()));
    assert_eq!(record.from_run_id, Some(run_a.clone()));
    assert_eq!(record.content, "Hello from A to B");
    assert_eq!(record.deliver_at_ms, 0); // immediate

    // Run B's inbox must contain the message
    let inbox = mailbox_svc
        .list_by_run(&run_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(inbox.len(), 1);
    assert_eq!(inbox[0].content, "Hello from A to B");
    assert_eq!(inbox[0].from_run_id, Some(run_a.clone()));
}

#[tokio::test]
async fn mailbox_delivery_multiple_messages_to_same_run() {
    let store = Arc::new(InMemoryStore::new());
    let mailbox_svc = Arc::new(MailboxServiceImpl::new(store.clone()));
    let delivery = MailboxDeliveryService::new(mailbox_svc.clone());

    let project = test_project();
    let sender = RunId::new("run_sender");
    let receiver = RunId::new("run_receiver");

    delivery.deliver(&project, Some(sender.clone()), Some(receiver.clone()), None, "msg 1".to_owned()).await.unwrap();
    delivery.deliver(&project, Some(sender.clone()), Some(receiver.clone()), None, "msg 2".to_owned()).await.unwrap();
    delivery.deliver(&project, Some(sender.clone()), Some(receiver.clone()), None, "msg 3".to_owned()).await.unwrap();

    let inbox = mailbox_svc.list_by_run(&receiver, 10, 0).await.unwrap();
    assert_eq!(inbox.len(), 3);
}

#[tokio::test]
async fn mailbox_watcher_finds_due_scheduled_messages() {
    let store = Arc::new(InMemoryStore::new());
    let mailbox_svc = Arc::new(MailboxServiceImpl::new(store.clone()));
    let delivery = MailboxDeliveryService::new(mailbox_svc.clone());
    let watcher = MailboxWatcher::new(store.clone());

    let project = test_project();
    let now_ms = 1_700_000_000_000u64;

    // Schedule a message due in the past (should be returned by watcher)
    delivery
        .schedule(
            &project,
            None,
            Some(RunId::new("run_c")),
            None,
            "scheduled msg".to_owned(),
            now_ms - 1000, // 1 second in the past
        )
        .await
        .unwrap();

    // Schedule a message due in the future (should NOT be returned)
    delivery
        .schedule(
            &project,
            None,
            Some(RunId::new("run_c")),
            None,
            "future msg".to_owned(),
            now_ms + 60_000, // 1 minute in the future
        )
        .await
        .unwrap();

    let due = watcher.due_messages(now_ms).await.unwrap();
    assert_eq!(due.len(), 1, "only the past-due message should be returned");
    assert_eq!(due[0].content, "scheduled msg");
}

#[tokio::test]
async fn mailbox_delivery_empty_inbox_before_send() {
    let store = Arc::new(InMemoryStore::new());
    let mailbox_svc = Arc::new(MailboxServiceImpl::new(store.clone()));

    let _project = test_project();
    let run_b = RunId::new("run_b_empty");

    let inbox = mailbox_svc.list_by_run(&run_b, 10, 0).await.unwrap();
    assert!(inbox.is_empty(), "inbox must be empty before any delivery");
}
