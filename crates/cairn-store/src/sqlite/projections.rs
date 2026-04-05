use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::RuntimeEvent;

use crate::error::StoreError;
use crate::event_log::StoredEvent;

/// SQLite-backed synchronous projection applier.
///
/// Mirrors PgSyncProjection but for SQLite local-mode.
/// All methods are async and operate within an existing transaction.
pub struct SqliteSyncProjection;

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
                sqlx::query(
                    "INSERT INTO sessions (session_id, tenant_id, workspace_id, project_id, state, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, 'open', 1, ?, ?)",
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
                sqlx::query(
                    "INSERT INTO tasks (task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, state, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, 'queued', 1, ?, ?)",
                )
                .bind(e.task_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.parent_run_id.as_ref().map(|id| id.as_str()))
                .bind(e.parent_task_id.as_ref().map(|id| id.as_str()))
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
                    "INSERT INTO approvals (approval_id, tenant_id, workspace_id, project_id, run_id, task_id, requirement, version, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
                )
                .bind(e.approval_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.run_id.as_ref().map(|id| id.as_str()))
                .bind(e.task_id.as_ref().map(|id| id.as_str()))
                .bind(requirement_str)
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
                sqlx::query(
                    "UPDATE tool_invocations SET state = 'failed', outcome = ?, error_message = ?, finished_at_ms = ?, version = version + 1, updated_at = ? WHERE invocation_id = ?",
                )
                .bind(outcome_str)
                .bind(e.error_message.as_deref())
                .bind(e.finished_at_ms as i64)
                .bind(now)
                .bind(e.invocation_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ExternalWorkerRegistered(_)
            | RuntimeEvent::ExternalWorkerReported(_)
            | RuntimeEvent::ExternalWorkerSuspended(_)
            | RuntimeEvent::ExternalWorkerReactivated(_)
            | RuntimeEvent::SoulPatchProposed(_)
            | RuntimeEvent::SoulPatchApplied(_)
            | RuntimeEvent::SessionCostUpdated(_)
            | RuntimeEvent::RunCostUpdated(_)
            | RuntimeEvent::SpendAlertTriggered(_)
            | RuntimeEvent::SubagentSpawned(_)
            | RuntimeEvent::RecoveryAttempted(_)
            | RuntimeEvent::RecoveryCompleted(_)
            | RuntimeEvent::SignalIngested(_)
            | RuntimeEvent::UserMessageAppended(_)
            | RuntimeEvent::IngestJobStarted(_)
            | RuntimeEvent::IngestJobCompleted(_)
            | RuntimeEvent::EvalRunStarted(_)
            | RuntimeEvent::EvalRunCompleted(_)
            | RuntimeEvent::PromptAssetCreated(_)
            | RuntimeEvent::PromptVersionCreated(_)
            | RuntimeEvent::ApprovalPolicyCreated(_)
            | RuntimeEvent::PromptReleaseCreated(_)
            | RuntimeEvent::PromptReleaseTransitioned(_)
            | RuntimeEvent::PromptRolloutStarted(_)
            | RuntimeEvent::TenantCreated(_)
            | RuntimeEvent::WorkspaceCreated(_)
            | RuntimeEvent::ProjectCreated(_)
            | RuntimeEvent::RouteDecisionMade(_)
            | RuntimeEvent::ProviderCallCompleted(_) => {}
            | RuntimeEvent::ProviderBudgetSet(_)
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
            | RuntimeEvent::ToolInvocationProgressUpdated(_) => {}
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
