//! Signal routing end-to-end integration test (RFC 012).
//!
//! Validates the full signal routing pipeline:
//!   (1) register a signal subscription targeting a run's mailbox
//!   (2) ingest a signal matching the subscription's kind
//!   (3) route the signal — verify mailbox message is created
//!   (4) filter expression: non-matching signal is NOT routed
//!   (5) multi-subscription fan-out: one signal routes to two subscribers
//!   (6) missing run: subscription to a non-existent run is rejected
//!   (7) project isolation: signal from another project is not routed

use std::sync::Arc;
use std::time::Duration;

use cairn_domain::{MailboxMessageId, ProjectKey, RunId, RuntimeEvent, SessionId, SignalId};
use cairn_runtime::services::{SignalRouterServiceImpl, SignalServiceImpl};
use cairn_runtime::{
    RunService, RunServiceImpl, SessionService, SessionServiceImpl, SignalRouterService,
    SignalService,
};
use cairn_store::projections::MailboxReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_sig", "ws_sig", "proj_sig")
}

async fn setup() -> (
    Arc<InMemoryStore>,
    SessionServiceImpl<InMemoryStore>,
    RunServiceImpl<InMemoryStore>,
    SignalServiceImpl<InMemoryStore>,
    SignalRouterServiceImpl<InMemoryStore>,
) {
    let store = Arc::new(InMemoryStore::new());
    let sessions = SessionServiceImpl::new(store.clone());
    let runs = RunServiceImpl::new(store.clone());
    let signals = SignalServiceImpl::new(store.clone());
    let router = SignalRouterServiceImpl::new(store.clone());
    (store, sessions, runs, signals, router)
}

/// Seed a session + pending run in the store.
async fn seed_run(
    sessions: &SessionServiceImpl<InMemoryStore>,
    runs: &RunServiceImpl<InMemoryStore>,
    session_id: &str,
    run_id: &str,
) {
    sessions
        .create(&project(), SessionId::new(session_id))
        .await
        .unwrap();
    runs.start(
        &project(),
        &SessionId::new(session_id),
        RunId::new(run_id),
        None,
    )
    .await
    .unwrap();
}

// ── (1) Register subscription — subscription event persisted ─────────────

#[tokio::test]
async fn subscribe_creates_subscription_event() {
    let (store, sessions, runs, _, router) = setup().await;
    seed_run(&sessions, &runs, "sess_sub_1", "run_sub_1").await;

    let sub = router
        .subscribe(
            project(),
            "alert".to_owned(),
            Some(RunId::new("run_sub_1")),
            Some("mbox_sub_1".to_owned()),
            None,
        )
        .await
        .unwrap();

    assert!(!sub.subscription_id.is_empty());
    assert_eq!(sub.signal_kind, "alert");

    let events = store.read_stream(None, 20).await.unwrap();
    let created = events.iter().any(|e| {
        matches!(
            &e.envelope.payload,
            RuntimeEvent::SignalSubscriptionCreated(ev)
                if ev.subscription_id == sub.subscription_id
        )
    });
    assert!(created, "SignalSubscriptionCreated event must be persisted");
}

// ── (2) Ingest + (3) Route — mailbox message created ─────────────────────

#[tokio::test]
async fn ingest_and_route_creates_mailbox_message() {
    let (store, sessions, runs, signals, router) = setup().await;
    seed_run(&sessions, &runs, "sess_route_1", "run_route_1").await;

    let sub = router
        .subscribe(
            project(),
            "heartbeat".to_owned(),
            Some(RunId::new("run_route_1")),
            Some("mbox_route_1".to_owned()),
            None,
        )
        .await
        .unwrap();

    signals
        .ingest(
            &project(),
            SignalId::new("sig_route_1"),
            "heartbeat".to_owned(),
            serde_json::json!({"tick": 1}),
            1_000,
        )
        .await
        .unwrap();

    let result = router
        .route_signal(&SignalId::new("sig_route_1"))
        .await
        .unwrap();

    assert_eq!(
        result.routed_count, 1,
        "one subscription should receive the signal"
    );
    assert_eq!(
        result.mailbox_message_ids,
        vec![MailboxMessageId::new("mbox_route_1")]
    );

    let msg = MailboxReadModel::get(store.as_ref(), &MailboxMessageId::new("mbox_route_1"))
        .await
        .unwrap();
    assert!(
        msg.is_some(),
        "mailbox message must be created after routing"
    );

    // SignalRouted event must be in the store.
    let events = store.read_stream(None, 30).await.unwrap();
    let routed = events.iter().any(|e| {
        matches!(
            &e.envelope.payload,
            RuntimeEvent::SignalRouted(ev)
                if ev.signal_id == SignalId::new("sig_route_1")
                    && ev.subscription_id == sub.subscription_id
        )
    });
    assert!(routed, "SignalRouted event must be persisted");
}

// ── (4) Filter expression: non-matching signal NOT routed ─────────────────

#[tokio::test]
async fn filter_expression_blocks_non_matching_signal() {
    let (_, sessions, runs, signals, router) = setup().await;
    seed_run(&sessions, &runs, "sess_filter", "run_filter").await;

    router
        .subscribe(
            project(),
            "metric".to_owned(),
            Some(RunId::new("run_filter")),
            Some("mbox_filter".to_owned()),
            Some("critical".to_owned()), // only route if payload contains "critical"
        )
        .await
        .unwrap();

    signals
        .ingest(
            &project(),
            SignalId::new("sig_filter_no_match"),
            "metric".to_owned(),
            serde_json::json!({"level": "info"}), // does NOT contain "critical"
            2_000,
        )
        .await
        .unwrap();

    let result = router
        .route_signal(&SignalId::new("sig_filter_no_match"))
        .await
        .unwrap();

    assert_eq!(
        result.routed_count, 0,
        "non-matching signal must not be routed"
    );
}

#[tokio::test]
async fn filter_expression_passes_matching_signal() {
    let (store, sessions, runs, signals, router) = setup().await;
    seed_run(&sessions, &runs, "sess_filter_ok", "run_filter_ok").await;

    router
        .subscribe(
            project(),
            "metric".to_owned(),
            Some(RunId::new("run_filter_ok")),
            Some("mbox_filter_ok".to_owned()),
            Some("critical".to_owned()),
        )
        .await
        .unwrap();

    signals
        .ingest(
            &project(),
            SignalId::new("sig_filter_match"),
            "metric".to_owned(),
            serde_json::json!({"level": "critical", "value": 99}),
            3_000,
        )
        .await
        .unwrap();

    let result = router
        .route_signal(&SignalId::new("sig_filter_match"))
        .await
        .unwrap();

    assert_eq!(result.routed_count, 1, "matching signal must be routed");
    let msg = MailboxReadModel::get(store.as_ref(), &MailboxMessageId::new("mbox_filter_ok"))
        .await
        .unwrap();
    assert!(
        msg.is_some(),
        "mailbox message must be created for matching signal"
    );
}

// ── (5) Multi-subscription fan-out ───────────────────────────────────────

#[tokio::test]
async fn signal_fans_out_to_multiple_subscriptions() {
    let (_, sessions, runs, signals, router) = setup().await;
    seed_run(&sessions, &runs, "sess_fan_1", "run_fan_1").await;
    seed_run(&sessions, &runs, "sess_fan_2", "run_fan_2").await;

    // Two subscribers for the same signal kind.
    // Sleep 2ms between subscribes: the router keys subscription_id on now_ms()
    // so same-millisecond creates would collide in the in-memory projection.
    router
        .subscribe(
            project(),
            "trigger".to_owned(),
            Some(RunId::new("run_fan_1")),
            Some("mbox_fan_1".to_owned()),
            None,
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    router
        .subscribe(
            project(),
            "trigger".to_owned(),
            Some(RunId::new("run_fan_2")),
            Some("mbox_fan_2".to_owned()),
            None,
        )
        .await
        .unwrap();

    signals
        .ingest(
            &project(),
            SignalId::new("sig_fan"),
            "trigger".to_owned(),
            serde_json::json!({"action": "start"}),
            4_000,
        )
        .await
        .unwrap();

    let result = router
        .route_signal(&SignalId::new("sig_fan"))
        .await
        .unwrap();

    assert_eq!(
        result.routed_count, 2,
        "signal must fan out to both subscribers"
    );
    assert_eq!(result.mailbox_message_ids.len(), 2);
}

// ── (6) Subscription to non-existent run is rejected ─────────────────────

#[tokio::test]
async fn subscribe_to_nonexistent_run_returns_error() {
    let (_, _, _, _, router) = setup().await;

    let result = router
        .subscribe(
            project(),
            "alert".to_owned(),
            Some(RunId::new("run_does_not_exist")),
            Some("mbox_err".to_owned()),
            None,
        )
        .await;

    assert!(
        result.is_err(),
        "subscribing to a non-existent run must fail"
    );
}

// ── (7) Project isolation ─────────────────────────────────────────────────

#[tokio::test]
async fn signal_from_other_project_is_not_routed_to_subscription() {
    let (_, sessions, runs, signals, router) = setup().await;
    seed_run(&sessions, &runs, "sess_iso", "run_iso").await;

    // Subscription in the default project.
    router
        .subscribe(
            project(),
            "ping".to_owned(),
            Some(RunId::new("run_iso")),
            Some("mbox_iso".to_owned()),
            None,
        )
        .await
        .unwrap();

    // Signal ingested from a DIFFERENT project.
    let other_project = ProjectKey::new("t_other", "ws_other", "proj_other");
    signals
        .ingest(
            &other_project,
            SignalId::new("sig_other_project"),
            "ping".to_owned(),
            serde_json::json!({"from": "other"}),
            5_000,
        )
        .await
        .unwrap();

    let result = router
        .route_signal(&SignalId::new("sig_other_project"))
        .await
        .unwrap();

    assert_eq!(
        result.routed_count, 0,
        "signal from another project must not route to subscriptions in a different project"
    );
}
