//! RFC 009 — provider health check system end-to-end integration tests.
//!
//! Tests the full health check lifecycle:
//!   1. Register a provider connection
//!   2. Set a health check schedule
//!   3. Record a health check result (healthy)
//!   4. Verify healthy status via read model
//!   5. Record consecutive failed checks — triggers Degraded
//!   6. Verify Degraded status and failure count
//!   7. List degraded providers

use std::sync::Arc;

use cairn_domain::providers::ProviderHealthStatus;
use cairn_domain::{ProviderConnectionId, TenantId, WorkspaceId};
use cairn_runtime::provider_connections::{ProviderConnectionConfig, ProviderConnectionService};
use cairn_runtime::provider_health::ProviderHealthService;
use cairn_runtime::services::{
    ProviderConnectionServiceImpl, ProviderHealthServiceImpl, TenantServiceImpl,
    WorkspaceServiceImpl,
};
use cairn_runtime::tenants::TenantService;
use cairn_runtime::workspaces::WorkspaceService;
use cairn_store::projections::{ProviderHealthReadModel, ProviderHealthScheduleReadModel};
use cairn_store::InMemoryStore;

fn tenant() -> TenantId {
    TenantId::new("t_health")
}

fn conn(id: &str) -> ProviderConnectionId {
    ProviderConnectionId::new(id)
}

async fn setup() -> (Arc<InMemoryStore>, ProviderHealthServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    TenantServiceImpl::new(store.clone())
        .create(tenant(), "Health Tenant".to_owned())
        .await
        .unwrap();
    WorkspaceServiceImpl::new(store.clone())
        .create(
            tenant(),
            WorkspaceId::new("w_health"),
            "Health WS".to_owned(),
        )
        .await
        .unwrap();
    let svc = ProviderHealthServiceImpl::new(store.clone());
    (store, svc)
}

async fn register_connection(
    store: &Arc<InMemoryStore>,
    conn_id: &ProviderConnectionId,
    family: &str,
) {
    ProviderConnectionServiceImpl::new(store.clone())
        .create(
            tenant(),
            conn_id.clone(),
            ProviderConnectionConfig {
                provider_family: family.to_owned(),
                adapter_type: "api".to_owned(),
                supported_models: vec![],
            },
        )
        .await
        .unwrap();
}

// ── Tests 1–4: register connection, schedule, healthy check ──────────────────

/// RFC 009 §6: health checks must be recorded and the status reflected
/// immediately in the read model.
#[tokio::test]
async fn register_schedule_and_record_healthy_check() {
    let (store, svc) = setup().await;

    // ── (1) Register a provider connection ────────────────────────────────
    register_connection(&store, &conn("conn_healthy"), "openai").await;

    // Verify no health record exists before any check.
    let before = ProviderHealthReadModel::get(store.as_ref(), &conn("conn_healthy"))
        .await
        .unwrap();
    assert!(
        before.is_none(),
        "no health record should exist before first check"
    );

    // ── (2) Set a health check schedule ───────────────────────────────────
    let schedule = svc
        .schedule_health_check(&conn("conn_healthy"), 60_000)
        .await
        .unwrap();

    assert_eq!(
        schedule.interval_ms, 60_000,
        "schedule interval must be persisted"
    );
    assert!(schedule.enabled, "schedule must be enabled by default");
    assert!(
        schedule.last_run_ms.is_none(),
        "last_run_ms must be None before first run"
    );

    // Schedule must be retrievable from the read model.
    let sched_read = ProviderHealthScheduleReadModel::get_schedule(
        store.as_ref(),
        conn("conn_healthy").as_str(),
    )
    .await
    .unwrap()
    .expect("schedule must be retrievable after set");
    assert_eq!(sched_read.interval_ms, 60_000);
    assert!(sched_read.enabled);

    // ── (3) Record a healthy check result ─────────────────────────────────
    let healthy_record = svc
        .record_check(&conn("conn_healthy"), 42, true) // success=true, 42ms latency
        .await
        .unwrap();

    // ── (4) Verify healthy status via read model ───────────────────────────
    assert_eq!(
        healthy_record.status,
        ProviderHealthStatus::Healthy,
        "RFC 009: successful check must yield Healthy status"
    );
    assert!(healthy_record.healthy, "healthy flag must be true");
    assert_eq!(
        healthy_record.consecutive_failures, 0,
        "consecutive_failures must be 0 after successful check"
    );
    assert!(
        healthy_record.last_checked_ms > 0,
        "last_checked_ms must be set"
    );

    // get() must return the same record.
    let fetched = svc.get(&conn("conn_healthy")).await.unwrap().unwrap();
    assert_eq!(fetched.status, ProviderHealthStatus::Healthy);
    assert_eq!(fetched.consecutive_failures, 0);
}

// ── Tests 5–7: consecutive failures → Degraded, list degraded ────────────────

/// RFC 009 §6: three consecutive failed checks must mark a connection as
/// Degraded; list() must include it in the degraded subset.
#[tokio::test]
async fn consecutive_failures_trigger_degraded_status() {
    let (store, svc) = setup().await;

    // ── (5a) Register connection ───────────────────────────────────────────
    register_connection(&store, &conn("conn_degrade"), "anthropic").await;

    // First failure → Unreachable (not yet Degraded).
    let after_f1 = svc
        .record_check(&conn("conn_degrade"), 0, false)
        .await
        .unwrap();
    assert_eq!(
        after_f1.status,
        ProviderHealthStatus::Unreachable,
        "1st failure must yield Unreachable, not Degraded"
    );
    assert_eq!(after_f1.consecutive_failures, 1);
    assert!(!after_f1.healthy);

    // Second failure → still Unreachable.
    let after_f2 = svc
        .record_check(&conn("conn_degrade"), 0, false)
        .await
        .unwrap();
    assert_eq!(after_f2.status, ProviderHealthStatus::Unreachable);
    assert_eq!(after_f2.consecutive_failures, 2);

    // ── (5b) Third failure → Degraded ─────────────────────────────────────
    let degraded_record = svc
        .record_check(&conn("conn_degrade"), 0, false)
        .await
        .unwrap();

    // ── (6) Verify Degraded status ────────────────────────────────────────
    assert_eq!(
        degraded_record.status,
        ProviderHealthStatus::Degraded,
        "RFC 009: 3 consecutive failures must yield Degraded status"
    );
    assert_eq!(
        degraded_record.consecutive_failures, 3,
        "consecutive_failures must equal 3 after third failure"
    );
    assert!(
        !degraded_record.healthy,
        "degraded provider must have healthy=false"
    );

    // get() must reflect Degraded.
    let fetched = svc.get(&conn("conn_degrade")).await.unwrap().unwrap();
    assert_eq!(fetched.status, ProviderHealthStatus::Degraded);

    // ── (7) List degraded providers ───────────────────────────────────────
    let all = svc.list(&tenant(), 10, 0).await.unwrap();
    let degraded: Vec<_> = all
        .iter()
        .filter(|r| r.status == ProviderHealthStatus::Degraded)
        .collect();

    assert_eq!(
        degraded.len(),
        1,
        "list must include exactly 1 degraded provider; got: {}",
        degraded.len()
    );
    assert!(!degraded[0].healthy);
}

// ── Degraded → recovery resets to Healthy ────────────────────────────────────

/// RFC 009 §6: mark_recovered must reset the status to Healthy and clear
/// the consecutive_failures counter.
#[tokio::test]
async fn mark_recovered_resets_to_healthy() {
    let (store, svc) = setup().await;

    register_connection(&store, &conn("conn_recover"), "bedrock").await;

    // Degrade the connection (3 failures).
    for _ in 0..3 {
        svc.record_check(&conn("conn_recover"), 0, false)
            .await
            .unwrap();
    }
    let degraded = svc.get(&conn("conn_recover")).await.unwrap().unwrap();
    assert_eq!(degraded.status, ProviderHealthStatus::Degraded);

    // Recover.
    let recovered = svc.mark_recovered(&conn("conn_recover")).await.unwrap();

    assert_eq!(
        recovered.status,
        ProviderHealthStatus::Healthy,
        "mark_recovered must reset status to Healthy"
    );
    assert!(recovered.healthy);
    assert_eq!(
        recovered.consecutive_failures, 0,
        "consecutive_failures must be 0 after recovery"
    );
    assert!(
        recovered.error_message.is_none(),
        "error_message must be cleared after recovery"
    );
}

// ── Healthy check does not degrade even with prior failures ──────────────────

/// Two failures followed by a success must remain Healthy (failures reset).
#[tokio::test]
async fn successful_check_resets_failure_counter() {
    let (store, svc) = setup().await;

    register_connection(&store, &conn("conn_reset"), "openrouter").await;

    // Two failures → Unreachable, failures=2.
    svc.record_check(&conn("conn_reset"), 0, false)
        .await
        .unwrap();
    svc.record_check(&conn("conn_reset"), 0, false)
        .await
        .unwrap();

    let mid = svc.get(&conn("conn_reset")).await.unwrap().unwrap();
    assert_eq!(mid.consecutive_failures, 2);
    assert_eq!(mid.status, ProviderHealthStatus::Unreachable);

    // Success → back to Healthy, failures reset.
    let after_success = svc
        .record_check(&conn("conn_reset"), 120, true)
        .await
        .unwrap();
    assert_eq!(
        after_success.status,
        ProviderHealthStatus::Healthy,
        "success after partial failures must yield Healthy"
    );
    assert_eq!(
        after_success.consecutive_failures, 0,
        "consecutive_failures must reset to 0 after success"
    );
}

// ── Multiple connections tracked independently ────────────────────────────────

/// Two connections in the same tenant must track health independently.
#[tokio::test]
async fn multiple_connections_tracked_independently() {
    let (store, svc) = setup().await;

    register_connection(&store, &conn("conn_a"), "openai").await;
    register_connection(&store, &conn("conn_b"), "anthropic").await;

    // conn_a: healthy.
    svc.record_check(&conn("conn_a"), 50, true).await.unwrap();

    // conn_b: 3 failures → Degraded.
    for _ in 0..3 {
        svc.record_check(&conn("conn_b"), 0, false).await.unwrap();
    }

    let a = svc.get(&conn("conn_a")).await.unwrap().unwrap();
    let b = svc.get(&conn("conn_b")).await.unwrap().unwrap();

    assert_eq!(
        a.status,
        ProviderHealthStatus::Healthy,
        "conn_a must be Healthy"
    );
    assert_eq!(
        b.status,
        ProviderHealthStatus::Degraded,
        "conn_b must be Degraded"
    );
    assert_eq!(a.consecutive_failures, 0);
    assert_eq!(b.consecutive_failures, 3);

    // list() must return both records.
    let all = svc.list(&tenant(), 10, 0).await.unwrap();
    assert_eq!(all.len(), 2, "list must return both connections");

    let degraded_count = all
        .iter()
        .filter(|r| r.status == ProviderHealthStatus::Degraded)
        .count();
    let healthy_count = all
        .iter()
        .filter(|r| r.status == ProviderHealthStatus::Healthy)
        .count();
    assert_eq!(degraded_count, 1, "exactly 1 degraded connection");
    assert_eq!(healthy_count, 1, "exactly 1 healthy connection");
}

// ── Health check requires connection to exist ─────────────────────────────────

#[tokio::test]
async fn record_check_for_unknown_connection_returns_not_found() {
    let (_store, svc) = setup().await;

    let err = svc
        .record_check(&conn("conn_ghost"), 50, true)
        .await
        .unwrap_err();

    assert!(
        matches!(
            err,
            cairn_runtime::error::RuntimeError::NotFound {
                entity: "provider_connection",
                ..
            }
        ),
        "record_check for unknown connection must return NotFound; got: {err:?}"
    );
}
