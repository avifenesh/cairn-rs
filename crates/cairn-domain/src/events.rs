use crate::errors::RuntimeEntityRef;
use crate::ids::{
    ApprovalId, CheckpointId, DecisionId, EvalRunId, EventId, IngestJobId, MailboxMessageId,
    OperatorId, OutcomeId, PromptAssetId, PromptReleaseId, PromptVersionId, ProviderBindingId,
    ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId, RouteDecisionId, RunId,
    RunTemplateId, ScheduledTaskId, SessionId, SignalId, TaskId, TenantId, ToolInvocationId,
    TriggerId, WorkspaceId,
};
use crate::lifecycle::{
    CheckpointDisposition, FailureClass, PauseReason, ResumeTrigger, RunState, SessionState,
    TaskState,
};
use crate::policy::{ApprovalDecision, ApprovalRequirement, ExecutionClass};
use crate::tenancy::{OwnershipKey, ProjectKey};
use crate::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
use crate::workers::ExternalWorkerReport;
use serde::{Deserialize, Serialize};

/// Shared event envelope for canonical product events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventEnvelope<T> {
    pub event_id: EventId,
    pub source: EventSource,
    pub ownership: OwnershipKey,
    pub causation_id: Option<crate::ids::CommandId>,
    pub correlation_id: Option<String>,
    pub payload: T,
}

impl<T> EventEnvelope<T> {
    pub fn new(
        event_id: impl Into<EventId>,
        source: EventSource,
        ownership: impl Into<OwnershipKey>,
        payload: T,
    ) -> Self {
        Self {
            event_id: event_id.into(),
            source,
            ownership: ownership.into(),
            causation_id: None,
            correlation_id: None,
            payload,
        }
    }

    pub fn with_causation_id(mut self, causation_id: impl Into<crate::ids::CommandId>) -> Self {
        self.causation_id = Some(causation_id.into());
        self
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }
}

impl EventEnvelope<RuntimeEvent> {
    pub fn for_runtime_event(
        event_id: impl Into<EventId>,
        source: EventSource,
        payload: RuntimeEvent,
    ) -> Self {
        let ownership = payload.project().clone();
        Self::new(event_id, source, ownership, payload)
    }

    pub fn project(&self) -> &ProjectKey {
        self.payload.project()
    }

    pub fn primary_entity_ref(&self) -> Option<RuntimeEntityRef> {
        self.payload.primary_entity_ref()
    }
}

/// Event source information used by runtime, operators, and workers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source_type", rename_all = "snake_case")]
pub enum EventSource {
    Operator { operator_id: crate::ids::OperatorId },
    Runtime,
    Scheduler,
    ExternalWorker { worker: String },
    System,
}

/// Minimal runtime event set used as the Week 1 shared contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RuntimeEvent {
    SessionCreated(SessionCreated),
    SessionStateChanged(SessionStateChanged),
    RunCreated(RunCreated),
    RunStateChanged(RunStateChanged),
    TaskCreated(TaskCreated),
    TaskLeaseClaimed(TaskLeaseClaimed),
    TaskLeaseHeartbeated(TaskLeaseHeartbeated),
    TaskStateChanged(TaskStateChanged),
    ApprovalRequested(ApprovalRequested),
    ApprovalResolved(ApprovalResolved),
    CheckpointRecorded(CheckpointRecorded),
    CheckpointRestored(CheckpointRestored),
    MailboxMessageAppended(MailboxMessageAppended),
    ToolInvocationStarted(ToolInvocationStarted),
    ToolInvocationCompleted(ToolInvocationCompleted),
    ToolInvocationFailed(ToolInvocationFailed),
    SignalIngested(SignalIngested),
    ExternalWorkerRegistered(ExternalWorkerRegistered),
    ExternalWorkerReported(ExternalWorkerReported),
    ExternalWorkerSuspended(ExternalWorkerSuspended),
    ExternalWorkerReactivated(ExternalWorkerReactivated),
    SubagentSpawned(SubagentSpawned),
    RecoveryAttempted(RecoveryAttempted),
    RecoveryCompleted(RecoveryCompleted),
    UserMessageAppended(UserMessageAppended),
    IngestJobStarted(IngestJobStarted),
    IngestJobCompleted(IngestJobCompleted),
    EvalRunStarted(EvalRunStarted),
    EvalRunCompleted(EvalRunCompleted),
    PromptAssetCreated(PromptAssetCreated),
    PromptVersionCreated(PromptVersionCreated),
    ApprovalPolicyCreated(ApprovalPolicyCreated),
    PromptReleaseCreated(PromptReleaseCreated),
    PromptReleaseTransitioned(PromptReleaseTransitioned),
    /// RFC 001: gradual traffic rollout started.
    PromptRolloutStarted(PromptRolloutStarted),
    TenantCreated(TenantCreated),
    WorkspaceCreated(WorkspaceCreated),
    ProjectCreated(ProjectCreated),
    RouteDecisionMade(RouteDecisionMade),
    ProviderCallCompleted(ProviderCallCompleted),
    SoulPatchProposed(SoulPatchProposed),
    SoulPatchApplied(SoulPatchApplied),
    /// GAP-006: session-level accumulated cost updated after a provider call.
    SessionCostUpdated(SessionCostUpdated),
    /// Run-level cost updated after a provider call.
    RunCostUpdated(RunCostUpdated),
    /// GAP-006: tenant-level spend alert triggered.
    SpendAlertTriggered(SpendAlertTriggered),
    ProviderBudgetSet(ProviderBudgetSet),
    ChannelCreated(ChannelCreated),
    ChannelMessageSent(ChannelMessageSent),
    ChannelMessageConsumed(ChannelMessageConsumed),
    DefaultSettingSet(DefaultSettingSet),
    DefaultSettingCleared(DefaultSettingCleared),
    LicenseActivated(LicenseActivated),
    EntitlementOverrideSet(EntitlementOverrideSet),
    NotificationPreferenceSet(NotificationPreferenceSet),
    NotificationSent(NotificationSent),
    ProviderPoolCreated(ProviderPoolCreated),
    ProviderPoolConnectionAdded(ProviderPoolConnectionAdded),
    ProviderPoolConnectionRemoved(ProviderPoolConnectionRemoved),
    TenantQuotaSet(TenantQuotaSet),
    TenantQuotaViolated(TenantQuotaViolated),
    RetentionPolicySet(RetentionPolicySet),
    RunCostAlertSet(RunCostAlertSet),
    RunCostAlertTriggered(RunCostAlertTriggered),
    WorkspaceMemberAdded(WorkspaceMemberAdded),
    WorkspaceMemberRemoved(WorkspaceMemberRemoved),
    // ── Second-wave events ──────────────────────────────────────────────────
    ApprovalDelegated(ApprovalDelegated),
    AuditLogEntryRecorded(AuditLogEntryRecorded),
    CheckpointStrategySet(CheckpointStrategySet),
    CredentialKeyRotated(CredentialKeyRotated),
    CredentialRevoked(CredentialRevoked),
    CredentialStored(CredentialStored),
    EvalBaselineLocked(EvalBaselineLocked),
    EvalBaselineSet(EvalBaselineSet),
    EvalDatasetCreated(EvalDatasetCreated),
    EvalDatasetEntryAdded(EvalDatasetEntryAdded),
    EvalRubricCreated(EvalRubricCreated),
    EventLogCompacted(EventLogCompacted),
    GuardrailPolicyCreated(GuardrailPolicyCreated),
    GuardrailPolicyEvaluated(GuardrailPolicyEvaluated),
    OperatorIntervention(OperatorIntervention),
    OperatorProfileCreated(OperatorProfileCreated),
    OperatorProfileUpdated(OperatorProfileUpdated),
    PauseScheduled(PauseScheduled),
    PermissionDecisionRecorded(PermissionDecisionRecorded),
    ProviderBindingCreated(ProviderBindingCreated),
    ProviderBindingStateChanged(ProviderBindingStateChanged),
    ProviderBudgetAlertTriggered(ProviderBudgetAlertTriggered),
    ProviderBudgetExceeded(ProviderBudgetExceeded),
    ProviderConnectionRegistered(ProviderConnectionRegistered),
    ProviderHealthChecked(ProviderHealthChecked),
    ProviderHealthScheduleSet(ProviderHealthScheduleSet),
    ProviderHealthScheduleTriggered(ProviderHealthScheduleTriggered),
    ProviderMarkedDegraded(ProviderMarkedDegraded),
    ProviderModelRegistered(ProviderModelRegistered),
    ProviderRecovered(ProviderRecovered),
    ProviderRetryPolicySet(ProviderRetryPolicySet),
    RecoveryEscalated(RecoveryEscalated),
    ResourceShareRevoked(ResourceShareRevoked),
    ResourceShared(ResourceShared),
    RoutePolicyCreated(RoutePolicyCreated),
    RoutePolicyUpdated(RoutePolicyUpdated),
    RunSlaBreached(RunSlaBreached),
    RunSlaSet(RunSlaSet),
    SignalRouted(SignalRouted),
    SignalSubscriptionCreated(SignalSubscriptionCreated),
    TriggerCreated(TriggerCreated),
    TriggerEnabled(TriggerEnabled),
    TriggerDisabled(TriggerDisabled),
    TriggerSuspended(TriggerSuspended),
    TriggerResumed(TriggerResumed),
    TriggerDeleted(TriggerDeleted),
    TriggerFired(TriggerFired),
    TriggerSkipped(TriggerSkipped),
    TriggerDenied(TriggerDenied),
    TriggerRateLimited(TriggerRateLimited),
    TriggerPendingApproval(TriggerPendingApproval),
    RunTemplateCreated(RunTemplateCreated),
    RunTemplateDeleted(RunTemplateDeleted),
    SnapshotCreated(SnapshotCreated),
    TaskDependencyAdded(TaskDependencyAdded),
    TaskDependencyResolved(TaskDependencyResolved),
    TaskLeaseExpired(TaskLeaseExpired),
    TaskPriorityChanged(TaskPriorityChanged),
    ToolInvocationProgressUpdated(ToolInvocationProgressUpdated),
    /// Evaluator–optimizer feedback loop: agents record observed outcomes
    /// so downstream eval pipelines can compare against expected outcomes.
    OutcomeRecorded(OutcomeRecorded),
    /// A tenant-scoped scheduled task was registered.
    ScheduledTaskCreated(ScheduledTaskCreated),
    // ── Plan review events (RFC 018) ──────────────────────────────────────
    /// A Plan-mode run emitted a `<proposed_plan>` artifact.
    PlanProposed(PlanProposed),
    /// An operator approved a plan artifact; next step is creating an Execute run.
    PlanApproved(PlanApproved),
    /// An operator rejected a plan artifact.
    PlanRejected(PlanRejected),
    /// An operator requested a revision; a new Plan-mode run was created.
    PlanRevisionRequested(PlanRevisionRequested),
}

impl RuntimeEvent {
    pub fn project(&self) -> &ProjectKey {
        match self {
            RuntimeEvent::SessionCreated(event) => &event.project,
            RuntimeEvent::SessionStateChanged(event) => &event.project,
            RuntimeEvent::RunCreated(event) => &event.project,
            RuntimeEvent::RunStateChanged(event) => &event.project,
            RuntimeEvent::TaskCreated(event) => &event.project,
            RuntimeEvent::TaskLeaseClaimed(event) => &event.project,
            RuntimeEvent::TaskLeaseHeartbeated(event) => &event.project,
            RuntimeEvent::TaskStateChanged(event) => &event.project,
            RuntimeEvent::ApprovalRequested(event) => &event.project,
            RuntimeEvent::ApprovalResolved(event) => &event.project,
            RuntimeEvent::CheckpointRecorded(event) => &event.project,
            RuntimeEvent::CheckpointRestored(event) => &event.project,
            RuntimeEvent::MailboxMessageAppended(event) => &event.project,
            RuntimeEvent::ToolInvocationStarted(event) => &event.project,
            RuntimeEvent::ToolInvocationCompleted(event) => &event.project,
            RuntimeEvent::ToolInvocationFailed(event) => &event.project,
            RuntimeEvent::SignalIngested(event) => &event.project,
            RuntimeEvent::ExternalWorkerRegistered(event) => &event.sentinel_project,
            RuntimeEvent::ExternalWorkerReported(event) => &event.report.project,
            RuntimeEvent::ExternalWorkerSuspended(event) => &event.sentinel_project,
            RuntimeEvent::ExternalWorkerReactivated(event) => &event.sentinel_project,
            RuntimeEvent::SubagentSpawned(event) => &event.project,
            RuntimeEvent::RecoveryAttempted(event) => &event.project,
            RuntimeEvent::RecoveryCompleted(event) => &event.project,
            RuntimeEvent::UserMessageAppended(event) => &event.project,
            RuntimeEvent::IngestJobStarted(event) => &event.project,
            RuntimeEvent::IngestJobCompleted(event) => &event.project,
            RuntimeEvent::EvalRunStarted(event) => &event.project,
            RuntimeEvent::EvalRunCompleted(event) => &event.project,
            RuntimeEvent::PromptAssetCreated(event) => &event.project,
            RuntimeEvent::PromptVersionCreated(event) => &event.project,
            RuntimeEvent::ApprovalPolicyCreated(event) => &event.project,
            RuntimeEvent::PromptReleaseCreated(event) => &event.project,
            RuntimeEvent::PromptReleaseTransitioned(event) => &event.project,
            RuntimeEvent::PromptRolloutStarted(event) => &event.project,
            RuntimeEvent::TenantCreated(event) => &event.project,
            RuntimeEvent::WorkspaceCreated(event) => &event.project,
            RuntimeEvent::ProjectCreated(event) => &event.project,
            RuntimeEvent::RouteDecisionMade(event) => &event.project,
            RuntimeEvent::ProviderCallCompleted(event) => &event.project,
            RuntimeEvent::SoulPatchProposed(event) => &event.project,
            RuntimeEvent::SoulPatchApplied(event) => &event.project,
            RuntimeEvent::SessionCostUpdated(event) => &event.project,
            RuntimeEvent::RunCostUpdated(event) => &event.project,
            RuntimeEvent::SpendAlertTriggered(event) => &event.project,
            RuntimeEvent::OutcomeRecorded(event) => &event.project,
            RuntimeEvent::PlanProposed(event) => &event.project,
            RuntimeEvent::PlanApproved(event) => &event.project,
            RuntimeEvent::PlanRejected(event) => &event.project,
            RuntimeEvent::PlanRevisionRequested(event) => &event.project,
            RuntimeEvent::TriggerCreated(event) => &event.project,
            RuntimeEvent::TriggerEnabled(event) => &event.project,
            RuntimeEvent::TriggerDisabled(event) => &event.project,
            RuntimeEvent::TriggerSuspended(event) => &event.project,
            RuntimeEvent::TriggerResumed(event) => &event.project,
            RuntimeEvent::TriggerDeleted(event) => &event.project,
            RuntimeEvent::TriggerFired(event) => &event.project,
            RuntimeEvent::TriggerSkipped(event) => &event.project,
            RuntimeEvent::TriggerDenied(event) => &event.project,
            RuntimeEvent::TriggerRateLimited(event) => &event.project,
            RuntimeEvent::TriggerPendingApproval(event) => &event.project,
            RuntimeEvent::RunTemplateCreated(event) => &event.project,
            RuntimeEvent::RunTemplateDeleted(event) => &event.project,
            RuntimeEvent::ProviderBudgetSet(_)
            | RuntimeEvent::ChannelCreated(_)
            | RuntimeEvent::ChannelMessageSent(_)
            | RuntimeEvent::ChannelMessageConsumed(_)
            | RuntimeEvent::DefaultSettingSet(_)
            | RuntimeEvent::DefaultSettingCleared(_)
            | RuntimeEvent::LicenseActivated(_)
            | RuntimeEvent::EntitlementOverrideSet(_)
            | RuntimeEvent::NotificationPreferenceSet(_)
            | RuntimeEvent::NotificationSent(_)
            | RuntimeEvent::ProviderPoolCreated(_)
            | RuntimeEvent::ProviderPoolConnectionAdded(_)
            | RuntimeEvent::ProviderPoolConnectionRemoved(_)
            | RuntimeEvent::TenantQuotaSet(_)
            | RuntimeEvent::TenantQuotaViolated(_)
            | RuntimeEvent::RetentionPolicySet(_)
            | RuntimeEvent::RunCostAlertSet(_)
            | RuntimeEvent::RunCostAlertTriggered(_)
            | RuntimeEvent::WorkspaceMemberAdded(_)
            | RuntimeEvent::WorkspaceMemberRemoved(_)
            | RuntimeEvent::ApprovalDelegated(_)
            | RuntimeEvent::AuditLogEntryRecorded(_)
            | RuntimeEvent::CheckpointStrategySet(_)
            | RuntimeEvent::CredentialKeyRotated(_)
            | RuntimeEvent::CredentialRevoked(_)
            | RuntimeEvent::CredentialStored(_)
            | RuntimeEvent::EvalBaselineLocked(_)
            | RuntimeEvent::EvalBaselineSet(_)
            | RuntimeEvent::EvalDatasetCreated(_)
            | RuntimeEvent::EvalDatasetEntryAdded(_)
            | RuntimeEvent::EvalRubricCreated(_)
            | RuntimeEvent::EventLogCompacted(_)
            | RuntimeEvent::GuardrailPolicyCreated(_)
            | RuntimeEvent::GuardrailPolicyEvaluated(_)
            | RuntimeEvent::OperatorIntervention(_)
            | RuntimeEvent::OperatorProfileCreated(_)
            | RuntimeEvent::OperatorProfileUpdated(_)
            | RuntimeEvent::PauseScheduled(_)
            | RuntimeEvent::PermissionDecisionRecorded(_)
            | RuntimeEvent::ProviderBindingCreated(_)
            | RuntimeEvent::ProviderBindingStateChanged(_)
            | RuntimeEvent::ProviderBudgetAlertTriggered(_)
            | RuntimeEvent::ProviderBudgetExceeded(_)
            | RuntimeEvent::ProviderConnectionRegistered(_)
            | RuntimeEvent::ProviderHealthChecked(_)
            | RuntimeEvent::ProviderHealthScheduleSet(_)
            | RuntimeEvent::ProviderHealthScheduleTriggered(_)
            | RuntimeEvent::ProviderMarkedDegraded(_)
            | RuntimeEvent::ProviderModelRegistered(_)
            | RuntimeEvent::ProviderRecovered(_)
            | RuntimeEvent::ProviderRetryPolicySet(_)
            | RuntimeEvent::RecoveryEscalated(_)
            | RuntimeEvent::ResourceShareRevoked(_)
            | RuntimeEvent::ResourceShared(_)
            | RuntimeEvent::RoutePolicyCreated(_)
            | RuntimeEvent::RoutePolicyUpdated(_)
            | RuntimeEvent::RunSlaBreached(_)
            | RuntimeEvent::RunSlaSet(_)
            | RuntimeEvent::SignalRouted(_)
            | RuntimeEvent::SignalSubscriptionCreated(_)
            | RuntimeEvent::SnapshotCreated(_)
            | RuntimeEvent::TaskDependencyAdded(_)
            | RuntimeEvent::TaskDependencyResolved(_)
            | RuntimeEvent::TaskLeaseExpired(_)
            | RuntimeEvent::TaskPriorityChanged(_)
            | RuntimeEvent::ToolInvocationProgressUpdated(_)
            | RuntimeEvent::ScheduledTaskCreated(_) => {
                // These events are tenant-scoped rather than project-scoped.
                // Return a static placeholder key.
                static SYSTEM_KEY: std::sync::OnceLock<crate::tenancy::ProjectKey> =
                    std::sync::OnceLock::new();
                SYSTEM_KEY.get_or_init(|| {
                    crate::tenancy::ProjectKey::new(
                        crate::ids::TenantId::new("_system"),
                        crate::ids::WorkspaceId::new("_system"),
                        "_system".to_owned(),
                    )
                })
            }
        }
    }

    pub fn primary_entity_ref(&self) -> Option<RuntimeEntityRef> {
        match self {
            RuntimeEvent::SessionCreated(event) => Some(RuntimeEntityRef::Session {
                session_id: event.session_id.clone(),
            }),
            RuntimeEvent::SessionStateChanged(event) => Some(RuntimeEntityRef::Session {
                session_id: event.session_id.clone(),
            }),
            RuntimeEvent::RunCreated(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
            }),
            RuntimeEvent::RunStateChanged(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
            }),
            RuntimeEvent::TaskCreated(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::TaskLeaseClaimed(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::TaskLeaseHeartbeated(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::TaskStateChanged(event) => Some(RuntimeEntityRef::Task {
                task_id: event.task_id.clone(),
            }),
            RuntimeEvent::ApprovalRequested(event) => Some(RuntimeEntityRef::Approval {
                approval_id: event.approval_id.clone(),
            }),
            RuntimeEvent::ApprovalResolved(event) => Some(RuntimeEntityRef::Approval {
                approval_id: event.approval_id.clone(),
            }),
            RuntimeEvent::CheckpointRecorded(event) => Some(RuntimeEntityRef::Checkpoint {
                checkpoint_id: event.checkpoint_id.clone(),
            }),
            RuntimeEvent::CheckpointRestored(event) => Some(RuntimeEntityRef::Checkpoint {
                checkpoint_id: event.checkpoint_id.clone(),
            }),
            RuntimeEvent::MailboxMessageAppended(event) => Some(RuntimeEntityRef::MailboxMessage {
                message_id: event.message_id.clone(),
            }),
            RuntimeEvent::ToolInvocationStarted(event) => Some(RuntimeEntityRef::ToolInvocation {
                invocation_id: event.invocation_id.clone(),
            }),
            RuntimeEvent::ToolInvocationCompleted(event) => {
                Some(RuntimeEntityRef::ToolInvocation {
                    invocation_id: event.invocation_id.clone(),
                })
            }
            RuntimeEvent::ToolInvocationFailed(event) => Some(RuntimeEntityRef::ToolInvocation {
                invocation_id: event.invocation_id.clone(),
            }),
            RuntimeEvent::SignalIngested(event) => Some(RuntimeEntityRef::Signal {
                signal_id: event.signal_id.clone(),
            }),
            RuntimeEvent::ExternalWorkerRegistered(_) => None,
            RuntimeEvent::ExternalWorkerReported(event) => Some(RuntimeEntityRef::Task {
                task_id: event.report.task_id.clone(),
            }),
            RuntimeEvent::ExternalWorkerSuspended(_) => None,
            RuntimeEvent::ExternalWorkerReactivated(_) => None,
            RuntimeEvent::SubagentSpawned(event) => Some(RuntimeEntityRef::Task {
                task_id: event.child_task_id.clone(),
            }),
            RuntimeEvent::RecoveryAttempted(event) => event
                .task_id
                .clone()
                .map(|task_id| RuntimeEntityRef::Task { task_id })
                .or_else(|| {
                    event
                        .run_id
                        .clone()
                        .map(|run_id| RuntimeEntityRef::Run { run_id })
                }),
            RuntimeEvent::RecoveryCompleted(event) => event
                .task_id
                .clone()
                .map(|task_id| RuntimeEntityRef::Task { task_id })
                .or_else(|| {
                    event
                        .run_id
                        .clone()
                        .map(|run_id| RuntimeEntityRef::Run { run_id })
                }),
            RuntimeEvent::UserMessageAppended(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
            }),
            RuntimeEvent::IngestJobStarted(event) => Some(RuntimeEntityRef::IngestJob {
                job_id: event.job_id.clone(),
            }),
            RuntimeEvent::IngestJobCompleted(event) => Some(RuntimeEntityRef::IngestJob {
                job_id: event.job_id.clone(),
            }),
            RuntimeEvent::EvalRunStarted(event) => Some(RuntimeEntityRef::EvalRun {
                eval_run_id: event.eval_run_id.clone(),
            }),
            RuntimeEvent::EvalRunCompleted(event) => Some(RuntimeEntityRef::EvalRun {
                eval_run_id: event.eval_run_id.clone(),
            }),
            RuntimeEvent::OutcomeRecorded(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
            }),
            RuntimeEvent::PlanProposed(event) => Some(RuntimeEntityRef::Run {
                run_id: event.plan_run_id.clone(),
            }),
            RuntimeEvent::PlanApproved(event) => Some(RuntimeEntityRef::Run {
                run_id: event.plan_run_id.clone(),
            }),
            RuntimeEvent::PlanRejected(event) => Some(RuntimeEntityRef::Run {
                run_id: event.plan_run_id.clone(),
            }),
            RuntimeEvent::PlanRevisionRequested(event) => Some(RuntimeEntityRef::Run {
                run_id: event.original_plan_run_id.clone(),
            }),
            RuntimeEvent::PromptAssetCreated(event) => Some(RuntimeEntityRef::PromptAsset {
                prompt_asset_id: event.prompt_asset_id.clone(),
            }),
            RuntimeEvent::PromptVersionCreated(event) => Some(RuntimeEntityRef::PromptVersion {
                prompt_version_id: event.prompt_version_id.clone(),
            }),
            RuntimeEvent::ApprovalPolicyCreated(_) => None,
            RuntimeEvent::PromptReleaseCreated(event) => Some(RuntimeEntityRef::PromptRelease {
                prompt_release_id: event.prompt_release_id.clone(),
            }),
            RuntimeEvent::PromptReleaseTransitioned(event) => {
                Some(RuntimeEntityRef::PromptRelease {
                    prompt_release_id: event.prompt_release_id.clone(),
                })
            }
            RuntimeEvent::PromptRolloutStarted(event) => Some(RuntimeEntityRef::PromptRelease {
                prompt_release_id: event.prompt_release_id.clone(),
            }),
            RuntimeEvent::TenantCreated(_) => None,
            RuntimeEvent::WorkspaceCreated(_) => None,
            RuntimeEvent::ProjectCreated(_) => None,
            RuntimeEvent::RouteDecisionMade(_) => None,
            RuntimeEvent::ProviderCallCompleted(_) => None,
            RuntimeEvent::SoulPatchProposed(_) => None,
            RuntimeEvent::SoulPatchApplied(_) => None,
            RuntimeEvent::SessionCostUpdated(_) => None,
            RuntimeEvent::RunCostUpdated(_) => None,
            RuntimeEvent::SpendAlertTriggered(_) => None,
            RuntimeEvent::ProviderBudgetSet(_)
            | RuntimeEvent::ChannelCreated(_)
            | RuntimeEvent::ChannelMessageSent(_)
            | RuntimeEvent::ChannelMessageConsumed(_)
            | RuntimeEvent::DefaultSettingSet(_)
            | RuntimeEvent::DefaultSettingCleared(_)
            | RuntimeEvent::LicenseActivated(_)
            | RuntimeEvent::EntitlementOverrideSet(_)
            | RuntimeEvent::NotificationPreferenceSet(_)
            | RuntimeEvent::NotificationSent(_)
            | RuntimeEvent::ProviderPoolCreated(_)
            | RuntimeEvent::ProviderPoolConnectionAdded(_)
            | RuntimeEvent::ProviderPoolConnectionRemoved(_)
            | RuntimeEvent::TenantQuotaSet(_)
            | RuntimeEvent::TenantQuotaViolated(_)
            | RuntimeEvent::RetentionPolicySet(_)
            | RuntimeEvent::RunCostAlertSet(_)
            | RuntimeEvent::RunCostAlertTriggered(_)
            | RuntimeEvent::WorkspaceMemberAdded(_)
            | RuntimeEvent::WorkspaceMemberRemoved(_)
            | RuntimeEvent::ApprovalDelegated(_)
            | RuntimeEvent::AuditLogEntryRecorded(_)
            | RuntimeEvent::CheckpointStrategySet(_)
            | RuntimeEvent::CredentialKeyRotated(_)
            | RuntimeEvent::CredentialRevoked(_)
            | RuntimeEvent::CredentialStored(_)
            | RuntimeEvent::EvalBaselineLocked(_)
            | RuntimeEvent::EvalBaselineSet(_)
            | RuntimeEvent::EvalDatasetCreated(_)
            | RuntimeEvent::EvalDatasetEntryAdded(_)
            | RuntimeEvent::EvalRubricCreated(_)
            | RuntimeEvent::EventLogCompacted(_)
            | RuntimeEvent::GuardrailPolicyCreated(_)
            | RuntimeEvent::GuardrailPolicyEvaluated(_)
            | RuntimeEvent::OperatorIntervention(_)
            | RuntimeEvent::OperatorProfileCreated(_)
            | RuntimeEvent::OperatorProfileUpdated(_)
            | RuntimeEvent::PauseScheduled(_)
            | RuntimeEvent::PermissionDecisionRecorded(_)
            | RuntimeEvent::ProviderBindingCreated(_)
            | RuntimeEvent::ProviderBindingStateChanged(_)
            | RuntimeEvent::ProviderBudgetAlertTriggered(_)
            | RuntimeEvent::ProviderBudgetExceeded(_)
            | RuntimeEvent::ProviderConnectionRegistered(_)
            | RuntimeEvent::ProviderHealthChecked(_)
            | RuntimeEvent::ProviderHealthScheduleSet(_)
            | RuntimeEvent::ProviderHealthScheduleTriggered(_)
            | RuntimeEvent::ProviderMarkedDegraded(_)
            | RuntimeEvent::ProviderModelRegistered(_)
            | RuntimeEvent::ProviderRecovered(_)
            | RuntimeEvent::ProviderRetryPolicySet(_)
            | RuntimeEvent::RecoveryEscalated(_)
            | RuntimeEvent::ResourceShareRevoked(_)
            | RuntimeEvent::ResourceShared(_)
            | RuntimeEvent::RoutePolicyCreated(_)
            | RuntimeEvent::RoutePolicyUpdated(_)
            | RuntimeEvent::RunSlaBreached(_)
            | RuntimeEvent::RunSlaSet(_)
            | RuntimeEvent::SignalRouted(_)
            | RuntimeEvent::SignalSubscriptionCreated(_)
            | RuntimeEvent::TriggerCreated(_)
            | RuntimeEvent::TriggerEnabled(_)
            | RuntimeEvent::TriggerDisabled(_)
            | RuntimeEvent::TriggerSuspended(_)
            | RuntimeEvent::TriggerResumed(_)
            | RuntimeEvent::TriggerDeleted(_)
            | RuntimeEvent::TriggerFired(_)
            | RuntimeEvent::TriggerSkipped(_)
            | RuntimeEvent::TriggerDenied(_)
            | RuntimeEvent::TriggerRateLimited(_)
            | RuntimeEvent::TriggerPendingApproval(_)
            | RuntimeEvent::RunTemplateCreated(_)
            | RuntimeEvent::RunTemplateDeleted(_)
            | RuntimeEvent::SnapshotCreated(_)
            | RuntimeEvent::TaskDependencyAdded(_)
            | RuntimeEvent::TaskDependencyResolved(_)
            | RuntimeEvent::TaskLeaseExpired(_)
            | RuntimeEvent::TaskPriorityChanged(_)
            | RuntimeEvent::ToolInvocationProgressUpdated(_)
            | RuntimeEvent::ScheduledTaskCreated(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransition<S> {
    pub from: Option<S>,
    pub to: S,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCreated {
    pub project: ProjectKey,
    pub session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionStateChanged {
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub transition: StateTransition<SessionState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCreated {
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub parent_run_id: Option<RunId>,
    pub prompt_release_id: Option<crate::ids::PromptReleaseId>,
    /// GAP-011: optional agent role attached at run creation.
    #[serde(default)]
    pub agent_role_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunStateChanged {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub transition: StateTransition<RunState>,
    pub failure_class: Option<FailureClass>,
    pub pause_reason: Option<PauseReason>,
    pub resume_trigger: Option<ResumeTrigger>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskCreated {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub parent_run_id: Option<RunId>,
    pub parent_task_id: Option<TaskId>,
    pub prompt_release_id: Option<crate::ids::PromptReleaseId>,
    /// Session the task is scoped to. `None` for bare (session-less)
    /// tasks that route through the solo `task_to_execution_id` mint path.
    ///
    /// Kept optional so event streams written before this field existed
    /// still deserialize; the projection falls back to walking
    /// `parent_run_id → session` when the field is `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<SessionId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLeaseClaimed {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub lease_owner: String,
    pub lease_token: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLeaseHeartbeated {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub lease_token: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskStateChanged {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub transition: StateTransition<TaskState>,
    pub failure_class: Option<FailureClass>,
    pub pause_reason: Option<PauseReason>,
    pub resume_trigger: Option<ResumeTrigger>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRequested {
    pub project: ProjectKey,
    pub approval_id: ApprovalId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub requirement: ApprovalRequirement,
    /// What the agent wants to do (e.g., "Create PR for issue #18").
    #[serde(default)]
    pub title: Option<String>,
    /// Detailed context — the agent's proposal, reasoning, affected files.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalResolved {
    pub project: ProjectKey,
    pub approval_id: ApprovalId,
    pub decision: ApprovalDecision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecorded {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub checkpoint_id: CheckpointId,
    pub disposition: CheckpointDisposition,
    pub data: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRestored {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub checkpoint_id: CheckpointId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxMessageAppended {
    pub project: ProjectKey,
    pub message_id: MailboxMessageId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub from_run_id: Option<RunId>,
    #[serde(default)]
    pub from_task_id: Option<TaskId>,
    #[serde(default)]
    pub deliver_at_ms: u64,
    /// RFC 002: display name or agent ID of the message sender.
    #[serde(default)]
    pub sender: Option<String>,
    /// RFC 002: display name or agent ID of the intended recipient.
    #[serde(default)]
    pub recipient: Option<String>,
    /// RFC 002: full message body (may differ from content for structured payloads).
    #[serde(default)]
    pub body: Option<String>,
    /// RFC 002: epoch-ms when the message was created by the sender.
    #[serde(default)]
    pub sent_at: Option<u64>,
    /// Delivery lifecycle state.
    #[serde(default)]
    pub delivery_status: Option<MailboxDeliveryStatus>,
}

/// Mailbox delivery lifecycle.
///
/// Wire-compatible snake_case with the pre-enum `String` shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxDeliveryStatus {
    /// Message created, not yet delivered.
    Pending,
    /// Message deferred until `deliver_at_ms`.
    Scheduled,
    /// Delivered to the recipient's mailbox.
    Delivered,
    /// Delivery attempt failed terminally.
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationStarted {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub target: ToolInvocationTarget,
    pub execution_class: ExecutionClass,
    pub prompt_release_id: Option<crate::ids::PromptReleaseId>,
    pub requested_at_ms: u64,
    pub started_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationCompleted {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    pub finished_at_ms: u64,
    pub outcome: ToolInvocationOutcomeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationFailed {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    pub finished_at_ms: u64,
    pub outcome: ToolInvocationOutcomeKind,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerRegistered {
    /// Sentinel project key (tenant-scoped event has no project).
    pub sentinel_project: ProjectKey,
    pub worker_id: crate::ids::WorkerId,
    pub tenant_id: TenantId,
    pub display_name: String,
    pub registered_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerReported {
    pub report: ExternalWorkerReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerSuspended {
    pub sentinel_project: ProjectKey,
    pub worker_id: crate::ids::WorkerId,
    pub tenant_id: TenantId,
    pub suspended_at: u64,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalWorkerReactivated {
    pub sentinel_project: ProjectKey,
    pub worker_id: crate::ids::WorkerId,
    pub tenant_id: TenantId,
    pub reactivated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubagentSpawned {
    pub project: ProjectKey,
    pub parent_run_id: RunId,
    pub parent_task_id: Option<TaskId>,
    pub child_task_id: TaskId,
    pub child_session_id: SessionId,
    pub child_run_id: Option<RunId>,
}

/// Recovery attempt fact per RFC 002.
///
/// At least one of `run_id` or `task_id` MUST be present — a targetless
/// recovery event has no semantic meaning and indicates a caller bug.
/// Callers should assert `has_target()` before appending this event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryAttempted {
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub reason: String,
}

impl RecoveryAttempted {
    /// Returns `true` when the event targets at least one recoverable entity.
    ///
    /// RFC 002 requires recovery events to be anchored to a run or task.
    /// A `false` return indicates a malformed event (both fields absent).
    pub fn has_target(&self) -> bool {
        self.run_id.is_some() || self.task_id.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalIngested {
    pub project: ProjectKey,
    pub signal_id: SignalId,
    pub source: String,
    pub payload: serde_json::Value,
    pub timestamp_ms: u64,
}

/// Recovery completion fact per RFC 002.
///
/// At least one of `run_id` or `task_id` MUST be present — a targetless
/// recovery event has no semantic meaning and indicates a caller bug.
/// Callers should assert `has_target()` before appending this event.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryCompleted {
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub recovered: bool,
}

impl RecoveryCompleted {
    /// Returns `true` when the event targets at least one recoverable entity.
    ///
    /// RFC 002 requires recovery events to be anchored to a run or task.
    /// A `false` return indicates a malformed event (both fields absent).
    pub fn has_target(&self) -> bool {
        self.run_id.is_some() || self.task_id.is_some()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserMessageAppended {
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub run_id: RunId,
    /// The user's message text. Empty string when used as a bare signal
    /// (backward-compatible — `#[serde(default)]` on old events).
    #[serde(default)]
    pub content: String,
    /// Optional sequence number within the session for stable ordering.
    #[serde(default)]
    pub sequence: u64,
    /// Unix milliseconds when the message was appended.
    #[serde(default)]
    pub appended_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestJobStarted {
    pub project: ProjectKey,
    pub job_id: IngestJobId,
    pub source_id: Option<crate::ids::SourceId>,
    pub document_count: u32,
    pub started_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestJobCompleted {
    pub project: ProjectKey,
    pub job_id: IngestJobId,
    pub success: bool,
    pub error_message: Option<String>,
    pub completed_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalRunStarted {
    pub project: ProjectKey,
    pub eval_run_id: EvalRunId,
    pub subject_kind: String,
    pub evaluator_type: String,
    pub started_at: u64,
    /// Prompt linkage — populated by the API eval surface so runs can be
    /// reconstructed from the event log on restart.
    #[serde(default)]
    pub prompt_asset_id: Option<PromptAssetId>,
    #[serde(default)]
    pub prompt_version_id: Option<PromptVersionId>,
    #[serde(default)]
    pub prompt_release_id: Option<PromptReleaseId>,
    #[serde(default)]
    pub created_by: Option<OperatorId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalRunCompleted {
    pub project: ProjectKey,
    pub eval_run_id: EvalRunId,
    pub success: bool,
    pub error_message: Option<String>,
    /// Node ID of the subject being evaluated (e.g. prompt_release_id).
    pub subject_node_id: Option<String>,
    pub completed_at: u64,
}

/// Actual outcome classification for an agent execution.
///
/// Part of the evaluator–optimizer feedback loop: agents record predicted
/// confidence before execution and actual outcome after, enabling
/// self-correction of confidence calibration over time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActualOutcome {
    Success,
    Failure,
    Partial,
}

/// Outcome recorded after an agent run completes.
///
/// Links a run to its predicted confidence and actual result, forming the
/// feedback signal for confidence calibration and evaluator tuning.
///
/// **`predicted_confidence` contract:** expected to be finite and in
/// `[0.0, 1.0]`. Storage is raw `f64` because the value originates in an LLM
/// response; the `PartialEq`/`Eq` impls below use `f64::to_bits` so that a
/// stray `NaN` round-trips deterministically (two `NaN`s with identical bit
/// patterns compare equal, respecting `Eq`'s reflexivity rule).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OutcomeRecorded {
    pub project: ProjectKey,
    pub outcome_id: OutcomeId,
    pub run_id: RunId,
    /// Agent type that produced this outcome (e.g. "code_review", "research").
    pub agent_type: String,
    /// Confidence the agent predicted before execution [0.0, 1.0].
    pub predicted_confidence: f64,
    /// What actually happened.
    pub actual_outcome: ActualOutcome,
    pub recorded_at: u64,
}

impl PartialEq for OutcomeRecorded {
    fn eq(&self, other: &Self) -> bool {
        self.project == other.project
            && self.outcome_id == other.outcome_id
            && self.run_id == other.run_id
            && self.agent_type == other.agent_type
            && self.predicted_confidence.to_bits() == other.predicted_confidence.to_bits()
            && self.actual_outcome == other.actual_outcome
            && self.recorded_at == other.recorded_at
    }
}

impl Eq for OutcomeRecorded {}

/// A tenant-scoped scheduled task was registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTaskCreated {
    pub tenant_id: TenantId,
    pub scheduled_task_id: ScheduledTaskId,
    pub name: String,
    pub cron_expression: String,
    pub next_run_at: Option<u64>,
    pub created_at: u64,
}

// ── Plan review events (RFC 018) ─────────────────────────────────────────────

/// A Plan-mode run produced a plan artifact via `<proposed_plan>`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanProposed {
    pub project: ProjectKey,
    pub plan_run_id: RunId,
    pub session_id: SessionId,
    pub plan_markdown: String,
    pub proposed_at: u64,
}

/// An operator approved the plan artifact. Next step: create an Execute-mode run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanApproved {
    pub project: ProjectKey,
    pub plan_run_id: RunId,
    pub approved_by: OperatorId,
    pub reviewer_comments: Option<String>,
    pub approved_at: u64,
}

/// An operator rejected the plan artifact. No execution run will be created.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRejected {
    pub project: ProjectKey,
    pub plan_run_id: RunId,
    pub rejected_by: OperatorId,
    pub reason: String,
    pub rejected_at: u64,
}

/// An operator requested plan revision. A new Plan-mode run has been created.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRevisionRequested {
    pub project: ProjectKey,
    pub original_plan_run_id: RunId,
    pub new_plan_run_id: RunId,
    pub reviewer_comments: String,
    pub requested_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptAssetCreated {
    pub project: ProjectKey,
    pub prompt_asset_id: PromptAssetId,
    pub name: String,
    pub kind: String,
    pub created_at: u64,
    /// RFC 006: workspace scope — prompt assets belong to a workspace, not a project.
    /// Extracted from `project.workspace_id` at creation time so downstream projections
    /// can scope queries at workspace level without re-deriving from the full project key.
    #[serde(default)]
    pub workspace_id: WorkspaceId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptVersionCreated {
    pub project: ProjectKey,
    pub prompt_version_id: PromptVersionId,
    pub prompt_asset_id: PromptAssetId,
    pub content_hash: String,
    pub created_at: u64,
    /// RFC 006: workspace scope — prompt versions inherit workspace from the
    /// owning asset. Extracted from `project.workspace_id` at creation time
    /// so projections can scope at workspace level without re-deriving.
    #[serde(default)]
    pub workspace_id: WorkspaceId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptReleaseCreated {
    pub project: ProjectKey,
    pub prompt_release_id: PromptReleaseId,
    pub prompt_asset_id: PromptAssetId,
    pub prompt_version_id: PromptVersionId,
    pub created_at: u64,
    /// RFC 006: optional human-readable tag for this release (e.g. "v1.2-beta").
    #[serde(default)]
    pub release_tag: Option<String>,
    /// RFC 006: operator or service account that authored this release.
    #[serde(default)]
    pub created_by: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptReleaseTransitioned {
    pub project: ProjectKey,
    pub prompt_release_id: PromptReleaseId,
    pub from_state: String,
    pub to_state: String,
    pub transitioned_at: u64,
    /// RFC 006: actor (operator id or service account) that triggered the transition.
    #[serde(default)]
    pub actor: Option<String>,
    /// RFC 006: free-text reason supplied at transition time (e.g. "approved by QA").
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPolicyCreated {
    pub project: ProjectKey,
    pub policy_id: String,
    pub tenant_id: TenantId,
    pub name: String,
    pub required_approvers: u32,
    pub allowed_approver_roles: Vec<crate::tenancy::WorkspaceRole>,
    pub auto_approve_after_ms: Option<u64>,
    pub auto_reject_after_ms: Option<u64>,
    pub created_at_ms: u64,
}

/// RFC 001: emitted when a partial rollout (percentage-based traffic split) is started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptRolloutStarted {
    pub project: ProjectKey,
    pub prompt_release_id: PromptReleaseId,
    pub percent: u8,
    pub started_at: u64,
    /// Alias for prompt_release_id used by operator views.
    #[serde(default)]
    pub release_id: Option<PromptReleaseId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantCreated {
    pub project: ProjectKey,
    pub tenant_id: TenantId,
    pub name: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCreated {
    pub project: ProjectKey,
    pub workspace_id: WorkspaceId,
    pub tenant_id: TenantId,
    pub name: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectCreated {
    pub project: ProjectKey,
    pub name: String,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecisionMade {
    pub project: ProjectKey,
    pub route_decision_id: RouteDecisionId,
    pub operation_kind: crate::providers::OperationKind,
    pub selected_provider_binding_id: Option<ProviderBindingId>,
    pub final_status: crate::providers::RouteDecisionStatus,
    pub attempt_count: u16,
    pub fallback_used: bool,
    pub decided_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCallCompleted {
    pub project: ProjectKey,
    pub provider_call_id: ProviderCallId,
    pub route_decision_id: RouteDecisionId,
    pub route_attempt_id: RouteAttemptId,
    pub provider_binding_id: ProviderBindingId,
    pub provider_connection_id: ProviderConnectionId,
    pub provider_model_id: ProviderModelId,
    pub operation_kind: crate::providers::OperationKind,
    pub status: crate::providers::ProviderCallStatus,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cost_micros: Option<u64>,
    pub completed_at: u64,
    /// Session context for LLM observability trace derivation.
    #[serde(default)]
    pub session_id: Option<SessionId>,
    /// Run context for LLM observability trace derivation.
    #[serde(default)]
    pub run_id: Option<RunId>,
    /// Provider error class (None on success).
    #[serde(default)]
    pub error_class: Option<crate::providers::ProviderCallErrorClass>,
    /// Raw error text from the provider (None on success).
    #[serde(default)]
    pub raw_error_message: Option<String>,
    /// Retry attempt index (0 = first attempt).
    #[serde(default)]
    pub retry_count: u8,
    /// Task that triggered this provider call, if any.
    #[serde(default)]
    pub task_id: Option<TaskId>,
    /// Prompt release being executed at the time of the call.
    #[serde(default)]
    pub prompt_release_id: Option<PromptReleaseId>,
    /// Position in the fallback chain (0 = primary, 1 = first fallback, …).
    #[serde(default)]
    pub fallback_position: u32,
    /// Unix epoch ms when the call was dispatched to the provider.
    #[serde(default)]
    pub started_at: u64,
    /// Unix epoch ms when the provider response was received.
    #[serde(default)]
    pub finished_at: u64,
}

/// A soul patch has been proposed and is awaiting operator review.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulPatchProposed {
    pub project: ProjectKey,
    pub patch_id: String,
    pub patch_content: String,
    pub requires_approval: bool,
    pub proposed_at: u64,
}

/// An approved soul patch has been applied to the document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SoulPatchApplied {
    pub project: ProjectKey,
    pub patch_id: String,
    pub new_version: u32,
    pub applied_at: u64,
}
/// GAP-006: session-level accumulated cost delta from a completed provider call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCostUpdated {
    pub project: ProjectKey,
    pub session_id: crate::ids::SessionId,
    pub tenant_id: crate::ids::TenantId,
    /// Cost delta from this specific provider call, in USD micros.
    pub delta_cost_micros: u64,
    /// Input tokens consumed by this call.
    pub delta_tokens_in: u64,
    /// Output tokens produced by this call.
    pub delta_tokens_out: u64,
    /// Provider call that produced this cost update.
    pub provider_call_id: String,
    pub updated_at_ms: u64,
}

/// Run-level accumulated cost updated after a provider call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCostUpdated {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub delta_cost_micros: u64,
    pub delta_tokens_in: u64,
    pub delta_tokens_out: u64,
    pub provider_call_id: String,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub session_id: Option<crate::ids::SessionId>,
    #[serde(default)]
    pub tenant_id: Option<crate::ids::TenantId>,
}

/// GAP-006: tenant-level spend alert triggered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendAlertTriggered {
    pub project: ProjectKey,
    pub alert_id: String,
    pub tenant_id: crate::ids::TenantId,
    pub session_id: crate::ids::SessionId,
    /// Threshold that was crossed, in USD micros.
    pub threshold_micros: u64,
    /// Session total cost at alert time, in USD micros.
    pub current_micros: u64,
    pub triggered_at_ms: u64,
}

// ── New event structs for extended service coverage ─────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBudgetSet {
    pub tenant_id: crate::ids::TenantId,
    pub budget_id: String,
    pub period: crate::providers::ProviderBudgetPeriod,
    pub limit_micros: u64,
    pub alert_threshold_percent: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelCreated {
    pub channel_id: crate::ids::ChannelId,
    pub project: crate::tenancy::ProjectKey,
    pub name: String,
    pub capacity: u32,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMessageSent {
    pub channel_id: crate::ids::ChannelId,
    pub project: crate::tenancy::ProjectKey,
    pub message_id: String,
    pub sender_id: String,
    pub body: String,
    pub sent_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChannelMessageConsumed {
    pub channel_id: crate::ids::ChannelId,
    pub project: crate::tenancy::ProjectKey,
    pub message_id: String,
    pub consumed_by: String,
    pub consumed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultSettingSet {
    pub scope: crate::tenancy::Scope,
    pub scope_id: String,
    pub key: String,
    pub value: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefaultSettingCleared {
    pub scope: crate::tenancy::Scope,
    pub scope_id: String,
    pub key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LicenseActivated {
    pub tenant_id: crate::ids::TenantId,
    pub license_id: String,
    pub tier: crate::commercial::ProductTier,
    pub valid_from_ms: u64,
    pub valid_until_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntitlementOverrideSet {
    pub tenant_id: crate::ids::TenantId,
    pub feature: String,
    pub allowed: bool,
    pub reason: Option<String>,
    pub set_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationPreferenceSet {
    pub tenant_id: crate::ids::TenantId,
    pub operator_id: String,
    pub event_types: Vec<String>,
    pub channels: Vec<crate::notification_prefs::NotificationChannel>,
    pub set_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotificationSent {
    pub record_id: String,
    pub tenant_id: crate::ids::TenantId,
    pub operator_id: String,
    pub event_type: String,
    pub channel_kind: String,
    pub channel_target: String,
    pub payload: serde_json::Value,
    pub sent_at_ms: u64,
    pub delivered: bool,
    pub delivery_error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPoolCreated {
    pub pool_id: String,
    pub tenant_id: crate::ids::TenantId,
    pub max_connections: u32,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPoolConnectionAdded {
    pub pool_id: String,
    pub tenant_id: crate::ids::TenantId,
    pub connection_id: ProviderConnectionId,
    pub added_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderPoolConnectionRemoved {
    pub pool_id: String,
    pub tenant_id: crate::ids::TenantId,
    pub connection_id: ProviderConnectionId,
    pub removed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantQuotaSet {
    pub tenant_id: crate::ids::TenantId,
    pub max_concurrent_runs: u32,
    pub max_sessions_per_hour: u32,
    pub max_tasks_per_run: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TenantQuotaViolated {
    pub tenant_id: crate::ids::TenantId,
    pub quota_type: String,
    pub current: u32,
    pub limit: u32,
    pub occurred_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionPolicySet {
    pub tenant_id: crate::ids::TenantId,
    pub policy_id: String,
    pub full_history_days: u32,
    pub current_state_days: u32,
    pub max_events_per_entity: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCostAlertSet {
    pub run_id: RunId,
    pub tenant_id: crate::ids::TenantId,
    pub threshold_micros: u64,
    pub set_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCostAlertTriggered {
    pub run_id: RunId,
    pub tenant_id: crate::ids::TenantId,
    pub threshold_micros: u64,
    pub actual_cost_micros: u64,
    pub triggered_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMemberAdded {
    pub workspace_key: crate::tenancy::WorkspaceKey,
    pub member_id: crate::ids::OperatorId,
    pub role: crate::tenancy::WorkspaceRole,
    pub added_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceMemberRemoved {
    pub workspace_key: crate::tenancy::WorkspaceKey,
    pub member_id: crate::ids::OperatorId,
    pub removed_at_ms: u64,
}

// ── Second-wave event structs ────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalDelegated {
    pub approval_id: ApprovalId,
    pub delegated_to: String,
    pub delegated_at_ms: u64,
}

/// Audit log entry event — carries only Eq-able fields; metadata is in the projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLogEntryRecorded {
    pub entry_id: String,
    pub tenant_id: TenantId,
    pub actor_id: String,
    pub action: String,
    pub resource_type: String,
    pub resource_id: String,
    pub outcome: crate::audit::AuditOutcome,
    pub occurred_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointStrategySet {
    pub strategy_id: String,
    pub description: String,
    pub set_at_ms: u64,
    #[serde(default)]
    pub run_id: Option<RunId>,
    #[serde(default)]
    pub interval_ms: u64,
    #[serde(default)]
    pub max_checkpoints: u32,
    #[serde(default)]
    pub trigger_on_task_complete: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialKeyRotated {
    pub tenant_id: TenantId,
    pub rotation_id: String,
    pub old_key_id: String,
    pub new_key_id: String,
    pub credential_ids_rotated: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRevoked {
    pub tenant_id: TenantId,
    pub credential_id: crate::ids::CredentialId,
    pub revoked_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialStored {
    pub tenant_id: TenantId,
    pub credential_id: crate::ids::CredentialId,
    pub provider_id: String,
    pub encrypted_value: Vec<u8>,
    pub key_id: Option<String>,
    pub key_version: Option<String>,
    pub encrypted_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalBaselineLocked {
    pub baseline_id: String,
    pub locked_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalBaselineSet {
    pub baseline_id: String,
    pub metric: String,
    pub value: String,
    pub set_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalDatasetCreated {
    pub dataset_id: String,
    pub name: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalDatasetEntryAdded {
    pub dataset_id: String,
    pub entry_id: String,
    pub added_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalRubricCreated {
    pub rubric_id: String,
    pub name: String,
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventLogCompacted {
    pub up_to_position: u64,
    pub compacted_at_ms: u64,
    #[serde(default)]
    pub tenant_id: TenantId,
    #[serde(default)]
    pub events_before: u64,
    #[serde(default)]
    pub events_after: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailPolicyCreated {
    pub tenant_id: TenantId,
    pub policy_id: String,
    pub name: String,
    pub rules: Vec<crate::policy::GuardrailRule>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailPolicyEvaluated {
    pub tenant_id: TenantId,
    pub policy_id: String,
    pub subject_type: crate::policy::GuardrailSubjectType,
    pub subject_id: Option<String>,
    pub action: String,
    pub decision: crate::policy::GuardrailDecisionKind,
    pub reason: Option<String>,
    pub evaluated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorIntervention {
    pub action: String,
    #[serde(default)]
    pub run_id: Option<RunId>,
    #[serde(default)]
    pub tenant_id: TenantId,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub intervened_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorProfileCreated {
    pub tenant_id: TenantId,
    pub profile_id: crate::ids::OperatorId,
    pub display_name: String,
    pub email: String,
    pub role: crate::tenancy::WorkspaceRole,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorProfileUpdated {
    pub tenant_id: TenantId,
    pub profile_id: crate::ids::OperatorId,
    pub display_name: Option<String>,
    pub email: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseScheduled {
    pub task_id: crate::ids::TaskId,
    pub resume_at_ms: u64,
    #[serde(default)]
    pub run_id: Option<RunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecisionRecorded {
    pub decision_id: String,
    pub principal: String,
    pub action: String,
    pub resource: String,
    pub allowed: bool,
    pub recorded_at_ms: u64,
    #[serde(default)]
    pub invocation_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBindingCreated {
    pub project: crate::tenancy::ProjectKey,
    pub provider_binding_id: crate::ids::ProviderBindingId,
    pub provider_connection_id: crate::ids::ProviderConnectionId,
    pub provider_model_id: crate::ids::ProviderModelId,
    pub operation_kind: crate::providers::OperationKind,
    pub settings: crate::providers::ProviderBindingSettings,
    pub policy_id: Option<String>,
    pub active: bool,
    pub created_at: u64,
    pub estimated_cost_micros: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBindingStateChanged {
    pub project: crate::tenancy::ProjectKey,
    pub provider_binding_id: crate::ids::ProviderBindingId,
    pub active: bool,
    pub changed_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBudgetAlertTriggered {
    pub budget_id: String,
    pub current_micros: u64,
    pub limit_micros: u64,
    pub triggered_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBudgetExceeded {
    pub budget_id: String,
    pub exceeded_by_micros: u64,
    pub exceeded_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConnectionRegistered {
    pub tenant: crate::tenancy::TenantKey,
    pub provider_connection_id: crate::ids::ProviderConnectionId,
    pub provider_family: String,
    pub adapter_type: String,
    /// Model identifiers served through this connection.
    #[serde(default)]
    pub supported_models: Vec<String>,
    pub status: crate::providers::ProviderConnectionStatus,
    pub registered_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthChecked {
    pub tenant_id: TenantId,
    pub connection_id: crate::ids::ProviderConnectionId,
    pub status: crate::providers::ProviderHealthStatus,
    pub latency_ms: Option<u64>,
    pub checked_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthScheduleSet {
    pub schedule_id: String,
    pub connection_id: crate::ids::ProviderConnectionId,
    pub tenant_id: TenantId,
    pub interval_ms: u64,
    pub enabled: bool,
    pub set_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthScheduleTriggered {
    pub schedule_id: String,
    pub connection_id: crate::ids::ProviderConnectionId,
    pub tenant_id: TenantId,
    pub triggered_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderMarkedDegraded {
    pub tenant_id: TenantId,
    pub connection_id: crate::ids::ProviderConnectionId,
    pub reason: String,
    pub marked_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderModelRegistered {
    pub tenant_id: TenantId,
    pub connection_id: ProviderConnectionId,
    pub model_id: String,
    /// Serialized capabilities — stored as JSON string to maintain Eq on RuntimeEvent.
    pub capabilities_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRecovered {
    pub tenant_id: TenantId,
    pub connection_id: crate::ids::ProviderConnectionId,
    pub recovered_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRetryPolicySet {
    pub connection_id: crate::ids::ProviderConnectionId,
    pub tenant_id: crate::ids::TenantId,
    pub policy: crate::providers::RetryPolicy,
    pub set_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryEscalated {
    pub task_id: crate::ids::TaskId,
    pub reason: String,
    pub escalated_at_ms: u64,
    #[serde(default)]
    pub run_id: Option<RunId>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub attempt_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceShareRevoked {
    pub share_id: String,
    pub revoked_at_ms: u64,
    #[serde(default)]
    pub tenant_id: TenantId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceShared {
    pub share_id: String,
    pub resource_type: String,
    #[serde(default)]
    pub grantee: String,
    pub shared_at_ms: u64,
    #[serde(default)]
    pub tenant_id: TenantId,
    #[serde(default)]
    pub source_workspace_id: WorkspaceId,
    #[serde(default)]
    pub target_workspace_id: WorkspaceId,
    #[serde(default)]
    pub resource_id: String,
    #[serde(default)]
    pub permissions: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePolicyCreated {
    pub tenant_id: TenantId,
    pub policy_id: String,
    pub name: String,
    pub rules: Vec<crate::providers::RoutePolicyRule>,
    /// Whether the policy is active at creation time (default: true).
    #[serde(default = "crate::events::default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePolicyUpdated {
    pub policy_id: String,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSlaBreached {
    pub run_id: RunId,
    pub tenant_id: TenantId,
    pub elapsed_ms: u64,
    pub target_ms: u64,
    pub breached_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunSlaSet {
    pub run_id: RunId,
    pub tenant_id: TenantId,
    pub target_completion_ms: u64,
    pub alert_at_percent: u8,
    pub set_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalRouted {
    pub project: crate::tenancy::ProjectKey,
    pub signal_id: crate::ids::SignalId,
    pub subscription_id: String,
    pub delivered_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalSubscriptionCreated {
    pub project: crate::tenancy::ProjectKey,
    pub subscription_id: String,
    pub signal_kind: String,
    pub target_run_id: Option<crate::ids::RunId>,
    pub target_mailbox_id: Option<String>,
    pub filter_expression: Option<String>,
    #[serde(default)]
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSkipReason {
    ConditionMismatch,
    ChainTooDeep,
    AlreadyFired,
    MissingRequiredField { field: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerSuspensionReason {
    RateLimitExceeded,
    BudgetExceeded,
    RepeatedFailures { failure_count: u32 },
    OperatorPaused,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerCreated {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub name: String,
    pub description: Option<String>,
    pub signal_type: String,
    pub plugin_id: Option<String>,
    pub conditions: Vec<serde_json::Value>,
    pub run_template_id: RunTemplateId,
    pub max_per_minute: u32,
    pub max_burst: u32,
    pub max_chain_depth: u8,
    pub created_by: OperatorId,
    pub created_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerEnabled {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub by: OperatorId,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDisabled {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub by: OperatorId,
    pub reason: Option<String>,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSuspended {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub reason: TriggerSuspensionReason,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerResumed {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDeleted {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub by: OperatorId,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerFired {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub signal_id: SignalId,
    pub signal_type: String,
    pub run_id: RunId,
    pub chain_depth: u8,
    pub fired_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSkipped {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub signal_id: SignalId,
    pub reason: TriggerSkipReason,
    pub skipped_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerDenied {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub signal_id: SignalId,
    pub decision_id: DecisionId,
    pub reason: String,
    pub denied_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerRateLimited {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub signal_id: SignalId,
    pub bucket_remaining: u32,
    pub bucket_capacity: u32,
    pub rate_limited_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerPendingApproval {
    pub project: ProjectKey,
    pub trigger_id: TriggerId,
    pub signal_id: SignalId,
    pub approval_id: ApprovalId,
    pub pending_at: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RunTemplateCreated {
    pub project: ProjectKey,
    pub template_id: RunTemplateId,
    pub name: String,
    pub description: Option<String>,
    pub default_mode: crate::decisions::RunMode,
    pub system_prompt: String,
    pub initial_user_message: Option<String>,
    pub plugin_allowlist: Option<Vec<String>>,
    pub tool_allowlist: Option<Vec<String>>,
    pub budget_max_tokens: Option<u64>,
    pub budget_max_wall_clock_ms: Option<u64>,
    pub budget_max_iterations: Option<u32>,
    pub budget_exploration_budget_share: Option<f32>,
    pub sandbox_hint: Option<String>,
    pub required_fields: Vec<String>,
    pub created_by: OperatorId,
    pub created_at: u64,
}

impl Eq for RunTemplateCreated {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunTemplateDeleted {
    pub project: ProjectKey,
    pub template_id: RunTemplateId,
    pub by: OperatorId,
    pub at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotCreated {
    pub snapshot_id: String,
    pub created_at_ms: u64,
    #[serde(default)]
    pub tenant_id: TenantId,
    #[serde(default)]
    pub event_position: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependencyAdded {
    pub task_id: crate::ids::TaskId,
    pub depends_on: crate::ids::TaskId,
    pub added_at_ms: u64,
    /// Alias for task_id (the dependent task).
    #[serde(default)]
    pub dependent_task_id: crate::ids::TaskId,
    /// Alias for depends_on (the prerequisite task).
    #[serde(default)]
    pub depends_on_task_id: crate::ids::TaskId,
    /// Edge kind forwarded to FF. Default `SuccessOnly` so pre-0.2
    /// event-log entries deserialise.
    #[serde(default)]
    pub dependency_kind: crate::task_dependencies::DependencyKind,
    /// Opaque caller-supplied reference stored on the FF edge. `None`
    /// for pre-0.2 event-log entries and for callers that don't supply
    /// one.
    #[serde(default)]
    pub data_passing_ref: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDependencyResolved {
    pub task_id: crate::ids::TaskId,
    pub prerequisite_id: crate::ids::TaskId,
    pub resolved_at_ms: u64,
    #[serde(default)]
    pub dependent_task_id: crate::ids::TaskId,
    #[serde(default)]
    pub depends_on_task_id: crate::ids::TaskId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskLeaseExpired {
    pub task_id: crate::ids::TaskId,
    pub expired_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskPriorityChanged {
    pub task_id: crate::ids::TaskId,
    pub new_priority: u32,
    pub changed_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationProgressUpdated {
    pub invocation_id: ToolInvocationId,
    pub progress_pct: u8,
    pub message: Option<String>,
    pub updated_at_ms: u64,
}

#[cfg(test)]
mod tests {
    use super::{
        ApprovalRequested, EventEnvelope, EventSource, ExternalWorkerReported, RuntimeEvent,
        SessionCreated, TaskCreated, ToolInvocationFailed, ToolInvocationStarted,
        UserMessageAppended,
    };
    use crate::ids::{ApprovalId, CommandId, EventId, RunId, TaskId};
    use crate::policy::ExecutionClass;
    use crate::tenancy::{OwnershipKey, ProjectKey};
    use crate::tool_invocation::ToolInvocationTarget;
    use crate::workers::ExternalWorkerReport;

    #[test]
    fn runtime_event_envelope_carries_project_ownership() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let event = EventEnvelope::for_runtime_event(
            EventId::new("evt_1"),
            EventSource::Runtime,
            RuntimeEvent::SessionCreated(SessionCreated {
                project,
                session_id: "session_1".into(),
            }),
        )
        .with_correlation_id("corr_1");

        assert!(matches!(event.payload, RuntimeEvent::SessionCreated(_)));
        assert!(matches!(event.ownership, OwnershipKey::Project(_)));
    }

    #[test]
    fn event_envelope_builders_set_causation_and_correlation() {
        let event = EventEnvelope::new(
            EventId::new("evt_2"),
            EventSource::Runtime,
            ProjectKey::new("tenant", "workspace", "project"),
            RuntimeEvent::SessionCreated(SessionCreated {
                project: ProjectKey::new("tenant", "workspace", "project"),
                session_id: "session_2".into(),
            }),
        )
        .with_causation_id(CommandId::new("cmd_2"))
        .with_correlation_id("corr_2");

        assert_eq!(
            event.causation_id.as_ref().map(|id| id.as_str()),
            Some("cmd_2")
        );
        assert_eq!(event.correlation_id.as_deref(), Some("corr_2"));
    }

    #[test]
    fn runtime_event_envelope_carries_tool_invocation_payload() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let event = EventEnvelope {
            event_id: EventId::new("evt_tool_1"),
            source: EventSource::Runtime,
            ownership: OwnershipKey::Project(project.clone()),
            causation_id: None,
            correlation_id: Some("corr_tool_1".to_owned()),
            payload: RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                project,
                invocation_id: "inv_1".into(),
                session_id: Some("session_1".into()),
                run_id: Some("run_1".into()),
                task_id: Some("task_1".into()),
                target: ToolInvocationTarget::Plugin {
                    plugin_id: "com.example.git".to_owned(),
                    tool_name: "git.status".to_owned(),
                },
                execution_class: ExecutionClass::SandboxedProcess,
                prompt_release_id: None,
                requested_at_ms: 10,
                started_at_ms: 11,
            }),
        };

        assert!(matches!(
            event.payload,
            RuntimeEvent::ToolInvocationStarted(_)
        ));
    }

    #[test]
    fn runtime_event_envelope_carries_external_worker_payload() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let event = EventEnvelope {
            event_id: EventId::new("evt_worker_1"),
            source: EventSource::ExternalWorker {
                worker: "worker_1".to_owned(),
            },
            ownership: OwnershipKey::Project(project.clone()),
            causation_id: None,
            correlation_id: Some("corr_worker_1".to_owned()),
            payload: RuntimeEvent::ExternalWorkerReported(ExternalWorkerReported {
                report: ExternalWorkerReport {
                    project,
                    worker_id: "worker_1".into(),
                    run_id: Some("run_1".into()),
                    task_id: "task_1".into(),
                    lease_token: 3,
                    reported_at_ms: 99,
                    progress: None,
                    outcome: None,
                },
            }),
        };

        assert!(matches!(
            event.payload,
            RuntimeEvent::ExternalWorkerReported(_)
        ));
    }

    #[test]
    fn runtime_event_reports_project_and_primary_entity() {
        let event = RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
            project: ProjectKey::new("tenant", "workspace", "project"),
            invocation_id: "inv_8".into(),
            task_id: Some("task_8".into()),
            tool_name: "fs.write".to_owned(),
            finished_at_ms: 14,
            outcome: crate::tool_invocation::ToolInvocationOutcomeKind::PermanentFailure,
            error_message: Some("bad input".to_owned()),
        });

        assert_eq!(event.project().project_id.as_str(), "project");
        assert!(matches!(
            event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::ToolInvocation { .. })
        ));
    }

    #[test]
    fn task_and_approval_events_already_carry_identity_for_enrichment() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let task_event = RuntimeEvent::TaskCreated(TaskCreated {
            project: project.clone(),
            task_id: TaskId::new("task_9"),
            parent_run_id: None,
            parent_task_id: None,
            prompt_release_id: None,
            session_id: None,
        });
        let approval_event = RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: project.clone(),
            approval_id: ApprovalId::new("approval_9"),
            run_id: None,
            task_id: Some(TaskId::new("task_9")),
            requirement: crate::policy::ApprovalRequirement::Required,
            title: None,
            description: None,
        });

        assert_eq!(task_event.project(), &project);
        assert_eq!(approval_event.project(), &project);
        assert!(matches!(
            task_event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Task { .. })
        ));
        assert!(matches!(
            approval_event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Approval { .. })
        ));
        assert_eq!(
            EventEnvelope::for_runtime_event(
                EventId::new("evt_task_9"),
                EventSource::Runtime,
                task_event
            )
            .project(),
            &project
        );
        assert_eq!(
            EventEnvelope::for_runtime_event(
                EventId::new("evt_approval_9"),
                EventSource::Runtime,
                approval_event
            )
            .project(),
            &project
        );
    }

    #[test]
    fn event_envelope_reports_project_and_primary_entity() {
        let event = EventEnvelope::for_runtime_event(
            EventId::new("evt_3"),
            EventSource::Runtime,
            RuntimeEvent::SessionCreated(SessionCreated {
                project: ProjectKey::new("tenant", "workspace", "project"),
                session_id: "session_3".into(),
            }),
        );

        assert_eq!(event.project().project_id.as_str(), "project");
        assert!(matches!(
            event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Session { .. })
        ));
    }

    #[test]
    fn user_message_appended_reports_project_and_run_entity() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let event = RuntimeEvent::UserMessageAppended(UserMessageAppended {
            project: project.clone(),
            session_id: "session_10".into(),
            run_id: RunId::new("run_10"),
            content: String::new(),
            sequence: 0,
            appended_at_ms: 0,
        });

        assert_eq!(event.project(), &project);
        assert!(matches!(
            event.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Run { ref run_id }) if run_id.as_str() == "run_10"
        ));
    }
}
