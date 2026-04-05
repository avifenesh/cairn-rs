//! Notification preferences system end-to-end integration tests.
//!
//! Tests the preference lifecycle:
//!   1. Set notification preferences for an operator (email enabled, slack disabled)
//!   2. Retrieve preferences and verify channels/event_types
//!   3. Update preferences (enable slack by overwriting with new channel list)
//!   4. Verify updated preferences reflect the new channel set
//!   5. Preferences are scoped per-operator — two operators in the same tenant
//!      each hold independent preferences
//!
//! Additional coverage:
//!   - notify_if_applicable dispatches only when the event_type matches
//!   - Multiple event types subscribed by one operator
//!   - Non-matching events produce no notifications
//!   - list_sent returns sent records scoped to tenant

use std::sync::Arc;

use cairn_domain::notification_prefs::NotificationChannel;
use cairn_domain::TenantId;
use cairn_runtime::notification_prefs::NotificationService;
use cairn_runtime::services::NotificationServiceImpl;
use cairn_store::InMemoryStore;

fn tenant() -> TenantId {
    TenantId::new("t_notif_e2e")
}

fn email_channel() -> NotificationChannel {
    NotificationChannel {
        kind: "email".to_owned(),
        target: "alice@example.com".to_owned(),
    }
}

fn slack_channel() -> NotificationChannel {
    NotificationChannel {
        kind: "slack".to_owned(),
        target: "#ops-alerts".to_owned(),
    }
}

// ── Tests 1–4: set preferences, retrieve, update, verify ─────────────────────

/// Set preferences with email only, verify retrieval, then enable slack and
/// confirm the updated channel set is persisted.
#[tokio::test]
async fn set_retrieve_update_preferences() {
    let store = Arc::new(InMemoryStore::new());
    let svc = NotificationServiceImpl::new(store);

    // ── (1) Set preferences: email enabled, slack not included ────────────
    svc.set_preferences(
        tenant(),
        "op_alice".to_owned(),
        vec!["run.failed".to_owned(), "approval.required".to_owned()],
        vec![email_channel()],
    )
    .await
    .unwrap();

    // ── (2) Retrieve and verify ────────────────────────────────────────────
    let prefs = svc
        .get_preferences(&tenant(), "op_alice")
        .await
        .unwrap()
        .expect("preferences must be retrievable after set");

    assert_eq!(prefs.operator_id, "op_alice");
    assert_eq!(prefs.tenant_id, tenant());
    assert_eq!(
        prefs.event_types,
        vec!["run.failed", "approval.required"],
        "event_types must round-trip exactly"
    );
    assert_eq!(prefs.channels.len(), 1, "only email channel should be present");
    assert_eq!(prefs.channels[0].kind, "email");
    assert_eq!(prefs.channels[0].target, "alice@example.com");

    // Verify slack is NOT present.
    let has_slack = prefs.channels.iter().any(|c| c.kind == "slack");
    assert!(!has_slack, "slack must not be present before it is added");

    // ── (3) Update preferences: enable slack alongside email ──────────────
    // set_preferences is idempotent-by-overwrite — calling it again replaces the record.
    svc.set_preferences(
        tenant(),
        "op_alice".to_owned(),
        vec!["run.failed".to_owned(), "approval.required".to_owned()],
        vec![email_channel(), slack_channel()],
    )
    .await
    .unwrap();

    // ── (4) Verify updated preferences ────────────────────────────────────
    let updated = svc
        .get_preferences(&tenant(), "op_alice")
        .await
        .unwrap()
        .expect("preferences must still be retrievable after update");

    assert_eq!(updated.channels.len(), 2, "both email and slack must be present after update");

    let channel_kinds: Vec<&str> = updated.channels.iter().map(|c| c.kind.as_str()).collect();
    assert!(
        channel_kinds.contains(&"email"),
        "email channel must still be present after update"
    );
    assert!(
        channel_kinds.contains(&"slack"),
        "slack channel must be present after update"
    );

    // Event types must be unchanged.
    assert_eq!(updated.event_types.len(), 2);
    assert!(updated.event_types.contains(&"run.failed".to_owned()));
    assert!(updated.event_types.contains(&"approval.required".to_owned()));
}

// ── Test 5: preferences are scoped per-operator ───────────────────────────────

/// Two operators in the same tenant must hold fully independent preferences.
/// Setting one operator's preferences must not affect the other's.
#[tokio::test]
async fn preferences_scoped_per_operator() {
    let store = Arc::new(InMemoryStore::new());
    let svc = NotificationServiceImpl::new(store);

    // ── (5a) Set preferences for operator Alice ────────────────────────────
    svc.set_preferences(
        tenant(),
        "op_alice".to_owned(),
        vec!["run.failed".to_owned()],
        vec![email_channel()],
    )
    .await
    .unwrap();

    // ── (5b) Set different preferences for operator Bob ────────────────────
    svc.set_preferences(
        tenant(),
        "op_bob".to_owned(),
        vec!["run.completed".to_owned(), "approval.required".to_owned()],
        vec![slack_channel()],
    )
    .await
    .unwrap();

    // Alice's preferences must be unchanged.
    let alice_prefs = svc
        .get_preferences(&tenant(), "op_alice")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(alice_prefs.event_types, vec!["run.failed"]);
    assert_eq!(alice_prefs.channels.len(), 1);
    assert_eq!(alice_prefs.channels[0].kind, "email");

    // Bob's preferences must be independent.
    let bob_prefs = svc
        .get_preferences(&tenant(), "op_bob")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bob_prefs.event_types,
        vec!["run.completed", "approval.required"]
    );
    assert_eq!(bob_prefs.channels.len(), 1);
    assert_eq!(bob_prefs.channels[0].kind, "slack");

    // Updating Alice must not affect Bob.
    svc.set_preferences(
        tenant(),
        "op_alice".to_owned(),
        vec!["run.failed".to_owned(), "run.completed".to_owned()],
        vec![email_channel(), slack_channel()],
    )
    .await
    .unwrap();

    let bob_after = svc
        .get_preferences(&tenant(), "op_bob")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bob_after.event_types,
        vec!["run.completed", "approval.required"],
        "Bob's preferences must be unaffected by updating Alice"
    );
}

// ── Unknown operator returns None ─────────────────────────────────────────────

#[tokio::test]
async fn get_preferences_for_unknown_operator_returns_none() {
    let store = Arc::new(InMemoryStore::new());
    let svc = NotificationServiceImpl::new(store);

    let result = svc.get_preferences(&tenant(), "op_ghost").await.unwrap();
    assert!(result.is_none(), "get_preferences must return None for an operator with no preferences");
}

// ── notify_if_applicable dispatches only matching event types ─────────────────

/// notify_if_applicable must dispatch to channels only when the event_type is
/// in the operator's subscription list; non-matching events must be silently ignored.
#[tokio::test]
async fn notify_dispatches_only_matching_event_type() {
    let store = Arc::new(InMemoryStore::new());
    let svc = NotificationServiceImpl::new(store.clone());

    svc.set_preferences(
        tenant(),
        "op_carol".to_owned(),
        vec!["run.failed".to_owned()],
        vec![slack_channel()],
    )
    .await
    .unwrap();

    // Matching event → 1 notification dispatched.
    let sent = svc
        .notify_if_applicable(
            &tenant(),
            "run.failed",
            serde_json::json!({ "run_id": "run_x" }),
        )
        .await
        .unwrap();
    assert_eq!(sent.len(), 1, "matching event must produce one notification");
    assert_eq!(sent[0].event_type, "run.failed");
    assert_eq!(sent[0].operator_id, "op_carol");
    assert_eq!(sent[0].channel_kind, "slack");

    // Non-matching event → no notifications.
    let none = svc
        .notify_if_applicable(
            &tenant(),
            "run.completed",
            serde_json::json!({ "run_id": "run_y" }),
        )
        .await
        .unwrap();
    assert!(
        none.is_empty(),
        "non-matching event must produce no notifications; got: {} records",
        none.len()
    );
}

// ── Multiple operators receive independent notifications ──────────────────────

/// When multiple operators subscribe to the same event type, each must receive
/// a notification on their configured channels independently.
#[tokio::test]
async fn multiple_operators_notified_independently() {
    let store = Arc::new(InMemoryStore::new());
    let svc = NotificationServiceImpl::new(store.clone());

    svc.set_preferences(
        tenant(),
        "op_dave".to_owned(),
        vec!["approval.required".to_owned()],
        vec![email_channel()],
    )
    .await
    .unwrap();

    svc.set_preferences(
        tenant(),
        "op_eve".to_owned(),
        vec!["approval.required".to_owned()],
        vec![slack_channel()],
    )
    .await
    .unwrap();

    let sent = svc
        .notify_if_applicable(
            &tenant(),
            "approval.required",
            serde_json::json!({ "approval_id": "ap_1" }),
        )
        .await
        .unwrap();

    assert_eq!(sent.len(), 2, "both operators must receive a notification");

    let ops: Vec<&str> = sent.iter().map(|r| r.operator_id.as_str()).collect();
    assert!(ops.contains(&"op_dave"), "op_dave must receive a notification");
    assert!(ops.contains(&"op_eve"), "op_eve must receive a notification");

    let dave_rec = sent.iter().find(|r| r.operator_id == "op_dave").unwrap();
    let eve_rec  = sent.iter().find(|r| r.operator_id == "op_eve").unwrap();
    assert_eq!(dave_rec.channel_kind, "email");
    assert_eq!(eve_rec.channel_kind,  "slack");
}

// ── list_sent returns records scoped to tenant ────────────────────────────────

#[tokio::test]
async fn list_sent_returns_tenant_scoped_records() {
    let store = Arc::new(InMemoryStore::new());
    let svc = NotificationServiceImpl::new(store.clone());

    let other_tenant = TenantId::new("t_other_notif");

    svc.set_preferences(
        tenant(),
        "op_main".to_owned(),
        vec!["run.failed".to_owned()],
        vec![slack_channel()],
    )
    .await
    .unwrap();

    svc.set_preferences(
        other_tenant.clone(),
        "op_other".to_owned(),
        vec!["run.failed".to_owned()],
        vec![email_channel()],
    )
    .await
    .unwrap();

    svc.notify_if_applicable(&tenant(),       "run.failed", serde_json::json!({})).await.unwrap();
    svc.notify_if_applicable(&other_tenant,   "run.failed", serde_json::json!({})).await.unwrap();

    let main_sent  = svc.list_sent(&tenant(),       0).await.unwrap();
    let other_sent = svc.list_sent(&other_tenant,   0).await.unwrap();

    assert_eq!(main_sent.len(),  1, "main tenant must see only its 1 notification");
    assert_eq!(other_sent.len(), 1, "other tenant must see only its 1 notification");
    assert_eq!(main_sent[0].channel_kind,  "slack");
    assert_eq!(other_sent[0].channel_kind, "email");
}
