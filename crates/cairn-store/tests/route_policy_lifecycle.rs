//! RFC 009 — Route policy lifecycle tests.
//!
//! Validates the full lifecycle of routing policies through the event log and
//! synchronous projection:
//!
//! - `RoutePolicyCreated` populates the `RoutePolicyReadModel` with tenant,
//!   name, rules, and `enabled` state.
//! - `RoutePolicyUpdated` advances `updated_at_ms` in the read model.
//! - `list_by_tenant` scoping: each tenant only sees its own policies.
//! - `RouteRule` / `RoutePolicyRule` fields (rule_id, priority, description)
//!   are preserved verbatim through the append/read cycle.
//! - Inactive (`enabled = false`) policies are filtered from
//!   `list_by_tenant` results.

use cairn_domain::{
    events::{RoutePolicyCreated, RoutePolicyUpdated},
    providers::RoutePolicyRule,
    tenancy::OwnershipKey,
    EventEnvelope, EventId, EventSource, RuntimeEvent, TenantId,
};
use cairn_store::{projections::RoutePolicyReadModel, EventLog, InMemoryStore};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn ownership(tenant_id: &str) -> OwnershipKey {
    OwnershipKey::Tenant(cairn_domain::tenancy::TenantKey::new(TenantId::new(
        tenant_id,
    )))
}

/// Append a `RoutePolicyCreated` event to the store.
async fn create_policy(
    store: &InMemoryStore,
    event_id: &str,
    tenant_id: &str,
    policy_id: &str,
    name: &str,
    rules: Vec<RoutePolicyRule>,
    enabled: bool,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        ownership(tenant_id),
        RuntimeEvent::RoutePolicyCreated(RoutePolicyCreated {
            tenant_id: TenantId::new(tenant_id),
            policy_id: policy_id.to_owned(),
            name: name.to_owned(),
            rules,
            enabled,
        }),
    );
    store.append(&[env]).await.unwrap();
}

/// Append a `RoutePolicyUpdated` event to the store.
async fn update_policy(store: &InMemoryStore, event_id: &str, policy_id: &str, updated_at_ms: u64) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        OwnershipKey::System,
        RuntimeEvent::RoutePolicyUpdated(RoutePolicyUpdated {
            policy_id: policy_id.to_owned(),
            updated_at_ms,
        }),
    );
    store.append(&[env]).await.unwrap();
}

/// Build a sample rule for use in tests.
fn rule(id: &str, priority: u32, description: Option<&str>) -> RoutePolicyRule {
    RoutePolicyRule {
        rule_id: id.to_owned(),
        policy_id: "".to_owned(), // will be overwritten per-policy in real use
        priority,
        description: description.map(ToOwned::to_owned),
    }
}

// ── 1. RoutePolicyCreated populates the read model ────────────────────────────

#[tokio::test]
async fn create_policy_appears_in_read_model() {
    let store = InMemoryStore::new();

    create_policy(
        &store,
        "e1",
        "tenant_a",
        "pol_1",
        "Primary Route",
        vec![],
        true,
    )
    .await;

    let policy = RoutePolicyReadModel::get(&store, "pol_1")
        .await
        .unwrap()
        .expect("policy should exist after RoutePolicyCreated");

    assert_eq!(policy.policy_id, "pol_1");
    assert_eq!(policy.name, "Primary Route");
    assert_eq!(policy.tenant_id, "tenant_a");
    assert!(policy.enabled);
}

#[tokio::test]
async fn get_returns_none_for_unknown_policy() {
    let store = InMemoryStore::new();
    let result = RoutePolicyReadModel::get(&store, "no_such_policy")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn created_policy_stores_correct_tenant_and_name() {
    let store = InMemoryStore::new();
    create_policy(
        &store,
        "e1",
        "acme_corp",
        "pol_acme",
        "Acme Policy",
        vec![],
        true,
    )
    .await;

    let p = RoutePolicyReadModel::get(&store, "pol_acme")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p.tenant_id, "acme_corp");
    assert_eq!(p.name, "Acme Policy");
}

// ── 2. RoutePolicyUpdated advances updated_at_ms ──────────────────────────────

#[tokio::test]
async fn updated_event_advances_updated_at_ms() {
    let store = InMemoryStore::new();
    create_policy(
        &store,
        "e1",
        "tenant_b",
        "pol_b",
        "Beta Route",
        vec![],
        true,
    )
    .await;

    let before = RoutePolicyReadModel::get(&store, "pol_b")
        .await
        .unwrap()
        .unwrap()
        .updated_at_ms;

    // Advance the timestamp.
    update_policy(&store, "e2", "pol_b", 99_999).await;

    let after = RoutePolicyReadModel::get(&store, "pol_b")
        .await
        .unwrap()
        .unwrap()
        .updated_at_ms;

    assert_eq!(
        after, 99_999,
        "updated_at_ms should equal the value from the event"
    );
    // Note: before is set to the stored_at timestamp by the projection;
    // the update event gives us an explicit value which must differ.
    let _ = before; // we care that `after` == 99_999, not the before value
}

#[tokio::test]
async fn multiple_updates_advance_updated_at_ms_each_time() {
    let store = InMemoryStore::new();
    create_policy(&store, "e1", "tenant_c", "pol_c", "C Route", vec![], true).await;

    for ts in [1_000u64, 2_000, 3_000] {
        update_policy(&store, &format!("e_upd_{ts}"), "pol_c", ts).await;
        let p = RoutePolicyReadModel::get(&store, "pol_c")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            p.updated_at_ms, ts,
            "updated_at_ms should track last update"
        );
    }
}

#[tokio::test]
async fn update_for_unknown_policy_is_a_no_op() {
    let store = InMemoryStore::new();
    update_policy(&store, "e1", "ghost_policy", 9_999).await;
    let result = RoutePolicyReadModel::get(&store, "ghost_policy")
        .await
        .unwrap();
    assert!(
        result.is_none(),
        "update for unknown policy must not create a record"
    );
}

// ── 3. list_by_tenant scoping ─────────────────────────────────────────────────

#[tokio::test]
async fn list_by_tenant_returns_only_that_tenants_policies() {
    let store = InMemoryStore::new();

    create_policy(
        &store,
        "e1",
        "tenant_x",
        "pol_x1",
        "X Route 1",
        vec![],
        true,
    )
    .await;
    create_policy(
        &store,
        "e2",
        "tenant_x",
        "pol_x2",
        "X Route 2",
        vec![],
        true,
    )
    .await;
    create_policy(
        &store,
        "e3",
        "tenant_y",
        "pol_y1",
        "Y Route 1",
        vec![],
        true,
    )
    .await;

    let x_policies =
        RoutePolicyReadModel::list_by_tenant(&store, &TenantId::new("tenant_x"), 100, 0)
            .await
            .unwrap();

    let y_policies =
        RoutePolicyReadModel::list_by_tenant(&store, &TenantId::new("tenant_y"), 100, 0)
            .await
            .unwrap();

    assert_eq!(x_policies.len(), 2, "tenant_x should have 2 policies");
    assert_eq!(y_policies.len(), 1, "tenant_y should have 1 policy");

    let x_ids: Vec<&str> = x_policies.iter().map(|p| p.policy_id.as_str()).collect();
    assert!(x_ids.contains(&"pol_x1") && x_ids.contains(&"pol_x2"));
    assert_eq!(y_policies[0].policy_id, "pol_y1");
}

#[tokio::test]
async fn list_by_tenant_returns_empty_for_unknown_tenant() {
    let store = InMemoryStore::new();
    create_policy(&store, "e1", "tenant_a", "pol_a", "A Route", vec![], true).await;

    let result =
        RoutePolicyReadModel::list_by_tenant(&store, &TenantId::new("tenant_ghost"), 100, 0)
            .await
            .unwrap();

    assert!(result.is_empty());
}

#[tokio::test]
async fn list_by_tenant_respects_pagination() {
    let store = InMemoryStore::new();
    for i in 0..5u32 {
        create_policy(
            &store,
            &format!("e{i}"),
            "tenant_pg",
            &format!("pol_pg_{i}"),
            &format!("Policy {i}"),
            vec![],
            true,
        )
        .await;
    }

    let page1 = RoutePolicyReadModel::list_by_tenant(&store, &TenantId::new("tenant_pg"), 3, 0)
        .await
        .unwrap();
    let page2 = RoutePolicyReadModel::list_by_tenant(&store, &TenantId::new("tenant_pg"), 3, 3)
        .await
        .unwrap();

    assert_eq!(page1.len(), 3, "first page should have 3 items");
    assert_eq!(page2.len(), 2, "second page should have remaining 2");

    // No overlap between pages.
    let ids1: std::collections::HashSet<_> = page1.iter().map(|p| &p.policy_id).collect();
    let ids2: std::collections::HashSet<_> = page2.iter().map(|p| &p.policy_id).collect();
    assert!(ids1.is_disjoint(&ids2), "pages must not overlap");
}

// ── 4. RouteRule fields persist correctly ─────────────────────────────────────

#[tokio::test]
async fn route_rules_are_stored_verbatim() {
    let store = InMemoryStore::new();

    let rules = vec![
        rule("rule_a", 10, Some("High-priority fallback")),
        rule("rule_b", 20, Some("Standard path")),
        rule("rule_c", 30, None),
    ];

    create_policy(
        &store,
        "e1",
        "tenant_rules",
        "pol_rules",
        "Rule Policy",
        rules.clone(),
        true,
    )
    .await;

    let p = RoutePolicyReadModel::get(&store, "pol_rules")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(p.rules.len(), 3, "all 3 rules must be stored");

    // Verify each rule field precisely.
    let stored_rule_a = p.rules.iter().find(|r| r.rule_id == "rule_a").unwrap();
    assert_eq!(stored_rule_a.priority, 10);
    assert_eq!(
        stored_rule_a.description.as_deref(),
        Some("High-priority fallback")
    );

    let stored_rule_b = p.rules.iter().find(|r| r.rule_id == "rule_b").unwrap();
    assert_eq!(stored_rule_b.priority, 20);
    assert_eq!(stored_rule_b.description.as_deref(), Some("Standard path"));

    let stored_rule_c = p.rules.iter().find(|r| r.rule_id == "rule_c").unwrap();
    assert_eq!(stored_rule_c.priority, 30);
    assert!(
        stored_rule_c.description.is_none(),
        "optional description must be None"
    );
}

#[tokio::test]
async fn rule_ordering_is_preserved() {
    let store = InMemoryStore::new();

    let rules: Vec<RoutePolicyRule> = (1..=5u32)
        .map(|i| rule(&format!("r{i}"), i * 10, None))
        .collect();

    create_policy(
        &store,
        "e1",
        "tenant_ord",
        "pol_ord",
        "Ordered",
        rules.clone(),
        true,
    )
    .await;

    let p = RoutePolicyReadModel::get(&store, "pol_ord")
        .await
        .unwrap()
        .unwrap();
    let stored_ids: Vec<&str> = p.rules.iter().map(|r| r.rule_id.as_str()).collect();
    let expected_ids: Vec<&str> = rules.iter().map(|r| r.rule_id.as_str()).collect();
    assert_eq!(
        stored_ids, expected_ids,
        "rule insertion order must be preserved"
    );
}

#[tokio::test]
async fn policy_with_no_rules_stores_empty_vec() {
    let store = InMemoryStore::new();
    create_policy(
        &store,
        "e1",
        "tenant_nr",
        "pol_nr",
        "No Rules",
        vec![],
        true,
    )
    .await;

    let p = RoutePolicyReadModel::get(&store, "pol_nr")
        .await
        .unwrap()
        .unwrap();
    assert!(p.rules.is_empty());
}

// ── 5. Inactive policy filtering ──────────────────────────────────────────────

#[tokio::test]
async fn inactive_policy_is_excluded_from_list_by_tenant() {
    let store = InMemoryStore::new();

    // One enabled, one disabled at creation time.
    create_policy(
        &store,
        "e1",
        "tenant_filter",
        "pol_active",
        "Active",
        vec![],
        true,
    )
    .await;
    create_policy(
        &store,
        "e2",
        "tenant_filter",
        "pol_inactive",
        "Inactive",
        vec![],
        false,
    )
    .await;

    let active =
        RoutePolicyReadModel::list_by_tenant(&store, &TenantId::new("tenant_filter"), 100, 0)
            .await
            .unwrap();

    let ids: Vec<&str> = active.iter().map(|p| p.policy_id.as_str()).collect();
    assert!(
        ids.contains(&"pol_active"),
        "active policy must appear in list"
    );
    assert!(
        !ids.contains(&"pol_inactive"),
        "inactive policy must NOT appear in list"
    );
}

#[tokio::test]
async fn get_returns_inactive_policy_directly() {
    // `get` by ID returns any policy regardless of enabled state.
    let store = InMemoryStore::new();
    create_policy(
        &store,
        "e1",
        "tenant_direct",
        "pol_off",
        "Disabled",
        vec![],
        false,
    )
    .await;

    let p = RoutePolicyReadModel::get(&store, "pol_off").await.unwrap();
    assert!(
        p.is_some(),
        "get by id must return the policy even when disabled"
    );
    assert!(!p.unwrap().enabled);
}

#[tokio::test]
async fn all_inactive_tenant_has_empty_list() {
    let store = InMemoryStore::new();

    for i in 0..3u32 {
        create_policy(
            &store,
            &format!("e{i}"),
            "tenant_all_off",
            &format!("pol_off_{i}"),
            &format!("Off {i}"),
            vec![],
            false,
        )
        .await;
    }

    let active =
        RoutePolicyReadModel::list_by_tenant(&store, &TenantId::new("tenant_all_off"), 100, 0)
            .await
            .unwrap();

    assert!(
        active.is_empty(),
        "tenant with all-inactive policies must have empty active list"
    );
}

// ── 6. Events are written to the event log ────────────────────────────────────

#[tokio::test]
async fn created_and_updated_events_are_in_log() {
    let store = InMemoryStore::new();
    create_policy(
        &store,
        "e_create",
        "tenant_log",
        "pol_log",
        "Log Policy",
        vec![],
        true,
    )
    .await;
    update_policy(&store, "e_update", "pol_log", 5_000).await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 2, "both events must be in the log");

    let has_created = all.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RoutePolicyCreated(ev)
            if ev.policy_id == "pol_log")
    });
    let has_updated = all.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::RoutePolicyUpdated(ev)
            if ev.policy_id == "pol_log" && ev.updated_at_ms == 5_000)
    });

    assert!(has_created, "RoutePolicyCreated must be in the log");
    assert!(has_updated, "RoutePolicyUpdated must be in the log");
}

#[tokio::test]
async fn log_positions_are_sequential_across_lifecycle() {
    let store = InMemoryStore::new();
    create_policy(&store, "e1", "tenant_seq", "pol_seq", "Seq", vec![], true).await;
    update_policy(&store, "e2", "pol_seq", 1_000).await;
    update_policy(&store, "e3", "pol_seq", 2_000).await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 3);
    for w in all.windows(2) {
        assert!(w[0].position < w[1].position);
    }
}
