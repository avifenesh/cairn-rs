//! Provider budget tracking tests (RFC 002 / RFC 009).
//!
//! Validates that provider spend budgets are correctly stored and that
//! alert/exceeded events update the current_spend_micros field.
//!
//! Budget lifecycle:
//!   ProviderBudgetSet          → creates budget with current_spend_micros=0
//!   ProviderBudgetAlertTriggered → updates current_spend_micros to current_micros
//!   ProviderBudgetExceeded      → sets current_spend_micros = limit + exceeded_by
//!
//! Budget key: "tenant_id:period" (e.g. "t_budget:Daily")
//! This key format is shared between ProviderBudgetSet and the alert events.
//!
//! Budget scoping: keyed by tenant_id + period — each tenant×period has one budget.

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProviderBudgetAlertTriggered, ProviderBudgetExceeded,
    ProviderBudgetSet, RuntimeEvent, TenantId,
};
use cairn_domain::providers::ProviderBudgetPeriod;
use cairn_store::{
    projections::ProviderBudgetReadModel,
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    use cairn_domain::OwnershipKey;
    EventEnvelope {
        event_id:       EventId::new(id),
        source:         EventSource::Runtime,
        ownership:      OwnershipKey::System,
        causation_id:   None,
        correlation_id: None,
        payload,
    }
}

/// Build the composite budget_id key matching the projection's format.
fn budget_key(tenant: &str, period: ProviderBudgetPeriod) -> String {
    format!("{tenant}:{period:?}")
}

fn set_budget(
    evt_id:    &str,
    tenant:    &str,
    period:    ProviderBudgetPeriod,
    limit:     u64,
    alert_pct: Option<u32>,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::ProviderBudgetSet(ProviderBudgetSet {
        tenant_id:                 TenantId::new(tenant),
        budget_id:                 budget_key(tenant, period),
        period,
        limit_micros:              limit,
        alert_threshold_percent:   alert_pct,
    }))
}

fn alert_triggered(
    evt_id:          &str,
    tenant:          &str,
    period:          ProviderBudgetPeriod,
    current_micros:  u64,
    limit_micros:    u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::ProviderBudgetAlertTriggered(ProviderBudgetAlertTriggered {
        budget_id:       budget_key(tenant, period),
        current_micros,
        limit_micros,
        triggered_at_ms: 1_000_000,
    }))
}

fn budget_exceeded(
    evt_id:            &str,
    tenant:            &str,
    period:            ProviderBudgetPeriod,
    exceeded_by:       u64,
) -> EventEnvelope<RuntimeEvent> {
    evt(evt_id, RuntimeEvent::ProviderBudgetExceeded(ProviderBudgetExceeded {
        budget_id:           budget_key(tenant, period),
        exceeded_by_micros:  exceeded_by,
        exceeded_at_ms:      1_000_000,
    }))
}

// ── 1. ProviderBudgetSet stores the budget record ─────────────────────────────

#[tokio::test]
async fn budget_set_stores_record_with_zero_spend() {
    let store = InMemoryStore::new();

    store.append(&[set_budget("e1", "t_budget", ProviderBudgetPeriod::Daily, 10_000_000, Some(80))])
        .await.unwrap();

    let budget = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_budget"), ProviderBudgetPeriod::Daily,
    ).await.unwrap()
     .expect("budget must exist after ProviderBudgetSet");

    assert_eq!(budget.tenant_id.as_str(), "t_budget");
    assert_eq!(budget.period, ProviderBudgetPeriod::Daily);
    assert_eq!(budget.limit_micros, 10_000_000, "$10 daily limit");
    assert_eq!(budget.alert_threshold_percent, 80);
    assert_eq!(budget.current_spend_micros, 0, "new budget starts at zero spend");
}

// ── 2. Daily and monthly budgets are independent ──────────────────────────────

#[tokio::test]
async fn daily_and_monthly_budgets_are_independent() {
    let store = InMemoryStore::new();

    store.append(&[
        set_budget("e1", "t_periods", ProviderBudgetPeriod::Daily,   1_000_000, None),
        set_budget("e2", "t_periods", ProviderBudgetPeriod::Monthly, 20_000_000, Some(90)),
    ]).await.unwrap();

    let daily = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_periods"), ProviderBudgetPeriod::Daily,
    ).await.unwrap().unwrap();
    assert_eq!(daily.limit_micros, 1_000_000);
    assert_eq!(daily.alert_threshold_percent, 80, "default alert threshold when None");

    let monthly = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_periods"), ProviderBudgetPeriod::Monthly,
    ).await.unwrap().unwrap();
    assert_eq!(monthly.limit_micros, 20_000_000);
    assert_eq!(monthly.alert_threshold_percent, 90);
}

// ── 3. get_by_tenant_period returns None for unknown tenant/period ─────────────

#[tokio::test]
async fn get_returns_none_for_unknown_budget() {
    let store = InMemoryStore::new();
    let result = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("nobody"), ProviderBudgetPeriod::Daily,
    ).await.unwrap();
    assert!(result.is_none());
}

// ── 4. ProviderBudgetAlertTriggered updates current_spend_micros ──────────────

#[tokio::test]
async fn alert_triggered_updates_current_spend() {
    let store = InMemoryStore::new();

    store.append(&[
        set_budget("e1", "t_alert", ProviderBudgetPeriod::Daily, 10_000_000, Some(80)),
    ]).await.unwrap();

    // Spend reaches 80% of limit → alert fires.
    let alert_spend = 8_000_000u64; // 80% of $10
    store.append(&[alert_triggered("e2", "t_alert", ProviderBudgetPeriod::Daily, alert_spend, 10_000_000)])
        .await.unwrap();

    let budget = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_alert"), ProviderBudgetPeriod::Daily,
    ).await.unwrap().unwrap();

    assert_eq!(budget.current_spend_micros, alert_spend,
        "current_spend_micros must be updated to alert snapshot value");
}

// ── 5. ProviderBudgetExceeded sets spend to limit + overage ───────────────────

#[tokio::test]
async fn budget_exceeded_sets_overage_spend() {
    let store = InMemoryStore::new();
    let limit = 5_000_000u64;

    store.append(&[
        set_budget("e1", "t_exceed", ProviderBudgetPeriod::Monthly, limit, Some(90)),
        budget_exceeded("e2", "t_exceed", ProviderBudgetPeriod::Monthly, 500_000),
    ]).await.unwrap();

    let budget = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_exceed"), ProviderBudgetPeriod::Monthly,
    ).await.unwrap().unwrap();

    assert_eq!(budget.current_spend_micros, limit + 500_000,
        "exceeded spend = limit + exceeded_by_micros");
    assert!(budget.current_spend_micros > budget.limit_micros,
        "spend must exceed the limit after ProviderBudgetExceeded");
}

// ── 6. spent_micros accumulates across events ─────────────────────────────────

#[tokio::test]
async fn spend_accumulates_through_alert_progression() {
    let store = InMemoryStore::new();
    let limit = 10_000_000u64;  // $10

    store.append(&[
        set_budget("e1", "t_accum", ProviderBudgetPeriod::Daily, limit, Some(70)),
    ]).await.unwrap();

    // Spend progresses through the day in steps.
    let steps: &[(u64, &str)] = &[
        (2_000_000, "e2"),  // 20%
        (5_000_000, "e3"),  // 50%
        (7_500_000, "e4"),  // 75% → alert fires
        (9_000_000, "e5"),  // 90%
    ];

    for (spend, evt_id) in steps {
        store.append(&[alert_triggered(evt_id, "t_accum", ProviderBudgetPeriod::Daily, *spend, limit)])
            .await.unwrap();

        let budget = ProviderBudgetReadModel::get_by_tenant_period(
            &store, &TenantId::new("t_accum"), ProviderBudgetPeriod::Daily,
        ).await.unwrap().unwrap();

        assert_eq!(budget.current_spend_micros, *spend,
            "after alert at {}µ: current_spend must be {}µ", spend, spend);
    }

    // Final alert triggers overage.
    store.append(&[budget_exceeded("e6", "t_accum", ProviderBudgetPeriod::Daily, 200_000)])
        .await.unwrap();

    let final_budget = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_accum"), ProviderBudgetPeriod::Daily,
    ).await.unwrap().unwrap();

    assert_eq!(final_budget.current_spend_micros, limit + 200_000,
        "final spend = limit + overage");
}

// ── 7. Budget scoping by tenant ───────────────────────────────────────────────

#[tokio::test]
async fn budgets_scoped_by_tenant() {
    let store = InMemoryStore::new();

    store.append(&[
        set_budget("e1", "tenant_x", ProviderBudgetPeriod::Daily, 5_000_000, None),
        set_budget("e2", "tenant_y", ProviderBudgetPeriod::Daily, 2_000_000, None),
        // Alert for tenant_x must not affect tenant_y.
        alert_triggered("e3", "tenant_x", ProviderBudgetPeriod::Daily, 4_000_000, 5_000_000),
    ]).await.unwrap();

    let x = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("tenant_x"), ProviderBudgetPeriod::Daily,
    ).await.unwrap().unwrap();
    assert_eq!(x.current_spend_micros, 4_000_000, "tenant_x spend updated by alert");
    assert_eq!(x.limit_micros, 5_000_000);

    let y = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("tenant_y"), ProviderBudgetPeriod::Daily,
    ).await.unwrap().unwrap();
    assert_eq!(y.current_spend_micros, 0, "tenant_y spend unaffected by tenant_x alert");
    assert_eq!(y.limit_micros, 2_000_000);
}

// ── 8. list_by_tenant returns all budgets for a tenant ────────────────────────

#[tokio::test]
async fn list_by_tenant_returns_all_periods() {
    let store = InMemoryStore::new();

    store.append(&[
        set_budget("e1", "t_list", ProviderBudgetPeriod::Daily,   1_000_000, None),
        set_budget("e2", "t_list", ProviderBudgetPeriod::Monthly, 25_000_000, None),
        // Different tenant — must not appear.
        set_budget("e3", "t_other", ProviderBudgetPeriod::Daily, 999, None),
    ]).await.unwrap();

    let budgets = ProviderBudgetReadModel::list_by_tenant(&store, &TenantId::new("t_list"))
        .await.unwrap();

    assert_eq!(budgets.len(), 2, "t_list has daily and monthly budgets");
    assert!(budgets.iter().all(|b| b.tenant_id.as_str() == "t_list"));

    let daily = budgets.iter().find(|b| b.period == ProviderBudgetPeriod::Daily).unwrap();
    let monthly = budgets.iter().find(|b| b.period == ProviderBudgetPeriod::Monthly).unwrap();
    assert_eq!(daily.limit_micros, 1_000_000);
    assert_eq!(monthly.limit_micros, 25_000_000);
}

// ── 9. Budget update (reset): ProviderBudgetSet overwrites existing record ─────

#[tokio::test]
async fn budget_set_overwrites_existing_record() {
    let store = InMemoryStore::new();

    // Set initial budget with spend.
    store.append(&[
        set_budget("e1", "t_reset", ProviderBudgetPeriod::Daily, 5_000_000, Some(80)),
        alert_triggered("e2", "t_reset", ProviderBudgetPeriod::Daily, 4_000_000, 5_000_000),
    ]).await.unwrap();

    let before = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_reset"), ProviderBudgetPeriod::Daily,
    ).await.unwrap().unwrap();
    assert_eq!(before.current_spend_micros, 4_000_000);

    // New period: operator resets budget with higher limit.
    store.append(&[set_budget("e3", "t_reset", ProviderBudgetPeriod::Daily, 10_000_000, Some(85))])
        .await.unwrap();

    let after = ProviderBudgetReadModel::get_by_tenant_period(
        &store, &TenantId::new("t_reset"), ProviderBudgetPeriod::Daily,
    ).await.unwrap().unwrap();

    assert_eq!(after.limit_micros, 10_000_000, "limit updated by new ProviderBudgetSet");
    assert_eq!(after.current_spend_micros, 0, "spend reset to 0 on new budget period");
    assert_eq!(after.alert_threshold_percent, 85);
}
