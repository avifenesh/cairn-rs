//! RFC 009 — provider budget and connection pool end-to-end integration tests.

use std::sync::Arc;

use cairn_domain::ids::{ProviderConnectionId, SessionId};
use cairn_domain::providers::ProviderBudgetPeriod;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RuntimeEvent, SessionCostUpdated, TenantId,
};
use cairn_runtime::budgets::BudgetService;
use cairn_runtime::error::RuntimeError;
use cairn_runtime::provider_pools::ProviderConnectionPoolService;
use cairn_runtime::services::{BudgetServiceImpl, ProviderConnectionPoolServiceImpl};
use cairn_store::{EventLog, InMemoryStore};

fn tenant() -> TenantId {
    TenantId::new("t_rfc009_budget")
}
fn project() -> ProjectKey {
    ProjectKey::new("t_rfc009_budget", "w_budget", "p_budget")
}
fn conn(id: &str) -> ProviderConnectionId {
    ProviderConnectionId::new(id)
}

async fn spend(store: &Arc<InMemoryStore>, id: &str, delta: u64) {
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new(id),
            EventSource::Runtime,
            RuntimeEvent::SessionCostUpdated(SessionCostUpdated {
                project: project(),
                session_id: SessionId::new(format!("sess_{id}")),
                tenant_id: tenant(),
                delta_cost_micros: delta,
                delta_tokens_in: 0,
                delta_tokens_out: 0,
                provider_call_id: format!("call_{id}"),
                updated_at_ms: 1_700_000_000_000,
            }),
        )])
        .await
        .unwrap();
}

#[tokio::test]
async fn set_budget_and_verify_retrieval() {
    let store = Arc::new(InMemoryStore::new());
    let svc = BudgetServiceImpl::new(store);

    let budget = svc
        .set_budget(tenant(), ProviderBudgetPeriod::Monthly, 10_000_000, 80)
        .await
        .unwrap();

    assert_eq!(budget.tenant_id, tenant());
    assert_eq!(budget.period, ProviderBudgetPeriod::Monthly);
    assert_eq!(budget.limit_micros, 10_000_000);
    assert_eq!(budget.alert_threshold_percent, 80);
    assert_eq!(
        budget.current_spend_micros, 0,
        "fresh budget must have zero spend"
    );

    let fetched = svc
        .get_budget(&tenant(), ProviderBudgetPeriod::Monthly)
        .await
        .unwrap()
        .expect("budget must be retrievable by tenant + period");
    assert_eq!(fetched.limit_micros, 10_000_000);
    assert_eq!(fetched.period, ProviderBudgetPeriod::Monthly);
}

#[tokio::test]
async fn create_pool_add_connections_verify_membership() {
    let store = Arc::new(InMemoryStore::new());
    let pool_svc = ProviderConnectionPoolServiceImpl::new(store);

    let pool = pool_svc
        .create_pool(tenant(), "pool_rfc009".to_owned(), 3)
        .await
        .unwrap();
    assert_eq!(pool.pool_id, "pool_rfc009");
    assert_eq!(pool.max_connections, 3);
    assert_eq!(pool.active_connections, 0);
    assert!(pool.connection_ids.is_empty());

    let fetched = pool_svc
        .get_pool("pool_rfc009")
        .await
        .unwrap()
        .expect("pool must be retrievable");
    assert_eq!(fetched.pool_id, "pool_rfc009");

    let after_first = pool_svc
        .add_connection("pool_rfc009", conn("conn_alpha"))
        .await
        .unwrap();
    assert_eq!(after_first.active_connections, 1);
    assert!(after_first.connection_ids.contains(&conn("conn_alpha")));

    let after_second = pool_svc
        .add_connection("pool_rfc009", conn("conn_beta"))
        .await
        .unwrap();
    assert_eq!(after_second.active_connections, 2);

    let after_third = pool_svc
        .add_connection("pool_rfc009", conn("conn_gamma"))
        .await
        .unwrap();
    assert_eq!(after_third.active_connections, 3);

    let full_pool = pool_svc.get_pool("pool_rfc009").await.unwrap().unwrap();
    assert_eq!(full_pool.active_connections, 3);
    assert!(full_pool.connection_ids.contains(&conn("conn_alpha")));
    assert!(full_pool.connection_ids.contains(&conn("conn_beta")));
    assert!(full_pool.connection_ids.contains(&conn("conn_gamma")));

    let err = pool_svc
        .add_connection("pool_rfc009", conn("conn_overflow"))
        .await
        .unwrap_err();
    assert!(
        matches!(&err, RuntimeError::PolicyDenied { reason } if reason.contains("pool_full")),
        "adding beyond max_connections must return PolicyDenied(pool_full); got: {err:?}"
    );
}

#[tokio::test]
async fn cost_events_update_budget_spend() {
    let store = Arc::new(InMemoryStore::new());
    let svc = BudgetServiceImpl::new(store.clone());

    svc.set_budget(tenant(), ProviderBudgetPeriod::Monthly, 5_000_000, 50)
        .await
        .unwrap();

    let status = svc.check_budget(&tenant()).await.unwrap();
    assert_eq!(status.percent_used, 0);
    assert!(!status.alert_triggered);
    assert!(!status.exceeded);

    spend(&store, "cost_1", 1_000_000).await;
    spend(&store, "cost_2", 1_000_000).await;

    let budget_partial = svc
        .get_budget(&tenant(), ProviderBudgetPeriod::Monthly)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        budget_partial.current_spend_micros, 2_000_000,
        "budget must reflect accumulated spend after cost events"
    );

    let status = svc.check_budget(&tenant()).await.unwrap();
    assert_eq!(status.percent_used, 40);
    assert!(
        !status.alert_triggered,
        "alert must not fire at 40% (threshold=50%)"
    );
    assert!(!status.exceeded);

    spend(&store, "cost_3", 1_000_000).await;

    let status = svc.check_budget(&tenant()).await.unwrap();
    assert!(status.percent_used >= 60);
    assert!(
        status.alert_triggered,
        "alert must trigger when spend crosses threshold"
    );
    assert!(!status.exceeded);

    spend(&store, "cost_4", 3_000_000).await;

    let budget_exceeded = svc
        .get_budget(&tenant(), ProviderBudgetPeriod::Monthly)
        .await
        .unwrap()
        .unwrap();
    assert!(
        budget_exceeded.current_spend_micros > budget_exceeded.limit_micros,
        "current_spend_micros must exceed limit_micros after overspend"
    );

    let status = svc.check_budget(&tenant()).await.unwrap();
    assert!(
        status.exceeded,
        "budget.exceeded must be true after hard limit is crossed"
    );
    assert_eq!(
        status.remaining_micros, 0,
        "remaining_micros must be 0 when exceeded"
    );
}

#[tokio::test]
async fn daily_and_monthly_budgets_coexist() {
    let store = Arc::new(InMemoryStore::new());
    let svc = BudgetServiceImpl::new(store);

    svc.set_budget(tenant(), ProviderBudgetPeriod::Daily, 1_000_000, 90)
        .await
        .unwrap();
    svc.set_budget(tenant(), ProviderBudgetPeriod::Monthly, 30_000_000, 80)
        .await
        .unwrap();

    let all = svc.list_budgets(&tenant()).await.unwrap();
    assert_eq!(all.len(), 2);

    let daily = svc
        .get_budget(&tenant(), ProviderBudgetPeriod::Daily)
        .await
        .unwrap()
        .unwrap();
    let monthly = svc
        .get_budget(&tenant(), ProviderBudgetPeriod::Monthly)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(daily.limit_micros, 1_000_000);
    assert_eq!(monthly.limit_micros, 30_000_000);
    assert_eq!(daily.period, ProviderBudgetPeriod::Daily);
    assert_eq!(monthly.period, ProviderBudgetPeriod::Monthly);
}

#[tokio::test]
async fn pool_get_available_returns_first_connection() {
    let store = Arc::new(InMemoryStore::new());
    let pool_svc = ProviderConnectionPoolServiceImpl::new(store);

    pool_svc
        .create_pool(tenant(), "pool_avail".to_owned(), 2)
        .await
        .unwrap();
    pool_svc
        .add_connection("pool_avail", conn("conn_first"))
        .await
        .unwrap();
    pool_svc
        .add_connection("pool_avail", conn("conn_second"))
        .await
        .unwrap();

    let available = pool_svc.get_available("pool_avail").await.unwrap();
    assert!(
        available.is_some(),
        "get_available must return a connection when pool is non-empty"
    );

    pool_svc
        .create_pool(tenant(), "pool_empty".to_owned(), 2)
        .await
        .unwrap();
    let none = pool_svc.get_available("pool_empty").await.unwrap();
    assert!(
        none.is_none(),
        "get_available must return None for an empty pool"
    );
}

#[tokio::test]
async fn pool_remove_connection_frees_capacity() {
    let store = Arc::new(InMemoryStore::new());
    let pool_svc = ProviderConnectionPoolServiceImpl::new(store);

    pool_svc
        .create_pool(tenant(), "pool_cap".to_owned(), 1)
        .await
        .unwrap();
    pool_svc
        .add_connection("pool_cap", conn("conn_sole"))
        .await
        .unwrap();

    let err = pool_svc
        .add_connection("pool_cap", conn("conn_new"))
        .await
        .unwrap_err();
    assert!(matches!(err, RuntimeError::PolicyDenied { .. }));

    let after_remove = pool_svc
        .remove_connection("pool_cap", &conn("conn_sole"))
        .await
        .unwrap();
    assert_eq!(after_remove.active_connections, 0);
    assert!(!after_remove.connection_ids.contains(&conn("conn_sole")));

    let after_add = pool_svc
        .add_connection("pool_cap", conn("conn_new"))
        .await
        .unwrap();
    assert_eq!(after_add.active_connections, 1);
}

#[tokio::test]
async fn duplicate_pool_create_returns_conflict() {
    let store = Arc::new(InMemoryStore::new());
    let pool_svc = ProviderConnectionPoolServiceImpl::new(store);

    pool_svc
        .create_pool(tenant(), "pool_dup".to_owned(), 2)
        .await
        .unwrap();
    let err = pool_svc
        .create_pool(tenant(), "pool_dup".to_owned(), 5)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            RuntimeError::Conflict {
                entity: "provider_pool",
                ..
            }
        ),
        "creating a pool with an existing ID must return Conflict; got: {err:?}"
    );
}

#[tokio::test]
async fn list_pools_scoped_to_tenant() {
    let store = Arc::new(InMemoryStore::new());
    let pool_svc = ProviderConnectionPoolServiceImpl::new(store);

    let other = TenantId::new("t_other_pool");

    pool_svc
        .create_pool(tenant(), "pool_t_1".to_owned(), 2)
        .await
        .unwrap();
    pool_svc
        .create_pool(tenant(), "pool_t_2".to_owned(), 2)
        .await
        .unwrap();
    pool_svc
        .create_pool(other.clone(), "pool_other".to_owned(), 2)
        .await
        .unwrap();

    let main_pools = pool_svc.list_pools(&tenant()).await.unwrap();
    assert_eq!(main_pools.len(), 2);

    let other_pools = pool_svc.list_pools(&other).await.unwrap();
    assert_eq!(other_pools.len(), 1);
}
