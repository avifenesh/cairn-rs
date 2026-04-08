//! Signal routing integration tests (RFC 012).
//!
//! Validates the signal system pipeline using `InMemoryStore` + `EventLog::append`.
//! Signals are external triggers (webhooks, schedules, tool outputs) that fan
//! out to subscribed runs via the routing layer.
//!
//! Read-model contract:
//!   SignalIngested            → SignalReadModel::get / list_by_project
//!   SignalSubscriptionCreated → SignalSubscriptionReadModel::get_subscription
//!                               / list_by_signal_type / list_by_project
//!   SignalRouted              → immutable audit record in the event log
//!                               (no projection state — verified via read_stream)
//!
//! Isolation contract:
//!   Signals and subscriptions created under project A are not visible
//!   when querying under project B.

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, SignalId,
    SignalIngested, SignalRouted, SignalSubscriptionCreated, TenantId, WorkspaceId,
};
use cairn_store::{
    projections::{SignalReadModel, SignalSubscriptionReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project(tenant: &str, workspace: &str, proj: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant),
        workspace_id: WorkspaceId::new(workspace),
        project_id: ProjectId::new(proj),
    }
}

fn default_project() -> ProjectKey {
    project("t_signal", "w_signal", "p_signal")
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

// ── 1. SignalIngested is stored and queryable ─────────────────────────────────

#[tokio::test]
async fn signal_ingested_is_stored_in_read_model() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let signal_id = SignalId::new("sig_001");

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::SignalIngested(SignalIngested {
                project: default_project(),
                signal_id: signal_id.clone(),
                source: "webhook:github".to_owned(),
                payload: serde_json::json!({ "event": "push", "ref": "refs/heads/main" }),
                timestamp_ms: ts,
            }),
        )])
        .await
        .unwrap();

    let record = SignalReadModel::get(&store, &signal_id)
        .await
        .unwrap()
        .expect("SignalRecord must exist after SignalIngested");

    assert_eq!(record.id, signal_id);
    assert_eq!(record.project, default_project());
    assert_eq!(record.source, "webhook:github");
    assert_eq!(record.payload["event"], "push");
    assert_eq!(record.payload["ref"], "refs/heads/main");
    assert_eq!(record.timestamp_ms, ts);
}

// ── 2. list_by_project returns all signals for the project ────────────────────

#[tokio::test]
async fn list_by_project_returns_all_ingested_signals() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::SignalIngested(SignalIngested {
                    project: default_project(),
                    signal_id: SignalId::new("sig_list_1"),
                    source: "schedule:daily".to_owned(),
                    payload: serde_json::json!({"tick": 1}),
                    timestamp_ms: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::SignalIngested(SignalIngested {
                    project: default_project(),
                    signal_id: SignalId::new("sig_list_2"),
                    source: "schedule:daily".to_owned(),
                    payload: serde_json::json!({"tick": 2}),
                    timestamp_ms: ts + 1,
                }),
            ),
        ])
        .await
        .unwrap();

    let signals = SignalReadModel::list_by_project(&store, &default_project(), 10, 0)
        .await
        .unwrap();
    assert_eq!(signals.len(), 2);
    // Sorted by timestamp_ms ascending.
    assert_eq!(signals[0].id.as_str(), "sig_list_1");
    assert_eq!(signals[1].id.as_str(), "sig_list_2");
}

// ── 3. SignalSubscriptionCreated is queryable ─────────────────────────────────

#[tokio::test]
async fn signal_subscription_created_is_queryable() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[evt(
            "e1",
            RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                project: default_project(),
                subscription_id: "sub_001".to_owned(),
                signal_kind: "github.push".to_owned(),
                target_run_id: None,
                target_mailbox_id: Some("mailbox_ops".to_owned()),
                filter_expression: Some("ref == 'refs/heads/main'".to_owned()),
                created_at_ms: ts,
            }),
        )])
        .await
        .unwrap();

    let sub = SignalSubscriptionReadModel::get_subscription(&store, "sub_001")
        .await
        .unwrap()
        .expect("subscription must exist after SignalSubscriptionCreated");

    assert_eq!(sub.subscription_id, "sub_001");
    assert_eq!(sub.signal_type, "github.push");
    assert_eq!(sub.target_mailbox_id.as_deref(), Some("mailbox_ops"));
    assert_eq!(
        sub.filter_expression.as_deref(),
        Some("ref == 'refs/heads/main'")
    );
    assert_eq!(sub.created_at_ms, ts);
}

// ── 4. list_by_signal_type finds all matching subscriptions ───────────────────

#[tokio::test]
async fn list_by_signal_type_returns_correct_subscriptions() {
    let store = InMemoryStore::new();
    let ts = now_ms();

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: default_project(),
                    subscription_id: "sub_push_1".to_owned(),
                    signal_kind: "github.push".to_owned(),
                    target_run_id: None,
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: default_project(),
                    subscription_id: "sub_push_2".to_owned(),
                    signal_kind: "github.push".to_owned(),
                    target_run_id: Some(cairn_domain::RunId::new("run_fanout")),
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts + 1,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: default_project(),
                    subscription_id: "sub_pr_1".to_owned(),
                    signal_kind: "github.pull_request".to_owned(),
                    target_run_id: None,
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts + 2,
                }),
            ),
        ])
        .await
        .unwrap();

    let push_subs = SignalSubscriptionReadModel::list_by_signal_type(&store, "github.push")
        .await
        .unwrap();
    assert_eq!(push_subs.len(), 2, "two push subscriptions");
    let ids: Vec<_> = push_subs
        .iter()
        .map(|s| s.subscription_id.as_str())
        .collect();
    assert!(ids.contains(&"sub_push_1"));
    assert!(ids.contains(&"sub_push_2"));

    let pr_subs = SignalSubscriptionReadModel::list_by_signal_type(&store, "github.pull_request")
        .await
        .unwrap();
    assert_eq!(pr_subs.len(), 1);
    assert_eq!(pr_subs[0].subscription_id, "sub_pr_1");

    // Unknown signal type returns empty.
    let none = SignalSubscriptionReadModel::list_by_signal_type(&store, "unknown.event")
        .await
        .unwrap();
    assert!(none.is_empty());
}

// ── 5. SignalRouted is stored as an audit record in the event log ─────────────

#[tokio::test]
async fn signal_routed_is_recorded_in_event_log() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let signal_id = SignalId::new("sig_route_1");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::SignalIngested(SignalIngested {
                    project: default_project(),
                    signal_id: signal_id.clone(),
                    source: "timer:hourly".to_owned(),
                    payload: serde_json::json!({"hour": 12}),
                    timestamp_ms: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: default_project(),
                    subscription_id: "sub_timer_1".to_owned(),
                    signal_kind: "timer.hourly".to_owned(),
                    target_run_id: Some(cairn_domain::RunId::new("run_timer")),
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::SignalRouted(SignalRouted {
                    project: default_project(),
                    signal_id: signal_id.clone(),
                    subscription_id: "sub_timer_1".to_owned(),
                    delivered_at_ms: ts + 5,
                }),
            ),
        ])
        .await
        .unwrap();

    // SignalRouted has no projection state — it lives in the event log as audit.
    let events = store.read_stream(None, 100).await.unwrap();
    assert_eq!(events.len(), 3);

    let routed_event = events
        .iter()
        .find(|e| matches!(&e.envelope.payload, RuntimeEvent::SignalRouted(_)));
    let routed_event = routed_event.expect("SignalRouted must be in the log");

    match &routed_event.envelope.payload {
        RuntimeEvent::SignalRouted(r) => {
            assert_eq!(r.signal_id, signal_id, "routing links correct signal");
            assert_eq!(
                r.subscription_id, "sub_timer_1",
                "routing links correct subscription"
            );
            assert_eq!(r.delivered_at_ms, ts + 5);
            assert_eq!(r.project, default_project());
        }
        _ => panic!("expected SignalRouted"),
    }
}

// ── 6. SignalRouted links signal_id to subscription_id ────────────────────────

#[tokio::test]
async fn signal_routed_links_signal_to_subscription() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let signal_id = SignalId::new("sig_link");

    // Ingest + two subscriptions + two routing deliveries (fan-out).
    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::SignalIngested(SignalIngested {
                    project: default_project(),
                    signal_id: signal_id.clone(),
                    source: "external:crm".to_owned(),
                    payload: serde_json::json!({"event_type": "deal.closed"}),
                    timestamp_ms: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: default_project(),
                    subscription_id: "sub_crm_a".to_owned(),
                    signal_kind: "crm.deal_closed".to_owned(),
                    target_run_id: Some(cairn_domain::RunId::new("run_notify")),
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts,
                }),
            ),
            evt(
                "e3",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: default_project(),
                    subscription_id: "sub_crm_b".to_owned(),
                    signal_kind: "crm.deal_closed".to_owned(),
                    target_run_id: Some(cairn_domain::RunId::new("run_analytics")),
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts + 1,
                }),
            ),
            // Fan-out: signal delivered to both subscribers.
            evt(
                "e4",
                RuntimeEvent::SignalRouted(SignalRouted {
                    project: default_project(),
                    signal_id: signal_id.clone(),
                    subscription_id: "sub_crm_a".to_owned(),
                    delivered_at_ms: ts + 10,
                }),
            ),
            evt(
                "e5",
                RuntimeEvent::SignalRouted(SignalRouted {
                    project: default_project(),
                    signal_id: signal_id.clone(),
                    subscription_id: "sub_crm_b".to_owned(),
                    delivered_at_ms: ts + 11,
                }),
            ),
        ])
        .await
        .unwrap();

    // Both routing events are in the log, each linking the same signal to a different subscription.
    let events = store.read_stream(None, 100).await.unwrap();
    let routed: Vec<_> = events
        .iter()
        .filter_map(|e| match &e.envelope.payload {
            RuntimeEvent::SignalRouted(r) => Some(r),
            _ => None,
        })
        .collect();

    assert_eq!(routed.len(), 2, "fan-out produces two routing records");
    assert!(routed.iter().all(|r| r.signal_id == signal_id));

    let sub_ids: Vec<_> = routed.iter().map(|r| r.subscription_id.as_str()).collect();
    assert!(sub_ids.contains(&"sub_crm_a"));
    assert!(sub_ids.contains(&"sub_crm_b"));

    // Signal read model still has the original signal.
    let sig_record = SignalReadModel::get(&store, &signal_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sig_record.source, "external:crm");
    assert_eq!(sig_record.payload["event_type"], "deal.closed");

    // Both subscriptions are still queryable.
    let crm_subs = SignalSubscriptionReadModel::list_by_signal_type(&store, "crm.deal_closed")
        .await
        .unwrap();
    assert_eq!(crm_subs.len(), 2);
}

// ── 7. Signals for different projects are isolated ────────────────────────────

#[tokio::test]
async fn signals_are_isolated_by_project() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let proj_a = project("tenant_a", "ws_a", "proj_a");
    let proj_b = project("tenant_b", "ws_b", "proj_b");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::SignalIngested(SignalIngested {
                    project: proj_a.clone(),
                    signal_id: SignalId::new("sig_a"),
                    source: "webhook:a".to_owned(),
                    payload: serde_json::json!({"tenant": "a"}),
                    timestamp_ms: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::SignalIngested(SignalIngested {
                    project: proj_b.clone(),
                    signal_id: SignalId::new("sig_b"),
                    source: "webhook:b".to_owned(),
                    payload: serde_json::json!({"tenant": "b"}),
                    timestamp_ms: ts + 1,
                }),
            ),
        ])
        .await
        .unwrap();

    // Project A only sees its own signal.
    let signals_a = SignalReadModel::list_by_project(&store, &proj_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(signals_a.len(), 1);
    assert_eq!(signals_a[0].id.as_str(), "sig_a");
    assert_eq!(signals_a[0].payload["tenant"], "a");

    // Project B only sees its own signal.
    let signals_b = SignalReadModel::list_by_project(&store, &proj_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(signals_b.len(), 1);
    assert_eq!(signals_b[0].id.as_str(), "sig_b");

    // Direct get by ID still returns the right record regardless of caller project.
    let sig_a = SignalReadModel::get(&store, &SignalId::new("sig_a"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(sig_a.project, proj_a);
}

// ── 8. Subscriptions for different projects are isolated ──────────────────────

#[tokio::test]
async fn subscriptions_are_isolated_by_project() {
    let store = InMemoryStore::new();
    let ts = now_ms();
    let proj_a = project("ta", "wa", "pa");
    let proj_b = project("tb", "wb", "pb");

    store
        .append(&[
            evt(
                "e1",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: proj_a.clone(),
                    subscription_id: "sub_a".to_owned(),
                    signal_kind: "deploy.completed".to_owned(),
                    target_run_id: None,
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts,
                }),
            ),
            evt(
                "e2",
                RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
                    project: proj_b.clone(),
                    subscription_id: "sub_b".to_owned(),
                    signal_kind: "deploy.completed".to_owned(),
                    target_run_id: None,
                    target_mailbox_id: None,
                    filter_expression: None,
                    created_at_ms: ts + 1,
                }),
            ),
        ])
        .await
        .unwrap();

    // Same signal_kind, but scoped queries return only the matching project's subscription.
    let subs_a = SignalSubscriptionReadModel::list_by_project(&store, &proj_a, 10, 0)
        .await
        .unwrap();
    assert_eq!(subs_a.len(), 1);
    assert_eq!(subs_a[0].subscription_id, "sub_a");

    let subs_b = SignalSubscriptionReadModel::list_by_project(&store, &proj_b, 10, 0)
        .await
        .unwrap();
    assert_eq!(subs_b.len(), 1);
    assert_eq!(subs_b[0].subscription_id, "sub_b");

    // list_by_signal_type is NOT project-scoped (it's a routing lookup), so both appear.
    let all_deploy = SignalSubscriptionReadModel::list_by_signal_type(&store, "deploy.completed")
        .await
        .unwrap();
    assert_eq!(
        all_deploy.len(),
        2,
        "type lookup is cross-project for routing"
    );
}
