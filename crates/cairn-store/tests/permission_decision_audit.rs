//! RFC 002 permission decision audit integration tests.
//!
//! PermissionDecisionRecorded is a durable audit event with no projection
//! read model. It is stored in the event log and retrieved via read_stream
//! filtered by decision_id, invocation_id, or principal. It has no project
//! field (returns the _system sentinel key) and no primary entity ref.
//!
//! Validates:
//! - PermissionDecisionRecorded(allowed=true) persists in the log.
//! - PermissionDecisionRecorded(allowed=false) persists and is distinguishable.
//! - read_by_entity on ToolInvocation returns the linked invocation event;
//!   permission decisions for that invocation are scoped via invocation_id.
//! - invocation_id links a permission decision to a specific tool call.
//! - Decisions from different projects/runs can be isolated by principal,
//!   action, and resource — proving cross-context isolation.

use std::sync::Arc;

use cairn_domain::events::{PermissionDecisionRecorded, ToolInvocationStarted};
use cairn_domain::policy::ExecutionClass;
use cairn_domain::tool_invocation::ToolInvocationTarget;
use cairn_domain::{
    EventEnvelope, EventId, EventSource, ProjectKey, RunCreated, RunId, RuntimeEvent,
    SessionCreated, SessionId, ToolInvocationId,
};
use cairn_store::{event_log::EntityRef, EventLog, InMemoryStore};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project_a() -> ProjectKey {
    ProjectKey::new("tenant_perm", "ws_perm", "proj_a")
}
fn ev<P: Into<RuntimeEvent>>(id: &str, payload: P) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new(id), EventSource::Runtime, payload.into())
}

fn allow_event(
    decision_id: &str,
    principal: &str,
    action: &str,
    resource: &str,
    invocation_id: Option<&str>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_perm_allow_{decision_id}"),
        RuntimeEvent::PermissionDecisionRecorded(PermissionDecisionRecorded {
            decision_id: decision_id.to_owned(),
            principal: principal.to_owned(),
            action: action.to_owned(),
            resource: resource.to_owned(),
            allowed: true,
            recorded_at_ms: ts,
            invocation_id: invocation_id.map(str::to_owned),
        }),
    )
}

fn deny_event(
    decision_id: &str,
    principal: &str,
    action: &str,
    resource: &str,
    invocation_id: Option<&str>,
    ts: u64,
) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_perm_deny_{decision_id}"),
        RuntimeEvent::PermissionDecisionRecorded(PermissionDecisionRecorded {
            decision_id: decision_id.to_owned(),
            principal: principal.to_owned(),
            action: action.to_owned(),
            resource: resource.to_owned(),
            allowed: false,
            recorded_at_ms: ts,
            invocation_id: invocation_id.map(str::to_owned),
        }),
    )
}

fn tool_invocation_event(invocation_id: &str, project: ProjectKey) -> EventEnvelope<RuntimeEvent> {
    ev(
        &format!("evt_inv_{invocation_id}"),
        RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
            project,
            invocation_id: ToolInvocationId::new(invocation_id),
            session_id: None,
            run_id: None,
            task_id: None,
            target: ToolInvocationTarget::Builtin {
                tool_name: "file_write".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            prompt_release_id: None,
            requested_at_ms: 1_000,
            started_at_ms: 1_100,
            args_json: None
        }),
    )
}

/// Extract all PermissionDecisionRecorded events from the global log.
async fn all_permission_events(
    store: &Arc<InMemoryStore>,
) -> Vec<cairn_store::event_log::StoredEvent> {
    EventLog::read_stream(store.as_ref(), None, 200)
        .await
        .unwrap()
        .into_iter()
        .filter(|e| {
            matches!(
                &e.envelope.payload,
                RuntimeEvent::PermissionDecisionRecorded(_)
            )
        })
        .collect()
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) + (2): PermissionDecisionRecorded(allowed=true) persists in the event log
/// with all fields preserved.
#[tokio::test]
async fn permission_allowed_persists_in_log() {
    let store = Arc::new(InMemoryStore::new());

    // (1) Append an allowed decision.
    store
        .append(&[allow_event(
            "dec_001",
            "agent:run_1",
            "write",
            "file:/tmp/output.json",
            None,
            5_000,
        )])
        .await
        .unwrap();

    // (2) Verify it persists.
    let perms = all_permission_events(&store).await;
    assert_eq!(perms.len(), 1, "one permission event must be stored");

    if let RuntimeEvent::PermissionDecisionRecorded(d) = &perms[0].envelope.payload {
        assert_eq!(d.decision_id, "dec_001");
        assert_eq!(d.principal, "agent:run_1");
        assert_eq!(d.action, "write");
        assert_eq!(d.resource, "file:/tmp/output.json");
        assert!(d.allowed, "decision must be allowed=true");
        assert_eq!(d.recorded_at_ms, 5_000);
        assert!(d.invocation_id.is_none());
    } else {
        panic!("expected PermissionDecisionRecorded");
    }
}

/// (3): Denied decision(allowed=false) is stored and distinguishable from allowed.
#[tokio::test]
async fn permission_denied_is_stored_and_distinguishable() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            allow_event("dec_a", "agent:run_2", "read", "db://users", None, 1_000),
            deny_event("dec_b", "agent:run_2", "write", "db://users", None, 2_000),
            allow_event(
                "dec_c",
                "agent:run_2",
                "read",
                "file:/config.json",
                None,
                3_000,
            ),
        ])
        .await
        .unwrap();

    let perms = all_permission_events(&store).await;
    assert_eq!(perms.len(), 3);

    let allowed_count = perms.iter().filter(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d) if d.allowed)
    }).count();
    let denied_count = perms.iter().filter(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d) if !d.allowed)
    }).count();

    assert_eq!(allowed_count, 2, "two allowed decisions");
    assert_eq!(denied_count, 1, "one denied decision");

    // The denied decision carries the correct action and resource.
    let denied = perms.iter().find(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
            if !d.allowed && d.action == "write")
    });
    assert!(denied.is_some(), "denied write decision must be findable");
    if let RuntimeEvent::PermissionDecisionRecorded(d) = &denied.unwrap().envelope.payload {
        assert_eq!(d.decision_id, "dec_b");
        assert_eq!(d.resource, "db://users");
    }
}

/// (4): read_by_entity on ToolInvocation returns the linked invocation event.
/// Permission decisions with matching invocation_id can then be found in the
/// global log — demonstrating the scoping chain.
#[tokio::test]
async fn read_by_entity_scoping_for_tool_invocation() {
    let store = Arc::new(InMemoryStore::new());
    let inv_id = "inv_abc123";

    // Seed a run and tool invocation.
    store
        .append(&[
            ev(
                "sess_scope",
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project_a(),
                    session_id: SessionId::new("sess_perm_scope"),
                }),
            ),
            ev(
                "run_scope",
                RuntimeEvent::RunCreated(RunCreated {
                    project: project_a(),
                    session_id: SessionId::new("sess_perm_scope"),
                    run_id: RunId::new("run_perm_scope"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            tool_invocation_event(inv_id, project_a()),
        ])
        .await
        .unwrap();

    // Append a permission decision linked to this invocation.
    store
        .append(&[allow_event(
            "dec_scoped",
            "agent:run_perm_scope",
            "execute",
            "tool:file_write",
            Some(inv_id),
            10_000,
        )])
        .await
        .unwrap();

    // (4) read_by_entity(ToolInvocation) returns the ToolInvocationStarted event.
    let inv_events = EventLog::read_by_entity(
        store.as_ref(),
        &EntityRef::ToolInvocation(ToolInvocationId::new(inv_id)),
        None,
        100,
    )
    .await
    .unwrap();
    assert_eq!(
        inv_events.len(),
        1,
        "one ToolInvocationStarted event for this invocation"
    );
    assert!(
        matches!(&inv_events[0].envelope.payload, RuntimeEvent::ToolInvocationStarted(t)
            if t.invocation_id == ToolInvocationId::new(inv_id)),
        "entity-scoped read must return the correct ToolInvocationStarted"
    );

    // The permission decision is in the global log, linkable by invocation_id.
    let perm_for_inv: Vec<_> = all_permission_events(&store)
        .await
        .into_iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
                if d.invocation_id.as_deref() == Some(inv_id))
        })
        .collect();
    assert_eq!(
        perm_for_inv.len(),
        1,
        "one permission decision linked to this invocation"
    );
}

/// (5): invocation_id links a permission decision to its tool call.
///
/// Two different tool invocations each have their own permission decision.
/// Filtering by invocation_id gives exactly the right decision for each.
#[tokio::test]
async fn invocation_id_links_permission_to_tool_call() {
    let store = Arc::new(InMemoryStore::new());

    store
        .append(&[
            tool_invocation_event("inv_tool_1", project_a()),
            tool_invocation_event("inv_tool_2", project_a()),
            allow_event(
                "dec_tool_1",
                "agent:x",
                "read",
                "db://records",
                Some("inv_tool_1"),
                1_000,
            ),
            deny_event(
                "dec_tool_2",
                "agent:x",
                "write",
                "db://records",
                Some("inv_tool_2"),
                2_000,
            ),
        ])
        .await
        .unwrap();

    let all_perms = all_permission_events(&store).await;
    assert_eq!(all_perms.len(), 2);

    // Permission for tool 1: allowed read.
    let perm_1: Vec<_> = all_perms
        .iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
            if d.invocation_id.as_deref() == Some("inv_tool_1"))
        })
        .collect();
    assert_eq!(perm_1.len(), 1);
    if let RuntimeEvent::PermissionDecisionRecorded(d) = &perm_1[0].envelope.payload {
        assert!(d.allowed, "tool 1 permission must be allowed");
        assert_eq!(d.action, "read");
    }

    // Permission for tool 2: denied write.
    let perm_2: Vec<_> = all_perms
        .iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
            if d.invocation_id.as_deref() == Some("inv_tool_2"))
        })
        .collect();
    assert_eq!(perm_2.len(), 1);
    if let RuntimeEvent::PermissionDecisionRecorded(d) = &perm_2[0].envelope.payload {
        assert!(!d.allowed, "tool 2 permission must be denied");
        assert_eq!(d.action, "write");
    }

    // Permission without invocation_id: no invocation_id set.
    store
        .append(&[allow_event(
            "dec_no_inv",
            "agent:y",
            "read",
            "file:/log",
            None,
            3_000,
        )])
        .await
        .unwrap();
    let no_inv: Vec<_> = all_permission_events(&store)
        .await
        .into_iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
                if d.invocation_id.is_none())
        })
        .collect();
    assert_eq!(
        no_inv.len(),
        1,
        "one permission decision without invocation_id"
    );
}

/// (6): Cross-project isolation via principal and action filtering.
///
/// PermissionDecisionRecorded has no project field (returns _system key).
/// Isolation is enforced by the principal (which carries run/project context)
/// and by reading the global log with principal-based filtering.
#[tokio::test]
async fn cross_project_isolation_via_principal_filtering() {
    let store = Arc::new(InMemoryStore::new());

    // Project A decisions (principal contains "proj_a").
    store
        .append(&[
            allow_event(
                "dec_a1",
                "proj_a:run_1",
                "read",
                "file:/a.json",
                None,
                1_000,
            ),
            allow_event(
                "dec_a2",
                "proj_a:run_1",
                "write",
                "file:/a.json",
                None,
                2_000,
            ),
            deny_event("dec_a3", "proj_a:run_1", "exec", "tool:shell", None, 3_000),
        ])
        .await
        .unwrap();

    // Project B decisions (principal contains "proj_b").
    store
        .append(&[
            allow_event(
                "dec_b1",
                "proj_b:run_2",
                "read",
                "file:/b.json",
                None,
                4_000,
            ),
            deny_event(
                "dec_b2",
                "proj_b:run_2",
                "write",
                "db://secret",
                None,
                5_000,
            ),
        ])
        .await
        .unwrap();

    let all_perms = all_permission_events(&store).await;
    assert_eq!(all_perms.len(), 5, "all 5 decisions in the global log");

    // Isolate project A's decisions by principal prefix.
    let proj_a_perms: Vec<_> = all_perms
        .iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
            if d.principal.starts_with("proj_a:"))
        })
        .collect();
    assert_eq!(proj_a_perms.len(), 3, "project A must have 3 decisions");

    // Project A: 2 allowed, 1 denied.
    let a_allowed = proj_a_perms.iter().filter(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d) if d.allowed)
    }).count();
    let a_denied = proj_a_perms.iter().filter(|e| {
        matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d) if !d.allowed)
    }).count();
    assert_eq!(a_allowed, 2, "project A: 2 allowed");
    assert_eq!(a_denied, 1, "project A: 1 denied");

    // Isolate project B's decisions.
    let proj_b_perms: Vec<_> = all_perms
        .iter()
        .filter(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
            if d.principal.starts_with("proj_b:"))
        })
        .collect();
    assert_eq!(proj_b_perms.len(), 2, "project B must have 2 decisions");

    // No cross-contamination: project A decisions never appear when filtering project B.
    assert!(
        !proj_b_perms.iter().any(|e| {
            matches!(&e.envelope.payload, RuntimeEvent::PermissionDecisionRecorded(d)
                if d.principal.starts_with("proj_a:"))
        }),
        "project B's filtered decisions must not contain project A's decisions"
    );
}

/// Audit completeness: decision_id, action, resource, timestamp are all preserved.
#[tokio::test]
async fn permission_audit_fields_are_fully_preserved() {
    let store = Arc::new(InMemoryStore::new());

    let inv_id = "inv_audit_1";
    store
        .append(&[deny_event(
            "audit_dec_001",
            "operator:alice",
            "delete",
            "run:run_sensitive_abc/outputs",
            Some(inv_id),
            1_712_345_678_000,
        )])
        .await
        .unwrap();

    let perms = all_permission_events(&store).await;
    assert_eq!(perms.len(), 1);

    if let RuntimeEvent::PermissionDecisionRecorded(d) = &perms[0].envelope.payload {
        assert_eq!(d.decision_id, "audit_dec_001");
        assert_eq!(d.principal, "operator:alice");
        assert_eq!(d.action, "delete");
        assert_eq!(d.resource, "run:run_sensitive_abc/outputs");
        assert!(!d.allowed, "must be denied");
        assert_eq!(d.recorded_at_ms, 1_712_345_678_000);
        assert_eq!(d.invocation_id.as_deref(), Some(inv_id));
    }
}
