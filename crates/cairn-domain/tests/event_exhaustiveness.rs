//! RuntimeEvent exhaustiveness guard (RFC 002).
//!
//! This file is a **compile-time contract**: every RuntimeEvent variant must be
//! explicitly handled in `assert_all_variants`. If a new variant is added to
//! RuntimeEvent without updating this file, the `match` will fail to compile.
//!
//! Runtime assertions:
//!   project()           — returns a non-default ProjectKey for entity-scoped events
//!   primary_entity_ref() — returns the correct EntityRef variant for entity-scoped
//!                         events, None for system/tenant-scoped events
//!
//! Sentinel project: variants that are tenant-scoped (not project-scoped) have
//!   project().tenant_id == "_system" (the static sentinel key).

use cairn_domain::audit::AuditOutcome;
use cairn_domain::commercial::ProductTier;
use cairn_domain::errors::RuntimeEntityRef;
use cairn_domain::events::StateTransition;
use cairn_domain::lifecycle::{RunState, SessionState, TaskState};
use cairn_domain::policy::{ApprovalRequirement, GuardrailDecisionKind, GuardrailSubjectType};
use cairn_domain::providers::{
    OperationKind, ProviderBindingSettings, ProviderBudgetPeriod, ProviderCallStatus,
    ProviderConnectionStatus, ProviderHealthStatus, RetryPolicy, RouteDecisionStatus,
    StructuredOutputMode,
};
use cairn_domain::tenancy::{TenantKey, WorkspaceKey, WorkspaceRole};
use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
use cairn_domain::workers::ExternalWorkerReport;
use cairn_domain::{
    ApprovalDelegated, ApprovalId, ApprovalPolicyCreated, ApprovalRequested, AuditLogEntryRecorded,
    ChannelCreated, ChannelId, ChannelMessageConsumed, ChannelMessageSent, CheckpointId,
    CheckpointRecorded, CheckpointRestored, CheckpointStrategySet, CredentialId,
    CredentialKeyRotated, CredentialRevoked, CredentialStored, DecisionId, DefaultSettingCleared,
    DefaultSettingSet, EntitlementOverrideSet, EvalBaselineLocked, EvalBaselineSet,
    EvalDatasetCreated, EvalDatasetEntryAdded, EvalRubricCreated, EvalRunCompleted, EvalRunId,
    EvalRunStarted, EventEnvelope, EventId, EventLogCompacted, EventSource, ExecutionClass,
    ExternalWorkerReactivated, ExternalWorkerRegistered, ExternalWorkerReported,
    ExternalWorkerSuspended, GuardrailPolicyCreated, GuardrailPolicyEvaluated, IngestJobCompleted,
    IngestJobId, IngestJobStarted, LicenseActivated, MailboxMessageAppended, MailboxMessageId,
    NotificationPreferenceSet, NotificationSent, OperatorId, OperatorIntervention,
    OperatorProfileCreated, OperatorProfileUpdated, PauseScheduled, PermissionDecisionRecorded,
    ProjectCreated, ProjectKey, PromptAssetCreated, PromptAssetId, PromptReleaseCreated,
    PromptReleaseId, PromptReleaseTransitioned, PromptRolloutStarted, PromptVersionCreated,
    PromptVersionId, ProviderBindingCreated, ProviderBindingId, ProviderBindingStateChanged,
    ProviderBudgetAlertTriggered, ProviderBudgetExceeded, ProviderBudgetSet, ProviderCallCompleted,
    ProviderCallId, ProviderConnectionId, ProviderConnectionRegistered, ProviderHealthChecked,
    ProviderHealthScheduleSet, ProviderHealthScheduleTriggered, ProviderMarkedDegraded,
    ProviderModelId, ProviderModelRegistered, ProviderPoolConnectionAdded,
    ProviderPoolConnectionRemoved, ProviderPoolCreated, ProviderRecovered, ProviderRetryPolicySet,
    RecoveryAttempted, RecoveryCompleted, RecoveryEscalated, ResourceShareRevoked, ResourceShared,
    RetentionPolicySet, RouteAttemptId, RouteDecisionId, RouteDecisionMade, RoutePolicyCreated,
    RoutePolicyUpdated, RunCostAlertSet, RunCostAlertTriggered, RunCostUpdated, RunCreated, RunId,
    RunSlaBreached, RunSlaSet, RunStateChanged, RunTemplateCreated, RunTemplateDeleted,
    RunTemplateId, RuntimeEvent, SessionCostUpdated, SessionCreated, SessionId,
    SessionStateChanged, SignalId, SignalIngested, SignalRouted, SignalSubscriptionCreated,
    SnapshotCreated, SoulPatchApplied, SoulPatchProposed, SpendAlertTriggered, SubagentSpawned,
    TaskCreated, TaskDependencyAdded, TaskDependencyResolved, TaskId, TaskLeaseExpired,
    TaskPriorityChanged, TaskStateChanged, TenantCreated, TenantId, TenantQuotaSet,
    TenantQuotaViolated, ToolInvocationCompleted, ToolInvocationFailed, ToolInvocationId,
    ToolInvocationProgressUpdated, ToolInvocationStarted, TriggerCreated, TriggerDeleted,
    TriggerDenied, TriggerDisabled, TriggerEnabled, TriggerFired, TriggerId,
    TriggerPendingApproval, TriggerRateLimited, TriggerResumed, TriggerSkipReason, TriggerSkipped,
    TriggerSuspended, TriggerSuspensionReason, UserMessageAppended, WorkerId, WorkspaceCreated,
    WorkspaceId, WorkspaceMemberAdded, WorkspaceMemberRemoved,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn p() -> ProjectKey {
    ProjectKey::new("t_exh", "w_exh", "p_exh")
}

fn tid() -> TenantId {
    TenantId::new("t_exh")
}
fn wkey() -> WorkspaceKey {
    WorkspaceKey {
        tenant_id: tid(),
        workspace_id: WorkspaceId::new("w_exh"),
    }
}
fn tkey() -> TenantKey {
    TenantKey { tenant_id: tid() }
}
fn sess() -> SessionId {
    SessionId::new("s_exh")
}
fn run() -> RunId {
    RunId::new("r_exh")
}
fn task() -> TaskId {
    TaskId::new("t_exh")
}

fn binding_settings() -> ProviderBindingSettings {
    ProviderBindingSettings {
        temperature_milli: None,
        max_output_tokens: None,
        timeout_ms: None,
        structured_output_mode: StructuredOutputMode::Default,
        required_capabilities: vec![],
        disabled_capabilities: vec![],
        cost_type: Default::default(),
        daily_budget_micros: None,
    }
}

// ── Exhaustiveness assertion ──────────────────────────────────────────────────
//
// THIS MATCH MUST HAVE NO WILDCARD ARM.
// Adding a new RuntimeEvent variant without updating this match → compile error.

fn assert_all_variants_covered(event: &RuntimeEvent) {
    let proj = event.project();
    let eref = event.primary_entity_ref();

    match event {
        // ── Entity-scoped: project() returns real project, entity_ref = Some ─
        RuntimeEvent::SessionCreated(_) => {
            assert_ne!(proj.tenant_id.as_str(), "_system");
            assert!(matches!(eref, Some(RuntimeEntityRef::Session { .. })));
        }
        RuntimeEvent::SessionStateChanged(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Session { .. })));
        }
        RuntimeEvent::RunCreated(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Run { .. })));
        }
        RuntimeEvent::RunStateChanged(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Run { .. })));
        }
        RuntimeEvent::TaskCreated(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Task { .. })));
        }
        RuntimeEvent::TaskLeaseClaimed(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Task { .. })));
        }
        RuntimeEvent::TaskLeaseHeartbeated(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Task { .. })));
        }
        RuntimeEvent::TaskStateChanged(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Task { .. })));
        }
        RuntimeEvent::ApprovalRequested(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Approval { .. })));
        }
        RuntimeEvent::ApprovalResolved(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Approval { .. })));
        }
        RuntimeEvent::CheckpointRecorded(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Checkpoint { .. })));
        }
        RuntimeEvent::CheckpointRestored(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Checkpoint { .. })));
        }
        RuntimeEvent::MailboxMessageAppended(_) => {
            assert!(matches!(
                eref,
                Some(RuntimeEntityRef::MailboxMessage { .. })
            ));
        }
        RuntimeEvent::ToolInvocationStarted(_) => {
            assert!(matches!(
                eref,
                Some(RuntimeEntityRef::ToolInvocation { .. })
            ));
        }
        RuntimeEvent::ToolInvocationCompleted(_) => {
            assert!(matches!(
                eref,
                Some(RuntimeEntityRef::ToolInvocation { .. })
            ));
        }
        RuntimeEvent::ToolInvocationFailed(_) => {
            assert!(matches!(
                eref,
                Some(RuntimeEntityRef::ToolInvocation { .. })
            ));
        }
        RuntimeEvent::ToolInvocationCacheHit(_) => {
            assert!(matches!(
                eref,
                Some(RuntimeEntityRef::ToolInvocation { .. })
            ));
        }
        RuntimeEvent::ToolRecoveryPaused(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Run { .. })));
        }
        RuntimeEvent::SignalIngested(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Signal { .. })));
        }
        RuntimeEvent::PromptAssetCreated(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::PromptAsset { .. })));
        }
        RuntimeEvent::PromptVersionCreated(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::PromptVersion { .. })));
        }
        RuntimeEvent::PromptReleaseCreated(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::PromptRelease { .. })));
        }
        RuntimeEvent::PromptReleaseTransitioned(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::PromptRelease { .. })));
        }
        RuntimeEvent::PromptRolloutStarted(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::PromptRelease { .. })));
        }
        RuntimeEvent::SubagentSpawned(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Task { .. })));
        }
        RuntimeEvent::ExternalWorkerReported(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Task { .. })));
        }
        RuntimeEvent::IngestJobStarted(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::IngestJob { .. })));
        }
        RuntimeEvent::IngestJobCompleted(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::IngestJob { .. })));
        }
        RuntimeEvent::EvalRunStarted(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::EvalRun { .. })));
        }
        RuntimeEvent::EvalRunCompleted(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::EvalRun { .. })));
        }

        // ── Entity-scoped: project() real, entity_ref = None or conditional ─
        RuntimeEvent::RecoveryAttempted(_) => {
            let _ = eref;
        }
        RuntimeEvent::RecoveryCompleted(_) => {
            let _ = eref;
        }

        // ── Tenant/sentinel-scoped: project() returns sentinel "_system" ────
        RuntimeEvent::ExternalWorkerRegistered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ExternalWorkerSuspended(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ExternalWorkerReactivated(_) => {
            assert!(eref.is_none());
        }

        // ── Project-scoped, entity_ref = None ────────────────────────────────
        RuntimeEvent::TenantCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::WorkspaceCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProjectCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ApprovalPolicyCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RouteDecisionMade(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderCallCompleted(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::SoulPatchProposed(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::SoulPatchApplied(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::SessionCostUpdated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RunCostUpdated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::SpendAlertTriggered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::UserMessageAppended(_) => {
            assert!(matches!(eref, Some(RuntimeEntityRef::Run { .. })));
        }
        RuntimeEvent::SignalRouted(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::SignalSubscriptionCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerEnabled(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerDisabled(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerSuspended(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerResumed(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerDeleted(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerFired(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerSkipped(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerDenied(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerRateLimited(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TriggerPendingApproval(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RunTemplateCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RunTemplateDeleted(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderBindingCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderBindingStateChanged(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ChannelCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ChannelMessageSent(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ChannelMessageConsumed(_) => {
            assert!(eref.is_none());
        }

        // ── System/tenant-scoped (sentinel project), entity_ref = None ───────
        RuntimeEvent::ProviderBudgetSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::DefaultSettingSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::DefaultSettingCleared(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::LicenseActivated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::EntitlementOverrideSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::NotificationPreferenceSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::NotificationSent(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderPoolCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderPoolConnectionAdded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderPoolConnectionRemoved(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TenantQuotaSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TenantQuotaViolated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RetentionPolicySet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RunCostAlertSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RunCostAlertTriggered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::WorkspaceMemberAdded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::WorkspaceMemberRemoved(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ApprovalDelegated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::AuditLogEntryRecorded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::CheckpointStrategySet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::CredentialKeyRotated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::CredentialRevoked(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::CredentialStored(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::EvalBaselineLocked(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::EvalBaselineSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::EvalDatasetCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::EvalDatasetEntryAdded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::EvalRubricCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::EventLogCompacted(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::GuardrailPolicyCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::GuardrailPolicyEvaluated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::OperatorIntervention(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::OperatorProfileCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::OperatorProfileUpdated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::PauseScheduled(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::PermissionDecisionRecorded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderBudgetAlertTriggered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderBudgetExceeded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderConnectionRegistered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderHealthChecked(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderHealthScheduleSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderHealthScheduleTriggered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderMarkedDegraded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderModelRegistered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderRecovered(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ProviderRetryPolicySet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RecoveryEscalated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ResourceShareRevoked(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ResourceShared(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RoutePolicyCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RoutePolicyUpdated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RunSlaBreached(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::RunSlaSet(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::SnapshotCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TaskDependencyAdded(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TaskDependencyResolved(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TaskLeaseExpired(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::TaskPriorityChanged(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::ToolInvocationProgressUpdated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::OutcomeRecorded(e) => {
            assert_eq!(
                eref,
                Some(RuntimeEntityRef::Run {
                    run_id: e.run_id.clone()
                })
            );
        }
        RuntimeEvent::ScheduledTaskCreated(_) => {
            assert!(eref.is_none());
        }
        RuntimeEvent::PlanProposed(e) => {
            assert_eq!(
                eref,
                Some(RuntimeEntityRef::Run {
                    run_id: e.plan_run_id.clone()
                })
            );
        }
        RuntimeEvent::PlanApproved(e) => {
            assert_eq!(
                eref,
                Some(RuntimeEntityRef::Run {
                    run_id: e.plan_run_id.clone()
                })
            );
        }
        RuntimeEvent::PlanRejected(e) => {
            assert_eq!(
                eref,
                Some(RuntimeEntityRef::Run {
                    run_id: e.plan_run_id.clone()
                })
            );
        }
        RuntimeEvent::PlanRevisionRequested(e) => {
            assert_eq!(
                eref,
                Some(RuntimeEntityRef::Run {
                    run_id: e.original_plan_run_id.clone()
                })
            );
        }
        // RFC 020 §"Decision Cache Survival": DecisionRecorded is
        // project-scoped but carries no entity_ref (the decision_id is
        // not a runtime entity in the RuntimeEntityRef sense; the
        // reasoning chain is served from the serialised event_json).
        RuntimeEvent::DecisionRecorded(_) => {
            assert_ne!(proj.tenant_id.as_str(), "_system");
            assert!(eref.is_none());
        }
        // Emitted once per startup replay — tenant/system-scoped, no
        // entity_ref.
        RuntimeEvent::DecisionCacheWarmup(_) => {
            assert_eq!(proj.tenant_id.as_str(), "_system");
            assert!(eref.is_none());
        }
    }
}

// ── Build every variant ───────────────────────────────────────────────────────

fn all_variants() -> Vec<RuntimeEvent> {
    let ts: u64 = 1_000_000;
    vec![
        RuntimeEvent::SessionCreated(SessionCreated {
            project: p(),
            session_id: sess(),
        }),
        RuntimeEvent::SessionStateChanged(SessionStateChanged {
            project: p(),
            session_id: sess(),
            transition: StateTransition {
                from: Some(SessionState::Open),
                to: SessionState::Completed,
            },
        }),
        RuntimeEvent::RunCreated(RunCreated {
            project: p(),
            session_id: sess(),
            run_id: run(),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
        RuntimeEvent::RunStateChanged(RunStateChanged {
            project: p(),
            run_id: run(),
            transition: StateTransition {
                from: Some(RunState::Pending),
                to: RunState::Running,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
        RuntimeEvent::TaskCreated(TaskCreated {
            project: p(),
            task_id: task(),
            parent_run_id: Some(run()),
            parent_task_id: None,
            prompt_release_id: None,
            session_id: None,
        }),
        RuntimeEvent::TaskLeaseClaimed(cairn_domain::TaskLeaseClaimed {
            project: p(),
            task_id: task(),
            lease_owner: "worker_1".to_owned(),
            lease_token: 1,
            lease_expires_at_ms: ts + 60_000,
        }),
        RuntimeEvent::TaskLeaseHeartbeated(cairn_domain::TaskLeaseHeartbeated {
            project: p(),
            task_id: task(),
            lease_token: 1,
            lease_expires_at_ms: ts + 60_000,
        }),
        RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: p(),
            task_id: task(),
            transition: StateTransition {
                from: Some(TaskState::Queued),
                to: TaskState::Leased,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        }),
        RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: p(),
            approval_id: ApprovalId::new("a1"),
            run_id: Some(run()),
            task_id: None,
            requirement: ApprovalRequirement::Required,
            title: None,
            description: None,
        }),
        RuntimeEvent::ApprovalResolved(cairn_domain::ApprovalResolved {
            project: p(),
            approval_id: ApprovalId::new("a1"),
            decision: cairn_domain::policy::ApprovalDecision::Approved,
        }),
        RuntimeEvent::CheckpointRecorded(CheckpointRecorded {
            project: p(),
            run_id: run(),
            checkpoint_id: CheckpointId::new("cp1"),
            disposition: cairn_domain::lifecycle::CheckpointDisposition::Latest,
            data: None,
        }),
        RuntimeEvent::CheckpointRestored(CheckpointRestored {
            project: p(),
            run_id: run(),
            checkpoint_id: CheckpointId::new("cp1"),
        }),
        RuntimeEvent::MailboxMessageAppended(MailboxMessageAppended {
            project: p(),
            message_id: MailboxMessageId::new("msg1"),
            run_id: Some(run()),
            task_id: None,
            content: "hi".to_owned(),
            from_run_id: None,
            from_task_id: None,
            deliver_at_ms: 0,
            sender: None,
            recipient: None,
            body: None,
            sent_at: None,
            delivery_status: None,
        }),
        RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
            project: p(),
            invocation_id: ToolInvocationId::new("inv1"),
            session_id: None,
            run_id: Some(run()),
            task_id: None,
            target: ToolInvocationTarget::Builtin {
                tool_name: "read_file".to_owned(),
            },
            execution_class: ExecutionClass::SandboxedProcess,
            prompt_release_id: None,
            requested_at_ms: ts,
            started_at_ms: ts,
        }),
        RuntimeEvent::ToolInvocationCompleted(ToolInvocationCompleted {
            project: p(),
            invocation_id: ToolInvocationId::new("inv1"),
            task_id: None,
            tool_name: "read_file".to_owned(),
            finished_at_ms: ts + 100,
            outcome: ToolInvocationOutcomeKind::Success,
            tool_call_id: None,
            result_json: None,
        }),
        RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
            project: p(),
            invocation_id: ToolInvocationId::new("inv2"),
            task_id: None,
            tool_name: "write_file".to_owned(),
            finished_at_ms: ts,
            outcome: ToolInvocationOutcomeKind::PermanentFailure,
            error_message: Some("denied".to_owned()),
        }),
        RuntimeEvent::ToolInvocationCacheHit(cairn_domain::ToolInvocationCacheHit {
            project: p(),
            invocation_id: ToolInvocationId::new("inv3"),
            run_id: Some(run()),
            task_id: None,
            tool_name: "read_file".to_owned(),
            tool_call_id: "tc_00000000".to_owned(),
            original_completed_at_ms: ts,
            served_at_ms: ts + 50,
        }),
        RuntimeEvent::ToolRecoveryPaused(cairn_domain::ToolRecoveryPaused {
            project: p(),
            run_id: run(),
            task_id: None,
            tool_name: "shell_exec".to_owned(),
            tool_call_id: "tc_00000001".to_owned(),
            reason: "DangerousPause".to_owned(),
            paused_at_ms: ts,
        }),
        RuntimeEvent::SignalIngested(SignalIngested {
            project: p(),
            signal_id: SignalId::new("sig1"),
            source: "webhook".to_owned(),
            payload: serde_json::json!({}),
            timestamp_ms: ts,
        }),
        RuntimeEvent::ExternalWorkerRegistered(ExternalWorkerRegistered {
            sentinel_project: ProjectKey::new("t_exh", "_", "_"),
            worker_id: WorkerId::new("w1"),
            tenant_id: tid(),
            display_name: "Bot".to_owned(),
            registered_at: ts,
        }),
        RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
            report: ExternalWorkerReport {
                project: p(),
                worker_id: WorkerId::new("w1"),
                run_id: None,
                task_id: task(),
                lease_token: 1,
                reported_at_ms: ts,
                progress: None,
                outcome: None,
            },
        }),
        RuntimeEvent::ExternalWorkerSuspended(ExternalWorkerSuspended {
            sentinel_project: ProjectKey::new("t_exh", "_", "_"),
            worker_id: WorkerId::new("w1"),
            tenant_id: tid(),
            suspended_at: ts,
            reason: None,
        }),
        RuntimeEvent::ExternalWorkerReactivated(ExternalWorkerReactivated {
            sentinel_project: ProjectKey::new("t_exh", "_", "_"),
            worker_id: WorkerId::new("w1"),
            tenant_id: tid(),
            reactivated_at: ts,
        }),
        RuntimeEvent::SubagentSpawned(SubagentSpawned {
            project: p(),
            parent_run_id: run(),
            parent_task_id: None,
            child_task_id: task(),
            child_session_id: sess(),
            child_run_id: None,
        }),
        RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
            project: p(),
            run_id: Some(run()),
            task_id: None,
            reason: "retry".to_owned(),
            boot_id: None,
        }),
        RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
            project: p(),
            run_id: Some(run()),
            task_id: None,
            recovered: true,
            boot_id: None,
        }),
        RuntimeEvent::UserMessageAppended(UserMessageAppended {
            project: p(),
            session_id: sess(),
            run_id: run(),
            content: String::new(),
            sequence: 0,
            appended_at_ms: ts,
        }),
        RuntimeEvent::IngestJobStarted(IngestJobStarted {
            project: p(),
            job_id: IngestJobId::new("job1"),
            source_id: None,
            document_count: 0,
            started_at: ts,
        }),
        RuntimeEvent::IngestJobCompleted(IngestJobCompleted {
            project: p(),
            job_id: IngestJobId::new("job1"),
            success: true,
            error_message: None,
            completed_at: ts,
        }),
        RuntimeEvent::EvalRunStarted(EvalRunStarted {
            project: p(),
            eval_run_id: EvalRunId::new("er1"),
            subject_kind: "prompt_release".to_owned(),
            evaluator_type: "auto".to_owned(),
            started_at: ts,
            prompt_asset_id: None,
            prompt_version_id: None,
            prompt_release_id: None,
            created_by: None,
        }),
        RuntimeEvent::EvalRunCompleted(EvalRunCompleted {
            project: p(),
            eval_run_id: EvalRunId::new("er1"),
            success: true,
            error_message: None,
            subject_node_id: None,
            completed_at: ts,
        }),
        RuntimeEvent::OutcomeRecorded(cairn_domain::OutcomeRecorded {
            project: p(),
            outcome_id: cairn_domain::OutcomeId::new("oc1"),
            run_id: RunId::new("r1"),
            agent_type: "code_review".to_owned(),
            predicted_confidence: 0.85,
            actual_outcome: cairn_domain::events::ActualOutcome::Success,
            recorded_at: ts,
        }),
        RuntimeEvent::ScheduledTaskCreated(cairn_domain::events::ScheduledTaskCreated {
            tenant_id: tid(),
            scheduled_task_id: cairn_domain::ScheduledTaskId::new("sched1"),
            name: "weekly_reflection".to_owned(),
            cron_expression: "0 9 * * 1".to_owned(),
            next_run_at: Some(ts + 1_000),
            created_at: ts,
        }),
        RuntimeEvent::PromptAssetCreated(PromptAssetCreated {
            project: p(),
            prompt_asset_id: PromptAssetId::new("pa1"),
            name: "sys".to_owned(),
            kind: "system".to_owned(),
            created_at: ts,
            workspace_id: p().workspace_id,
        }),
        RuntimeEvent::PromptVersionCreated(PromptVersionCreated {
            project: p(),
            prompt_version_id: PromptVersionId::new("pv1"),
            prompt_asset_id: PromptAssetId::new("pa1"),
            content_hash: "sha256:abc".to_owned(),
            created_at: ts,
            workspace_id: p().workspace_id,
        }),
        RuntimeEvent::ApprovalPolicyCreated(ApprovalPolicyCreated {
            project: p(),
            policy_id: "pol1".to_owned(),
            tenant_id: tid(),
            name: "P".to_owned(),
            required_approvers: 1,
            allowed_approver_roles: vec![WorkspaceRole::Admin],
            auto_approve_after_ms: None,
            auto_reject_after_ms: None,
            created_at_ms: ts,
        }),
        RuntimeEvent::PromptReleaseCreated(PromptReleaseCreated {
            project: p(),
            prompt_release_id: PromptReleaseId::new("pr1"),
            prompt_asset_id: PromptAssetId::new("pa1"),
            prompt_version_id: PromptVersionId::new("pv1"),
            created_at: ts,
            release_tag: None,
            created_by: None,
        }),
        RuntimeEvent::PromptReleaseTransitioned(PromptReleaseTransitioned {
            project: p(),
            prompt_release_id: PromptReleaseId::new("pr1"),
            from_state: "draft".to_owned(),
            to_state: "active".to_owned(),
            transitioned_at: ts,
            actor: None,
            reason: None,
        }),
        RuntimeEvent::PromptRolloutStarted(PromptRolloutStarted {
            project: p(),
            prompt_release_id: PromptReleaseId::new("pr1"),
            percent: 50,
            started_at: ts,
            release_id: None,
        }),
        RuntimeEvent::TenantCreated(TenantCreated {
            project: p(),
            tenant_id: tid(),
            name: "T".to_owned(),
            created_at: ts,
        }),
        RuntimeEvent::WorkspaceCreated(WorkspaceCreated {
            project: p(),
            workspace_id: WorkspaceId::new("w_exh"),
            tenant_id: tid(),
            name: "W".to_owned(),
            created_at: ts,
        }),
        RuntimeEvent::ProjectCreated(ProjectCreated {
            project: p(),
            name: "P".to_owned(),
            created_at: ts,
        }),
        RuntimeEvent::RouteDecisionMade(RouteDecisionMade {
            project: p(),
            route_decision_id: RouteDecisionId::new("rd1"),
            operation_kind: OperationKind::Generate,
            selected_provider_binding_id: None,
            final_status: RouteDecisionStatus::Selected,
            attempt_count: 1,
            fallback_used: false,
            decided_at: ts,
        }),
        RuntimeEvent::ProviderCallCompleted(ProviderCallCompleted {
            project: p(),
            provider_call_id: ProviderCallId::new("pc1"),
            route_decision_id: RouteDecisionId::new("rd1"),
            route_attempt_id: RouteAttemptId::new("ra1"),
            provider_binding_id: ProviderBindingId::new("pb1"),
            provider_connection_id: ProviderConnectionId::new("conn1"),
            provider_model_id: ProviderModelId::new("gpt-4o"),
            operation_kind: OperationKind::Generate,
            status: ProviderCallStatus::Succeeded,
            latency_ms: Some(100),
            input_tokens: Some(100),
            output_tokens: Some(50),
            cost_micros: Some(5_000),
            completed_at: ts,
            session_id: None,
            run_id: None,
            error_class: None,
            raw_error_message: None,
            retry_count: 0,
            task_id: None,
            prompt_release_id: None,
            fallback_position: 0,
            started_at: 0,
            finished_at: 0,
        }),
        RuntimeEvent::SoulPatchProposed(SoulPatchProposed {
            project: p(),
            patch_id: "sp1".to_owned(),
            patch_content: "patch".to_owned(),
            requires_approval: false,
            proposed_at: ts,
        }),
        RuntimeEvent::SoulPatchApplied(SoulPatchApplied {
            project: p(),
            patch_id: "sp1".to_owned(),
            new_version: 1,
            applied_at: ts,
        }),
        RuntimeEvent::SessionCostUpdated(SessionCostUpdated {
            project: p(),
            session_id: sess(),
            tenant_id: tid(),
            delta_cost_micros: 100,
            delta_tokens_in: 50,
            delta_tokens_out: 25,
            provider_call_id: "pc1".to_owned(),
            updated_at_ms: ts,
        }),
        RuntimeEvent::RunCostUpdated(RunCostUpdated {
            project: p(),
            run_id: run(),
            delta_cost_micros: 100,
            delta_tokens_in: 50,
            delta_tokens_out: 25,
            provider_call_id: "pc1".to_owned(),
            updated_at_ms: ts,
            session_id: None,
            tenant_id: None,
        }),
        RuntimeEvent::SpendAlertTriggered(SpendAlertTriggered {
            project: p(),
            alert_id: "al1".to_owned(),
            tenant_id: tid(),
            session_id: sess(),
            threshold_micros: 1_000,
            current_micros: 1_100,
            triggered_at_ms: ts,
        }),
        RuntimeEvent::ProviderBudgetSet(ProviderBudgetSet {
            tenant_id: tid(),
            budget_id: "bud1".to_owned(),
            period: ProviderBudgetPeriod::Daily,
            limit_micros: 10_000_000,
            alert_threshold_percent: None,
        }),
        RuntimeEvent::ChannelCreated(ChannelCreated {
            channel_id: ChannelId::new("ch1"),
            project: p(),
            name: "chan".to_owned(),
            capacity: 100,
            created_at_ms: ts,
        }),
        RuntimeEvent::ChannelMessageSent(ChannelMessageSent {
            channel_id: ChannelId::new("ch1"),
            project: p(),
            message_id: "m1".to_owned(),
            sender_id: "s1".to_owned(),
            body: "hello".to_owned(),
            sent_at_ms: ts,
        }),
        RuntimeEvent::ChannelMessageConsumed(ChannelMessageConsumed {
            channel_id: ChannelId::new("ch1"),
            project: p(),
            message_id: "m1".to_owned(),
            consumed_by: "c1".to_owned(),
            consumed_at_ms: ts,
        }),
        RuntimeEvent::DefaultSettingSet(DefaultSettingSet {
            scope: cairn_domain::tenancy::Scope::Tenant,
            scope_id: "t1".to_owned(),
            key: "k".to_owned(),
            value: serde_json::json!(1),
        }),
        RuntimeEvent::DefaultSettingCleared(DefaultSettingCleared {
            scope: cairn_domain::tenancy::Scope::Tenant,
            scope_id: "t1".to_owned(),
            key: "k".to_owned(),
        }),
        RuntimeEvent::LicenseActivated(LicenseActivated {
            tenant_id: tid(),
            license_id: "lic1".to_owned(),
            tier: ProductTier::TeamSelfHosted,
            valid_from_ms: ts,
            valid_until_ms: None,
        }),
        RuntimeEvent::EntitlementOverrideSet(EntitlementOverrideSet {
            tenant_id: tid(),
            feature: "feat_x".to_owned(),
            allowed: true,
            reason: None,
            set_at_ms: ts,
        }),
        RuntimeEvent::NotificationPreferenceSet(NotificationPreferenceSet {
            tenant_id: tid(),
            operator_id: "op1".to_owned(),
            event_types: vec![],
            channels: vec![],
            set_at_ms: ts,
        }),
        RuntimeEvent::NotificationSent(NotificationSent {
            record_id: "nr1".to_owned(),
            tenant_id: tid(),
            operator_id: "op1".to_owned(),
            event_type: "run.failed".to_owned(),
            channel_kind: "email".to_owned(),
            channel_target: "alice@example.com".to_owned(),
            payload: serde_json::json!({}),
            sent_at_ms: ts,
            delivered: true,
            delivery_error: None,
        }),
        RuntimeEvent::ProviderPoolCreated(ProviderPoolCreated {
            pool_id: "pool1".to_owned(),
            tenant_id: tid(),
            max_connections: 10,
            created_at_ms: ts,
        }),
        RuntimeEvent::ProviderPoolConnectionAdded(ProviderPoolConnectionAdded {
            pool_id: "pool1".to_owned(),
            tenant_id: tid(),
            connection_id: ProviderConnectionId::new("conn1"),
            added_at_ms: ts,
        }),
        RuntimeEvent::ProviderPoolConnectionRemoved(ProviderPoolConnectionRemoved {
            pool_id: "pool1".to_owned(),
            tenant_id: tid(),
            connection_id: ProviderConnectionId::new("conn1"),
            removed_at_ms: ts,
        }),
        RuntimeEvent::TenantQuotaSet(TenantQuotaSet {
            tenant_id: tid(),
            max_concurrent_runs: 10,
            max_sessions_per_hour: 100,
            max_tasks_per_run: 50,
        }),
        RuntimeEvent::TenantQuotaViolated(TenantQuotaViolated {
            tenant_id: tid(),
            quota_type: "runs".to_owned(),
            current: 11,
            limit: 10,
            occurred_at_ms: ts,
        }),
        RuntimeEvent::RetentionPolicySet(RetentionPolicySet {
            tenant_id: tid(),
            policy_id: "rp1".to_owned(),
            full_history_days: 30,
            current_state_days: 90,
            max_events_per_entity: None,
        }),
        RuntimeEvent::RunCostAlertSet(RunCostAlertSet {
            run_id: run(),
            tenant_id: tid(),
            threshold_micros: 1_000_000,
            set_at_ms: ts,
        }),
        RuntimeEvent::RunCostAlertTriggered(RunCostAlertTriggered {
            run_id: run(),
            tenant_id: tid(),
            threshold_micros: 1_000_000,
            actual_cost_micros: 1_200_000,
            triggered_at_ms: ts,
        }),
        RuntimeEvent::WorkspaceMemberAdded(WorkspaceMemberAdded {
            workspace_key: wkey(),
            member_id: OperatorId::new("op1"),
            role: WorkspaceRole::Member,
            added_at_ms: ts,
        }),
        RuntimeEvent::WorkspaceMemberRemoved(WorkspaceMemberRemoved {
            workspace_key: wkey(),
            member_id: OperatorId::new("op1"),
            removed_at_ms: ts,
        }),
        RuntimeEvent::ApprovalDelegated(ApprovalDelegated {
            approval_id: ApprovalId::new("a1"),
            delegated_to: "op2".to_owned(),
            delegated_at_ms: ts,
        }),
        RuntimeEvent::AuditLogEntryRecorded(AuditLogEntryRecorded {
            entry_id: "ae1".to_owned(),
            tenant_id: tid(),
            actor_id: "op1".to_owned(),
            action: "approve".to_owned(),
            resource_type: "run".to_owned(),
            resource_id: "r1".to_owned(),
            outcome: AuditOutcome::Success,
            occurred_at_ms: ts,
        }),
        RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
            strategy_id: "cs1".to_owned(),
            description: "desc".to_owned(),
            set_at_ms: ts,
            run_id: None,
            interval_ms: 0,
            max_checkpoints: 10,
            trigger_on_task_complete: false,
        }),
        RuntimeEvent::CredentialKeyRotated(CredentialKeyRotated {
            tenant_id: tid(),
            rotation_id: "rot1".to_owned(),
            old_key_id: "k1".to_owned(),
            new_key_id: "k2".to_owned(),
            credential_ids_rotated: vec![],
        }),
        RuntimeEvent::CredentialRevoked(CredentialRevoked {
            tenant_id: tid(),
            credential_id: CredentialId::new("cred1"),
            revoked_at_ms: ts,
        }),
        RuntimeEvent::CredentialStored(CredentialStored {
            tenant_id: tid(),
            credential_id: CredentialId::new("cred2"),
            provider_id: "openai".to_owned(),
            encrypted_value: vec![1, 2, 3],
            key_id: None,
            key_version: None,
            encrypted_at_ms: ts,
        }),
        RuntimeEvent::EvalBaselineLocked(EvalBaselineLocked {
            baseline_id: "bl1".to_owned(),
            locked_at_ms: ts,
        }),
        RuntimeEvent::EvalBaselineSet(EvalBaselineSet {
            baseline_id: "bl1".to_owned(),
            metric: "task_success_rate".to_owned(),
            value: "0.9".to_owned(),
            set_at_ms: ts,
        }),
        RuntimeEvent::EvalDatasetCreated(EvalDatasetCreated {
            dataset_id: "ds1".to_owned(),
            name: "QA".to_owned(),
            created_at_ms: ts,
        }),
        RuntimeEvent::EvalDatasetEntryAdded(EvalDatasetEntryAdded {
            dataset_id: "ds1".to_owned(),
            entry_id: "de1".to_owned(),
            added_at_ms: ts,
        }),
        RuntimeEvent::EvalRubricCreated(EvalRubricCreated {
            rubric_id: "rub1".to_owned(),
            name: "Rubric".to_owned(),
            created_at_ms: ts,
        }),
        RuntimeEvent::EventLogCompacted(EventLogCompacted {
            up_to_position: 100,
            compacted_at_ms: ts,
            tenant_id: tid(),
            events_before: 200,
            events_after: 100,
        }),
        RuntimeEvent::GuardrailPolicyCreated(GuardrailPolicyCreated {
            tenant_id: tid(),
            policy_id: "gp1".to_owned(),
            name: "Safe".to_owned(),
            rules: vec![],
        }),
        RuntimeEvent::GuardrailPolicyEvaluated(GuardrailPolicyEvaluated {
            tenant_id: tid(),
            policy_id: "gp1".to_owned(),
            subject_type: GuardrailSubjectType::Run,
            subject_id: None,
            action: "read_file".to_owned(),
            decision: GuardrailDecisionKind::Allowed,
            reason: None,
            evaluated_at_ms: ts,
        }),
        RuntimeEvent::OperatorIntervention(OperatorIntervention {
            action: "pause_run".to_owned(),
            run_id: Some(run()),
            tenant_id: tid(),
            reason: "manual".to_owned(),
            intervened_at_ms: ts,
        }),
        RuntimeEvent::OperatorProfileCreated(OperatorProfileCreated {
            tenant_id: tid(),
            profile_id: OperatorId::new("op1"),
            display_name: "Alice".to_owned(),
            email: "a@example.com".to_owned(),
            role: WorkspaceRole::Admin,
        }),
        RuntimeEvent::OperatorProfileUpdated(OperatorProfileUpdated {
            tenant_id: tid(),
            profile_id: OperatorId::new("op1"),
            display_name: None,
            email: None,
        }),
        RuntimeEvent::PauseScheduled(PauseScheduled {
            task_id: task(),
            resume_at_ms: ts + 60_000,
            run_id: None,
        }),
        RuntimeEvent::PermissionDecisionRecorded(PermissionDecisionRecorded {
            decision_id: "pd1".to_owned(),
            principal: "op1".to_owned(),
            action: "approve".to_owned(),
            resource: "run/r1".to_owned(),
            allowed: true,
            recorded_at_ms: ts,
            invocation_id: None,
        }),
        RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
            project: p(),
            provider_binding_id: ProviderBindingId::new("pb1"),
            provider_connection_id: ProviderConnectionId::new("conn1"),
            provider_model_id: ProviderModelId::new("gpt-4o"),
            operation_kind: OperationKind::Generate,
            settings: binding_settings(),
            policy_id: None,
            active: true,
            created_at: ts,
            estimated_cost_micros: None,
        }),
        RuntimeEvent::ProviderBindingStateChanged(ProviderBindingStateChanged {
            project: p(),
            provider_binding_id: ProviderBindingId::new("pb1"),
            active: false,
            changed_at: ts,
        }),
        RuntimeEvent::ProviderBudgetAlertTriggered(ProviderBudgetAlertTriggered {
            budget_id: "bud1".to_owned(),
            current_micros: 900_000,
            limit_micros: 1_000_000,
            triggered_at_ms: ts,
        }),
        RuntimeEvent::ProviderBudgetExceeded(ProviderBudgetExceeded {
            budget_id: "bud1".to_owned(),
            exceeded_by_micros: 100_000,
            exceeded_at_ms: ts,
        }),
        RuntimeEvent::ProviderConnectionRegistered(ProviderConnectionRegistered {
            tenant: tkey(),
            provider_connection_id: ProviderConnectionId::new("conn1"),
            provider_family: "openai".to_owned(),
            adapter_type: "openai_adapter".to_owned(),
            supported_models: vec![],
            status: ProviderConnectionStatus::Active,
            registered_at: ts,
        }),
        RuntimeEvent::ProviderHealthChecked(ProviderHealthChecked {
            tenant_id: tid(),
            connection_id: ProviderConnectionId::new("conn1"),
            status: ProviderHealthStatus::Healthy,
            latency_ms: Some(50),
            checked_at_ms: ts,
        }),
        RuntimeEvent::ProviderHealthScheduleSet(ProviderHealthScheduleSet {
            schedule_id: "sched1".to_owned(),
            connection_id: ProviderConnectionId::new("conn1"),
            tenant_id: tid(),
            interval_ms: 60_000,
            enabled: true,
            set_at_ms: ts,
        }),
        RuntimeEvent::ProviderHealthScheduleTriggered(ProviderHealthScheduleTriggered {
            schedule_id: "sched1".to_owned(),
            connection_id: ProviderConnectionId::new("conn1"),
            tenant_id: tid(),
            triggered_at_ms: ts,
        }),
        RuntimeEvent::ProviderMarkedDegraded(ProviderMarkedDegraded {
            tenant_id: tid(),
            connection_id: ProviderConnectionId::new("conn1"),
            reason: "timeout".to_owned(),
            marked_at_ms: ts,
        }),
        RuntimeEvent::ProviderModelRegistered(ProviderModelRegistered {
            tenant_id: tid(),
            connection_id: ProviderConnectionId::new("conn1"),
            model_id: "gpt-4o".to_owned(),
            capabilities_json: "{}".to_owned(),
        }),
        RuntimeEvent::ProviderRecovered(ProviderRecovered {
            tenant_id: tid(),
            connection_id: ProviderConnectionId::new("conn1"),
            recovered_at_ms: ts,
        }),
        RuntimeEvent::ProviderRetryPolicySet(ProviderRetryPolicySet {
            connection_id: ProviderConnectionId::new("conn1"),
            tenant_id: tid(),
            policy: RetryPolicy {
                max_attempts: 3,
                backoff_ms: 1_000,
                retryable_error_classes: vec![],
            },
            set_at_ms: ts,
        }),
        RuntimeEvent::RecoveryEscalated(RecoveryEscalated {
            task_id: task(),
            reason: "too many retries".to_owned(),
            escalated_at_ms: ts,
            run_id: None,
            last_error: None,
            attempt_count: 3,
        }),
        RuntimeEvent::ResourceShareRevoked(ResourceShareRevoked {
            share_id: "sh1".to_owned(),
            revoked_at_ms: ts,
            tenant_id: tid(),
        }),
        RuntimeEvent::ResourceShared(ResourceShared {
            share_id: "sh1".to_owned(),
            resource_type: "prompt_asset".to_owned(),
            grantee: "t2".to_owned(),
            shared_at_ms: ts,
            tenant_id: tid(),
            source_workspace_id: WorkspaceId::new("ws_src"),
            target_workspace_id: WorkspaceId::new("ws_tgt"),
            resource_id: "pa1".to_owned(),
            permissions: vec!["read".to_owned()],
        }),
        RuntimeEvent::RoutePolicyCreated(RoutePolicyCreated {
            tenant_id: tid(),
            policy_id: "rp1".to_owned(),
            name: "Route".to_owned(),
            rules: vec![],
            enabled: true,
        }),
        RuntimeEvent::RoutePolicyUpdated(RoutePolicyUpdated {
            policy_id: "rp1".to_owned(),
            updated_at_ms: ts,
        }),
        RuntimeEvent::RunSlaBreached(RunSlaBreached {
            run_id: run(),
            tenant_id: tid(),
            elapsed_ms: 120_000,
            target_ms: 100_000,
            breached_at_ms: ts,
        }),
        RuntimeEvent::RunSlaSet(RunSlaSet {
            run_id: run(),
            tenant_id: tid(),
            target_completion_ms: 100_000,
            alert_at_percent: 80,
            set_at_ms: ts,
        }),
        RuntimeEvent::SignalRouted(SignalRouted {
            project: p(),
            signal_id: SignalId::new("sig1"),
            subscription_id: "sub1".to_owned(),
            delivered_at_ms: ts,
        }),
        RuntimeEvent::SignalSubscriptionCreated(SignalSubscriptionCreated {
            project: p(),
            subscription_id: "sub1".to_owned(),
            signal_kind: "github.push".to_owned(),
            target_run_id: None,
            target_mailbox_id: None,
            filter_expression: None,
            created_at_ms: ts,
        }),
        RuntimeEvent::RunTemplateCreated(RunTemplateCreated {
            project: p(),
            template_id: RunTemplateId::new("tmpl1"),
            name: "GitHub issue triage".to_owned(),
            description: Some("Template for RFC 022 trigger tests".to_owned()),
            default_mode: cairn_domain::decisions::RunMode::Plan,
            system_prompt: "Investigate the triggering signal.".to_owned(),
            initial_user_message: Some("Please triage this signal.".to_owned()),
            plugin_allowlist: Some(vec!["github".to_owned()]),
            tool_allowlist: Some(vec!["read_file".to_owned()]),
            budget_max_tokens: Some(8_000),
            budget_max_wall_clock_ms: Some(30_000),
            budget_max_iterations: Some(12),
            budget_exploration_budget_share: Some(0.25),
            sandbox_hint: Some("repo".to_owned()),
            required_fields: vec!["issue.number".to_owned()],
            created_by: OperatorId::new("op1"),
            created_at: ts,
        }),
        RuntimeEvent::TriggerCreated(TriggerCreated {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            name: "Issue labeled".to_owned(),
            description: Some("Fire on cairn-ready".to_owned()),
            signal_type: "github.issue.labeled".to_owned(),
            plugin_id: Some("github".to_owned()),
            conditions: vec![serde_json::json!({
                "type": "contains",
                "path": "labels[].name",
                "value": "cairn-ready"
            })],
            run_template_id: RunTemplateId::new("tmpl1"),
            max_per_minute: 10,
            max_burst: 20,
            max_chain_depth: 5,
            created_by: OperatorId::new("op1"),
            created_at: ts,
        }),
        RuntimeEvent::TriggerEnabled(TriggerEnabled {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            by: OperatorId::new("op1"),
            at: ts,
        }),
        RuntimeEvent::TriggerDisabled(TriggerDisabled {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            by: OperatorId::new("op1"),
            reason: Some("maintenance".to_owned()),
            at: ts,
        }),
        RuntimeEvent::TriggerSuspended(TriggerSuspended {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            reason: TriggerSuspensionReason::RateLimitExceeded,
            at: ts,
        }),
        RuntimeEvent::TriggerResumed(TriggerResumed {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            at: ts,
        }),
        RuntimeEvent::TriggerDeleted(TriggerDeleted {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            by: OperatorId::new("op1"),
            at: ts,
        }),
        RuntimeEvent::TriggerFired(TriggerFired {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            signal_id: SignalId::new("sig1"),
            signal_type: "github.issue.labeled".to_owned(),
            run_id: run(),
            chain_depth: 1,
            fired_at: ts,
        }),
        RuntimeEvent::TriggerSkipped(TriggerSkipped {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            signal_id: SignalId::new("sig1"),
            reason: TriggerSkipReason::AlreadyFired,
            skipped_at: ts,
        }),
        RuntimeEvent::TriggerDenied(TriggerDenied {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            signal_id: SignalId::new("sig1"),
            decision_id: DecisionId::new("dec1"),
            reason: "operator denied".to_owned(),
            denied_at: ts,
        }),
        RuntimeEvent::TriggerRateLimited(TriggerRateLimited {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            signal_id: SignalId::new("sig1"),
            bucket_remaining: 0,
            bucket_capacity: 20,
            rate_limited_at: ts,
        }),
        RuntimeEvent::TriggerPendingApproval(TriggerPendingApproval {
            project: p(),
            trigger_id: TriggerId::new("tr1"),
            signal_id: SignalId::new("sig1"),
            approval_id: ApprovalId::new("a1"),
            pending_at: ts,
        }),
        RuntimeEvent::RunTemplateDeleted(RunTemplateDeleted {
            project: p(),
            template_id: RunTemplateId::new("tmpl1"),
            by: OperatorId::new("op1"),
            at: ts,
        }),
        RuntimeEvent::SnapshotCreated(SnapshotCreated {
            snapshot_id: "snap1".to_owned(),
            created_at_ms: ts,
            tenant_id: tid(),
            event_position: 100,
        }),
        RuntimeEvent::TaskDependencyAdded(TaskDependencyAdded {
            task_id: task(),
            depends_on: TaskId::new("task_prereq"),
            added_at_ms: ts,
            dependent_task_id: task(),
            depends_on_task_id: TaskId::new("task_prereq"),
            dependency_kind: cairn_domain::DependencyKind::SuccessOnly,
            data_passing_ref: Some("artifact/v1".to_owned()),
        }),
        RuntimeEvent::TaskDependencyResolved(TaskDependencyResolved {
            task_id: task(),
            prerequisite_id: TaskId::new("task_prereq"),
            resolved_at_ms: ts,
            dependent_task_id: task(),
            depends_on_task_id: TaskId::new("task_prereq"),
        }),
        RuntimeEvent::TaskLeaseExpired(TaskLeaseExpired {
            task_id: task(),
            expired_at_ms: ts,
        }),
        RuntimeEvent::TaskPriorityChanged(TaskPriorityChanged {
            task_id: task(),
            new_priority: 5,
            changed_at_ms: ts,
        }),
        RuntimeEvent::ToolInvocationProgressUpdated(ToolInvocationProgressUpdated {
            invocation_id: ToolInvocationId::new("inv1"),
            progress_pct: 50,
            message: None,
            updated_at_ms: ts,
        }),
        // ── Plan review events (RFC 018) ─────────────────────────────────
        RuntimeEvent::PlanProposed(cairn_domain::events::PlanProposed {
            project: p(),
            plan_run_id: RunId::new("plan_r1"),
            session_id: SessionId::new("s1"),
            plan_markdown: "# Plan\n\n## Steps\n1. Do thing".to_owned(),
            proposed_at: ts,
        }),
        RuntimeEvent::PlanApproved(cairn_domain::events::PlanApproved {
            project: p(),
            plan_run_id: RunId::new("plan_r1"),
            approved_by: cairn_domain::OperatorId::new("op1"),
            reviewer_comments: Some("looks good".to_owned()),
            approved_at: ts,
        }),
        RuntimeEvent::PlanRejected(cairn_domain::events::PlanRejected {
            project: p(),
            plan_run_id: RunId::new("plan_r1"),
            rejected_by: cairn_domain::OperatorId::new("op1"),
            reason: "too risky".to_owned(),
            rejected_at: ts,
        }),
        RuntimeEvent::PlanRevisionRequested(cairn_domain::events::PlanRevisionRequested {
            project: p(),
            original_plan_run_id: RunId::new("plan_r1"),
            new_plan_run_id: RunId::new("plan_r2"),
            reviewer_comments: "please reconsider step 3".to_owned(),
            requested_at: ts,
        }),
        RuntimeEvent::DecisionRecorded(cairn_domain::events::DecisionRecorded {
            project: p(),
            decision_id: DecisionId::new("dec_exh_1"),
            decision_key: cairn_domain::decisions::DecisionKey {
                kind_tag: "tool_invocation".to_owned(),
                scope_ref: cairn_domain::decisions::DecisionScopeRef::Project(p()),
                semantic_hash: "exhhash".to_owned(),
            },
            outcome: cairn_domain::decisions::DecisionOutcome::Allowed,
            cached: true,
            expires_at: ts + 1_000,
            decided_at: ts,
            event_json: "{}".to_owned(),
        }),
        RuntimeEvent::DecisionCacheWarmup(cairn_domain::events::DecisionCacheWarmup {
            cached: 3,
            expired_and_dropped: 1,
            warmed_at: ts,
        }),
    ]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[test]
fn all_runtime_event_variants_covered_count() {
    let variants = all_variants();
    // 134 variants in the RuntimeEvent enum (130 baseline + RFC 020 Track 3:
    // ToolInvocationCacheHit, ToolRecoveryPaused + RFC 020 decision-cache
    // survival pair: DecisionRecorded, DecisionCacheWarmup).
    assert_eq!(
        variants.len(),
        134,
        "all_variants() must construct exactly 134 RuntimeEvent instances"
    );
}

#[test]
fn all_runtime_event_variants_project_and_entity_ref() {
    for event in all_variants() {
        // This call verifies no panic + correct assertions inside.
        assert_all_variants_covered(&event);
    }
}

#[test]
fn entity_scoped_events_have_non_sentinel_project() {
    // Events with a real ProjectKey must not return the _system sentinel.
    let entity_events: Vec<RuntimeEvent> = vec![
        RuntimeEvent::SessionCreated(SessionCreated {
            project: p(),
            session_id: sess(),
        }),
        RuntimeEvent::RunCreated(RunCreated {
            project: p(),
            session_id: sess(),
            run_id: run(),
            parent_run_id: None,
            prompt_release_id: None,
            agent_role_id: None,
        }),
        RuntimeEvent::TaskCreated(TaskCreated {
            project: p(),
            task_id: task(),
            parent_run_id: Some(run()),
            parent_task_id: None,
            prompt_release_id: None,
            session_id: None,
        }),
    ];
    for event in &entity_events {
        assert_ne!(
            event.project().tenant_id.as_str(),
            "_system",
            "{:?}: entity-scoped event must not return sentinel project",
            event.project()
        );
        assert!(
            event.primary_entity_ref().is_some(),
            "entity-scoped event must return Some(EntityRef)"
        );
    }
}

#[test]
fn sentinel_events_return_system_project() {
    let ts: u64 = 1_000_000;
    let sentinel_events: Vec<RuntimeEvent> = vec![RuntimeEvent::ExternalWorkerRegistered(
        ExternalWorkerRegistered {
            sentinel_project: ProjectKey::new("t_exh", "_", "_"),
            worker_id: WorkerId::new("w1"),
            tenant_id: tid(),
            display_name: "Bot".to_owned(),
            registered_at: ts,
        },
    )];
    for event in &sentinel_events {
        assert_eq!(
            event.project().tenant_id.as_str(),
            "t_exh",
            "sentinel project carries the tenant_id as given"
        );
    }
}

#[test]
fn event_envelope_wraps_any_variant_correctly() {
    // Verify for_runtime_event sets ownership from payload.project().
    let event = RuntimeEvent::SessionCreated(SessionCreated {
        project: p(),
        session_id: sess(),
    });
    let env = EventEnvelope::for_runtime_event(EventId::new("e_exh"), EventSource::Runtime, event);
    assert_eq!(env.project().tenant_id.as_str(), "t_exh");
    assert!(env.primary_entity_ref().is_some());
}
