use crate::errors::RuntimeEntityRef;
use crate::ids::{
    ApprovalId, CheckpointId, CommandId, EvalRunId, IngestJobId, MailboxMessageId, OutcomeId,
    PromptAssetId, PromptReleaseId, PromptVersionId, RunId, ScheduledTaskId, SessionId, SignalId,
    TaskId, TenantId, ToolInvocationId, WorkspaceId,
};
use crate::lifecycle::{
    FailureClass, PauseReason, ResumeTrigger, RunResumeTarget, TaskResumeTarget,
};
use crate::policy::{ApprovalDecision, ExecutionClass};
use crate::tenancy::{OwnershipKey, ProjectKey};
use crate::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
use crate::workers::ExternalWorkerReport;
use serde::{Deserialize, Serialize};

/// Shared command envelope across API, runtime, and external worker boundaries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandEnvelope<T> {
    pub command_id: CommandId,
    pub issued_by: CommandActor,
    pub ownership: OwnershipKey,
    pub correlation_id: Option<String>,
    pub payload: T,
}

impl<T> CommandEnvelope<T> {
    pub fn new(
        command_id: impl Into<CommandId>,
        issued_by: CommandActor,
        ownership: impl Into<OwnershipKey>,
        payload: T,
    ) -> Self {
        Self {
            command_id: command_id.into(),
            issued_by,
            ownership: ownership.into(),
            correlation_id: None,
            payload,
        }
    }

    pub fn with_correlation_id(mut self, correlation_id: impl Into<String>) -> Self {
        self.correlation_id = Some(correlation_id.into());
        self
    }
}

impl CommandEnvelope<RuntimeCommand> {
    pub fn for_runtime_command(
        command_id: impl Into<CommandId>,
        issued_by: CommandActor,
        payload: RuntimeCommand,
    ) -> Self {
        let ownership = payload.project().clone();
        Self::new(command_id, issued_by, ownership, payload)
    }

    pub fn project(&self) -> &ProjectKey {
        self.payload.project()
    }

    pub fn primary_entity_ref(&self) -> Option<RuntimeEntityRef> {
        self.payload.primary_entity_ref()
    }
}

/// Who asked the system to perform the command.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "actor_type", rename_all = "snake_case")]
pub enum CommandActor {
    Operator { operator_id: crate::ids::OperatorId },
    Runtime,
    Scheduler,
    ExternalWorker { worker: String },
    System,
}

/// Minimal runtime command set used as the Week 1 shared contract.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum RuntimeCommand {
    CreateSession(CreateSession),
    StartRun(StartRun),
    SubmitTask(SubmitTask),
    ClaimTask(ClaimTask),
    HeartbeatTaskLease(HeartbeatTaskLease),
    PauseRun(PauseRun),
    PauseTask(PauseTask),
    ResumeRun(ResumeRun),
    ResumeTask(ResumeTask),
    RequestApproval(RequestApproval),
    RecordApprovalDecision(RecordApprovalDecision),
    RecordCheckpoint(RecordCheckpoint),
    RestoreCheckpoint(RestoreCheckpoint),
    AppendMailboxMessage(AppendMailboxMessage),
    StartToolInvocation(StartToolInvocation),
    FinishToolInvocation(FinishToolInvocation),
    IngestSignal(IngestSignal),
    ReportExternalWorker(ReportExternalWorker),
    SpawnSubagent(SpawnSubagent),
    RecordRecoverySweep(RecordRecoverySweep),
    CompleteRun(CompleteRun),
    FailRun(FailRun),
    CancelRun(CancelRun),
    CompleteTask(CompleteTask),
    FailTask(FailTask),
    CancelTask(CancelTask),
    AppendUserMessage(AppendUserMessage),
    StartIngestJob(StartIngestJob),
    CompleteIngestJob(CompleteIngestJob),
    StartEvalRun(StartEvalRun),
    CompleteEvalRun(CompleteEvalRun),
    RecordOutcome(RecordOutcome),
    CreatePromptAsset(CreatePromptAsset),
    CreatePromptVersion(CreatePromptVersion),
    CreatePromptRelease(CreatePromptRelease),
    TransitionPromptRelease(TransitionPromptRelease),
    CreateTenant(CreateTenant),
    CreateWorkspace(CreateWorkspace),
    CreateProject(CreateProject),
    CreateScheduledTask(CreateScheduledTask),
}

impl RuntimeCommand {
    pub fn project(&self) -> &ProjectKey {
        match self {
            RuntimeCommand::CreateSession(command) => &command.project,
            RuntimeCommand::StartRun(command) => &command.project,
            RuntimeCommand::SubmitTask(command) => &command.project,
            RuntimeCommand::ClaimTask(command) => &command.project,
            RuntimeCommand::HeartbeatTaskLease(command) => &command.project,
            RuntimeCommand::PauseRun(command) => &command.project,
            RuntimeCommand::PauseTask(command) => &command.project,
            RuntimeCommand::ResumeRun(command) => &command.project,
            RuntimeCommand::ResumeTask(command) => &command.project,
            RuntimeCommand::RequestApproval(command) => &command.project,
            RuntimeCommand::RecordApprovalDecision(command) => &command.project,
            RuntimeCommand::RecordCheckpoint(command) => &command.project,
            RuntimeCommand::RestoreCheckpoint(command) => &command.project,
            RuntimeCommand::AppendMailboxMessage(command) => &command.project,
            RuntimeCommand::StartToolInvocation(command) => &command.project,
            RuntimeCommand::FinishToolInvocation(command) => &command.project,
            RuntimeCommand::IngestSignal(command) => &command.project,
            RuntimeCommand::ReportExternalWorker(command) => &command.report.project,
            RuntimeCommand::SpawnSubagent(command) => &command.project,
            RuntimeCommand::RecordRecoverySweep(command) => &command.project,
            RuntimeCommand::CompleteRun(command) => &command.project,
            RuntimeCommand::FailRun(command) => &command.project,
            RuntimeCommand::CancelRun(command) => &command.project,
            RuntimeCommand::CompleteTask(command) => &command.project,
            RuntimeCommand::FailTask(command) => &command.project,
            RuntimeCommand::CancelTask(command) => &command.project,
            RuntimeCommand::AppendUserMessage(command) => &command.project,
            RuntimeCommand::StartIngestJob(command) => &command.project,
            RuntimeCommand::CompleteIngestJob(command) => &command.project,
            RuntimeCommand::StartEvalRun(command) => &command.project,
            RuntimeCommand::CompleteEvalRun(command) => &command.project,
            RuntimeCommand::RecordOutcome(command) => &command.project,
            RuntimeCommand::CreatePromptAsset(command) => &command.project,
            RuntimeCommand::CreatePromptVersion(command) => &command.project,
            RuntimeCommand::CreatePromptRelease(command) => &command.project,
            RuntimeCommand::TransitionPromptRelease(command) => &command.project,
            RuntimeCommand::CreateTenant(command) => &command.project,
            RuntimeCommand::CreateWorkspace(command) => &command.project,
            RuntimeCommand::CreateProject(command) => &command.project,
            RuntimeCommand::CreateScheduledTask(command) => &command.project,
        }
    }

    pub fn primary_entity_ref(&self) -> Option<RuntimeEntityRef> {
        match self {
            RuntimeCommand::CreateSession(command) => Some(RuntimeEntityRef::Session {
                session_id: command.session_id.clone(),
            }),
            RuntimeCommand::StartRun(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::SubmitTask(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::ClaimTask(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::HeartbeatTaskLease(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::PauseTask(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::ResumeTask(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::PauseRun(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::ResumeRun(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::RequestApproval(command) => Some(RuntimeEntityRef::Approval {
                approval_id: command.approval_id.clone(),
            }),
            RuntimeCommand::RecordApprovalDecision(command) => Some(RuntimeEntityRef::Approval {
                approval_id: command.approval_id.clone(),
            }),
            RuntimeCommand::RecordCheckpoint(command) => Some(RuntimeEntityRef::Checkpoint {
                checkpoint_id: command.checkpoint_id.clone(),
            }),
            RuntimeCommand::RestoreCheckpoint(command) => Some(RuntimeEntityRef::Checkpoint {
                checkpoint_id: command.checkpoint_id.clone(),
            }),
            RuntimeCommand::AppendMailboxMessage(command) => {
                Some(RuntimeEntityRef::MailboxMessage {
                    message_id: command.message_id.clone(),
                })
            }
            RuntimeCommand::StartToolInvocation(command) => {
                Some(RuntimeEntityRef::ToolInvocation {
                    invocation_id: command.invocation_id.clone(),
                })
            }
            RuntimeCommand::FinishToolInvocation(command) => {
                Some(RuntimeEntityRef::ToolInvocation {
                    invocation_id: command.invocation_id.clone(),
                })
            }
            RuntimeCommand::IngestSignal(command) => Some(RuntimeEntityRef::Signal {
                signal_id: command.signal_id.clone(),
            }),
            RuntimeCommand::ReportExternalWorker(command) => Some(RuntimeEntityRef::Task {
                task_id: command.report.task_id.clone(),
            }),
            RuntimeCommand::SpawnSubagent(command) => Some(RuntimeEntityRef::Task {
                task_id: command.child_task_id.clone(),
            }),
            RuntimeCommand::RecordRecoverySweep(command) => command
                .task_id
                .clone()
                .map(|task_id| RuntimeEntityRef::Task { task_id })
                .or_else(|| {
                    command
                        .run_id
                        .clone()
                        .map(|run_id| RuntimeEntityRef::Run { run_id })
                }),
            RuntimeCommand::CompleteRun(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::FailRun(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::CancelRun(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::CompleteTask(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::FailTask(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::CancelTask(command) => Some(RuntimeEntityRef::Task {
                task_id: command.task_id.clone(),
            }),
            RuntimeCommand::AppendUserMessage(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::StartIngestJob(command) => Some(RuntimeEntityRef::IngestJob {
                job_id: command.job_id.clone(),
            }),
            RuntimeCommand::CompleteIngestJob(command) => Some(RuntimeEntityRef::IngestJob {
                job_id: command.job_id.clone(),
            }),
            RuntimeCommand::StartEvalRun(command) => Some(RuntimeEntityRef::EvalRun {
                eval_run_id: command.eval_run_id.clone(),
            }),
            RuntimeCommand::CompleteEvalRun(command) => Some(RuntimeEntityRef::EvalRun {
                eval_run_id: command.eval_run_id.clone(),
            }),
            RuntimeCommand::RecordOutcome(command) => Some(RuntimeEntityRef::Run {
                run_id: command.run_id.clone(),
            }),
            RuntimeCommand::CreatePromptAsset(command) => Some(RuntimeEntityRef::PromptAsset {
                prompt_asset_id: command.prompt_asset_id.clone(),
            }),
            RuntimeCommand::CreatePromptVersion(command) => Some(RuntimeEntityRef::PromptVersion {
                prompt_version_id: command.prompt_version_id.clone(),
            }),
            RuntimeCommand::CreatePromptRelease(command) => Some(RuntimeEntityRef::PromptRelease {
                prompt_release_id: command.prompt_release_id.clone(),
            }),
            RuntimeCommand::TransitionPromptRelease(command) => {
                Some(RuntimeEntityRef::PromptRelease {
                    prompt_release_id: command.prompt_release_id.clone(),
                })
            }
            RuntimeCommand::CreateTenant(_) => None,
            RuntimeCommand::CreateWorkspace(_) => None,
            RuntimeCommand::CreateProject(_) => None,
            RuntimeCommand::CreateScheduledTask(_) => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateSession {
    pub project: ProjectKey,
    pub session_id: SessionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartRun {
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub run_id: RunId,
    pub parent_run_id: Option<RunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitTask {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub parent_run_id: Option<RunId>,
    pub parent_task_id: Option<TaskId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimTask {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub lease_owner: String,
    pub lease_token: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatTaskLease {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub lease_token: u64,
    pub lease_expires_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseRun {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub reason: PauseReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PauseTask {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub reason: PauseReason,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeRun {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub trigger: ResumeTrigger,
    pub target_state: RunResumeTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeTask {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub trigger: ResumeTrigger,
    pub target_state: TaskResumeTarget,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestApproval {
    pub project: ProjectKey,
    pub approval_id: ApprovalId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordApprovalDecision {
    pub project: ProjectKey,
    pub approval_id: ApprovalId,
    pub decision: ApprovalDecision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordCheckpoint {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub checkpoint_id: CheckpointId,
    pub mark_latest: bool,
    pub data: Option<serde_json::Value>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestoreCheckpoint {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub checkpoint_id: CheckpointId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendMailboxMessage {
    pub project: ProjectKey,
    pub message_id: MailboxMessageId,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub from_run_id: Option<RunId>,
    #[serde(default)]
    pub deliver_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartToolInvocation {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub session_id: Option<SessionId>,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub target: ToolInvocationTarget,
    pub execution_class: ExecutionClass,
    pub requested_at_ms: u64,
    pub started_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FinishToolInvocation {
    pub project: ProjectKey,
    pub invocation_id: ToolInvocationId,
    pub finished_at_ms: u64,
    pub outcome: ToolInvocationOutcomeKind,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestSignal {
    pub project: ProjectKey,
    pub signal_id: SignalId,
    pub source: String,
    pub payload: serde_json::Value,
    pub timestamp_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportExternalWorker {
    pub report: ExternalWorkerReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnSubagent {
    pub project: ProjectKey,
    pub parent_run_id: RunId,
    pub parent_task_id: Option<TaskId>,
    pub child_task_id: TaskId,
    pub child_session_id: SessionId,
    pub child_run_id: Option<RunId>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordRecoverySweep {
    pub project: ProjectKey,
    pub run_id: Option<RunId>,
    pub task_id: Option<TaskId>,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteRun {
    pub project: ProjectKey,
    pub run_id: RunId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailRun {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub failure_class: FailureClass,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelRun {
    pub project: ProjectKey,
    pub run_id: RunId,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteTask {
    pub project: ProjectKey,
    pub task_id: TaskId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailTask {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub failure_class: FailureClass,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelTask {
    pub project: ProjectKey,
    pub task_id: TaskId,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppendUserMessage {
    pub project: ProjectKey,
    pub session_id: SessionId,
    pub run_id: RunId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartIngestJob {
    pub project: ProjectKey,
    pub job_id: IngestJobId,
    pub source_id: Option<crate::ids::SourceId>,
    pub document_count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteIngestJob {
    pub project: ProjectKey,
    pub job_id: IngestJobId,
    pub success: bool,
    pub error_message: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartEvalRun {
    pub project: ProjectKey,
    pub eval_run_id: EvalRunId,
    pub subject_kind: String,
    pub evaluator_type: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompleteEvalRun {
    pub project: ProjectKey,
    pub eval_run_id: EvalRunId,
    pub success: bool,
    pub error_message: Option<String>,
}

/// Record the outcome of an agent run for confidence calibration.
///
/// **Invariant:** `predicted_confidence` MUST be finite (no `NaN`, no `inf`).
/// The manual `Eq` impl below trusts this invariant; emitters that permit a
/// `NaN` here will produce a value that violates the `Eq` reflexivity rule
/// (`NaN != NaN`) and can silently break `HashSet`/`BTreeSet` containers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecordOutcome {
    pub project: ProjectKey,
    pub outcome_id: OutcomeId,
    pub run_id: RunId,
    pub agent_type: String,
    pub predicted_confidence: f64,
    pub actual_outcome: crate::events::ActualOutcome,
}

// Manual `Eq` lets this struct be embedded in `RuntimeCommand` which derives
// `Eq`. The `f64` field is safe here ONLY when the finite-value invariant on
// `predicted_confidence` holds (see the struct-level docstring).
impl Eq for RecordOutcome {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePromptAsset {
    pub project: ProjectKey,
    pub prompt_asset_id: PromptAssetId,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePromptVersion {
    pub project: ProjectKey,
    pub prompt_version_id: PromptVersionId,
    pub prompt_asset_id: PromptAssetId,
    pub content: String,
    pub content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreatePromptRelease {
    pub project: ProjectKey,
    pub prompt_release_id: PromptReleaseId,
    pub prompt_asset_id: PromptAssetId,
    pub prompt_version_id: PromptVersionId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransitionPromptRelease {
    pub project: ProjectKey,
    pub prompt_release_id: PromptReleaseId,
    pub to_state: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateTenant {
    pub project: ProjectKey,
    pub tenant_id: TenantId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateWorkspace {
    pub project: ProjectKey,
    pub workspace_id: WorkspaceId,
    pub tenant_id: TenantId,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateProject {
    pub project: ProjectKey,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateScheduledTask {
    pub project: ProjectKey,
    pub scheduled_task_id: ScheduledTaskId,
    pub tenant_id: TenantId,
    pub name: String,
    pub cron_expression: String,
    /// When this task should first fire (Unix ms). `None` means unscheduled
    /// until a cron evaluator computes it.
    pub next_run_at: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::{
        CommandActor, CommandEnvelope, CreateSession, ReportExternalWorker, RuntimeCommand,
        StartToolInvocation,
    };
    use crate::ids::CommandId;
    use crate::policy::ExecutionClass;
    use crate::tenancy::{OwnershipKey, ProjectKey};
    use crate::tool_invocation::ToolInvocationTarget;
    use crate::workers::ExternalWorkerReport;

    #[test]
    fn command_envelope_carries_runtime_payload() {
        let project = ProjectKey::new("tenant", "workspace", "project");
        let command = CommandEnvelope::for_runtime_command(
            CommandId::new("cmd_1"),
            CommandActor::Runtime,
            RuntimeCommand::CreateSession(CreateSession {
                project,
                session_id: "session_1".into(),
            }),
        )
        .with_correlation_id("corr_1");

        assert!(matches!(command.payload, RuntimeCommand::CreateSession(_)));
        assert!(matches!(command.ownership, OwnershipKey::Project(_)));
    }

    #[test]
    fn command_envelope_carries_tool_invocation_payload() {
        let command = CommandEnvelope {
            command_id: CommandId::new("cmd_tool_1"),
            issued_by: CommandActor::Runtime,
            ownership: OwnershipKey::Project(ProjectKey::new("tenant", "workspace", "project")),
            correlation_id: Some("corr_tool_1".to_owned()),
            payload: RuntimeCommand::StartToolInvocation(StartToolInvocation {
                project: ProjectKey::new("tenant", "workspace", "project"),
                invocation_id: "inv_1".into(),
                session_id: Some("session_1".into()),
                run_id: Some("run_1".into()),
                task_id: Some("task_1".into()),
                target: ToolInvocationTarget::Builtin {
                    tool_name: "fs.read".to_owned(),
                },
                execution_class: ExecutionClass::SupervisedProcess,
                requested_at_ms: 10,
                started_at_ms: 11,
            }),
        };

        assert!(matches!(
            command.payload,
            RuntimeCommand::StartToolInvocation(_)
        ));
    }

    #[test]
    fn command_envelope_carries_external_worker_report() {
        let command = CommandEnvelope {
            command_id: CommandId::new("cmd_worker_1"),
            issued_by: CommandActor::ExternalWorker {
                worker: "worker_1".to_owned(),
            },
            ownership: OwnershipKey::Project(ProjectKey::new("tenant", "workspace", "project")),
            correlation_id: Some("corr_worker_1".to_owned()),
            payload: RuntimeCommand::ReportExternalWorker(ReportExternalWorker {
                report: ExternalWorkerReport {
                    project: ProjectKey::new("tenant", "workspace", "project"),
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
            command.payload,
            RuntimeCommand::ReportExternalWorker(_)
        ));
    }

    #[test]
    fn runtime_command_reports_project_and_primary_entity() {
        let command = RuntimeCommand::StartToolInvocation(StartToolInvocation {
            project: ProjectKey::new("tenant", "workspace", "project"),
            invocation_id: "inv_7".into(),
            session_id: Some("session_1".into()),
            run_id: Some("run_1".into()),
            task_id: Some("task_1".into()),
            target: ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            requested_at_ms: 10,
            started_at_ms: 11,
        });

        assert_eq!(command.project().project_id.as_str(), "project");
        assert!(matches!(
            command.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::ToolInvocation { .. })
        ));
    }

    #[test]
    fn command_envelope_reports_project_and_primary_entity() {
        let command = CommandEnvelope::for_runtime_command(
            CommandId::new("cmd_2"),
            CommandActor::Runtime,
            RuntimeCommand::CreateSession(CreateSession {
                project: ProjectKey::new("tenant", "workspace", "project"),
                session_id: "session_2".into(),
            }),
        );

        assert_eq!(command.project().project_id.as_str(), "project");
        assert!(matches!(
            command.primary_entity_ref(),
            Some(crate::errors::RuntimeEntityRef::Session { .. })
        ));
    }
}
