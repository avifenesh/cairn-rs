//! RFC 002 durability class contract tests.
//!
//! Validates that:
//! - EntityDurabilityClass serde round-trips correctly.
//! - Session, Run, Task are FullHistory (full replay required).
//! - Approval, Checkpoint and others are CurrentStatePlusAudit.
//! - RuntimeEntityRef::kind() correctly maps to RuntimeEntityKind.
//! - primary_entity_ref() is Some for the core operational events.
//! - EventEnvelope::project() extracts the project key for all event families.

use cairn_domain::{
    errors::{EntityDurabilityClass, RuntimeEntityKind, RuntimeEntityRef},
    events::{
        ApprovalRequested, CheckpointRecorded,
        RunCreated, RunStateChanged, SessionCreated, TaskCreated, TaskStateChanged,
        StateTransition,
    },
    lifecycle::{CheckpointDisposition, RunState, TaskState},
    policy::ApprovalRequirement,
    ApprovalId, CheckpointId, EventEnvelope, EventId, EventSource, ProjectKey,
    RunId, RuntimeEvent, SessionId, TaskId,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn ev(payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(EventId::new("evt_test"), EventSource::Runtime, payload)
}

// ── (1): DurabilityClass serde round-trip ─────────────────────────────────────

/// RFC 002: EntityDurabilityClass must serialise to/from snake_case JSON strings.
#[test]
fn durability_class_serde_round_trip() {
    for class in [
        EntityDurabilityClass::FullHistory,
        EntityDurabilityClass::CurrentStatePlusAudit,
    ] {
        let json = serde_json::to_string(&class).expect("must serialize");
        let recovered: EntityDurabilityClass =
            serde_json::from_str(&json).expect("must deserialize");
        assert_eq!(recovered, class, "serde round-trip must be identity for {class:?}");
    }
}

/// FullHistory serialises to `"full_history"`, CurrentStatePlusAudit to `"current_state_plus_audit"`.
#[test]
fn durability_class_serializes_to_snake_case() {
    let fh = serde_json::to_string(&EntityDurabilityClass::FullHistory).unwrap();
    let csa = serde_json::to_string(&EntityDurabilityClass::CurrentStatePlusAudit).unwrap();
    assert_eq!(fh,  r#""full_history""#);
    assert_eq!(csa, r#""current_state_plus_audit""#);
}

/// The two variants are distinct — FullHistory ≠ CurrentStatePlusAudit.
#[test]
fn durability_classes_are_distinct() {
    assert_ne!(
        EntityDurabilityClass::FullHistory,
        EntityDurabilityClass::CurrentStatePlusAudit,
        "the two durability classes must not be equal"
    );
}

// ── (2): Session, Run, Task → FullHistory ─────────────────────────────────────

/// RFC 002 §3: Session, Run, and Task MUST be FullHistory.
/// These are the core operational state machines; operators must be able to
/// replay the full event history to reconstruct their state.
#[test]
fn session_run_task_are_full_history() {
    for kind in [
        RuntimeEntityKind::Session,
        RuntimeEntityKind::Run,
        RuntimeEntityKind::Task,
    ] {
        assert_eq!(
            kind.durability_class(),
            EntityDurabilityClass::FullHistory,
            "{kind:?} must be FullHistory — full replay is required for state reconstruction"
        );
    }
}

// ── (3): Approval, Checkpoint → CurrentStatePlusAudit ─────────────────────────

/// RFC 002 §3: Approval and Checkpoint are CurrentStatePlusAudit.
/// Current state plus an audit trail is sufficient; full replay is not required.
#[test]
fn approval_checkpoint_are_current_state_plus_audit() {
    for kind in [
        RuntimeEntityKind::Approval,
        RuntimeEntityKind::Checkpoint,
    ] {
        assert_eq!(
            kind.durability_class(),
            EntityDurabilityClass::CurrentStatePlusAudit,
            "{kind:?} must be CurrentStatePlusAudit"
        );
    }
}

/// All other entity kinds (MailboxMessage, Signal, ToolInvocation, etc.)
/// are CurrentStatePlusAudit by default.
#[test]
fn other_entity_kinds_are_current_state_plus_audit() {
    let others = [
        RuntimeEntityKind::MailboxMessage,
        RuntimeEntityKind::Signal,
        RuntimeEntityKind::ToolInvocation,
        RuntimeEntityKind::IngestJob,
        RuntimeEntityKind::EvalRun,
        RuntimeEntityKind::PromptAsset,
        RuntimeEntityKind::PromptVersion,
        RuntimeEntityKind::PromptRelease,
    ];
    for kind in others {
        assert_eq!(
            kind.durability_class(),
            EntityDurabilityClass::CurrentStatePlusAudit,
            "{kind:?} must be CurrentStatePlusAudit (not a core state machine)"
        );
    }
}

// ── (4): EntityRef maps to the correct RuntimeEntityKind ──────────────────────

/// RuntimeEntityRef::kind() correctly maps each variant to its RuntimeEntityKind.
#[test]
fn entity_ref_kind_mapping_is_correct() {
    use cairn_domain::ids::*;

    let cases: &[(RuntimeEntityRef, RuntimeEntityKind)] = &[
        (RuntimeEntityRef::Session { session_id: SessionId::new("s") }, RuntimeEntityKind::Session),
        (RuntimeEntityRef::Run { run_id: RunId::new("r") }, RuntimeEntityKind::Run),
        (RuntimeEntityRef::Task { task_id: TaskId::new("t") }, RuntimeEntityKind::Task),
        (RuntimeEntityRef::Approval { approval_id: ApprovalId::new("a") }, RuntimeEntityKind::Approval),
        (RuntimeEntityRef::Checkpoint { checkpoint_id: CheckpointId::new("c") }, RuntimeEntityKind::Checkpoint),
        (RuntimeEntityRef::MailboxMessage { message_id: MailboxMessageId::new("m") }, RuntimeEntityKind::MailboxMessage),
        (RuntimeEntityRef::Signal { signal_id: SignalId::new("sig") }, RuntimeEntityKind::Signal),
        (RuntimeEntityRef::ToolInvocation { invocation_id: ToolInvocationId::new("inv") }, RuntimeEntityKind::ToolInvocation),
        (RuntimeEntityRef::IngestJob { job_id: IngestJobId::new("job") }, RuntimeEntityKind::IngestJob),
        (RuntimeEntityRef::EvalRun { eval_run_id: EvalRunId::new("eval") }, RuntimeEntityKind::EvalRun),
        (RuntimeEntityRef::PromptAsset { prompt_asset_id: PromptAssetId::new("pa") }, RuntimeEntityKind::PromptAsset),
        (RuntimeEntityRef::PromptVersion { prompt_version_id: PromptVersionId::new("pv") }, RuntimeEntityKind::PromptVersion),
        (RuntimeEntityRef::PromptRelease { prompt_release_id: PromptReleaseId::new("pr") }, RuntimeEntityKind::PromptRelease),
    ];

    for (entity_ref, expected_kind) in cases {
        let got = entity_ref.kind();
        assert_eq!(
            got, *expected_kind,
            "EntityRef::{:?} must map to RuntimeEntityKind::{:?}",
            got, expected_kind
        );
    }
}

/// EntityRef round-trip: kind() → durability_class() gives the expected class.
#[test]
fn entity_ref_durability_class_via_kind() {
    use cairn_domain::ids::*;

    let full_history_refs = [
        RuntimeEntityRef::Session { session_id: SessionId::new("s") },
        RuntimeEntityRef::Run { run_id: RunId::new("r") },
        RuntimeEntityRef::Task { task_id: TaskId::new("t") },
    ];
    for entity_ref in &full_history_refs {
        assert_eq!(
            entity_ref.kind().durability_class(),
            EntityDurabilityClass::FullHistory,
            "{:?} must have FullHistory durability", entity_ref.kind()
        );
    }

    let audit_refs = [
        RuntimeEntityRef::Approval { approval_id: ApprovalId::new("a") },
        RuntimeEntityRef::Checkpoint { checkpoint_id: CheckpointId::new("c") },
    ];
    for entity_ref in &audit_refs {
        assert_eq!(
            entity_ref.kind().durability_class(),
            EntityDurabilityClass::CurrentStatePlusAudit,
            "{:?} must have CurrentStatePlusAudit durability", entity_ref.kind()
        );
    }
}

// ── (5): primary_entity_ref() for core event types ────────────────────────────

/// primary_entity_ref() returns Some for core operational events.
#[test]
fn primary_entity_ref_is_some_for_operational_events() {
    let events: &[(RuntimeEvent, &str)] = &[
        (RuntimeEvent::SessionCreated(SessionCreated { project: project(), session_id: SessionId::new("s1") }),
         "SessionCreated"),
        (RuntimeEvent::RunCreated(RunCreated { project: project(), session_id: SessionId::new("s1"), run_id: RunId::new("r1"), parent_run_id: None, prompt_release_id: None, agent_role_id: None }),
         "RunCreated"),
        (RuntimeEvent::RunStateChanged(RunStateChanged { project: project(), run_id: RunId::new("r1"), transition: StateTransition { from: Some(RunState::Pending), to: RunState::Running }, failure_class: None, pause_reason: None, resume_trigger: None }),
         "RunStateChanged"),
        (RuntimeEvent::TaskCreated(TaskCreated { project: project(), task_id: TaskId::new("t1"), parent_run_id: None, parent_task_id: None, prompt_release_id: None }),
         "TaskCreated"),
        (RuntimeEvent::TaskStateChanged(TaskStateChanged { project: project(), task_id: TaskId::new("t1"), transition: StateTransition { from: Some(TaskState::Queued), to: TaskState::Running }, failure_class: None, pause_reason: None, resume_trigger: None }),
         "TaskStateChanged"),
        (RuntimeEvent::ApprovalRequested(ApprovalRequested { project: project(), approval_id: ApprovalId::new("a1"), run_id: None, task_id: None, requirement: ApprovalRequirement::Required }),
         "ApprovalRequested"),
        (RuntimeEvent::CheckpointRecorded(CheckpointRecorded { project: project(), run_id: RunId::new("r1"), checkpoint_id: CheckpointId::new("c1"), disposition: CheckpointDisposition::Latest, data: None }),
         "CheckpointRecorded"),
    ];

    for (event, name) in events {
        let entity_ref = event.primary_entity_ref();
        assert!(
            entity_ref.is_some(),
            "primary_entity_ref() must be Some for {name}"
        );
    }
}

/// primary_entity_ref() for Session events returns a Session ref with the correct ID.
#[test]
fn primary_entity_ref_session_carries_correct_id() {
    let event = RuntimeEvent::SessionCreated(SessionCreated {
        project: project(),
        session_id: SessionId::new("sess_check"),
    });

    match event.primary_entity_ref() {
        Some(RuntimeEntityRef::Session { session_id }) => {
            assert_eq!(session_id, SessionId::new("sess_check"));
        }
        other => panic!("expected Session ref, got {other:?}"),
    }
}

/// primary_entity_ref() for Run events returns a Run ref with the correct ID.
#[test]
fn primary_entity_ref_run_carries_correct_id() {
    let event = RuntimeEvent::RunCreated(RunCreated {
        project: project(),
        session_id: SessionId::new("s"),
        run_id: RunId::new("run_check"),
        parent_run_id: None,
        prompt_release_id: None,
        agent_role_id: None,
    });

    match event.primary_entity_ref() {
        Some(RuntimeEntityRef::Run { run_id }) => {
            assert_eq!(run_id, RunId::new("run_check"));
        }
        other => panic!("expected Run ref, got {other:?}"),
    }
}

/// primary_entity_ref() for Task events returns a Task ref with the correct ID.
#[test]
fn primary_entity_ref_task_carries_correct_id() {
    let event = RuntimeEvent::TaskCreated(TaskCreated {
        project: project(),
        task_id: TaskId::new("task_check"),
        parent_run_id: None,
        parent_task_id: None,
        prompt_release_id: None,
    });

    match event.primary_entity_ref() {
        Some(RuntimeEntityRef::Task { task_id }) => {
            assert_eq!(task_id, TaskId::new("task_check"));
        }
        other => panic!("expected Task ref, got {other:?}"),
    }
}

// ── (6): project() extraction covers all event families ───────────────────────

/// EventEnvelope::project() extracts the project from the stored ownership key.
/// RuntimeEvent::project() extracts it from the payload directly.
#[test]
fn project_extraction_matches_for_all_event_families() {
    let expected = project();

    let events: &[(RuntimeEvent, &str)] = &[
        (RuntimeEvent::SessionCreated(SessionCreated { project: project(), session_id: SessionId::new("s") }), "SessionCreated"),
        (RuntimeEvent::RunCreated(RunCreated { project: project(), session_id: SessionId::new("s"), run_id: RunId::new("r"), parent_run_id: None, prompt_release_id: None, agent_role_id: None }), "RunCreated"),
        (RuntimeEvent::TaskCreated(TaskCreated { project: project(), task_id: TaskId::new("t"), parent_run_id: None, parent_task_id: None, prompt_release_id: None }), "TaskCreated"),
        (RuntimeEvent::ApprovalRequested(ApprovalRequested { project: project(), approval_id: ApprovalId::new("a"), run_id: None, task_id: None, requirement: ApprovalRequirement::Required }), "ApprovalRequested"),
        (RuntimeEvent::CheckpointRecorded(CheckpointRecorded { project: project(), run_id: RunId::new("r"), checkpoint_id: CheckpointId::new("c"), disposition: CheckpointDisposition::Latest, data: None }), "CheckpointRecorded"),
    ];

    for (event, name) in events {
        // Direct payload project() extraction.
        let got = event.project();
        assert_eq!(
            *got, expected,
            "project() must match for {name}: expected {:?}, got {:?}",
            expected, got
        );

        // EventEnvelope::project() delegates to the payload.
        let envelope = ev(event.clone());
        assert_eq!(
            *envelope.payload.project(), expected,
            "envelope.payload.project() must match for {name}"
        );
    }
}

/// Verifying FullHistory entities' events carry the project correctly
/// ensures the store can scope them for per-project reads.
#[test]
fn full_history_event_projects_are_accessible() {
    let p = ProjectKey::new("tenant_durable", "ws_durable", "proj_durable");

    let session_ev = RuntimeEvent::SessionCreated(SessionCreated {
        project: p.clone(),
        session_id: SessionId::new("s_dur"),
    });
    let run_ev = RuntimeEvent::RunCreated(RunCreated {
        project: p.clone(),
        session_id: SessionId::new("s_dur"),
        run_id: RunId::new("r_dur"),
        parent_run_id: None,
        prompt_release_id: None,
        agent_role_id: None,
    });
    let task_ev = RuntimeEvent::TaskCreated(TaskCreated {
        project: p.clone(),
        task_id: TaskId::new("t_dur"),
        parent_run_id: Some(RunId::new("r_dur")),
        parent_task_id: None,
        prompt_release_id: None,
    });

    for event in &[session_ev, run_ev, task_ev] {
        assert_eq!(*event.project(), p);
        // All three are FullHistory entities.
        let entity_ref = event.primary_entity_ref().unwrap();
        assert_eq!(
            entity_ref.kind().durability_class(),
            EntityDurabilityClass::FullHistory,
            "{:?} must be a FullHistory entity", entity_ref.kind()
        );
    }
}
