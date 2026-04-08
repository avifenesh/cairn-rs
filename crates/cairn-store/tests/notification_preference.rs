//! RFC 002 notification preference lifecycle integration tests.
//!
//! Validates the operator notification pipeline through InMemoryStore:
//! - NotificationPreferenceSet stores preferences (tenant + operator scoped).
//! - NotificationSent creates an audit record with delivery status.
//! - Per-tenant preference scoping: each tenant sees only its own preferences.
//! - Multiple channels (email, slack, webhook) coexist on one preference set.
//! - Failed deliveries are separable from successful ones.

use std::sync::Arc;

use cairn_domain::events::{NotificationPreferenceSet, NotificationSent};
use cairn_domain::notification_prefs::NotificationChannel;
use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, TenantId};
use cairn_store::{projections::NotificationReadModel, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant(n: &str) -> TenantId {
    TenantId::new(format!("tenant_notif_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::System, payload.into())
}

fn channel(kind: &str, target: &str) -> NotificationChannel {
    NotificationChannel {
        kind: kind.to_owned(),
        target: target.to_owned(),
    }
}

fn pref_event(
    tenant_n: &str,
    operator: &str,
    event_types: &[&str],
    channels: Vec<NotificationChannel>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_pref_{tenant_n}_{operator}"),
        RuntimeEvent::NotificationPreferenceSet(NotificationPreferenceSet {
            tenant_id: tenant(tenant_n),
            operator_id: operator.to_owned(),
            event_types: event_types.iter().map(|s| s.to_string()).collect(),
            channels,
            set_at_ms: ts,
        }),
    )
}

fn sent_event(
    record_id: &str,
    tenant_n: &str,
    operator: &str,
    event_type: &str,
    channel_kind: &str,
    channel_target: &str,
    delivered: bool,
    delivery_error: Option<&str>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_sent_{record_id}"),
        RuntimeEvent::NotificationSent(NotificationSent {
            record_id: record_id.to_owned(),
            tenant_id: tenant(tenant_n),
            operator_id: operator.to_owned(),
            event_type: event_type.to_owned(),
            channel_kind: channel_kind.to_owned(),
            channel_target: channel_target.to_owned(),
            payload: serde_json::json!({ "event_type": event_type, "record_id": record_id }),
            sent_at_ms: ts,
            delivered,
            delivery_error: delivery_error.map(str::to_owned),
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): NotificationPreferenceSet stores preference;
/// get_preferences returns the record with event_types and channels preserved.
#[tokio::test]
async fn preference_set_is_stored_and_readable() {
    let store = Arc::new(InMemoryStore::new());

    // (1) Append the preference event.
    store
        .append(&[pref_event(
            "a",
            "operator_alice",
            &["task_failed", "run_stalled", "approval_required"],
            vec![channel("email", "alice@example.com")],
            1_000,
        )])
        .await
        .unwrap();

    // (2) Verify it is stored and readable.
    let pref =
        NotificationReadModel::get_preferences(store.as_ref(), &tenant("a"), "operator_alice")
            .await
            .unwrap()
            .expect("preference must exist after NotificationPreferenceSet");

    assert_eq!(pref.tenant_id, tenant("a"));
    assert_eq!(pref.operator_id, "operator_alice");
    assert_eq!(pref.event_types.len(), 3);
    assert!(pref.event_types.contains(&"task_failed".to_owned()));
    assert!(pref.event_types.contains(&"approval_required".to_owned()));
    assert_eq!(pref.channels.len(), 1);
    assert_eq!(pref.channels[0].kind, "email");
    assert_eq!(pref.channels[0].target, "alice@example.com");
}

/// Setting a preference twice for the same operator replaces the previous one.
#[tokio::test]
async fn preference_update_replaces_previous() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[pref_event(
            "update",
            "operator_bob",
            &["run_failed"],
            vec![channel("slack", "#alerts")],
            1_000,
        )])
        .await
        .unwrap();

    // Update with new event types and channel.
    store
        .append(&[pref_event(
            "update",
            "operator_bob",
            &["run_failed", "task_expired", "cost_alert"],
            vec![
                channel("slack", "#ops"),
                channel("email", "bob@example.com"),
            ],
            2_000,
        )])
        .await
        .unwrap();

    let pref =
        NotificationReadModel::get_preferences(store.as_ref(), &tenant("update"), "operator_bob")
            .await
            .unwrap()
            .unwrap();

    // Latest preference wins.
    assert_eq!(
        pref.event_types.len(),
        3,
        "updated event types must replace old ones"
    );
    assert!(pref.event_types.contains(&"cost_alert".to_owned()));
    assert_eq!(
        pref.channels.len(),
        2,
        "updated channels must replace old channels"
    );
}

/// (3) + (4): NotificationSent creates an audit record with all fields preserved.
#[tokio::test]
async fn notification_sent_is_recorded() {
    let store = Arc::new(InMemoryStore::new());

    // Set preference first.
    store
        .append(&[pref_event(
            "b",
            "operator_carol",
            &["run_stalled"],
            vec![channel("email", "carol@example.com")],
            1_000,
        )])
        .await
        .unwrap();

    // (3) Append NotificationSent.
    store
        .append(&[sent_event(
            "rec_001",
            "b",
            "operator_carol",
            "run_stalled",
            "email",
            "carol@example.com",
            true,
            None,
            5_000,
        )])
        .await
        .unwrap();

    // (4) Verify the notification record.
    let records = NotificationReadModel::list_sent_notifications(store.as_ref(), &tenant("b"), 0)
        .await
        .unwrap();

    assert_eq!(records.len(), 1, "one notification record must exist");
    let rec = &records[0];
    assert_eq!(rec.record_id, "rec_001");
    assert_eq!(rec.operator_id, "operator_carol");
    assert_eq!(rec.event_type, "run_stalled");
    assert_eq!(rec.channel_kind, "email");
    assert_eq!(rec.channel_target, "carol@example.com");
    assert!(rec.delivered, "notification must be marked delivered=true");
    assert!(rec.delivery_error.is_none());
    assert_eq!(rec.sent_at_ms, 5_000);
    assert!(
        rec.payload.get("event_type").is_some(),
        "payload must be preserved"
    );
}

/// (5): Per-tenant preference scoping.
/// Tenant A's preferences are not visible to tenant B and vice versa.
#[tokio::test]
async fn per_tenant_preference_scoping() {
    let store = Arc::new(InMemoryStore::new());

    // Tenant A: 2 operators.
    store
        .append(&[
            pref_event(
                "ta",
                "op_a1",
                &["task_failed"],
                vec![channel("email", "a1@example.com")],
                1_000,
            ),
            pref_event(
                "ta",
                "op_a2",
                &["run_stalled"],
                vec![channel("slack", "#eng")],
                2_000,
            ),
        ])
        .await
        .unwrap();

    // Tenant B: 1 operator.
    store
        .append(&[pref_event(
            "tb",
            "op_b1",
            &["cost_alert"],
            vec![channel("email", "b1@example.com")],
            3_000,
        )])
        .await
        .unwrap();

    // Tenant A sees its own 2 preferences.
    let a_prefs = NotificationReadModel::list_preferences_by_tenant(store.as_ref(), &tenant("ta"))
        .await
        .unwrap();
    assert_eq!(a_prefs.len(), 2, "tenant A must have 2 preferences");
    assert!(
        a_prefs.iter().all(|p| p.tenant_id == tenant("ta")),
        "all tenant A preferences must be scoped to tenant A"
    );

    // Tenant B sees only its own 1 preference.
    let b_prefs = NotificationReadModel::list_preferences_by_tenant(store.as_ref(), &tenant("tb"))
        .await
        .unwrap();
    assert_eq!(b_prefs.len(), 1, "tenant B must have 1 preference");
    assert_eq!(b_prefs[0].operator_id, "op_b1");

    // Cross-tenant: tenant A preferences don't leak into tenant B listing.
    assert!(
        !b_prefs.iter().any(|p| p.tenant_id == tenant("ta")),
        "tenant B listing must not include tenant A preferences"
    );

    // get_preferences for tenant A's operator returns None when queried for tenant B.
    let cross = NotificationReadModel::get_preferences(store.as_ref(), &tenant("tb"), "op_a1")
        .await
        .unwrap();
    assert!(
        cross.is_none(),
        "tenant B must not see tenant A's op_a1 preference"
    );
}

/// (6): Multiple channels (email, slack, webhook) coexist on one preference set.
#[tokio::test]
async fn multiple_channels_coexist() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[pref_event(
            "multi",
            "operator_dave",
            &["approval_required", "run_failed", "cost_alert"],
            vec![
                channel("email", "dave@example.com"),
                channel("slack", "#on-call"),
                channel("webhook", "https://hooks.example.com/cairn"),
            ],
            1_000,
        )])
        .await
        .unwrap();

    let pref =
        NotificationReadModel::get_preferences(store.as_ref(), &tenant("multi"), "operator_dave")
            .await
            .unwrap()
            .unwrap();

    assert_eq!(pref.channels.len(), 3, "all 3 channels must coexist");

    let kinds: Vec<&str> = pref.channels.iter().map(|c| c.kind.as_str()).collect();
    assert!(kinds.contains(&"email"), "email channel must be present");
    assert!(kinds.contains(&"slack"), "slack channel must be present");
    assert!(
        kinds.contains(&"webhook"),
        "webhook channel must be present"
    );

    // Targets are preserved per channel.
    let email_ch = pref.channels.iter().find(|c| c.kind == "email").unwrap();
    assert_eq!(email_ch.target, "dave@example.com");

    let slack_ch = pref.channels.iter().find(|c| c.kind == "slack").unwrap();
    assert_eq!(slack_ch.target, "#on-call");

    let wh_ch = pref.channels.iter().find(|c| c.kind == "webhook").unwrap();
    assert_eq!(wh_ch.target, "https://hooks.example.com/cairn");

    // Notifications sent via each channel land in the audit log.
    for (kind, target) in [
        ("email", "dave@example.com"),
        ("slack", "#on-call"),
        ("webhook", "https://hooks.example.com/cairn"),
    ] {
        store
            .append(&[sent_event(
                &format!("rec_multi_{kind}"),
                "multi",
                "operator_dave",
                "approval_required",
                kind,
                target,
                true,
                None,
                2_000,
            )])
            .await
            .unwrap();
    }

    let records =
        NotificationReadModel::list_sent_notifications(store.as_ref(), &tenant("multi"), 0)
            .await
            .unwrap();
    assert_eq!(records.len(), 3, "one sent record per channel");

    let sent_kinds: Vec<&str> = records.iter().map(|r| r.channel_kind.as_str()).collect();
    assert!(sent_kinds.contains(&"email"));
    assert!(sent_kinds.contains(&"slack"));
    assert!(sent_kinds.contains(&"webhook"));
}

/// Failed deliveries are separable from successes via list_failed_notifications.
#[tokio::test]
async fn failed_deliveries_are_separable_from_successes() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            pref_event(
                "fail",
                "op_fail",
                &["run_failed"],
                vec![channel("webhook", "https://bad.example.com")],
                1_000,
            ),
            sent_event(
                "rec_ok",
                "fail",
                "op_fail",
                "run_failed",
                "email",
                "op@example.com",
                true,
                None,
                2_000,
            ),
            sent_event(
                "rec_fail",
                "fail",
                "op_fail",
                "run_failed",
                "webhook",
                "https://bad.example.com",
                false,
                Some("connection refused"),
                3_000,
            ),
        ])
        .await
        .unwrap();

    // list_sent_notifications returns both.
    let all = NotificationReadModel::list_sent_notifications(store.as_ref(), &tenant("fail"), 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);

    // list_failed_notifications returns only the failed one.
    let failed = NotificationReadModel::list_failed_notifications(store.as_ref(), &tenant("fail"))
        .await
        .unwrap();
    assert_eq!(failed.len(), 1, "only 1 failed notification");
    assert_eq!(failed[0].record_id, "rec_fail");
    assert!(!failed[0].delivered);
    assert_eq!(
        failed[0].delivery_error.as_deref(),
        Some("connection refused")
    );

    // since_ms filter: records sent before ts=2500 → only rec_ok (ts=2000).
    let since_2500 =
        NotificationReadModel::list_sent_notifications(store.as_ref(), &tenant("fail"), 2_500)
            .await
            .unwrap();
    assert_eq!(since_2500.len(), 1, "only rec_fail has ts >= 2500");
    assert_eq!(since_2500[0].record_id, "rec_fail");
}
