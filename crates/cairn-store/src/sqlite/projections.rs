use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::RuntimeEvent;

use crate::error::StoreError;
use crate::event_log::StoredEvent;

/// SQLite-backed synchronous projection applier for local-mode deploys.
///
/// **Coverage gap relative to PgSyncProjection.** This applier implements
/// projections for the core operational state machines (session, run, task,
/// approval, checkpoint, tool_invocation, mailbox) and silently ignores the
/// remaining ~95 `RuntimeEvent` variants. The append path in
/// `SqliteEventLog` invokes this applier inside the insert transaction, but
/// stubbed variants commit only to the `event_log` table — their projection
/// tables either do not exist in the SQLite schema or would be overwritten
/// on replay.
///
/// Each stubbed variant is logged at `tracing::warn!` level so operators
/// running `--db sqlite:…` can see in real time which RFC features are
/// being silently dropped by the local-mode backend. If you land on this
/// warning in a production log, either (a) switch to the Postgres backend,
/// which projects every variant, or (b) extend this applier to cover the
/// variant you care about.
///
/// Audit reference: `.claude/audit-state/review-queue.md` §T2-C2.
pub struct SqliteSyncProjection;

/// Log a received-but-unprojected event variant. Keeps the stub-match arms
/// uniform and makes the coverage gap visible without spamming logs when
/// no stub variants are ever received.
fn log_stub(variant: &'static str) {
    tracing::warn!(
        event_variant = variant,
        "sqlite projection stub: event committed to event_log but no projection table updated \
         (see SqliteSyncProjection docstring for the coverage gap)"
    );
}

impl SqliteSyncProjection {
    /// Async projection application within a SQLite transaction.
    pub async fn apply_async(
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        event: &StoredEvent,
    ) -> Result<(), StoreError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        match &event.envelope.payload {
            RuntimeEvent::SessionCreated(e) => {
                // Idempotent: a second SessionCreated event for the same id
                // must not blow up the transaction. The event log is the
                // durable source of truth; the projection is derived and
                // should tolerate duplicates (mirrors PG's ON CONFLICT
                // DO NOTHING shape). Matches pg/projections.rs.
                sqlx::query(
                    "INSERT INTO sessions (session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, 'open', 1, ?, ?)
                     ON CONFLICT(session_id) DO NOTHING",
                )
                .bind(e.session_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::SessionStateChanged(e) => {
                let state_str = enum_to_str(&e.transition.to)?;
                sqlx::query(
                    "UPDATE sessions SET state = ?, version = version + 1, updated_at = ? WHERE session_id = ?",
                )
                .bind(state_str)
                .bind(now)
                .bind(e.session_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::RunCreated(e) => {
                sqlx::query(
                    "INSERT INTO runs (run_id, session_id, parent_run_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, 'pending', 1, ?, ?)",
                )
                .bind(e.run_id.as_str())
                .bind(e.session_id.as_str())
                .bind(e.parent_run_id.as_ref().map(|id| id.as_str()))
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::RunStateChanged(e) => {
                let state_str = enum_to_str(&e.transition.to)?;
                let failure = e.failure_class.as_ref().map(enum_to_str).transpose()?;
                sqlx::query(
                    "UPDATE runs SET state = ?, failure_class = ?, version = version + 1, updated_at = ? WHERE run_id = ?",
                )
                .bind(state_str)
                .bind(failure)
                .bind(now)
                .bind(e.run_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::TaskCreated(e) => {
                // Prefer the session_id on the event; fall back to the
                // parent run's session_id for tasks that carried no binding.
                // COALESCE lets SQLite resolve both in one statement.
                let session_id_on_event = e.session_id.as_ref().map(|s| s.as_str());
                sqlx::query(
                    "INSERT INTO tasks (task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, session_id, state, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?,
                        COALESCE(?, (SELECT session_id FROM runs WHERE run_id = ?)),
                        'queued', 1, ?, ?)",
                )
                .bind(e.task_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.parent_run_id.as_ref().map(|id| id.as_str()))
                .bind(e.parent_task_id.as_ref().map(|id| id.as_str()))
                .bind(session_id_on_event)
                .bind(e.parent_run_id.as_ref().map(|id| id.as_str()))
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::TaskLeaseClaimed(e) => {
                sqlx::query(
                    "UPDATE tasks SET state = 'leased', lease_owner = ?, lease_expires_at = ?, lease_version = ?, version = version + 1, updated_at = ? WHERE task_id = ?",
                )
                .bind(&e.lease_owner)
                .bind(e.lease_expires_at_ms as i64)
                .bind(e.lease_token as i64)
                .bind(now)
                .bind(e.task_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::TaskLeaseHeartbeated(e) => {
                sqlx::query(
                    "UPDATE tasks SET lease_expires_at = ?, lease_version = ?, version = version + 1, updated_at = ? WHERE task_id = ?",
                )
                .bind(e.lease_expires_at_ms as i64)
                .bind(e.lease_token as i64)
                .bind(now)
                .bind(e.task_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::TaskStateChanged(e) => {
                let state_str = enum_to_str(&e.transition.to)?;
                let failure = e.failure_class.as_ref().map(enum_to_str).transpose()?;
                sqlx::query(
                    "UPDATE tasks SET state = ?, failure_class = ?, version = version + 1, updated_at = ? WHERE task_id = ?",
                )
                .bind(state_str)
                .bind(failure)
                .bind(now)
                .bind(e.task_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ApprovalRequested(e) => {
                let requirement_str = enum_to_str(&e.requirement)?;
                sqlx::query(
                    "INSERT INTO approvals (approval_id, tenant_id, workspace_id, project_id, run_id, task_id, requirement, title, description, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
                )
                .bind(e.approval_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.run_id.as_ref().map(|id| id.as_str()))
                .bind(e.task_id.as_ref().map(|id| id.as_str()))
                .bind(requirement_str)
                .bind(e.title.as_deref())
                .bind(e.description.as_deref())
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ApprovalResolved(e) => {
                let decision_str = enum_to_str(&e.decision)?;
                sqlx::query(
                    "UPDATE approvals SET decision = ?, version = version + 1, updated_at = ? WHERE approval_id = ?",
                )
                .bind(decision_str)
                .bind(now)
                .bind(e.approval_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::CheckpointRecorded(e) => {
                let disposition_str = enum_to_str(&e.disposition)?;

                if disposition_str == "latest" {
                    sqlx::query(
                        "UPDATE checkpoints SET disposition = 'superseded', version = version + 1 WHERE run_id = ? AND disposition = 'latest'",
                    )
                    .bind(e.run_id.as_str())
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| StoreError::Internal(e.to_string()))?;
                }

                sqlx::query(
                    "INSERT INTO checkpoints (checkpoint_id, tenant_id, workspace_id, project_id, run_id, disposition, version, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, 1, ?)",
                )
                .bind(e.checkpoint_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.run_id.as_str())
                .bind(disposition_str)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::CheckpointRestored(_) => {}

            RuntimeEvent::MailboxMessageAppended(e) => {
                sqlx::query(
                    "INSERT INTO mailbox_messages (message_id, tenant_id, workspace_id, project_id, run_id, task_id, version, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, 1, ?)",
                )
                .bind(e.message_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.run_id.as_ref().map(|id| id.as_str()))
                .bind(e.task_id.as_ref().map(|id| id.as_str()))
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ToolInvocationStarted(e) => {
                let target = serde_json::to_string(&e.target)
                    .map_err(|e| StoreError::Serialization(e.to_string()))?;
                let exec_class_str = enum_to_str(&e.execution_class)?;

                sqlx::query(
                    "INSERT INTO tool_invocations (invocation_id, tenant_id, workspace_id, project_id, session_id, run_id, task_id, target, execution_class, state, requested_at_ms, started_at_ms, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'started', ?, ?, 1, ?, ?)",
                )
                .bind(e.invocation_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.session_id.as_ref().map(|id| id.as_str()))
                .bind(e.run_id.as_ref().map(|id| id.as_str()))
                .bind(e.task_id.as_ref().map(|id| id.as_str()))
                .bind(&target)
                .bind(exec_class_str)
                .bind(e.requested_at_ms as i64)
                .bind(e.started_at_ms as i64)
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ToolInvocationCompleted(e) => {
                let outcome_str = enum_to_str(&e.outcome)?;
                sqlx::query(
                    "UPDATE tool_invocations SET state = 'completed', outcome = ?, finished_at_ms = ?, version = version + 1, updated_at = ? WHERE invocation_id = ?",
                )
                .bind(outcome_str)
                .bind(e.finished_at_ms as i64)
                .bind(now)
                .bind(e.invocation_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ToolInvocationFailed(e) => {
                let outcome_str = enum_to_str(&e.outcome)?;
                // Route terminal state through the same helper PG uses so a
                // canceled outcome lands as `state='canceled'` (not `'failed'`);
                // pre-T2-H5 SQLite hardcoded `'failed'` and mislabeled cancels.
                let state_str = enum_to_str(&e.outcome.terminal_state())?;
                sqlx::query(
                    "UPDATE tool_invocations SET state = ?, outcome = ?, error_message = ?, finished_at_ms = ?, version = version + 1, updated_at = ? WHERE invocation_id = ?",
                )
                .bind(state_str)
                .bind(outcome_str)
                .bind(e.error_message.as_deref())
                .bind(e.finished_at_ms as i64)
                .bind(now)
                .bind(e.invocation_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            // ── UNPROJECTED STUBS ──────────────────────────────────────
            // These variants commit to event_log but do NOT update any
            // projection table on the SQLite backend. See the struct
            // docstring for the coverage-gap rationale and logging.
            RuntimeEvent::ExternalWorkerRegistered(_) => log_stub("ExternalWorkerRegistered"),
            RuntimeEvent::ExternalWorkerReported(_) => log_stub("ExternalWorkerReported"),
            RuntimeEvent::ExternalWorkerSuspended(_) => log_stub("ExternalWorkerSuspended"),
            RuntimeEvent::ExternalWorkerReactivated(_) => log_stub("ExternalWorkerReactivated"),
            RuntimeEvent::SoulPatchProposed(_) => log_stub("SoulPatchProposed"),
            RuntimeEvent::SoulPatchApplied(_) => log_stub("SoulPatchApplied"),
            RuntimeEvent::SessionCostUpdated(_) => log_stub("SessionCostUpdated"),
            RuntimeEvent::RunCostUpdated(_) => log_stub("RunCostUpdated"),
            RuntimeEvent::SpendAlertTriggered(_) => log_stub("SpendAlertTriggered"),
            RuntimeEvent::SubagentSpawned(_) => log_stub("SubagentSpawned"),
            RuntimeEvent::RecoveryAttempted(_) => log_stub("RecoveryAttempted"),
            RuntimeEvent::RecoveryCompleted(_) => log_stub("RecoveryCompleted"),
            RuntimeEvent::SignalIngested(_) => log_stub("SignalIngested"),
            RuntimeEvent::UserMessageAppended(_) => log_stub("UserMessageAppended"),
            RuntimeEvent::IngestJobStarted(_) => log_stub("IngestJobStarted"),
            RuntimeEvent::IngestJobCompleted(_) => log_stub("IngestJobCompleted"),
            RuntimeEvent::EvalRunStarted(_) => log_stub("EvalRunStarted"),
            RuntimeEvent::EvalRunCompleted(_) => log_stub("EvalRunCompleted"),
            RuntimeEvent::PromptAssetCreated(_) => log_stub("PromptAssetCreated"),
            RuntimeEvent::PromptVersionCreated(_) => log_stub("PromptVersionCreated"),
            RuntimeEvent::ApprovalPolicyCreated(_) => log_stub("ApprovalPolicyCreated"),
            RuntimeEvent::PromptReleaseCreated(_) => log_stub("PromptReleaseCreated"),
            RuntimeEvent::PromptReleaseTransitioned(_) => log_stub("PromptReleaseTransitioned"),
            RuntimeEvent::PromptRolloutStarted(_) => log_stub("PromptRolloutStarted"),
            RuntimeEvent::TenantCreated(_) => log_stub("TenantCreated"),
            RuntimeEvent::WorkspaceCreated(_) => log_stub("WorkspaceCreated"),
            RuntimeEvent::ProjectCreated(_) => log_stub("ProjectCreated"),
            RuntimeEvent::RouteDecisionMade(_) => log_stub("RouteDecisionMade"),
            RuntimeEvent::ProviderCallCompleted(_) => log_stub("ProviderCallCompleted"),
            RuntimeEvent::OutcomeRecorded(_) => log_stub("OutcomeRecorded"),
            RuntimeEvent::ScheduledTaskCreated(_) => log_stub("ScheduledTaskCreated"),
            RuntimeEvent::PlanProposed(_) => log_stub("PlanProposed"),
            RuntimeEvent::PlanApproved(_) => log_stub("PlanApproved"),
            RuntimeEvent::PlanRejected(_) => log_stub("PlanRejected"),
            RuntimeEvent::PlanRevisionRequested(_) => log_stub("PlanRevisionRequested"),
            RuntimeEvent::ProviderBudgetSet(_) => log_stub("ProviderBudgetSet"),
            RuntimeEvent::ChannelCreated(_) => log_stub("ChannelCreated"),
            RuntimeEvent::ChannelMessageSent(_) => log_stub("ChannelMessageSent"),
            RuntimeEvent::ChannelMessageConsumed(_) => log_stub("ChannelMessageConsumed"),
            RuntimeEvent::DefaultSettingSet(_) => log_stub("DefaultSettingSet"),
            RuntimeEvent::DefaultSettingCleared(_) => log_stub("DefaultSettingCleared"),
            RuntimeEvent::LicenseActivated(_) => log_stub("LicenseActivated"),
            RuntimeEvent::EntitlementOverrideSet(_) => log_stub("EntitlementOverrideSet"),
            RuntimeEvent::NotificationPreferenceSet(_) => log_stub("NotificationPreferenceSet"),
            RuntimeEvent::NotificationSent(_) => log_stub("NotificationSent"),
            RuntimeEvent::ProviderPoolCreated(_) => log_stub("ProviderPoolCreated"),
            RuntimeEvent::ProviderPoolConnectionAdded(_) => log_stub("ProviderPoolConnectionAdded"),
            RuntimeEvent::ProviderPoolConnectionRemoved(_) => {
                log_stub("ProviderPoolConnectionRemoved")
            }
            RuntimeEvent::TenantQuotaSet(_) => log_stub("TenantQuotaSet"),
            RuntimeEvent::TenantQuotaViolated(_) => log_stub("TenantQuotaViolated"),
            RuntimeEvent::RetentionPolicySet(_) => log_stub("RetentionPolicySet"),
            RuntimeEvent::RunCostAlertSet(_) => log_stub("RunCostAlertSet"),
            RuntimeEvent::RunCostAlertTriggered(_) => log_stub("RunCostAlertTriggered"),
            RuntimeEvent::WorkspaceMemberAdded(_) => log_stub("WorkspaceMemberAdded"),
            RuntimeEvent::WorkspaceMemberRemoved(_) => log_stub("WorkspaceMemberRemoved"),
            RuntimeEvent::ApprovalDelegated(_) => log_stub("ApprovalDelegated"),
            RuntimeEvent::AuditLogEntryRecorded(_) => log_stub("AuditLogEntryRecorded"),
            RuntimeEvent::CheckpointStrategySet(_) => log_stub("CheckpointStrategySet"),
            RuntimeEvent::CredentialKeyRotated(_) => log_stub("CredentialKeyRotated"),
            RuntimeEvent::CredentialRevoked(_) => log_stub("CredentialRevoked"),
            RuntimeEvent::CredentialStored(_) => log_stub("CredentialStored"),
            RuntimeEvent::EvalBaselineLocked(_) => log_stub("EvalBaselineLocked"),
            RuntimeEvent::EvalBaselineSet(_) => log_stub("EvalBaselineSet"),
            RuntimeEvent::EvalDatasetCreated(_) => log_stub("EvalDatasetCreated"),
            RuntimeEvent::EvalDatasetEntryAdded(_) => log_stub("EvalDatasetEntryAdded"),
            RuntimeEvent::EvalRubricCreated(_) => log_stub("EvalRubricCreated"),
            RuntimeEvent::EventLogCompacted(_) => log_stub("EventLogCompacted"),
            RuntimeEvent::GuardrailPolicyCreated(_) => log_stub("GuardrailPolicyCreated"),
            RuntimeEvent::GuardrailPolicyEvaluated(_) => log_stub("GuardrailPolicyEvaluated"),
            RuntimeEvent::OperatorIntervention(_) => log_stub("OperatorIntervention"),
            RuntimeEvent::OperatorProfileCreated(_) => log_stub("OperatorProfileCreated"),
            RuntimeEvent::OperatorProfileUpdated(_) => log_stub("OperatorProfileUpdated"),
            RuntimeEvent::PauseScheduled(_) => log_stub("PauseScheduled"),
            RuntimeEvent::PermissionDecisionRecorded(_) => log_stub("PermissionDecisionRecorded"),
            RuntimeEvent::ProviderBindingCreated(_) => log_stub("ProviderBindingCreated"),
            RuntimeEvent::ProviderBindingStateChanged(_) => log_stub("ProviderBindingStateChanged"),
            RuntimeEvent::ProviderBudgetAlertTriggered(_) => {
                log_stub("ProviderBudgetAlertTriggered")
            }
            RuntimeEvent::ProviderBudgetExceeded(_) => log_stub("ProviderBudgetExceeded"),
            RuntimeEvent::ProviderConnectionRegistered(_) => {
                log_stub("ProviderConnectionRegistered")
            }
            RuntimeEvent::ProviderHealthChecked(_) => log_stub("ProviderHealthChecked"),
            RuntimeEvent::ProviderHealthScheduleSet(_) => log_stub("ProviderHealthScheduleSet"),
            RuntimeEvent::ProviderHealthScheduleTriggered(_) => {
                log_stub("ProviderHealthScheduleTriggered")
            }
            RuntimeEvent::ProviderMarkedDegraded(_) => log_stub("ProviderMarkedDegraded"),
            RuntimeEvent::ProviderModelRegistered(_) => log_stub("ProviderModelRegistered"),
            RuntimeEvent::ProviderRecovered(_) => log_stub("ProviderRecovered"),
            RuntimeEvent::ProviderRetryPolicySet(_) => log_stub("ProviderRetryPolicySet"),
            RuntimeEvent::RecoveryEscalated(_) => log_stub("RecoveryEscalated"),
            RuntimeEvent::ResourceShareRevoked(_) => log_stub("ResourceShareRevoked"),
            RuntimeEvent::ResourceShared(_) => log_stub("ResourceShared"),
            RuntimeEvent::RoutePolicyCreated(_) => log_stub("RoutePolicyCreated"),
            RuntimeEvent::RoutePolicyUpdated(_) => log_stub("RoutePolicyUpdated"),
            RuntimeEvent::RunSlaBreached(_) => log_stub("RunSlaBreached"),
            RuntimeEvent::RunSlaSet(_) => log_stub("RunSlaSet"),
            RuntimeEvent::SignalRouted(_) => log_stub("SignalRouted"),
            RuntimeEvent::SignalSubscriptionCreated(_) => log_stub("SignalSubscriptionCreated"),
            RuntimeEvent::TriggerCreated(_) => log_stub("TriggerCreated"),
            RuntimeEvent::TriggerEnabled(_) => log_stub("TriggerEnabled"),
            RuntimeEvent::TriggerDisabled(_) => log_stub("TriggerDisabled"),
            RuntimeEvent::TriggerSuspended(_) => log_stub("TriggerSuspended"),
            RuntimeEvent::TriggerResumed(_) => log_stub("TriggerResumed"),
            RuntimeEvent::TriggerDeleted(_) => log_stub("TriggerDeleted"),
            RuntimeEvent::TriggerFired(_) => log_stub("TriggerFired"),
            RuntimeEvent::TriggerSkipped(_) => log_stub("TriggerSkipped"),
            RuntimeEvent::TriggerDenied(_) => log_stub("TriggerDenied"),
            RuntimeEvent::TriggerRateLimited(_) => log_stub("TriggerRateLimited"),
            RuntimeEvent::TriggerPendingApproval(_) => log_stub("TriggerPendingApproval"),
            RuntimeEvent::RunTemplateCreated(_) => log_stub("RunTemplateCreated"),
            RuntimeEvent::RunTemplateDeleted(_) => log_stub("RunTemplateDeleted"),
            RuntimeEvent::SnapshotCreated(_) => log_stub("SnapshotCreated"),
            RuntimeEvent::TaskDependencyAdded(_) => log_stub("TaskDependencyAdded"),
            RuntimeEvent::TaskDependencyResolved(_) => log_stub("TaskDependencyResolved"),
            RuntimeEvent::TaskLeaseExpired(_) => log_stub("TaskLeaseExpired"),
            RuntimeEvent::TaskPriorityChanged(_) => log_stub("TaskPriorityChanged"),
            RuntimeEvent::ToolInvocationProgressUpdated(_) => {
                log_stub("ToolInvocationProgressUpdated")
            }
            // RFC 020 Track 3: audit-only events; no projection update needed.
            RuntimeEvent::ToolInvocationCacheHit(_) => log_stub("ToolInvocationCacheHit"),
            RuntimeEvent::ToolRecoveryPaused(_) => log_stub("ToolRecoveryPaused"),
        }

        Ok(())
    }
}

fn enum_to_str<T: serde::Serialize>(val: &T) -> Result<String, StoreError> {
    let v = serde_json::to_value(val).map_err(|e| StoreError::Serialization(e.to_string()))?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        _ => Ok(v.to_string().trim_matches('"').to_owned()),
    }
}
