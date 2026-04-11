//! RFC 009 — route policy lifecycle end-to-end integration tests.
//!
//! Tests the full route policy arc:
//!   1. Create a route policy with rules
//!   2. Verify it is retrievable and enabled
//!   3. Update the policy (toggle enabled flag via direct event — the
//!      RoutePolicyService trait has no update method; this tests the
//!      projection behaviour directly)
//!   4. Verify the updated state reflects enabled=false
//!   5. List policies — disabled policies are excluded from list_by_tenant
//!
//! Additional coverage:
//!   - Multiple policies tracked independently per tenant
//!   - Tenant isolation: list_by_tenant scopes to the correct tenant
//!   - get() on unknown ID returns None
//!   - Creating for a non-existent tenant returns NotFound

use std::sync::Arc;
use std::time::Duration;

use cairn_domain::providers::RoutePolicyRule;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, RoutePolicyCreated, RuntimeEvent, TenantId,
};
use cairn_runtime::error::RuntimeError;
use cairn_runtime::route_policies::RoutePolicyService;
use cairn_runtime::services::{RoutePolicyServiceImpl, TenantServiceImpl};
use cairn_runtime::tenants::TenantService;
use cairn_store::projections::RoutePolicyReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn tenant() -> TenantId {
    TenantId::new("t_rfc009")
}

fn make_rule(rule_id: &str, policy_id: &str, priority: u32) -> RoutePolicyRule {
    RoutePolicyRule {
        rule_id: rule_id.to_owned(),
        policy_id: policy_id.to_owned(),
        priority,
        description: Some(format!("Rule {rule_id} at priority {priority}")),
    }
}

async fn setup() -> (Arc<InMemoryStore>, RoutePolicyServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    TenantServiceImpl::new(store.clone())
        .create(tenant(), "RFC 009 Tenant".to_owned())
        .await
        .unwrap();
    let svc = RoutePolicyServiceImpl::new(store.clone());
    (store, svc)
}

// ── Test 1 + 2: create policy with rules, verify retrievable and enabled ──────

/// RFC 009 §4: a route policy must be created with all rules intact,
/// enabled by default, and immediately retrievable by ID.
#[tokio::test]
async fn create_route_policy_with_rules_and_verify() {
    let (_store, svc) = setup().await;

    // ── (1) Create a route policy with two rules ──────────────────────────
    let policy = svc
        .create(
            tenant(),
            "primary-routing-policy".to_owned(),
            vec![
                make_rule("rule_prefer_openai", "tbd", 10),
                make_rule("rule_fallback_bedrock", "tbd", 20),
            ],
        )
        .await
        .unwrap();

    // ── (2) Verify it is retrievable and enabled ───────────────────────────
    assert!(
        !policy.policy_id.is_empty(),
        "policy must have a non-empty ID"
    );
    assert_eq!(policy.name, "primary-routing-policy");
    assert!(
        policy.enabled,
        "RFC 009: newly created policy must be enabled by default"
    );
    assert_eq!(
        policy.rules.len(),
        2,
        "both rules must be persisted on the policy"
    );
    assert_eq!(policy.tenant_id, tenant().as_str());

    // Rule fields must survive the round-trip.
    let r1 = &policy.rules[0];
    let r2 = &policy.rules[1];
    assert_eq!(r1.priority, 10, "rule priorities must be preserved");
    assert_eq!(r2.priority, 20);
    assert!(
        r1.description.is_some(),
        "rule descriptions must be preserved"
    );

    // get() must return the identical record.
    let fetched = svc
        .get(&policy.policy_id)
        .await
        .unwrap()
        .expect("policy must be retrievable by ID");
    assert_eq!(fetched.policy_id, policy.policy_id);
    assert_eq!(fetched.name, policy.name);
    assert!(fetched.enabled);
    assert_eq!(fetched.rules.len(), 2);
}

// ── Test 3 + 4: update (toggle enabled flag), verify updated state ────────────

/// RFC 009 §4: toggling the enabled flag must be reflected in the read model.
///
/// Note: `RoutePolicyService` has no update method — the service gap is
/// documented here.  The test exercises the projection behaviour directly
/// by appending a `RoutePolicyCreated` event with `enabled: false` against
/// the same `policy_id` (the in-memory projection uses `.insert()` so it
/// overwrites the record, simulating what a fully implemented update would do).
#[tokio::test]
async fn update_policy_toggle_enabled_flag() {
    let (store, svc) = setup().await;

    // Create the policy initially enabled.
    let policy = svc
        .create(
            tenant(),
            "toggle-test-policy".to_owned(),
            vec![make_rule("rule_main", "tbd", 5)],
        )
        .await
        .unwrap();

    assert!(policy.enabled, "pre-condition: policy must start enabled");

    // ── (3) Update: toggle enabled=false ─────────────────────────────────
    // The RoutePolicyService trait has no update method (RFC 009 service gap).
    // Simulate the update by appending the domain event directly, which is
    // what a RoutePolicyService::update() implementation would do.
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_policy_disable"),
            EventSource::Runtime,
            RuntimeEvent::RoutePolicyCreated(RoutePolicyCreated {
                tenant_id: tenant(),
                policy_id: policy.policy_id.clone(),
                name: policy.name.clone(),
                rules: policy.rules.clone(),
                enabled: false, // toggled off
            }),
        )])
        .await
        .unwrap();

    // ── (4) Verify the updated state ──────────────────────────────────────
    let updated = svc
        .get(&policy.policy_id)
        .await
        .unwrap()
        .expect("policy must still be retrievable after disabling");

    assert!(
        !updated.enabled,
        "RFC 009: policy must be disabled after enabled=false update"
    );
    assert_eq!(
        updated.policy_id, policy.policy_id,
        "policy_id must be stable across updates"
    );
    assert_eq!(
        updated.rules.len(),
        1,
        "rules must be preserved after update"
    );
}

// ── Test 5: list policies — disabled ones are excluded ────────────────────────

/// RFC 009 §4: list_by_tenant must return only enabled policies for a tenant.
/// Disabled policies must be excluded from the listing.
#[tokio::test]
async fn list_policies_excludes_disabled() {
    let (store, svc) = setup().await;

    // Create three policies — sleep between each to ensure unique ms timestamps
    // (policy_id is generated as "route_policy_{now_ms()}").
    let p1 = svc
        .create(tenant(), "policy-alpha".to_owned(), vec![])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    let p2 = svc
        .create(tenant(), "policy-beta".to_owned(), vec![])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    let p3 = svc
        .create(tenant(), "policy-gamma".to_owned(), vec![])
        .await
        .unwrap();

    // ── (5) List: all three are enabled → list returns 3 ─────────────────
    let all = RoutePolicyReadModel::list_by_tenant(store.as_ref(), &tenant(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        all.len(),
        3,
        "all three enabled policies must appear in the list"
    );

    // Disable p2 by overwriting with enabled=false.
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_disable_p2"),
            EventSource::Runtime,
            RuntimeEvent::RoutePolicyCreated(RoutePolicyCreated {
                tenant_id: tenant(),
                policy_id: p2.policy_id.clone(),
                name: p2.name.clone(),
                rules: p2.rules.clone(),
                enabled: false,
            }),
        )])
        .await
        .unwrap();

    // List should now return only 2 (p1 and p3).
    let after_disable = RoutePolicyReadModel::list_by_tenant(store.as_ref(), &tenant(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        after_disable.len(),
        2,
        "disabled policy must be excluded from list_by_tenant"
    );

    let ids: Vec<&str> = after_disable.iter().map(|p| p.policy_id.as_str()).collect();
    assert!(
        ids.contains(&p1.policy_id.as_str()),
        "p1 (enabled) must be in the list"
    );
    assert!(
        ids.contains(&p3.policy_id.as_str()),
        "p3 (enabled) must be in the list"
    );
    assert!(
        !ids.contains(&p2.policy_id.as_str()),
        "p2 (disabled) must NOT be in the list"
    );

    // Pagination: limit=1 must return 1 result.
    let page = RoutePolicyReadModel::list_by_tenant(store.as_ref(), &tenant(), 1, 0)
        .await
        .unwrap();
    assert_eq!(page.len(), 1, "pagination limit must be respected");
}

// ── Tenant isolation ───────────────────────────────────────────────────────────

/// RFC 009 §4: list_by_tenant must only return policies for the queried
/// tenant, not policies belonging to other tenants.
#[tokio::test]
async fn tenant_isolation_in_list() {
    let (store, svc) = setup().await;

    // Create a second tenant.
    TenantServiceImpl::new(store.clone())
        .create(TenantId::new("t_other_rfc009"), "Other Tenant".to_owned())
        .await
        .unwrap();
    let other_svc = RoutePolicyServiceImpl::new(store.clone());

    // Two policies for the main tenant, one for the other.
    // Sleep between creates to ensure unique ms-based policy IDs.
    svc.create(tenant(), "main-policy-1".to_owned(), vec![])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    svc.create(tenant(), "main-policy-2".to_owned(), vec![])
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(2)).await;
    other_svc
        .create(
            TenantId::new("t_other_rfc009"),
            "other-policy".to_owned(),
            vec![],
        )
        .await
        .unwrap();

    let main_list = RoutePolicyReadModel::list_by_tenant(store.as_ref(), &tenant(), 10, 0)
        .await
        .unwrap();
    assert_eq!(
        main_list.len(),
        2,
        "main tenant must see only its 2 policies"
    );

    let other_list = RoutePolicyReadModel::list_by_tenant(
        store.as_ref(),
        &TenantId::new("t_other_rfc009"),
        10,
        0,
    )
    .await
    .unwrap();
    assert_eq!(
        other_list.len(),
        1,
        "other tenant must see only its 1 policy"
    );
}

// ── get() on unknown ID returns None ──────────────────────────────────────────

#[tokio::test]
async fn get_unknown_policy_returns_none() {
    let (_store, svc) = setup().await;

    let result = svc.get("no_such_policy").await.unwrap();
    assert!(
        result.is_none(),
        "get() on unknown policy_id must return None"
    );
}

// ── Creating for a non-existent tenant returns NotFound ───────────────────────

#[tokio::test]
async fn create_for_missing_tenant_returns_not_found() {
    let store = Arc::new(InMemoryStore::new());
    let svc = RoutePolicyServiceImpl::new(store);

    let err = svc
        .create(TenantId::new("ghost"), "p".to_owned(), vec![])
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            RuntimeError::NotFound {
                entity: "tenant",
                ..
            }
        ),
        "creating a policy for a non-existent tenant must return NotFound; got: {err:?}"
    );
}

// ── Policy with no rules is valid ─────────────────────────────────────────────

/// RFC 009 allows empty rule sets — a policy with no rules acts as a
/// no-op placeholder that can have rules attached later.
#[tokio::test]
async fn policy_with_no_rules_is_valid() {
    let (_store, svc) = setup().await;

    let policy = svc
        .create(tenant(), "empty-rule-policy".to_owned(), vec![])
        .await
        .unwrap();

    assert!(policy.enabled);
    assert!(
        policy.rules.is_empty(),
        "policy with no rules must be accepted and stored with empty rules"
    );

    let fetched = svc.get(&policy.policy_id).await.unwrap().unwrap();
    assert!(fetched.rules.is_empty());
}

// ── Policy rule priority ordering is preserved ────────────────────────────────

/// Rules must be stored and returned in the order provided; the operator
/// defines priority through the `priority` field value, not list position.
#[tokio::test]
async fn rule_priorities_preserved_on_create() {
    let (_store, svc) = setup().await;

    let rules = vec![
        make_rule("rule_high", "tbd", 1),
        make_rule("rule_medium", "tbd", 50),
        make_rule("rule_low", "tbd", 100),
    ];

    let policy = svc
        .create(tenant(), "priority-test-policy".to_owned(), rules)
        .await
        .unwrap();

    assert_eq!(policy.rules.len(), 3);
    assert_eq!(policy.rules[0].priority, 1);
    assert_eq!(policy.rules[1].priority, 50);
    assert_eq!(policy.rules[2].priority, 100);

    let fetched = svc.get(&policy.policy_id).await.unwrap().unwrap();
    let priorities: Vec<u32> = fetched.rules.iter().map(|r| r.priority).collect();
    assert_eq!(
        priorities,
        vec![1, 50, 100],
        "rule priorities must be preserved exactly"
    );
}
