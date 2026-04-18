use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{tool_invocation::ToolInvocationOutcomeKind, RuntimeEvent};

use crate::error::StoreError;
use crate::event_log::StoredEvent;

/// Postgres-backed synchronous projection applier.
///
/// Dispatches stored events to current-state table upserts.
/// All methods are async and operate within an existing transaction.
pub struct PgSyncProjection;

impl PgSyncProjection {
    /// Async projection application within a transaction.
    ///
    /// This is the real implementation used by PgEventLog when it
    /// appends events within a transaction.
    pub async fn apply_async(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
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
                     VALUES ($1, $2, $3, $4, 'open', 1, $5, $5)",
                )
                .bind(e.session_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::SessionStateChanged(e) => {
                let state_str = enum_to_str(&e.transition.to)?;
                sqlx::query(
                    "UPDATE sessions SET state = $1, version = version + 1, updated_at = $2 WHERE session_id = $3",
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
                     VALUES ($1, $2, $3, $4, $5, $6, 'pending', 1, $7, $7)",
                )
                .bind(e.run_id.as_str())
                .bind(e.session_id.as_str())
                .bind(e.parent_run_id.as_ref().map(|id| id.as_str()))
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::RunStateChanged(e) => {
                let state_str = enum_to_str(&e.transition.to)?;
                let failure = e.failure_class.as_ref().map(enum_to_str).transpose()?;
                sqlx::query(
                    "UPDATE runs SET state = $1, failure_class = $2, version = version + 1, updated_at = $3 WHERE run_id = $4",
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
                    "INSERT INTO tasks (task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, state, title, description, version, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 'queued', NULL, NULL, 1, $7, $7)",
                )
                .bind(e.task_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.parent_run_id.as_ref().map(|id| id.as_str()))
                .bind(e.parent_task_id.as_ref().map(|id| id.as_str()))
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::TaskLeaseClaimed(e) => {
                sqlx::query(
                    "UPDATE tasks SET state = 'leased', lease_owner = $1, lease_expires_at = $2, lease_version = $3, version = version + 1, updated_at = $4 WHERE task_id = $5",
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
                    "UPDATE tasks SET lease_expires_at = $1, lease_version = $2, version = version + 1, updated_at = $3 WHERE task_id = $4",
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
                    "UPDATE tasks SET state = $1, failure_class = $2, version = version + 1, updated_at = $3 WHERE task_id = $4",
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
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, $10, $10)",
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
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ApprovalResolved(e) => {
                let decision_str = enum_to_str(&e.decision)?;
                sqlx::query(
                    "UPDATE approvals SET decision = $1, version = version + 1, updated_at = $2 WHERE approval_id = $3",
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
                        "UPDATE checkpoints SET disposition = 'superseded', version = version + 1 WHERE run_id = $1 AND disposition = 'latest'",
                    )
                    .bind(e.run_id.as_str())
                    .execute(&mut **tx)
                    .await
                    .map_err(|e| StoreError::Internal(e.to_string()))?;
                }

                sqlx::query(
                    "INSERT INTO checkpoints (checkpoint_id, tenant_id, workspace_id, project_id, run_id, disposition, version, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 1, $7)",
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

            RuntimeEvent::CheckpointRestored(_e) => {
                // Restore events are linkage records, not state mutations.
                // The checkpoint table does not change disposition on restore.
                // Logged in the event log for replay/audit purposes.
            }

            RuntimeEvent::MailboxMessageAppended(e) => {
                sqlx::query(
                    "INSERT INTO mailbox_messages (message_id, tenant_id, workspace_id, project_id, run_id, task_id, version, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6, 1, $7)",
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
                let target = serde_json::to_value(&e.target)
                    .map_err(|e| StoreError::Serialization(e.to_string()))?;
                let exec_class_str = enum_to_str(&e.execution_class)?;

                sqlx::query(
                    "INSERT INTO tool_invocations (invocation_id, tenant_id, workspace_id, project_id, session_id, run_id, task_id, target, execution_class, state, requested_at_ms, started_at_ms, version, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'started', $10, $11, 1, $12, $12)",
                )
                .bind(e.invocation_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.session_id.as_ref().map(|id| id.as_str()))
                .bind(e.run_id.as_ref().map(|id| id.as_str()))
                .bind(e.task_id.as_ref().map(|id| id.as_str()))
                .bind(target)
                .bind(exec_class_str)
                .bind(e.requested_at_ms as i64)
                .bind(e.started_at_ms as i64)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
            }

            RuntimeEvent::ToolInvocationCompleted(e) => {
                let outcome_str = enum_to_str(&e.outcome)?;
                sqlx::query(
                    "UPDATE tool_invocations SET state = 'completed', outcome = $1, finished_at_ms = $2, version = version + 1, updated_at = $3 WHERE invocation_id = $4",
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
                let state_str = tool_invocation_terminal_state_str(e.outcome)?;
                let outcome_str = enum_to_str(&e.outcome)?;
                sqlx::query(
                    "UPDATE tool_invocations SET state = $1, outcome = $2, error_message = $3, finished_at_ms = $4, version = version + 1, updated_at = $5 WHERE invocation_id = $6",
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
            // projection table on the Postgres backend. Audit reference:
            // `.claude/audit-state/review-queue.md` §T2-H3. If you land on
            // this warning in production, extend this applier to cover the
            // specific variant and its projection table(s).
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
            RuntimeEvent::ProviderPoolConnectionAdded(_) => {
                log_stub("ProviderPoolConnectionAdded")
            }
            RuntimeEvent::ProviderPoolConnectionRemoved(_) => {
                log_stub("ProviderPoolConnectionRemoved")
            }
            RuntimeEvent::TenantQuotaSet(_) => log_stub("TenantQuotaSet"),
            RuntimeEvent::TenantQuotaViolated(_) => log_stub("TenantQuotaViolated"),
            RuntimeEvent::RetentionPolicySet(_) => log_stub("RetentionPolicySet"),
            RuntimeEvent::RunCostAlertSet(_) => log_stub("RunCostAlertSet"),
            RuntimeEvent::RunCostAlertTriggered(_) => log_stub("RunCostAlertTriggered"),
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
            RuntimeEvent::PermissionDecisionRecorded(_) => {
                log_stub("PermissionDecisionRecorded")
            }
            RuntimeEvent::ProviderBindingCreated(_) => log_stub("ProviderBindingCreated"),
            RuntimeEvent::ProviderBindingStateChanged(_) => {
                log_stub("ProviderBindingStateChanged")
            }
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
            RuntimeEvent::RoutePolicyUpdated(_) => log_stub("RoutePolicyUpdated"),

            RuntimeEvent::TenantCreated(e) => {
                sqlx::query(
                    "INSERT INTO tenants (tenant_id, name, created_at, updated_at)
                     VALUES ($1, $2, $3, $3)
                     ON CONFLICT (tenant_id) DO NOTHING",
                )
                .bind(e.tenant_id.as_str())
                .bind(&e.name)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::WorkspaceCreated(e) => {
                sqlx::query(
                    "INSERT INTO workspaces (workspace_id, tenant_id, name, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $4)
                     ON CONFLICT (workspace_id) DO NOTHING",
                )
                .bind(e.workspace_id.as_str())
                .bind(e.tenant_id.as_str())
                .bind(&e.name)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::ProjectCreated(e) => {
                sqlx::query(
                    "INSERT INTO projects (project_id, workspace_id, tenant_id, name, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $5)
                     ON CONFLICT (project_id) DO NOTHING",
                )
                .bind(e.project.project_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(&e.name)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::RoutePolicyCreated(e) => {
                let rules = serde_json::to_value(&e.rules)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                sqlx::query(
                    "INSERT INTO route_policies (policy_id, tenant_id, name, rules, enabled, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, TRUE, $5, $5)
                     ON CONFLICT (policy_id) DO UPDATE
                     SET name = EXCLUDED.name, rules = EXCLUDED.rules, updated_at = EXCLUDED.updated_at",
                )
                .bind(&e.policy_id)
                .bind(e.tenant_id.as_str())
                .bind(&e.name)
                .bind(rules)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::WorkspaceMemberAdded(e) => {
                let role = enum_to_str(&e.role)?;
                sqlx::query(
                    "INSERT INTO workspace_members (workspace_id, operator_id, role, added_at_ms)
                     VALUES ($1, $2, $3, $4)
                     ON CONFLICT (workspace_id, operator_id) DO UPDATE SET role = EXCLUDED.role",
                )
                .bind(e.workspace_key.workspace_id.as_str())
                .bind(e.member_id.as_str())
                .bind(role)
                .bind(e.added_at_ms as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::WorkspaceMemberRemoved(e) => {
                sqlx::query(
                    "DELETE FROM workspace_members WHERE workspace_id = $1 AND operator_id = $2",
                )
                .bind(e.workspace_key.workspace_id.as_str())
                .bind(e.member_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::PromptAssetCreated(e) => {
                sqlx::query(
                    "INSERT INTO prompt_assets
                         (prompt_asset_id, tenant_id, workspace_id, project_id, name, kind,
                          scope, status, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, NULL, 'draft', $7, $8)
                     ON CONFLICT (prompt_asset_id) DO NOTHING",
                )
                .bind(e.prompt_asset_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(&e.name)
                .bind(&e.kind)
                .bind(e.created_at as i64)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::PromptVersionCreated(e) => {
                // Allocate the next version_number under row-level lock.
                //
                // Pre-T2-H4 this used `COUNT(*) + 1` under READ COMMITTED
                // which is a classic race — two concurrent appends for
                // the same asset both read N and try to insert N+1,
                // colliding on the unique constraint or silently producing
                // duplicates.
                //
                // The first serialisation attempt used `SELECT MAX(...)
                // FROM prompt_versions WHERE prompt_asset_id = $1 FOR
                // UPDATE`, but `FOR UPDATE` on an aggregate query with
                // zero matching rows locks nothing. We now take a lock on
                // the parent `prompt_assets` row (which must exist before
                // any version for it can be created) and derive the
                // version number from the versions table under that held
                // lock. Concurrent appends for the same asset serialise
                // behind the parent row lock.
                //
                // Errors propagate (no `unwrap_or` swallowing) so the
                // transaction aborts rather than inserting
                // `version_number = 1` on top of a transient DB failure.
                sqlx::query_scalar::<_, String>(
                    "SELECT prompt_asset_id FROM prompt_assets
                     WHERE prompt_asset_id = $1
                     FOR UPDATE",
                )
                .bind(e.prompt_asset_id.as_str())
                .fetch_optional(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;

                let version_number: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(version_number), 0) + 1
                     FROM prompt_versions
                     WHERE prompt_asset_id = $1",
                )
                .bind(e.prompt_asset_id.as_str())
                .fetch_one(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;

                sqlx::query(
                    "INSERT INTO prompt_versions
                         (prompt_version_id, prompt_asset_id, tenant_id, workspace_id, project_id,
                          version_number, content_hash, content, format, created_by, created_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, NULL, NULL, NULL, $8)
                     ON CONFLICT (prompt_version_id) DO NOTHING",
                )
                .bind(e.prompt_version_id.as_str())
                .bind(e.prompt_asset_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(version_number)
                .bind(&e.content_hash)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::PromptReleaseCreated(e) => {
                sqlx::query(
                    "INSERT INTO prompt_releases
                         (prompt_release_id, prompt_asset_id, prompt_version_id,
                          tenant_id, workspace_id, project_id,
                          release_tag, state, rollout_target, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, 'draft', NULL, $8, $8)
                     ON CONFLICT (prompt_release_id) DO NOTHING",
                )
                .bind(e.prompt_release_id.as_str())
                .bind(e.prompt_asset_id.as_str())
                .bind(e.prompt_version_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.release_tag.as_deref())
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::PromptReleaseTransitioned(e) => {
                sqlx::query(
                    "UPDATE prompt_releases
                     SET state = $1, updated_at = $2
                     WHERE prompt_release_id = $3",
                )
                .bind(&e.to_state)
                .bind(now)
                .bind(e.prompt_release_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::RouteDecisionMade(e) => {
                let operation_kind = enum_to_str(&e.operation_kind)?;
                let final_status   = enum_to_str(&e.final_status)?;
                let selector_ctx: Option<serde_json::Value> = None; // not carried by event
                sqlx::query(
                    "INSERT INTO route_decisions
                         (route_decision_id, tenant_id, workspace_id, project_id,
                          operation_kind, route_policy_id, terminal_route_attempt_id,
                          selected_provider_binding_id, selected_route_attempt_id,
                          selector_context, attempt_count, fallback_used, final_status,
                          created_at)
                     VALUES ($1, $2, $3, $4, $5, NULL, NULL, $6, NULL, $7, $8, $9, $10, $11)
                     ON CONFLICT (route_decision_id) DO NOTHING",
                )
                .bind(e.route_decision_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(operation_kind)
                .bind(e.selected_provider_binding_id.as_ref().map(|id| id.as_str()))
                .bind(selector_ctx)
                .bind(e.attempt_count as i32)
                .bind(e.fallback_used)
                .bind(final_status)
                .bind(e.decided_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            RuntimeEvent::ProviderCallCompleted(e) => {
                let operation_kind = enum_to_str(&e.operation_kind)?;
                let status         = enum_to_str(&e.status)?;
                let error_class    = e.error_class.as_ref().map(enum_to_str).transpose()?;
                // Derive latency from timestamps if not explicit.
                let latency_ms: Option<i64> = e.latency_ms.map(|v| v as i64).or_else(|| {
                    if e.started_at > 0 && e.finished_at >= e.started_at {
                        Some((e.finished_at - e.started_at) as i64)
                    } else {
                        None
                    }
                });
                sqlx::query(
                    "INSERT INTO provider_calls
                         (provider_call_id, route_decision_id, route_attempt_id,
                          tenant_id, workspace_id, project_id,
                          operation_kind, provider_binding_id, provider_connection_id,
                          provider_adapter, provider_model_id,
                          task_id, run_id, prompt_release_id, fallback_position,
                          status, latency_ms, input_tokens, output_tokens, cost_micros,
                          error_class, raw_error_message, retry_count, created_at)
                     VALUES
                         ($1, $2, $3, $4, $5, $6, $7, $8, $9, '', $10,
                          $11, $12, $13, $14, $15, $16, $17, $18, $19,
                          $20, $21, $22, $23)
                     ON CONFLICT (provider_call_id) DO NOTHING",
                )
                .bind(e.provider_call_id.as_str())
                .bind(e.route_decision_id.as_str())
                .bind(e.route_attempt_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(operation_kind)
                .bind(e.provider_binding_id.as_str())
                .bind(e.provider_connection_id.as_str())
                .bind(e.provider_model_id.as_str())
                .bind(e.task_id.as_ref().map(|id| id.as_str()))
                .bind(e.run_id.as_ref().map(|id| id.as_str()))
                .bind(e.prompt_release_id.as_ref().map(|id| id.as_str()))
                .bind(e.fallback_position as i32)
                .bind(status)
                .bind(latency_ms)
                .bind(e.input_tokens.map(|v| v as i32))
                .bind(e.output_tokens.map(|v| v as i32))
                .bind(e.cost_micros.map(|v| v as i64))
                .bind(error_class)
                .bind(e.raw_error_message.as_deref())
                .bind(e.retry_count as i32)
                .bind(e.completed_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

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
            // RFC 005 approval policies — no durable table yet
            | RuntimeEvent::ApprovalPolicyCreated(_)
            // RFC 001 gradual rollout — state tracked via prompt_releases table
            | RuntimeEvent::PromptRolloutStarted(_) => {}
        }

        Ok(())
    }
}

fn tool_invocation_terminal_state_str(
    outcome: ToolInvocationOutcomeKind,
) -> Result<String, StoreError> {
    enum_to_str(&outcome.terminal_state())
}

/// Log an event variant the PG applier does not project.
///
/// Matches the `log_stub` pattern used by `SqliteSyncProjection`. See
/// `.claude/audit-state/review-queue.md` §T2-H3 for the coverage gap.
fn log_stub(variant: &'static str) {
    tracing::warn!(
        event_variant = variant,
        "pg projection stub: event committed to event_log but no projection table updated \
         (see PgSyncProjection unprojected-stubs list for the coverage gap)"
    );
}

/// Serialize a serde-serializable enum variant to its snake_case string form.
fn enum_to_str<T: serde::Serialize>(val: &T) -> Result<String, StoreError> {
    let v = serde_json::to_value(val).map_err(|e| StoreError::Serialization(e.to_string()))?;
    match v {
        serde_json::Value::String(s) => Ok(s),
        _ => Ok(v.to_string().trim_matches('"').to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::tool_invocation_terminal_state_str;
    use cairn_domain::tool_invocation::ToolInvocationOutcomeKind;

    #[test]
    fn canceled_tool_outcome_keeps_canceled_terminal_state() {
        let state = tool_invocation_terminal_state_str(ToolInvocationOutcomeKind::Canceled)
            .expect("state string");
        assert_eq!(state, "canceled");
    }

    #[test]
    fn failure_tool_outcome_keeps_failed_terminal_state() {
        let state = tool_invocation_terminal_state_str(ToolInvocationOutcomeKind::PermanentFailure)
            .expect("state string");
        assert_eq!(state, "failed");
    }
}
