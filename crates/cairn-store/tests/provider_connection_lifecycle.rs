//! RFC 007 provider connection lifecycle integration tests.
//!
//! Validates the provider health pipeline through InMemoryStore:
//! - ProviderConnectionRegistered creates a connection record.
//! - ProviderHealthChecked updates health status with latency and timestamps.
//! - ProviderMarkedDegraded transitions to degraded with a reason.
//! - ProviderRecovered clears error state and restores healthy status.
//! - Consecutive failure counter increments on unhealthy checks and resets on recovery.
//! - Cross-tenant isolation: health records are tenant-scoped.

use std::sync::Arc;

use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProviderConnectionId, ProviderHealthChecked,
    ProviderMarkedDegraded, ProviderRecovered, RuntimeEvent, TenantId,
};
use cairn_domain::events::ProviderConnectionRegistered;
use cairn_domain::providers::{ProviderConnectionStatus, ProviderHealthStatus};
use cairn_domain::tenancy::TenantKey;
use cairn_store::{
    projections::{ProviderConnectionReadModel, ProviderHealthReadModel},
    EventLog, InMemoryStore,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn tenant_id() -> TenantId {
    TenantId::new("tenant_provider")
}

fn tenant_key() -> TenantKey {
    TenantKey::new("tenant_provider")
}

fn conn_id(n: &str) -> ProviderConnectionId {
    ProviderConnectionId::new(format!("conn_{n}"))
}

fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(id),
        EventSource::System,
        payload.into(),
    )
}

fn register_event(conn: &str, family: &str) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_reg_{conn}"),
        RuntimeEvent::ProviderConnectionRegistered(ProviderConnectionRegistered {
            tenant: tenant_key(),
            provider_connection_id: conn_id(conn),
            provider_family: family.to_owned(),
            adapter_type: "responses".to_owned(),
            supported_models: vec![],
            status: ProviderConnectionStatus::Active,
            registered_at: 1_000_000,
        }),
    )
}

fn health_check_event(
    conn: &str,
    status: ProviderHealthStatus,
    latency_ms: Option<u64>,
    checked_at: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_health_{conn}_{checked_at}"),
        RuntimeEvent::ProviderHealthChecked(ProviderHealthChecked {
            tenant_id: tenant_id(),
            connection_id: conn_id(conn),
            status,
            latency_ms,
            checked_at_ms: checked_at,
        }),
    )
}

fn degrade_event(conn: &str, reason: &str, at: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_degrade_{conn}"),
        RuntimeEvent::ProviderMarkedDegraded(ProviderMarkedDegraded {
            tenant_id: tenant_id(),
            connection_id: conn_id(conn),
            reason: reason.to_owned(),
            marked_at_ms: at,
        }),
    )
}

fn recover_event(conn: &str, at: u64) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_recover_{conn}"),
        RuntimeEvent::ProviderRecovered(ProviderRecovered {
            tenant_id: tenant_id(),
            connection_id: conn_id(conn),
            recovered_at_ms: at,
        }),
    )
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Append ProviderConnectionRegistered.
/// (2) Verify connection read model stores the connection with correct status.
#[tokio::test]
async fn connection_registered_and_readable() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[register_event("openai", "openai")]).await.unwrap();

    let conn = ProviderConnectionReadModel::get(store.as_ref(), &conn_id("openai"))
        .await
        .unwrap()
        .expect("connection record must exist after ProviderConnectionRegistered");

    assert_eq!(conn.provider_connection_id, conn_id("openai"));
    assert_eq!(conn.tenant_id, tenant_id());
    assert_eq!(conn.provider_family, "openai");
    assert_eq!(conn.adapter_type, "responses");
    assert_eq!(conn.status, ProviderConnectionStatus::Active);
    assert_eq!(conn.created_at, 1_000_000);

    // list_by_tenant must include the connection.
    let connections = ProviderConnectionReadModel::list_by_tenant(
        store.as_ref(), &tenant_id(), 10, 0
    ).await.unwrap();
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0].provider_connection_id, conn_id("openai"));
}

/// (3) Append ProviderHealthChecked with healthy status.
/// (4) Verify health record is created and updated correctly.
#[tokio::test]
async fn health_checked_healthy_updates_record() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[register_event("anthropic", "anthropic")]).await.unwrap();

    // Initial health check: healthy, 85ms latency.
    store.append(&[
        health_check_event("anthropic", ProviderHealthStatus::Healthy, Some(85), 2_000_000)
    ]).await.unwrap();

    let health = ProviderHealthReadModel::get(store.as_ref(), &conn_id("anthropic"))
        .await
        .unwrap()
        .expect("health record must exist after ProviderHealthChecked");

    assert!(health.healthy, "health record must be healthy after Healthy status check");
    assert_eq!(health.status, ProviderHealthStatus::Healthy);
    assert_eq!(health.last_checked_ms, 2_000_000, "last_checked_ms must match checked_at_ms");
    assert_eq!(health.consecutive_failures, 0, "no consecutive failures after healthy check");
    assert!(health.error_message.is_none(), "no error message for healthy connection");
}

/// Full lifecycle: register → healthy → degraded → recovered.
/// Steps (1)-(8) as a sequential state machine.
#[tokio::test]
async fn full_lifecycle_register_healthy_degraded_recovered() {
    let store = Arc::new(InMemoryStore::new());

    // (1) Register the connection.
    store.append(&[register_event("bedrock", "bedrock")]).await.unwrap();

    // Verify connection exists.
    let conn = ProviderConnectionReadModel::get(store.as_ref(), &conn_id("bedrock"))
        .await.unwrap().unwrap();
    assert_eq!(conn.status, ProviderConnectionStatus::Active);

    // (3) Health check — healthy.
    store.append(&[
        health_check_event("bedrock", ProviderHealthStatus::Healthy, Some(120), 3_000_000)
    ]).await.unwrap();

    // (4) Verify health record shows healthy.
    let health_healthy = ProviderHealthReadModel::get(store.as_ref(), &conn_id("bedrock"))
        .await.unwrap().unwrap();
    assert!(health_healthy.healthy);
    assert_eq!(health_healthy.status, ProviderHealthStatus::Healthy);
    assert_eq!(health_healthy.consecutive_failures, 0);

    // (5) Provider marked degraded.
    store.append(&[
        degrade_event("bedrock", "connection timeout after 3 retries", 4_000_000)
    ]).await.unwrap();

    // (6) Verify degraded status.
    let health_degraded = ProviderHealthReadModel::get(store.as_ref(), &conn_id("bedrock"))
        .await.unwrap().unwrap();
    assert!(!health_degraded.healthy, "health must be false after ProviderMarkedDegraded");
    assert_eq!(
        health_degraded.status,
        ProviderHealthStatus::Degraded,
        "status must be Degraded"
    );
    assert!(
        health_degraded.error_message.is_some(),
        "error_message must be set on degraded record"
    );
    assert!(
        health_degraded.error_message.as_ref().unwrap().contains("timeout"),
        "error_message must contain the degradation reason"
    );
    assert_eq!(health_degraded.last_checked_ms, 4_000_000);

    // (7) Provider recovered.
    store.append(&[recover_event("bedrock", 5_000_000)]).await.unwrap();

    // (8) Verify status back to healthy.
    let health_recovered = ProviderHealthReadModel::get(store.as_ref(), &conn_id("bedrock"))
        .await.unwrap().unwrap();
    assert!(
        health_recovered.healthy,
        "health must be true after ProviderRecovered"
    );
    assert_eq!(
        health_recovered.status,
        ProviderHealthStatus::Healthy,
        "status must be Healthy after recovery"
    );
    assert!(
        health_recovered.error_message.is_none(),
        "error_message must be cleared after recovery"
    );
    assert_eq!(health_recovered.consecutive_failures, 0, "consecutive_failures must reset to 0");
    assert_eq!(health_recovered.last_checked_ms, 5_000_000);
}

/// Consecutive failure counter increments on each unhealthy check
/// and resets to zero after a healthy check.
#[tokio::test]
async fn consecutive_failures_tracked_and_reset() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[register_event("openrouter", "openrouter")]).await.unwrap();

    // Three consecutive unhealthy checks.
    for (i, ts) in [(1u64, 1_000u64), (2, 2_000), (3, 3_000)] {
        store.append(&[
            health_check_event(
                "openrouter",
                ProviderHealthStatus::Degraded,
                None,
                ts,
            )
        ]).await.unwrap();

        let h = ProviderHealthReadModel::get(store.as_ref(), &conn_id("openrouter"))
            .await.unwrap().unwrap();
        assert_eq!(
            h.consecutive_failures, i as u32,
            "consecutive_failures must be {i} after {i} unhealthy checks"
        );
        assert!(!h.healthy);
    }

    // One healthy check — failures must reset to 0.
    store.append(&[
        health_check_event("openrouter", ProviderHealthStatus::Healthy, Some(90), 4_000)
    ]).await.unwrap();

    let h = ProviderHealthReadModel::get(store.as_ref(), &conn_id("openrouter"))
        .await.unwrap().unwrap();
    assert_eq!(h.consecutive_failures, 0, "consecutive_failures must reset to 0 after healthy check");
    assert!(h.healthy);
}

/// Multiple connections for the same tenant are tracked independently.
#[tokio::test]
async fn multiple_connections_tracked_independently() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[
        register_event("conn_a", "openai"),
        register_event("conn_b", "anthropic"),
        register_event("conn_c", "bedrock"),
    ]).await.unwrap();

    // Healthy check for conn_a, degraded for conn_b, no check for conn_c.
    store.append(&[
        health_check_event("conn_a", ProviderHealthStatus::Healthy,  Some(80),  10_000),
        health_check_event("conn_b", ProviderHealthStatus::Degraded, None,      10_000),
    ]).await.unwrap();

    let ha = ProviderHealthReadModel::get(store.as_ref(), &conn_id("conn_a"))
        .await.unwrap().unwrap();
    let hb = ProviderHealthReadModel::get(store.as_ref(), &conn_id("conn_b"))
        .await.unwrap().unwrap();
    let hc = ProviderHealthReadModel::get(store.as_ref(), &conn_id("conn_c"))
        .await.unwrap();

    assert!(ha.healthy, "conn_a must be healthy");
    assert!(!hb.healthy, "conn_b must be unhealthy");
    assert!(hc.is_none(), "conn_c has no health check yet — record must be None");

    // list_by_tenant returns all registered connections.
    let conns = ProviderConnectionReadModel::list_by_tenant(
        store.as_ref(), &tenant_id(), 10, 0
    ).await.unwrap();
    assert_eq!(conns.len(), 3, "all 3 connections must be listed for tenant");

    // list health records by tenant returns only those with health records.
    let health_records = ProviderHealthReadModel::list_by_tenant(
        store.as_ref(), &tenant_id(), 10, 0
    ).await.unwrap();
    assert_eq!(
        health_records.len(), 2,
        "only 2 health records exist (conn_c has no health check)"
    );
}

/// ProviderMarkedDegraded on a connection with no prior health record
/// creates a new record in degraded state.
#[tokio::test]
async fn degrade_without_prior_health_check_creates_record() {
    let store = Arc::new(InMemoryStore::new());

    store.append(&[register_event("cold_conn", "openai")]).await.unwrap();

    // No health checks yet — degrade directly.
    store.append(&[
        degrade_event("cold_conn", "initial contact failed", 1_500_000)
    ]).await.unwrap();

    let health = ProviderHealthReadModel::get(store.as_ref(), &conn_id("cold_conn"))
        .await.unwrap()
        .expect("ProviderMarkedDegraded must create a health record even with no prior check");

    assert!(!health.healthy);
    assert_eq!(health.status, ProviderHealthStatus::Degraded);
    assert!(health.error_message.as_ref().unwrap().contains("initial contact failed"));
    assert_eq!(health.last_checked_ms, 1_500_000);

    // Recover from cold degraded state.
    store.append(&[recover_event("cold_conn", 2_000_000)]).await.unwrap();

    let recovered = ProviderHealthReadModel::get(store.as_ref(), &conn_id("cold_conn"))
        .await.unwrap().unwrap();
    assert!(recovered.healthy);
    assert!(recovered.error_message.is_none());
}
