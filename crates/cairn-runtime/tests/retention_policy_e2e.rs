//! RFC 002 - retention policy system end-to-end integration tests.

use std::sync::Arc;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RuntimeEvent, SessionCreated, SessionId,
    SessionState, SessionStateChanged, StateTransition, TenantId, WorkspaceId,
};
use cairn_runtime::error::RuntimeError;
use cairn_runtime::retention::RetentionService;
use cairn_runtime::services::{RetentionServiceImpl, TenantServiceImpl, WorkspaceServiceImpl};
use cairn_runtime::tenants::TenantService;
use cairn_runtime::workspaces::WorkspaceService;
use cairn_store::{EntityRef, EventLog, InMemoryStore};

fn tenant() -> TenantId { TenantId::new("t_retention") }
fn project() -> ProjectKey { ProjectKey::new("t_retention", "w_retention", "p_retention") }

fn evt(id: &str, payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload)
}

async fn setup() -> (Arc<InMemoryStore>, RetentionServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    TenantServiceImpl::new(store.clone()).create(tenant(), "Retention Tenant".to_owned()).await.unwrap();
    WorkspaceServiceImpl::new(store.clone())
        .create(tenant(), WorkspaceId::new("w_retention"), "Workspace".to_owned()).await.unwrap();
    let svc = RetentionServiceImpl::new(store.clone());
    (store, svc)
}

/// (1+2) Set 30-day policy, verify all fields.
#[tokio::test]
async fn set_policy_and_verify_retrieval() {
    let (_store, svc) = setup().await;
    let policy = svc.set_policy(tenant(), 30, 90, 50).await.unwrap();
    assert_eq!(policy.tenant_id, tenant());
    assert_eq!(policy.full_history_days, 30);
    assert_eq!(policy.current_state_days, 90);
    assert_eq!(policy.max_events_per_entity, 50);
    assert!(!policy.policy_id.is_empty());

    let fetched = svc.get_policy(&tenant()).await.unwrap().expect("policy must be retrievable");
    assert_eq!(fetched.full_history_days, 30);
    assert_eq!(fetched.current_state_days, 90);
    assert_eq!(fetched.max_events_per_entity, 50);
}

/// (3+4) Update policy — set_policy is an upsert keyed by tenant.
#[tokio::test]
async fn update_policy_replaces_previous() {
    let (_store, svc) = setup().await;
    let first = svc.set_policy(tenant(), 30, 90, 50).await.unwrap();
    let first_id = first.policy_id.clone();

    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let updated = svc.set_policy(tenant(), 7, 30, 10).await.unwrap();

    assert_eq!(updated.full_history_days, 7);
    assert_eq!(updated.current_state_days, 30);
    assert_eq!(updated.max_events_per_entity, 10);
    assert_ne!(updated.policy_id, first_id, "updated policy must have a new policy_id");

    let current = svc.get_policy(&tenant()).await.unwrap().unwrap();
    assert_eq!(current.full_history_days, 7);
    assert_eq!(current.max_events_per_entity, 10);
}

/// (5) apply_retention prunes events beyond max_events_per_entity (retention window).
#[tokio::test]
async fn apply_retention_enforces_retention_window() {
    let (store, svc) = setup().await;
    let session_id = SessionId::new("sess_retention");

    store.append(&[evt("evt_sess_create", RuntimeEvent::SessionCreated(SessionCreated {
        project: project(), session_id: session_id.clone(),
    }))]).await.unwrap();

    for i in 0..9usize {
        store.append(&[evt(&format!("evt_sess_state_{i}"),
            RuntimeEvent::SessionStateChanged(SessionStateChanged {
                project: project(),
                session_id: session_id.clone(),
                transition: StateTransition { from: Some(SessionState::Open), to: SessionState::Open },
            }),
        )]).await.unwrap();
    }

    let before = store.read_by_entity(&EntityRef::Session(session_id.clone()), None, 100).await.unwrap();
    assert_eq!(before.len(), 10, "10 events must exist before retention");

    svc.set_policy(tenant(), 0, 30, 3).await.unwrap();
    let result = svc.apply_retention(&tenant()).await.unwrap();

    assert!(result.events_pruned >= 7,
        "at least 7 events must be pruned (10 - 3); got: {}", result.events_pruned);
    assert!(result.entities_affected >= 1);

    let after = store.read_by_entity(&EntityRef::Session(session_id.clone()), None, 100).await.unwrap();
    assert!(after.len() <= 3,
        "at most 3 events must remain; got: {}", after.len());
    assert!(!after.is_empty(), "most recent events must be preserved");
}

/// apply_retention without a policy set must be a no-op.
#[tokio::test]
async fn apply_retention_without_policy_is_noop() {
    let (store, svc) = setup().await;
    let session_id = SessionId::new("sess_noop");
    store.append(&[evt("evt_noop", RuntimeEvent::SessionCreated(SessionCreated {
        project: project(), session_id: session_id.clone(),
    }))]).await.unwrap();

    let result = svc.apply_retention(&tenant()).await.unwrap();
    assert_eq!(result.events_pruned, 0);
    assert_eq!(result.entities_affected, 0);
}

/// set_policy for a non-existent tenant must return NotFound.
#[tokio::test]
async fn set_policy_for_missing_tenant_returns_not_found() {
    let store = Arc::new(InMemoryStore::new());
    let svc = RetentionServiceImpl::new(store);
    let err = svc.set_policy(TenantId::new("ghost"), 30, 90, 100).await.unwrap_err();
    assert!(
        matches!(err, RuntimeError::NotFound { entity: "tenant", .. }),
        "expected NotFound; got: {err:?}"
    );
}

/// get_policy before any policy is set returns None.
#[tokio::test]
async fn get_policy_without_any_set_returns_none() {
    let (_store, svc) = setup().await;
    let result = svc.get_policy(&tenant()).await.unwrap();
    assert!(result.is_none());
}

/// Two tenants have independent retention policies.
#[tokio::test]
async fn tenant_policies_are_independent() {
    let store = Arc::new(InMemoryStore::new());
    let t1 = TenantId::new("t_ret_a");
    let t2 = TenantId::new("t_ret_b");
    let tenant_svc = TenantServiceImpl::new(store.clone());
    tenant_svc.create(t1.clone(), "Tenant A".to_owned()).await.unwrap();
    tenant_svc.create(t2.clone(), "Tenant B".to_owned()).await.unwrap();
    let svc = RetentionServiceImpl::new(store.clone());

    svc.set_policy(t1.clone(), 30, 90, 100).await.unwrap();
    svc.set_policy(t2.clone(), 7, 30, 10).await.unwrap();

    let p1 = svc.get_policy(&t1).await.unwrap().unwrap();
    let p2 = svc.get_policy(&t2).await.unwrap().unwrap();
    assert_eq!(p1.full_history_days, 30);
    assert_eq!(p1.max_events_per_entity, 100);
    assert_eq!(p2.full_history_days, 7);
    assert_eq!(p2.max_events_per_entity, 10);

    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    svc.set_policy(t1.clone(), 1, 7, 5).await.unwrap();
    let p2_after = svc.get_policy(&t2).await.unwrap().unwrap();
    assert_eq!(p2_after.full_history_days, 7, "updating t1 must not affect t2");
    assert_eq!(p2_after.max_events_per_entity, 10);
}

/// max_events_per_entity=0 in the policy must skip pruning.
#[tokio::test]
async fn zero_max_events_per_entity_skips_pruning() {
    let (store, svc) = setup().await;
    let session_id = SessionId::new("sess_zero");

    for i in 0..5usize {
        store.append(&[evt(&format!("evt_zero_{i}"),
            RuntimeEvent::SessionCreated(SessionCreated {
                project: project(), session_id: session_id.clone(),
            }),
        )]).await.unwrap();
    }

    // max_events_per_entity=0 means unlimited — nothing pruned.
    svc.set_policy(tenant(), 30, 90, 0).await.unwrap();
    let result = svc.apply_retention(&tenant()).await.unwrap();
    assert_eq!(result.events_pruned, 0, "max=0 must skip pruning (unlimited)");
}
