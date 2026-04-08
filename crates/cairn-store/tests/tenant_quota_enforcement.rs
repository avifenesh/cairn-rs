//! RFC 008 — Tenant quota enforcement tests.
//!
//! Validates the full quota lifecycle through the event log and synchronous
//! projection:
//!
//! - `TenantCreated` registers the tenant.
//! - `TenantQuotaSet` populates `QuotaReadModel` with limit fields.
//! - `TenantQuotaViolated` is recorded in the event log (no separate read
//!   model — violations are an audit fact on the log itself).
//! - `get_quota` returns `None` for tenants without a quota record.
//! - Quota records are tenant-scoped: different tenants have independent limits.
//! - Multiple quota types (`max_concurrent_runs`, `max_sessions_per_hour`,
//!   `max_tasks_per_run`) coexist on the same tenant and all persist.

use cairn_domain::{
    events::{TenantCreated, TenantQuotaSet, TenantQuotaViolated},
    tenancy::OwnershipKey,
    EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, RuntimeEvent, TenantId,
    WorkspaceId,
};
use cairn_store::{projections::QuotaReadModel, EventLog, InMemoryStore};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn project_for(tenant_id: &str) -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new(tenant_id),
        workspace_id: WorkspaceId::new("ws_quota"),
        project_id: ProjectId::new("proj_quota"),
    }
}

async fn create_tenant(store: &InMemoryStore, event_id: &str, tenant_id: &str) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        OwnershipKey::Tenant(cairn_domain::tenancy::TenantKey::new(TenantId::new(
            tenant_id,
        ))),
        RuntimeEvent::TenantCreated(TenantCreated {
            project: project_for(tenant_id),
            tenant_id: TenantId::new(tenant_id),
            name: format!("Tenant {tenant_id}"),
            created_at: 1_000,
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn set_quota(
    store: &InMemoryStore,
    event_id: &str,
    tenant_id: &str,
    max_concurrent_runs: u32,
    max_sessions_per_hour: u32,
    max_tasks_per_run: u32,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        OwnershipKey::Tenant(cairn_domain::tenancy::TenantKey::new(TenantId::new(
            tenant_id,
        ))),
        RuntimeEvent::TenantQuotaSet(TenantQuotaSet {
            tenant_id: TenantId::new(tenant_id),
            max_concurrent_runs,
            max_sessions_per_hour,
            max_tasks_per_run,
        }),
    );
    store.append(&[env]).await.unwrap();
}

async fn record_violation(
    store: &InMemoryStore,
    event_id: &str,
    tenant_id: &str,
    quota_type: &str,
    current: u32,
    limit: u32,
    at: u64,
) {
    let env = EventEnvelope::new(
        EventId::new(event_id),
        EventSource::Runtime,
        OwnershipKey::Tenant(cairn_domain::tenancy::TenantKey::new(TenantId::new(
            tenant_id,
        ))),
        RuntimeEvent::TenantQuotaViolated(TenantQuotaViolated {
            tenant_id: TenantId::new(tenant_id),
            quota_type: quota_type.to_owned(),
            current,
            limit,
            occurred_at_ms: at,
        }),
    );
    store.append(&[env]).await.unwrap();
}

// ── 1. Tenant creation ────────────────────────────────────────────────────────

#[tokio::test]
async fn tenant_without_quota_returns_none() {
    let store = InMemoryStore::new();
    create_tenant(&store, "e1", "tenant_no_quota").await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_no_quota"))
        .await
        .unwrap();
    assert!(
        quota.is_none(),
        "tenant without TenantQuotaSet must have no quota record"
    );
}

#[tokio::test]
async fn completely_unknown_tenant_returns_none() {
    let store = InMemoryStore::new();
    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("ghost_tenant"))
        .await
        .unwrap();
    assert!(quota.is_none());
}

// ── 2. TenantQuotaSet stores limits ──────────────────────────────────────────

#[tokio::test]
async fn quota_set_stores_max_concurrent_runs() {
    let store = InMemoryStore::new();
    create_tenant(&store, "e1", "tenant_q1").await;
    set_quota(&store, "e2", "tenant_q1", 5, 20, 10).await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_q1"))
        .await
        .unwrap()
        .expect("quota must exist after TenantQuotaSet");

    assert_eq!(
        quota.max_concurrent_runs, 5,
        "max_concurrent_runs must be 5"
    );
    assert_eq!(quota.tenant_id.as_str(), "tenant_q1");
}

#[tokio::test]
async fn quota_set_stores_all_three_limit_types() {
    let store = InMemoryStore::new();
    create_tenant(&store, "e1", "tenant_limits").await;
    set_quota(&store, "e2", "tenant_limits", 5, 30, 50).await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_limits"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        quota.max_concurrent_runs, 5,
        "max_concurrent_runs must be 5"
    );
    assert_eq!(
        quota.max_sessions_per_hour, 30,
        "max_sessions_per_hour must be 30"
    );
    assert_eq!(quota.max_tasks_per_run, 50, "max_tasks_per_run must be 50");
}

#[tokio::test]
async fn quota_set_without_prior_tenant_event_still_stores() {
    // The quota projection doesn't require TenantCreated first.
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_direct", 3, 10, 5).await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_direct"))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(quota.max_concurrent_runs, 3);
}

// ── 3. Quota overwrite via second TenantQuotaSet ──────────────────────────────

#[tokio::test]
async fn second_quota_set_overwrites_limits() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_ow", 5, 20, 10).await;
    set_quota(&store, "e2", "tenant_ow", 10, 40, 20).await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_ow"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        quota.max_concurrent_runs, 10,
        "second set must override first"
    );
    assert_eq!(
        quota.max_sessions_per_hour, 40,
        "second set must override first"
    );
    assert_eq!(
        quota.max_tasks_per_run, 20,
        "second set must override first"
    );
}

// ── 4. TenantQuotaViolated is recorded in the event log ──────────────────────

#[tokio::test]
async fn quota_violated_event_appears_in_event_log() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_viol", 2, 10, 5).await;
    record_violation(
        &store,
        "e2",
        "tenant_viol",
        "max_concurrent_runs",
        3,
        2,
        5_000,
    )
    .await;

    let all = store.read_stream(None, 100).await.unwrap();
    let has_violation = all.iter().any(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::TenantQuotaViolated(ev)
            if ev.tenant_id.as_str() == "tenant_viol"
                && ev.quota_type == "max_concurrent_runs"
                && ev.current == 3
                && ev.limit == 2)
    });
    assert!(
        has_violation,
        "TenantQuotaViolated must be persisted in the event log"
    );
}

#[tokio::test]
async fn quota_violated_fields_are_preserved_verbatim() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_fields", 5, 10, 20).await;
    record_violation(
        &store,
        "e2",
        "tenant_fields",
        "max_sessions_per_hour",
        11,
        10,
        9_999,
    )
    .await;

    let all = store.read_stream(None, 100).await.unwrap();
    let violation = all
        .iter()
        .find(|e| matches!(&e.envelope.payload, RuntimeEvent::TenantQuotaViolated(_)))
        .expect("violation event must exist");

    if let RuntimeEvent::TenantQuotaViolated(ev) = &violation.envelope.payload {
        assert_eq!(ev.quota_type, "max_sessions_per_hour");
        assert_eq!(ev.current, 11);
        assert_eq!(ev.limit, 10);
        assert_eq!(ev.occurred_at_ms, 9_999);
    } else {
        panic!("expected TenantQuotaViolated payload");
    }
}

#[tokio::test]
async fn multiple_violations_are_all_recorded() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_multi_viol", 1, 1, 1).await;

    for i in 0..3u32 {
        record_violation(
            &store,
            &format!("e_viol_{i}"),
            "tenant_multi_viol",
            "max_concurrent_runs",
            2,
            1,
            1_000 + i as u64,
        )
        .await;
    }

    let all = store.read_stream(None, 100).await.unwrap();
    let violation_count = all
        .iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::TenantQuotaViolated(ev)
            if ev.tenant_id.as_str() == "tenant_multi_viol")
        })
        .count();
    assert_eq!(
        violation_count, 3,
        "all 3 violations must be in the event log"
    );
}

// ── 5. Quota scoping by tenant ────────────────────────────────────────────────

#[tokio::test]
async fn quota_is_scoped_to_individual_tenant() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_a", 5, 20, 10).await;
    set_quota(&store, "e2", "tenant_b", 10, 50, 25).await;

    let qa = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_a"))
        .await
        .unwrap()
        .unwrap();
    let qb = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_b"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        qa.max_concurrent_runs, 5,
        "tenant_a must have its own limit"
    );
    assert_eq!(
        qb.max_concurrent_runs, 10,
        "tenant_b must have its own limit"
    );
    assert_eq!(qa.max_sessions_per_hour, 20);
    assert_eq!(qb.max_sessions_per_hour, 50);
}

#[tokio::test]
async fn updating_one_tenants_quota_does_not_affect_another() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_x", 5, 20, 10).await;
    set_quota(&store, "e2", "tenant_y", 3, 15, 8).await;

    // Update only tenant_x.
    set_quota(&store, "e3", "tenant_x", 100, 200, 300).await;

    let qx = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_x"))
        .await
        .unwrap()
        .unwrap();
    let qy = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_y"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(
        qx.max_concurrent_runs, 100,
        "tenant_x limit must be updated"
    );
    assert_eq!(
        qy.max_concurrent_runs, 3,
        "tenant_y limit must be unchanged"
    );
}

#[tokio::test]
async fn violation_for_one_tenant_does_not_appear_for_another() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_iso_a", 1, 1, 1).await;
    set_quota(&store, "e2", "tenant_iso_b", 1, 1, 1).await;

    record_violation(
        &store,
        "e3",
        "tenant_iso_a",
        "max_concurrent_runs",
        2,
        1,
        1_000,
    )
    .await;

    let all = store.read_stream(None, 100).await.unwrap();
    let b_violations = all
        .iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::TenantQuotaViolated(ev)
            if ev.tenant_id.as_str() == "tenant_iso_b")
        })
        .count();
    assert_eq!(b_violations, 0, "tenant_b must have no violations");
}

// ── 6. Multiple quota types on the same tenant ────────────────────────────────

#[tokio::test]
async fn all_three_quota_types_coexist_on_same_tenant() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_3q", 7, 42, 99).await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_3q"))
        .await
        .unwrap()
        .unwrap();

    // All three limits independently stored.
    assert_eq!(quota.max_concurrent_runs, 7);
    assert_eq!(quota.max_sessions_per_hour, 42);
    assert_eq!(quota.max_tasks_per_run, 99);
}

#[tokio::test]
async fn violations_for_different_quota_types_on_same_tenant_are_independent() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_qtypes", 2, 3, 4).await;

    record_violation(
        &store,
        "e2",
        "tenant_qtypes",
        "max_concurrent_runs",
        3,
        2,
        1_000,
    )
    .await;
    record_violation(
        &store,
        "e3",
        "tenant_qtypes",
        "max_sessions_per_hour",
        4,
        3,
        2_000,
    )
    .await;
    record_violation(
        &store,
        "e4",
        "tenant_qtypes",
        "max_tasks_per_run",
        5,
        4,
        3_000,
    )
    .await;

    let all = store.read_stream(None, 100).await.unwrap();
    let violations: Vec<_> = all
        .iter()
        .filter_map(|e| {
            if let RuntimeEvent::TenantQuotaViolated(ev) = &e.envelope.payload {
                Some(ev)
            } else {
                None
            }
        })
        .collect();

    assert_eq!(
        violations.len(),
        3,
        "all three quota type violations must be in the log"
    );

    let types: Vec<&str> = violations.iter().map(|v| v.quota_type.as_str()).collect();
    assert!(types.contains(&"max_concurrent_runs"));
    assert!(types.contains(&"max_sessions_per_hour"));
    assert!(types.contains(&"max_tasks_per_run"));
}

#[tokio::test]
async fn zero_limits_are_stored_correctly() {
    // Zero limits (effectively blocking all activity) must be stored as-is.
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_zero", 0, 0, 0).await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_zero"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(quota.max_concurrent_runs, 0);
    assert_eq!(quota.max_sessions_per_hour, 0);
    assert_eq!(quota.max_tasks_per_run, 0);
}

// ── 7. Current counters reflect live state ────────────────────────────────────

#[tokio::test]
async fn current_active_runs_starts_at_zero() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_fresh", 5, 10, 20).await;

    let quota = QuotaReadModel::get_quota(&store, &TenantId::new("tenant_fresh"))
        .await
        .unwrap()
        .unwrap();

    assert_eq!(quota.current_active_runs, 0, "no runs created yet");
    assert_eq!(quota.sessions_this_hour, 0, "no sessions created yet");
}

// ── 8. Event log completeness ─────────────────────────────────────────────────

#[tokio::test]
async fn quota_set_and_violation_events_are_sequential_in_log() {
    let store = InMemoryStore::new();
    create_tenant(&store, "e1", "tenant_seq").await;
    set_quota(&store, "e2", "tenant_seq", 2, 5, 10).await;
    record_violation(
        &store,
        "e3",
        "tenant_seq",
        "max_concurrent_runs",
        3,
        2,
        1_000,
    )
    .await;

    let all = store.read_stream(None, 100).await.unwrap();
    assert_eq!(all.len(), 3, "all three events must be in the log");

    for w in all.windows(2) {
        assert!(
            w[0].position < w[1].position,
            "positions must be strictly increasing"
        );
    }
}

#[tokio::test]
async fn quota_set_event_has_correct_payload_in_log() {
    let store = InMemoryStore::new();
    set_quota(&store, "e1", "tenant_payload", 8, 16, 32).await;

    let all = store.read_stream(None, 100).await.unwrap();
    let set_event = all
        .iter()
        .find(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::TenantQuotaSet(ev)
            if ev.tenant_id.as_str() == "tenant_payload")
        })
        .expect("TenantQuotaSet must be in the log");

    if let RuntimeEvent::TenantQuotaSet(ev) = &set_event.envelope.payload {
        assert_eq!(ev.max_concurrent_runs, 8);
        assert_eq!(ev.max_sessions_per_hour, 16);
        assert_eq!(ev.max_tasks_per_run, 32);
    }
}
