//! RFC 002 audit log system end-to-end integration test.
//!
//! The InMemoryStore's AuditLogReadModel is a read-time stub; audit facts are
//! durably stored as AuditLogEntryRecorded events in the event log.  This test
//! uses store.read_stream() to query those events — the same data source that
//! a real projection rebuild would consume.
//!
//! Tests:
//!   (1) perform several operations (create tenant, create run, complete run)
//!   (2) query the event log and verify AuditLogEntryRecorded entries exist
//!   (3) filter audit events by resource_type
//!   (4) verify entries have correct actor, resource_id, and timestamp
//!   (5) AuditOutcome::Failure is recorded and verifiable
//!   (6) multiple records from different tenants are isolated in the log
//!   (7) AuditService::record() returns a fully-populated AuditLogEntry

use std::sync::Arc;

use cairn_domain::{AuditOutcome, AuditLogEntryRecorded, RuntimeEvent, TenantId};
use cairn_runtime::AuditService;
use cairn_runtime::services::AuditServiceImpl;
use cairn_store::{EventLog, InMemoryStore};

fn tenant() -> TenantId {
    TenantId::new("tenant_audit")
}

fn setup() -> (Arc<InMemoryStore>, AuditServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let audit = AuditServiceImpl::new(store.clone());
    (store, audit)
}

/// Extract all AuditLogEntryRecorded payloads from the event stream.
async fn read_audit_events(store: &Arc<InMemoryStore>) -> Vec<AuditLogEntryRecorded> {
    store
        .read_stream(None, 200)
        .await
        .unwrap()
        .into_iter()
        .filter_map(|s| {
            if let RuntimeEvent::AuditLogEntryRecorded(e) = s.envelope.payload {
                Some(e)
            } else {
                None
            }
        })
        .collect()
}

// ── (1)+(2) Record operations and verify events exist ────────────────────

#[tokio::test]
async fn three_operations_produce_three_audit_events() {
    let (store, audit) = setup();

    // Simulate: create tenant
    audit.record(
        tenant(), "op_system".to_owned(), "create_tenant".to_owned(),
        "tenant".to_owned(), "tenant_audit".to_owned(),
        AuditOutcome::Success, serde_json::json!({"name": "Audit Tenant"}),
    ).await.unwrap();

    // Simulate: create run
    audit.record(
        tenant(), "op_alice".to_owned(), "create_run".to_owned(),
        "run".to_owned(), "run_audit_1".to_owned(),
        AuditOutcome::Success, serde_json::json!({"session_id": "sess_1"}),
    ).await.unwrap();

    // Simulate: complete run
    audit.record(
        tenant(), "op_alice".to_owned(), "complete_run".to_owned(),
        "run".to_owned(), "run_audit_1".to_owned(),
        AuditOutcome::Success, serde_json::json!({}),
    ).await.unwrap();

    let events = read_audit_events(&store).await;
    assert_eq!(events.len(), 3, "one AuditLogEntryRecorded per record() call");

    let actions: Vec<&str> = events.iter().map(|e| e.action.as_str()).collect();
    assert!(actions.contains(&"create_tenant"));
    assert!(actions.contains(&"create_run"));
    assert!(actions.contains(&"complete_run"));
}

// ── (3) Filter audit events by resource type ─────────────────────────────

#[tokio::test]
async fn filter_audit_events_by_resource_type() {
    let (store, audit) = setup();

    audit.record(tenant(), "op_sys".to_owned(), "create_tenant".to_owned(),
        "tenant".to_owned(), "t1".to_owned(), AuditOutcome::Success, serde_json::json!({}))
        .await.unwrap();
    audit.record(tenant(), "op_alice".to_owned(), "create_run".to_owned(),
        "run".to_owned(), "run_x".to_owned(), AuditOutcome::Success, serde_json::json!({}))
        .await.unwrap();
    audit.record(tenant(), "op_alice".to_owned(), "complete_run".to_owned(),
        "run".to_owned(), "run_x".to_owned(), AuditOutcome::Success, serde_json::json!({}))
        .await.unwrap();

    let all_events = read_audit_events(&store).await;

    // Filter to "run" resource type.
    let run_events: Vec<_> = all_events.iter()
        .filter(|e| e.resource_type == "run")
        .collect();
    assert_eq!(run_events.len(), 2, "two run-type audit events expected");
    assert!(run_events.iter().all(|e| e.resource_id == "run_x"));

    // Filter to "tenant" resource type.
    let tenant_events: Vec<_> = all_events.iter()
        .filter(|e| e.resource_type == "tenant")
        .collect();
    assert_eq!(tenant_events.len(), 1);
    assert_eq!(tenant_events[0].action, "create_tenant");
}

// ── (4) Entries have correct actor, resource_id, and timestamp ────────────

#[tokio::test]
async fn audit_event_carries_correct_actor_resource_and_timestamp() {
    let (store, audit) = setup();

    let before_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    let entry = audit.record(
        tenant(), "op_bob".to_owned(), "approve_release".to_owned(),
        "prompt_release".to_owned(), "rel_007".to_owned(),
        AuditOutcome::Success, serde_json::json!({"approved_by": "op_bob"}),
    ).await.unwrap();

    let after_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;

    // Verify returned AuditLogEntry fields.
    assert_eq!(entry.actor_id, "op_bob");
    assert_eq!(entry.resource_id, "rel_007");
    assert_eq!(entry.tenant_id, tenant());
    assert_eq!(entry.action, "approve_release");
    assert!(!entry.entry_id.is_empty());
    assert!(
        entry.occurred_at_ms >= before_ms && entry.occurred_at_ms <= after_ms,
        "occurred_at_ms must fall within the test window"
    );

    // Verify the persisted event matches.
    let events = read_audit_events(&store).await;
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert_eq!(ev.entry_id, entry.entry_id);
    assert_eq!(ev.actor_id, "op_bob");
    assert_eq!(ev.resource_type, "prompt_release");
    assert_eq!(ev.resource_id, "rel_007");
    assert_eq!(ev.occurred_at_ms, entry.occurred_at_ms);
}

// ── (5) Failure outcome recorded ─────────────────────────────────────────

#[tokio::test]
async fn failure_outcome_is_stored_in_audit_event() {
    let (store, audit) = setup();

    let entry = audit.record(
        tenant(), "op_carol".to_owned(), "delete_credential".to_owned(),
        "credential".to_owned(), "cred_99".to_owned(),
        AuditOutcome::Failure, serde_json::json!({"reason": "permission_denied"}),
    ).await.unwrap();

    assert_eq!(entry.outcome, AuditOutcome::Failure);

    let events = read_audit_events(&store).await;
    let ev = events.iter().find(|e| e.resource_id == "cred_99").unwrap();
    assert_eq!(ev.outcome, AuditOutcome::Failure);
    assert_eq!(ev.actor_id, "op_carol");
}

// ── (6) Tenant isolation in the raw event log ─────────────────────────────

#[tokio::test]
async fn audit_events_from_different_tenants_coexist_in_log() {
    let (store, audit) = setup();

    audit.record(
        TenantId::new("tenant_a"), "op_1".to_owned(), "action_a".to_owned(),
        "run".to_owned(), "run_a".to_owned(),
        AuditOutcome::Success, serde_json::json!({}),
    ).await.unwrap();
    audit.record(
        TenantId::new("tenant_b"), "op_2".to_owned(), "action_b".to_owned(),
        "run".to_owned(), "run_b".to_owned(),
        AuditOutcome::Success, serde_json::json!({}),
    ).await.unwrap();

    let all = read_audit_events(&store).await;
    assert_eq!(all.len(), 2, "both tenant events are in the log");

    // Tenant-scoped filtering.
    let a_events: Vec<_> = all.iter().filter(|e| e.tenant_id == TenantId::new("tenant_a")).collect();
    let b_events: Vec<_> = all.iter().filter(|e| e.tenant_id == TenantId::new("tenant_b")).collect();
    assert_eq!(a_events.len(), 1);
    assert_eq!(b_events.len(), 1);
    assert_eq!(a_events[0].resource_id, "run_a");
    assert_eq!(b_events[0].resource_id, "run_b");
}

// ── (7) record() returns fully-populated AuditLogEntry ───────────────────

#[tokio::test]
async fn record_returns_fully_populated_audit_log_entry() {
    let (_, audit) = setup();

    let entry = audit.record(
        tenant(),
        "op_dave".to_owned(),
        "create_workspace".to_owned(),
        "workspace".to_owned(),
        "ws_new".to_owned(),
        AuditOutcome::Success,
        serde_json::json!({"plan": "pro"}),
    ).await.unwrap();

    assert!(!entry.entry_id.is_empty(), "entry_id must be non-empty");
    assert_eq!(entry.tenant_id, tenant());
    assert_eq!(entry.actor_id, "op_dave");
    assert_eq!(entry.action, "create_workspace");
    assert_eq!(entry.resource_type, "workspace");
    assert_eq!(entry.resource_id, "ws_new");
    assert_eq!(entry.outcome, AuditOutcome::Success);
    assert!(entry.occurred_at_ms > 0, "timestamp must be set");
    assert_eq!(entry.metadata["plan"], "pro");
}

// ── Ordering: events appear in insertion order ────────────────────────────

#[tokio::test]
async fn audit_events_preserve_insertion_order() {
    let (store, audit) = setup();

    for i in 0u32..5 {
        audit.record(
            tenant(), "op_sys".to_owned(), format!("action_{i}"),
            "run".to_owned(), format!("run_{i}"),
            AuditOutcome::Success, serde_json::json!({}),
        ).await.unwrap();
    }

    let events = read_audit_events(&store).await;
    assert_eq!(events.len(), 5);

    // Timestamps must be non-decreasing.
    for window in events.windows(2) {
        assert!(
            window[0].occurred_at_ms <= window[1].occurred_at_ms,
            "audit events must have non-decreasing timestamps"
        );
    }

    // Resource IDs in insertion order.
    for (i, ev) in events.iter().enumerate() {
        assert_eq!(ev.resource_id, format!("run_{i}"));
    }
}
