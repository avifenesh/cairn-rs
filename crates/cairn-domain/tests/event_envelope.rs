//! EventEnvelope contract tests (RFC 002).
//!
//! Validates the canonical event envelope structure, serialization contract,
//! and payload-level helpers.  Every event in the system wraps its payload in
//! this envelope; these tests prove the contract that downstream consumers
//! (store, SSE, graph) can rely on.
//!
//! Note on EventSource variants:
//!   The manager referred to User/System/Agent, but the actual domain enum is:
//!   Runtime | System | Operator { operator_id } | Scheduler | ExternalWorker { worker }
//!   Tests cover all five variants.
//!
//! Serde contracts:
//!   EventSource   → internally tagged with "source_type", snake_case
//!   OwnershipKey  → internally tagged with "scope",       snake_case
//!   RuntimeEvent  → internally tagged with "event",       snake_case
//!   RuntimeEntityRef → internally tagged with "entity",   snake_case

use cairn_domain::{
    ApprovalId, CheckpointId, CommandId, EventEnvelope, EventId, EventSource,
    MailboxMessageId, OperatorId, ProjectId, ProjectKey, RunCreated, RunId,
    RuntimeEvent, SessionCreated, SessionId, TaskCreated, TaskId, TenantId,
    WorkspaceId,
};
use cairn_domain::errors::RuntimeEntityRef;
use cairn_domain::tenancy::{OwnershipKey, TenantKey, WorkspaceKey};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey {
        tenant_id: TenantId::new("t_env"),
        workspace_id: WorkspaceId::new("w_env"),
        project_id: ProjectId::new("p_env"),
    }
}

fn session_evt(event_id: &str) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(event_id),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project: project(),
            session_id: SessionId::new("sess_env"),
        }),
    )
}

// ── 1. EventEnvelope with all fields populated ────────────────────────────────

#[test]
fn event_envelope_carries_all_fields() {
    let envelope = EventEnvelope::<RuntimeEvent> {
        event_id:       EventId::new("evt_full"),
        source:         EventSource::Operator { operator_id: OperatorId::new("op_alice") },
        ownership:      OwnershipKey::Project(project()),
        causation_id:   Some(CommandId::new("cmd_001")),
        correlation_id: Some("corr-xyz-123".to_owned()),
        payload: RuntimeEvent::SessionCreated(SessionCreated {
            project:    project(),
            session_id: SessionId::new("sess_full"),
        }),
    };

    assert_eq!(envelope.event_id.as_str(), "evt_full");
    assert!(matches!(
        envelope.source,
        EventSource::Operator { ref operator_id } if operator_id.as_str() == "op_alice"
    ));
    assert!(envelope.causation_id.is_some());
    assert_eq!(envelope.correlation_id.as_deref(), Some("corr-xyz-123"));
    assert!(matches!(envelope.ownership, OwnershipKey::Project(_)));
}

#[test]
fn event_envelope_new_constructor_leaves_optional_fields_none() {
    let env = EventEnvelope::new(
        EventId::new("evt_new"),
        EventSource::Runtime,
        OwnershipKey::Project(project()),
        RuntimeEvent::SessionCreated(SessionCreated {
            project:    project(),
            session_id: SessionId::new("s"),
        }),
    );

    assert!(env.causation_id.is_none(), "causation_id starts as None");
    assert!(env.correlation_id.is_none(), "correlation_id starts as None");
}

#[test]
fn with_causation_id_and_correlation_id_builders() {
    let env = EventEnvelope::new(
        EventId::new("e"),
        EventSource::System,
        OwnershipKey::System,
        RuntimeEvent::SessionCreated(SessionCreated {
            project:    project(),
            session_id: SessionId::new("s"),
        }),
    )
    .with_causation_id(CommandId::new("cmd_causation"))
    .with_correlation_id("corr-builder");

    assert_eq!(
        env.causation_id.as_ref().map(|c| c.as_str()),
        Some("cmd_causation")
    );
    assert_eq!(env.correlation_id.as_deref(), Some("corr-builder"));
}

// ── 2. JSON serialization round-trip ─────────────────────────────────────────

#[test]
fn event_envelope_json_round_trip_minimal() {
    let original = session_evt("evt_rt");
    let json = serde_json::to_string(&original).unwrap();
    let back: EventEnvelope<RuntimeEvent> = serde_json::from_str(&json).unwrap();

    assert_eq!(back.event_id, original.event_id);
    assert_eq!(back.source,   original.source);
    assert_eq!(back.payload,  original.payload);
    assert_eq!(back.causation_id,   None);
    assert_eq!(back.correlation_id, None);
}

#[test]
fn event_envelope_json_round_trip_all_fields() {
    let original = EventEnvelope::<RuntimeEvent> {
        event_id:       EventId::new("evt_all"),
        source:         EventSource::Operator { operator_id: OperatorId::new("op_bob") },
        ownership:      OwnershipKey::Project(project()),
        causation_id:   Some(CommandId::new("cmd_rt")),
        correlation_id: Some("correlation-123".to_owned()),
        payload: RuntimeEvent::RunCreated(RunCreated {
            project:           project(),
            session_id:        SessionId::new("sess_rt"),
            run_id:            RunId::new("run_rt"),
            parent_run_id:     None,
            prompt_release_id: None,
            agent_role_id:     None,
        }),
    };

    let json = serde_json::to_string(&original).unwrap();
    let back: EventEnvelope<RuntimeEvent> = serde_json::from_str(&json).unwrap();

    assert_eq!(back, original);
}

#[test]
fn event_envelope_json_contains_tagged_source_type() {
    let env = session_evt("evt_json");
    let json = serde_json::to_value(&env).unwrap();

    // EventSource is tagged with "source_type".
    assert_eq!(json["source"]["source_type"], "runtime",
        "EventSource::Runtime must serialize as source_type=runtime");
}

#[test]
fn event_envelope_json_contains_tagged_ownership_scope() {
    let env = session_evt("evt_scope");
    let json = serde_json::to_value(&env).unwrap();

    // OwnershipKey is tagged with "scope".
    assert_eq!(json["ownership"]["scope"], "project",
        "OwnershipKey::Project must serialize as scope=project");
}

#[test]
fn event_envelope_json_payload_has_event_tag() {
    let env = session_evt("evt_payload");
    let json = serde_json::to_value(&env).unwrap();

    // RuntimeEvent is tagged with "event".
    assert_eq!(json["payload"]["event"], "session_created",
        "SessionCreated must serialize as event=session_created");
}

// ── 3. EventSource variants ───────────────────────────────────────────────────

#[test]
fn event_source_runtime_serializes_correctly() {
    let s = EventSource::Runtime;
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["source_type"], "runtime");
    let back: EventSource = serde_json::from_value(json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn event_source_system_serializes_correctly() {
    let s = EventSource::System;
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["source_type"], "system");
    let back: EventSource = serde_json::from_value(json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn event_source_operator_carries_operator_id() {
    let s = EventSource::Operator { operator_id: OperatorId::new("op_carol") };
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["source_type"], "operator");
    assert_eq!(json["operator_id"], "op_carol");
    let back: EventSource = serde_json::from_value(json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn event_source_scheduler_serializes_correctly() {
    let s = EventSource::Scheduler;
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["source_type"], "scheduler");
    let back: EventSource = serde_json::from_value(json).unwrap();
    assert_eq!(back, s);
}

#[test]
fn event_source_external_worker_carries_worker_name() {
    let s = EventSource::ExternalWorker { worker: "worker-node-7".to_owned() };
    let json = serde_json::to_value(&s).unwrap();
    assert_eq!(json["source_type"], "external_worker");
    assert_eq!(json["worker"], "worker-node-7");
    let back: EventSource = serde_json::from_value(json).unwrap();
    assert_eq!(back, s);
}

// ── 4. OwnershipKey variants ──────────────────────────────────────────────────

#[test]
fn ownership_key_system_serializes_correctly() {
    let k = OwnershipKey::System;
    let json = serde_json::to_value(&k).unwrap();
    assert_eq!(json["scope"], "system");
    let back: OwnershipKey = serde_json::from_value(json).unwrap();
    assert_eq!(back, k);
}

#[test]
fn ownership_key_tenant_carries_tenant_id() {
    let k = OwnershipKey::Tenant(TenantKey { tenant_id: TenantId::new("t_own") });
    let json = serde_json::to_value(&k).unwrap();
    assert_eq!(json["scope"], "tenant");
    assert_eq!(json["tenant_id"], "t_own");
    let back: OwnershipKey = serde_json::from_value(json).unwrap();
    assert_eq!(back, k);
}

#[test]
fn ownership_key_workspace_carries_tenant_and_workspace_ids() {
    let k = OwnershipKey::Workspace(WorkspaceKey {
        tenant_id:    TenantId::new("t_ws"),
        workspace_id: WorkspaceId::new("w_ws"),
    });
    let json = serde_json::to_value(&k).unwrap();
    assert_eq!(json["scope"], "workspace");
    assert_eq!(json["tenant_id"],    "t_ws");
    assert_eq!(json["workspace_id"], "w_ws");
    let back: OwnershipKey = serde_json::from_value(json).unwrap();
    assert_eq!(back, k);
}

#[test]
fn ownership_key_project_carries_full_project_key() {
    let k = OwnershipKey::Project(project());
    let json = serde_json::to_value(&k).unwrap();
    assert_eq!(json["scope"],        "project");
    assert_eq!(json["tenant_id"],    "t_env");
    assert_eq!(json["workspace_id"], "w_env");
    assert_eq!(json["project_id"],   "p_env");
    let back: OwnershipKey = serde_json::from_value(json).unwrap();
    assert_eq!(back, k);
}

// ── 5. project() extraction from RuntimeEvent ─────────────────────────────────

#[test]
fn for_runtime_event_sets_ownership_from_payload_project() {
    let env = EventEnvelope::for_runtime_event(
        EventId::new("e"),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project:    project(),
            session_id: SessionId::new("s"),
        }),
    );

    // Ownership must be automatically derived from the payload's project key.
    assert!(
        matches!(&env.ownership, OwnershipKey::Project(p) if p == &project()),
        "for_runtime_event must set ownership=Project(payload.project())"
    );
}

#[test]
fn project_method_returns_payload_project_key() {
    let env = session_evt("e_proj");
    assert_eq!(env.project(), &project());
    assert_eq!(env.project().tenant_id.as_str(),    "t_env");
    assert_eq!(env.project().workspace_id.as_str(), "w_env");
    assert_eq!(env.project().project_id.as_str(),   "p_env");
}

#[test]
fn run_created_event_project_extraction() {
    let env = EventEnvelope::for_runtime_event(
        EventId::new("e_run"),
        EventSource::Runtime,
        RuntimeEvent::RunCreated(RunCreated {
            project:           project(),
            session_id:        SessionId::new("sess"),
            run_id:            RunId::new("run_proj"),
            parent_run_id:     None,
            prompt_release_id: None,
            agent_role_id:     None,
        }),
    );
    assert_eq!(env.project(), &project());
}

// ── 6. primary_entity_ref() for Session, Run, Task events ────────────────────

#[test]
fn session_created_primary_entity_ref_is_session() {
    let env = EventEnvelope::for_runtime_event(
        EventId::new("e_sess"),
        EventSource::Runtime,
        RuntimeEvent::SessionCreated(SessionCreated {
            project:    project(),
            session_id: SessionId::new("sess_ref"),
        }),
    );

    let entity = env.primary_entity_ref().expect("SessionCreated must have entity ref");
    assert!(
        matches!(&entity, RuntimeEntityRef::Session { session_id } if session_id.as_str() == "sess_ref"),
        "SessionCreated must return RuntimeEntityRef::Session"
    );
}

#[test]
fn run_created_primary_entity_ref_is_run() {
    let env = EventEnvelope::for_runtime_event(
        EventId::new("e_run"),
        EventSource::Runtime,
        RuntimeEvent::RunCreated(RunCreated {
            project:           project(),
            session_id:        SessionId::new("sess"),
            run_id:            RunId::new("run_ref"),
            parent_run_id:     None,
            prompt_release_id: None,
            agent_role_id:     None,
        }),
    );

    let entity = env.primary_entity_ref().expect("RunCreated must have entity ref");
    assert!(
        matches!(&entity, RuntimeEntityRef::Run { run_id } if run_id.as_str() == "run_ref"),
        "RunCreated must return RuntimeEntityRef::Run"
    );
}

#[test]
fn task_created_primary_entity_ref_is_task() {
    let env = EventEnvelope::for_runtime_event(
        EventId::new("e_task"),
        EventSource::Runtime,
        RuntimeEvent::TaskCreated(TaskCreated {
            project:           project(),
            task_id:           TaskId::new("task_ref"),
            parent_run_id:     None,
            parent_task_id:    None,
            prompt_release_id: None,
        }),
    );

    let entity = env.primary_entity_ref().expect("TaskCreated must have entity ref");
    assert!(
        matches!(&entity, RuntimeEntityRef::Task { task_id } if task_id.as_str() == "task_ref"),
        "TaskCreated must return RuntimeEntityRef::Task"
    );
}

// ── 7. RuntimeEntityRef kind() method ────────────────────────────────────────

#[test]
fn runtime_entity_ref_kind_is_consistent() {
    use cairn_domain::errors::RuntimeEntityKind;

    let session_ref = RuntimeEntityRef::Session { session_id: SessionId::new("s") };
    let run_ref     = RuntimeEntityRef::Run     { run_id:     RunId::new("r")     };
    let task_ref    = RuntimeEntityRef::Task    { task_id:    TaskId::new("t")    };

    assert_eq!(session_ref.kind(), RuntimeEntityKind::Session);
    assert_eq!(run_ref.kind(),     RuntimeEntityKind::Run);
    assert_eq!(task_ref.kind(),    RuntimeEntityKind::Task);

    let approval_ref = RuntimeEntityRef::Approval { approval_id: ApprovalId::new("a") };
    assert_eq!(approval_ref.kind(), RuntimeEntityKind::Approval);
}

// ── 8. RuntimeEntityRef JSON serialization ────────────────────────────────────

#[test]
fn runtime_entity_ref_session_serializes_with_entity_tag() {
    let r = RuntimeEntityRef::Session { session_id: SessionId::new("sess_json") };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["entity"],     "session");
    assert_eq!(json["session_id"], "sess_json");
    let back: RuntimeEntityRef = serde_json::from_value(json).unwrap();
    assert_eq!(back, r);
}

#[test]
fn runtime_entity_ref_run_serializes_with_entity_tag() {
    let r = RuntimeEntityRef::Run { run_id: RunId::new("run_json") };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["entity"],  "run");
    assert_eq!(json["run_id"],  "run_json");
    let back: RuntimeEntityRef = serde_json::from_value(json).unwrap();
    assert_eq!(back, r);
}

#[test]
fn runtime_entity_ref_task_serializes_with_entity_tag() {
    let r = RuntimeEntityRef::Task { task_id: TaskId::new("task_json") };
    let json = serde_json::to_value(&r).unwrap();
    assert_eq!(json["entity"],  "task");
    assert_eq!(json["task_id"], "task_json");
}

// ── 9. OwnershipKey From conversions ─────────────────────────────────────────

#[test]
fn ownership_key_converts_from_project_key() {
    let k: OwnershipKey = project().into();
    assert!(matches!(k, OwnershipKey::Project(_)));
}

#[test]
fn ownership_key_converts_from_workspace_key() {
    let wk = WorkspaceKey {
        tenant_id:    TenantId::new("t"),
        workspace_id: WorkspaceId::new("w"),
    };
    let k: OwnershipKey = wk.into();
    assert!(matches!(k, OwnershipKey::Workspace(_)));
}

#[test]
fn ownership_key_converts_from_tenant_key() {
    let tk = TenantKey { tenant_id: TenantId::new("t") };
    let k: OwnershipKey = tk.into();
    assert!(matches!(k, OwnershipKey::Tenant(_)));
}

// ── 10. Checkpoint and Mailbox entity refs ────────────────────────────────────

#[test]
fn checkpoint_and_mailbox_entity_refs_are_correct() {
    use cairn_domain::errors::RuntimeEntityKind;

    let cp = RuntimeEntityRef::Checkpoint { checkpoint_id: CheckpointId::new("cp1") };
    assert_eq!(cp.kind(), RuntimeEntityKind::Checkpoint);
    let json = serde_json::to_value(&cp).unwrap();
    assert_eq!(json["entity"], "checkpoint");

    let mb = RuntimeEntityRef::MailboxMessage { message_id: MailboxMessageId::new("msg1") };
    assert_eq!(mb.kind(), RuntimeEntityKind::MailboxMessage);
    let json = serde_json::to_value(&mb).unwrap();
    assert_eq!(json["entity"], "mailbox_message");
}
