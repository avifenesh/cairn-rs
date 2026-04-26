use crate::approvals::{ApprovalMatchPolicy, ApprovalScope};
use crate::errors::RuntimeEntityRef;
use crate::ids::{
    ApprovalId, CheckpointId, DecisionId, EvalRunId, EventId, IngestJobId, MailboxMessageId,
    OperatorId, OutcomeId, PromptAssetId, PromptReleaseId, PromptVersionId, ProviderBindingId,
    ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId, RouteDecisionId, RunId,
    RunTemplateId, ScheduledTaskId, SessionId, SignalId, TaskId, TenantId, ToolCallId,
    ToolInvocationId, TriggerId, WorkspaceId,
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
    /// PR BP-1: tool-call approval proposed (foundation — not yet
    /// emitted; legacy `ApprovalRequested` is still the live tool-call
    /// approval event).
    ToolCallProposed(ToolCallProposed),
    /// PR BP-1: tool-call approval granted (foundation — not yet emitted).
    ToolCallApproved(ToolCallApproved),
    /// PR BP-1: tool-call approval denied (foundation — not yet emitted).
    ToolCallRejected(ToolCallRejected),
    /// PR BP-1: operator amended proposed tool-call arguments before
    /// resolving (foundation — not yet emitted).
    ToolCallAmended(ToolCallAmended),
    CheckpointRecorded(CheckpointRecorded),
    CheckpointRestored(CheckpointRestored),
    MailboxMessageAppended(MailboxMessageAppended),
    ToolInvocationStarted(ToolInvocationStarted),
    ToolInvocationCompleted(ToolInvocationCompleted),
    ToolInvocationFailed(ToolInvocationFailed),
    /// RFC 020 Track 3: tool call served from `ToolCallResultCache` on replay.
    ToolInvocationCacheHit(ToolInvocationCacheHit),
    /// RFC 020 Track 3: recovery paused on a `DangerousPause` tool with no cached result.
    ToolRecoveryPaused(ToolRecoveryPaused),
    SignalIngested(SignalIngested),
    ExternalWorkerRegistered(ExternalWorkerRegistered),
    ExternalWorkerReported(ExternalWorkerReported),
    ExternalWorkerSuspended(ExternalWorkerSuspended),
    ExternalWorkerReactivated(ExternalWorkerReactivated),
    SubagentSpawned(SubagentSpawned),
    RecoveryAttempted(RecoveryAttempted),
    RecoveryCompleted(RecoveryCompleted),
    /// RFC 020 Track 4: once-per-boot aggregate summary of the recovery
    /// sweep. Emitted at the end of `RecoveryService::recover_all` with
    /// per-branch counts (runs, sandboxes, cache entries, …).
    RecoverySummaryEmitted(RecoverySummaryEmitted),
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
    WorkspaceArchived(WorkspaceArchived),
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
    /// Operator removed a provider connection. The projection hard-removes
    /// the row so the ID can be re-used; full history remains in the event
    /// log. F40.
    ProviderConnectionDeleted(ProviderConnectionDeleted),
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
    // ── Decision layer persistence (RFC 019 + RFC 020) ────────────────────
    /// A decision was evaluated and (optionally) cached. Durable audit
    /// record that lets the decision cache projection be rebuilt on
    /// startup — closes the RFC 020 §"Decision Cache Survival" gap.
    DecisionRecorded(DecisionRecorded),
    /// Startup replay completed for the decision cache. Counts how many
    /// entries were restored and how many were dropped because their TTL
    /// had already expired at replay time.
    DecisionCacheWarmup(DecisionCacheWarmup),
    /// F47 PR2: run completion annotated with the LLM's free-text summary
    /// and the extractor-produced `CompletionVerification` sidecar.
    ///
    /// Emitted AFTER the orchestrator loop returns `LoopTermination::Completed`
    /// and after `runs.complete` has flipped the run to the terminal state.
    /// This event does not drive state transitions — `RunStateChanged` is
    /// still the authority. It purely annotates the already-terminal run
    /// with the truth-vs-claim gap so operators can inspect the evidence
    /// on a run detail page after the SSE stream is gone.
    ///
    /// Replay-safe: the projection stores summary + verification on
    /// nullable columns, so event logs written before F47 PR2 deserialize
    /// cleanly and surface `completion: None` at the REST boundary.
    RunCompletionAnnotated(RunCompletionAnnotated),
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
            RuntimeEvent::ToolCallProposed(event) => &event.project,
            RuntimeEvent::ToolCallApproved(event) => &event.project,
            RuntimeEvent::ToolCallRejected(event) => &event.project,
            RuntimeEvent::ToolCallAmended(event) => &event.project,
            RuntimeEvent::CheckpointRecorded(event) => &event.project,
            RuntimeEvent::CheckpointRestored(event) => &event.project,
            RuntimeEvent::MailboxMessageAppended(event) => &event.project,
            RuntimeEvent::ToolInvocationStarted(event) => &event.project,
            RuntimeEvent::ToolInvocationCompleted(event) => &event.project,
            RuntimeEvent::ToolInvocationFailed(event) => &event.project,
            RuntimeEvent::ToolInvocationCacheHit(event) => &event.project,
            RuntimeEvent::ToolRecoveryPaused(event) => &event.project,
            RuntimeEvent::SignalIngested(event) => &event.project,
            RuntimeEvent::ExternalWorkerRegistered(event) => &event.sentinel_project,
            RuntimeEvent::ExternalWorkerReported(event) => &event.report.project,
            RuntimeEvent::ExternalWorkerSuspended(event) => &event.sentinel_project,
            RuntimeEvent::ExternalWorkerReactivated(event) => &event.sentinel_project,
            RuntimeEvent::SubagentSpawned(event) => &event.project,
            RuntimeEvent::RecoveryAttempted(event) => &event.project,
            RuntimeEvent::RecoveryCompleted(event) => &event.project,
            RuntimeEvent::RecoverySummaryEmitted(event) => &event.sentinel_project,
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
            RuntimeEvent::WorkspaceArchived(event) => &event.project,
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
            RuntimeEvent::DecisionRecorded(event) => &event.project,
            RuntimeEvent::RunCompletionAnnotated(event) => &event.project,
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
            | RuntimeEvent::ProviderConnectionDeleted(_)
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
            | RuntimeEvent::ScheduledTaskCreated(_)
            | RuntimeEvent::DecisionCacheWarmup(_) => {
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
            // PR BP-1: ToolCallId is not (yet) a `RuntimeEntityRef`
            // variant — returning `None` here keeps the foundation
            // additive. A follow-up PR that introduces projection state
            // will extend `RuntimeEntityRef` and update these arms.
            RuntimeEvent::ToolCallProposed(_) => None,
            RuntimeEvent::ToolCallApproved(_) => None,
            RuntimeEvent::ToolCallRejected(_) => None,
            RuntimeEvent::ToolCallAmended(_) => None,
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
            RuntimeEvent::ToolInvocationCacheHit(event) => Some(RuntimeEntityRef::ToolInvocation {
                invocation_id: event.invocation_id.clone(),
            }),
            RuntimeEvent::ToolRecoveryPaused(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
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
            RuntimeEvent::WorkspaceArchived(_) => None,
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
            | RuntimeEvent::ProviderConnectionDeleted(_)
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
            | RuntimeEvent::RecoverySummaryEmitted(_)
            | RuntimeEvent::ScheduledTaskCreated(_)
            | RuntimeEvent::DecisionRecorded(_)
            | RuntimeEvent::DecisionCacheWarmup(_) => None,
            RuntimeEvent::RunCompletionAnnotated(event) => Some(RuntimeEntityRef::Run {
                run_id: event.run_id.clone(),
            }),
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

// ── Tool-call approval events (PR BP-1 foundation) ────────────────────────────
//
// These four events form the type-level foundation for tool-call approval
// workflows. They are strictly additive — the legacy
// `ApprovalRequested` / `ApprovalResolved` pair above remains in use for
// plan review (RFC 018), RFC 022 trigger approvals, prompt-release
// governance, and the current tool-call approval emission in
// `execute_impl.rs`. A later PR in the wave migrates the tool-call
// emission site from `ApprovalRequested` to [`ToolCallProposed`].
//
// All four events carry `project: ProjectKey` so the event log can own
// them; `primary_entity_ref()` returns `None` because `ToolCallId` is not
// (yet) a member of `RuntimeEntityRef` — adding it is deferred to a
// follow-up PR that introduces projection state.

/// The orchestrator has proposed a tool call that requires operator
/// approval before execution.
///
/// The operator surface consumes `display_summary` to render a
/// human-friendly prompt and uses `match_policy` to seed the default
/// "remember this decision for the session" UX.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallProposed {
    pub project: ProjectKey,
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    /// Short human-readable summary of what the tool call would do
    /// (e.g. `"Read /workspaces/cairn/Cargo.toml"`). Rendered in the
    /// approval UI; may be empty if the caller has nothing useful to
    /// offer.
    pub display_summary: String,
    /// How the operator's decision should match future calls if they
    /// pick `ApprovalScope::Session`.
    pub match_policy: ApprovalMatchPolicy,
    pub proposed_at_ms: u64,
}

/// An operator approved a proposed tool call.
///
/// If the operator edited the arguments before approving (the "approve
/// with amendment" flow), `approved_tool_args` holds the edited payload
/// and the execute phase uses those instead of the original
/// `ToolCallProposed.tool_args`.
///
/// # Source-of-truth invariant for the executed arguments
///
/// On replay, the final arguments the execute phase runs are
/// deterministically the **last** of the following to appear for a
/// given `call_id`, in event-log order:
///
/// 1. `ToolCallApproved.approved_tool_args` if `Some`.
/// 2. `ToolCallAmended.new_tool_args` if any amendments were emitted.
/// 3. `ToolCallProposed.tool_args` otherwise.
///
/// Concretely: `ToolCallAmended` records *preview* edits an operator
/// made before resolving. A subsequent `ToolCallApproved` either
/// repeats the amended args in `approved_tool_args: Some(...)` (the
/// normal path, so projections can ignore earlier `ToolCallAmended`
/// events) or carries `approved_tool_args: None`, which means "approve
/// whatever the most recent `ToolCallAmended` settled on, or the
/// original `ToolCallProposed.tool_args` if none was emitted".
///
/// This invariant keeps projection state reconstructable from the
/// event log alone without cross-referencing in-memory UI state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallApproved {
    pub project: ProjectKey,
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub operator_id: OperatorId,
    pub scope: ApprovalScope,
    /// Operator-edited arguments attached to the approval itself.
    /// `Some` overrides any earlier `ToolCallAmended` payload and the
    /// original `ToolCallProposed.tool_args`. `None` means "approve
    /// whatever arguments the most recent preceding `ToolCallAmended`
    /// or `ToolCallProposed` carried" (see the struct-level invariant
    /// for the full precedence order).
    pub approved_tool_args: Option<serde_json::Value>,
    pub approved_at_ms: u64,
}

/// An operator rejected a proposed tool call.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallRejected {
    pub project: ProjectKey,
    pub call_id: ToolCallId,
    pub session_id: SessionId,
    pub operator_id: OperatorId,
    /// Optional operator-supplied reason; surfaced in audit log and to
    /// the agent as a rejection message.
    pub reason: Option<String>,
    pub rejected_at_ms: u64,
}

/// An operator amended a proposed tool call's arguments without yet
/// resolving it. Enables the "edit before approval" flow where an
/// operator tweaks arguments, reviews the updated display, and then
/// emits a separate [`ToolCallApproved`] / [`ToolCallRejected`].
///
/// This event is intentionally separate from `ToolCallApproved` so that
/// the audit log preserves the full chain of edits an operator made
/// before committing to a final decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallAmended {
    pub project: ProjectKey,
    pub call_id: ToolCallId,
    /// Session the amended call belongs to. Included (like the other
    /// three tool-call approval events) so downstream projections can
    /// index by session without walking prior events to recover the
    /// association.
    pub session_id: SessionId,
    pub operator_id: OperatorId,
    pub new_tool_args: serde_json::Value,
    pub amended_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecorded {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub checkpoint_id: CheckpointId,
    pub disposition: CheckpointDisposition,
    pub data: Option<serde_json::Value>,
    /// RFC 020 Track 4: dual checkpoint — `Intent` captures the decide
    /// output + planned tool-call IDs before execute; `Result` captures the
    /// post-execute message history after the iteration settles. `None` for
    /// legacy (pre-Track-4) checkpoints.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<crate::recovery::CheckpointKind>,
    /// RFC 020 Track 4: size (bytes) of the serialized message history at
    /// the moment this checkpoint was recorded. Populated for observability
    /// so operators can monitor checkpoint body cost and decide when (if
    /// ever) to add diff-based compaction (Gap 3 — deferred to Track 4b).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_history_size: Option<u32>,
    /// RFC 020 Track 4: deterministic `ToolCallId`s planned at this
    /// checkpoint. Populated on `Intent`; empty on `Result` (the Intent
    /// checkpoint already carries the full planned list; duplicating on
    /// the Result checkpoint would only inflate the event body).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_call_ids: Vec<String>,
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
    /// RFC 020 Track 3: deterministic tool-call ID for idempotent recovery.
    /// `None` when the orchestrator has not minted one (legacy event-log
    /// entries and non-orchestrator callers like `handlers/tools.rs`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// RFC 020 Track 3: cached tool-result payload. Enables the startup
    /// replay to rebuild `ToolCallResultCache` from the event log so a
    /// resumed run on a fresh process still serves cache hits.
    /// `None` when the tool returned no useful result or the legacy
    /// record path was used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_json: Option<serde_json::Value>,
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

/// RFC 020 Track 3: a tool invocation was served from the result cache
/// instead of being re-dispatched. Emitted when a resumed run recomputes
/// the same `ToolCallId` (deterministic by run_id + step + call_index +
/// tool_name + normalized_args) and finds a prior completion in the cache.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInvocationCacheHit {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    /// Deterministic tool-call identifier (stringified `ToolCallId`).
    pub tool_call_id: String,
    /// When the cached result was first produced (ms since epoch).
    pub original_completed_at_ms: u64,
    /// When the cache hit was served on this boot (ms since epoch).
    pub served_at_ms: u64,
}

/// RFC 020 Track 3: recovery of a tool call that cannot be safely re-dispatched
/// (classified as `DangerousPause`) — the run transitions to `WaitingApproval`
/// and the operator must confirm before proceeding.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRecoveryPaused {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub task_id: Option<TaskId>,
    pub tool_name: String,
    /// Deterministic tool-call identifier (stringified `ToolCallId`).
    pub tool_call_id: String,
    /// Human-readable reason — e.g. "DangerousPause tool with no cached result on recovery".
    pub reason: String,
    pub paused_at_ms: u64,
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
    /// RFC 020 Track 1: the cairn-app boot that originated this recovery sweep.
    /// `None` for legacy FF-authored recovery events (task lease expiry etc.)
    /// so existing callers keep deserialising cleanly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<String>,
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
    /// RFC 020 Track 1: the cairn-app boot that originated this recovery sweep.
    /// `None` for legacy FF-authored recovery events so pre-RFC-020 events
    /// still round-trip through serde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<String>,
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

/// RFC 020 Track 4 — once-per-boot recovery audit summary.
///
/// Emitted exactly once at the end of `RecoveryService::recover_all` with
/// per-branch counts. The `boot_id` correlates this summary with the
/// `RecoveryAttempted`/`RecoveryCompleted` pairs emitted during the same
/// sweep, giving operators a single wire event to observe startup recovery
/// cost without re-aggregating the stream.
///
/// Branch counts that Track 4 cannot populate directly (sandbox, graph,
/// memory, trigger, webhook dedup — each owned by a sibling recovery
/// service) default to 0 and will be filled in by their respective tracks.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverySummaryEmitted {
    /// Tenant-scoped sentinel project (no per-project recovery summary in v1).
    pub sentinel_project: ProjectKey,
    /// Unique identifier for this cairn-app boot.
    pub boot_id: String,
    pub recovered_runs: u32,
    pub recovered_tasks: u32,
    pub recovered_sandboxes: u32,
    pub preserved_sandboxes: u32,
    pub orphaned_sandboxes_cleaned: u32,
    pub decision_cache_entries: u32,
    pub stale_pending_cleared: u32,
    pub tool_result_cache_entries: u32,
    pub memory_projection_entries: u32,
    pub graph_nodes_recovered: u32,
    pub graph_edges_recovered: u32,
    pub webhook_dedup_entries: u32,
    pub trigger_projections: u32,
    /// Wall-clock ms from process start to recovery completion.
    pub startup_ms: u64,
    /// Unix-ms timestamp when the summary was emitted.
    pub summary_at_ms: u64,
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
    /// Dataset binding captured at run-create time.
    ///
    /// Persisted on `EvalRunStarted` so `replay_evals` can restore the
    /// dataset linkage after a restart — the in-memory eval service
    /// previously lost this binding because only the run-create path
    /// wrote to it. Defaulted to `None` for backward compatibility with
    /// pre-#220 event log entries.
    #[serde(default)]
    pub dataset_id: Option<String>,
    /// Rubric id attached at run-create time (issue #223). `#[serde(default)]`
    /// for backward-compat with pre-#223 event-log entries.
    #[serde(default)]
    pub rubric_id: Option<String>,
    /// Baseline id attached at run-create time (issue #223).
    #[serde(default)]
    pub baseline_id: Option<String>,
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

/// Emitted when a workspace is soft-deleted (archived). The workspace record
/// is preserved for audit/history; list endpoints filter it out by default.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceArchived {
    pub project: ProjectKey,
    pub workspace_id: WorkspaceId,
    pub tenant_id: TenantId,
    pub archived_at: u64,
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

/// Projection directive emitted when the operator deletes a provider
/// connection. The in-memory projection hard-removes the row so the
/// `provider_connection_id` is free to be re-used immediately. Prior
/// `ProviderConnectionRegistered` events for the same ID are preserved
/// in the event log for audit. F40.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConnectionDeleted {
    pub tenant: crate::tenancy::TenantKey,
    pub provider_connection_id: crate::ids::ProviderConnectionId,
    pub deleted_at: u64,
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

/// Durable record of a single decision evaluation.
///
/// Persisting this event lets cairn-app rebuild the RFC 019 decision cache
/// on startup (RFC 020 §"Decision Cache Survival"). The payload carries
/// only the fields needed to reconstruct a cache entry plus an opaque
/// `event_json` blob with the full `cairn_domain::decisions::DecisionEvent`
/// so `GET /v1/decisions/{id}` can serve the reasoning chain post-replay.
///
/// `event_json` is stored as a JSON string rather than a typed structure
/// so the event log stays portable (SQLite TEXT / Postgres TEXT, no JSONB
/// operators) and to sidestep the fact that the richer `DecisionEvent`
/// enum does not implement `Eq` (its `CostEstimate` carries `f64`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecorded {
    pub project: ProjectKey,
    pub decision_id: DecisionId,
    pub decision_key: crate::decisions::DecisionKey,
    pub outcome: crate::decisions::DecisionOutcome,
    /// `true` when the decision was written to the cache at step 7. Replay
    /// skips events where `cached == false` (never-cache policies,
    /// `cache_write: skip`).
    pub cached: bool,
    /// Cache TTL expiry in epoch-ms. `0` when `cached == false`.
    pub expires_at: u64,
    pub decided_at: u64,
    /// Full `DecisionEvent::DecisionRecorded` serialized via `serde_json`.
    /// Used by `get_decision` after replay so the reasoning chain is
    /// preserved across restarts.
    pub event_json: String,
}

/// Emitted once at the end of startup replay for the decision cache.
///
/// Counts how many cached decisions survived the restart and how many
/// were dropped because their TTL had already elapsed. RFC 020
/// §"Decision Cache Survival" requires this as audit-trail evidence
/// that the cache rebuild ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionCacheWarmup {
    pub cached: u32,
    pub expired_and_dropped: u32,
    pub warmed_at: u64,
}

/// F47 PR2: annotates a completed run with the LLM's free-text summary
/// and the extractor-produced `CompletionVerification` sidecar.
///
/// Emitted after `runs.complete` has flipped the run to the terminal
/// state and after the orchestrator loop has returned
/// `LoopTermination::Completed`. Does not drive state transitions —
/// projections use it to populate nullable `completion_summary` /
/// `completion_verification_json` columns so the evidence survives
/// past the SSE `orchestrate_finished` frame.
///
/// Per-field `#[serde(default)]` keeps event logs written before this
/// variant existed deserialising cleanly: a legacy log simply has no
/// `RunCompletionAnnotated` entries, so every projected run shows
/// `completion: None` at the REST boundary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletionAnnotated {
    pub project: crate::tenancy::ProjectKey,
    pub session_id: crate::ids::SessionId,
    pub run_id: crate::ids::RunId,
    pub summary: String,
    #[serde(default)]
    pub verification: crate::orchestrator::CompletionVerification,
    pub occurred_at_ms: u64,
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

    // ── PR BP-1: tool-call approval foundation events ─────────────────────
    //
    // Strictly additive: these tests pin the JSON wire shape for the 4 new
    // variants + assert `project()` returns the carried `ProjectKey` and
    // `primary_entity_ref()` returns `None` (ToolCallId is not yet a
    // `RuntimeEntityRef` variant; that is PR BP-2+ scope).

    #[test]
    fn tool_call_proposed_roundtrips_and_reports_project() {
        let project = ProjectKey::new("t", "w", "p");
        let event = RuntimeEvent::ToolCallProposed(super::ToolCallProposed {
            project: project.clone(),
            call_id: crate::ids::ToolCallId::new("tc_1"),
            session_id: "sess_1".into(),
            run_id: RunId::new("run_1"),
            tool_name: "read_file".to_owned(),
            tool_args: serde_json::json!({"path": "/tmp/x"}),
            display_summary: "Read /tmp/x".to_owned(),
            match_policy: crate::approvals::ApprovalMatchPolicy::Exact,
            proposed_at_ms: 42,
        });
        assert_eq!(event.project(), &project);
        assert!(event.primary_entity_ref().is_none());
        let json = serde_json::to_string(&event).expect("serialize");
        let back: RuntimeEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, back);
    }

    #[test]
    fn tool_call_approved_roundtrips_and_reports_project() {
        let project = ProjectKey::new("t", "w", "p");
        let event = RuntimeEvent::ToolCallApproved(super::ToolCallApproved {
            project: project.clone(),
            call_id: crate::ids::ToolCallId::new("tc_2"),
            session_id: "sess_2".into(),
            operator_id: crate::ids::OperatorId::new("op_1"),
            scope: crate::approvals::ApprovalScope::Session {
                match_policy: crate::approvals::ApprovalMatchPolicy::ProjectScopedPath {
                    project_root: "/w/p".to_owned(),
                },
            },
            approved_tool_args: Some(serde_json::json!({"path": "/w/p/file.rs"})),
            approved_at_ms: 1_000,
        });
        assert_eq!(event.project(), &project);
        assert!(event.primary_entity_ref().is_none());
        let json = serde_json::to_string(&event).expect("serialize");
        let back: RuntimeEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, back);
    }

    #[test]
    fn tool_call_rejected_roundtrips_and_reports_project() {
        let project = ProjectKey::new("t", "w", "p");
        let event = RuntimeEvent::ToolCallRejected(super::ToolCallRejected {
            project: project.clone(),
            call_id: crate::ids::ToolCallId::new("tc_3"),
            session_id: "sess_3".into(),
            operator_id: crate::ids::OperatorId::new("op_1"),
            reason: Some("unsafe".to_owned()),
            rejected_at_ms: 2_000,
        });
        assert_eq!(event.project(), &project);
        assert!(event.primary_entity_ref().is_none());
        let json = serde_json::to_string(&event).expect("serialize");
        let back: RuntimeEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, back);
    }

    #[test]
    fn tool_call_amended_roundtrips_and_reports_project() {
        let project = ProjectKey::new("t", "w", "p");
        let event = RuntimeEvent::ToolCallAmended(super::ToolCallAmended {
            project: project.clone(),
            call_id: crate::ids::ToolCallId::new("tc_4"),
            session_id: "sess_4".into(),
            operator_id: crate::ids::OperatorId::new("op_1"),
            new_tool_args: serde_json::json!({"path": "/tmp/y"}),
            amended_at_ms: 3_000,
        });
        assert_eq!(event.project(), &project);
        assert!(event.primary_entity_ref().is_none());
        let json = serde_json::to_string(&event).expect("serialize");
        let back: RuntimeEvent = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(event, back);
    }

    #[test]
    fn tool_call_events_use_snake_case_discriminator() {
        let event = RuntimeEvent::ToolCallProposed(super::ToolCallProposed {
            project: ProjectKey::new("t", "w", "p"),
            call_id: crate::ids::ToolCallId::new("tc_x"),
            session_id: "s".into(),
            run_id: RunId::new("r"),
            tool_name: "read_file".to_owned(),
            tool_args: serde_json::json!({}),
            display_summary: String::new(),
            match_policy: crate::approvals::ApprovalMatchPolicy::Exact,
            proposed_at_ms: 0,
        });
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains("\"event\":\"tool_call_proposed\""),
            "expected snake_case event discriminator, got {json}"
        );
    }
}
