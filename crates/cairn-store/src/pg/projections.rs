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
                // Prefer the session_id on the event; fall back to the
                // parent run's session_id for tasks that carried no binding.
                let session_id_on_event = e.session_id.as_ref().map(|s| s.as_str());
                sqlx::query(
                    "INSERT INTO tasks (task_id, tenant_id, workspace_id, project_id, parent_run_id, parent_task_id, session_id, state, title, description, version, created_at, updated_at)
                     VALUES ($1, $2, $3, $4, $5, $6,
                        COALESCE($7, (SELECT session_id FROM runs WHERE run_id = $5)),
                        'queued', NULL, NULL, 1, $8, $8)",
                )
                .bind(e.task_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.parent_run_id.as_ref().map(|id| id.as_str()))
                .bind(e.parent_task_id.as_ref().map(|id| id.as_str()))
                .bind(session_id_on_event)
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
            // F29 CD-2: upsert the per-session record + fold into the
            // project/workspace rollups in the same transaction. All three
            // upserts succeed together or the whole event append rolls
            // back, so read-model consistency is preserved.
            RuntimeEvent::SessionCostUpdated(e) => {
                // InMemory sources tenant from the explicit `e.tenant_id`
                // field, not `e.project.tenant_id`, because fixtures can
                // carry a sentinel project triple (see the budget-
                // blocking test in cairn-runtime). Mirror that choice on
                // the durable path so pg and InMemory agree on which
                // tenant a cost is attributed to.
                upsert_cost_rollups_pg(
                    tx,
                    e.session_id.as_str(),
                    e.tenant_id.as_str(),
                    &e.project,
                    e.delta_cost_micros,
                    e.delta_tokens_in,
                    e.delta_tokens_out,
                    e.updated_at_ms,
                )
                .await?;
            }
            RuntimeEvent::RunCostUpdated(_) => log_stub("RunCostUpdated"),
            RuntimeEvent::SpendAlertTriggered(_) => log_stub("SpendAlertTriggered"),
            RuntimeEvent::SubagentSpawned(_) => log_stub("SubagentSpawned"),
            // F39: durable projections for RFC 002 recovery audits.
            // Keys on envelope event_id so replay is idempotent. Nullable
            // run_id/task_id/boot_id mirror the event struct exactly; the
            // `has_target()` invariant is enforced by the emitter, not the
            // projection (an already-appended malformed event still
            // projects, matching InMemory's pass-through behavior).
            RuntimeEvent::RecoveryAttempted(e) => {
                sqlx::query(
                    "INSERT INTO recovery_attempts
                         (event_id, tenant_id, workspace_id, project_id,
                          run_id, task_id, reason, boot_id, recorded_at_ms)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT (event_id) DO NOTHING",
                )
                .bind(event.envelope.event_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.run_id.as_ref().map(|r| r.as_str()))
                .bind(e.task_id.as_ref().map(|t| t.as_str()))
                .bind(&e.reason)
                .bind(e.boot_id.as_deref())
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::RecoveryCompleted(e) => {
                sqlx::query(
                    "INSERT INTO recovery_completions
                         (event_id, tenant_id, workspace_id, project_id,
                          run_id, task_id, recovered, boot_id, recorded_at_ms)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                     ON CONFLICT (event_id) DO NOTHING",
                )
                .bind(event.envelope.event_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.run_id.as_ref().map(|r| r.as_str()))
                .bind(e.task_id.as_ref().map(|t| t.as_str()))
                .bind(e.recovered)
                .bind(e.boot_id.as_deref())
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
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
            RuntimeEvent::ProviderConnectionDeleted(_) => {
                log_stub("ProviderConnectionDeleted")
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
            // RFC 020 Track 3: audit-only events; no projection update needed.
            // ToolCallResultCache consumes these via in-memory scan / replay.
            RuntimeEvent::ToolInvocationCacheHit(_) => log_stub("ToolInvocationCacheHit"),
            RuntimeEvent::ToolRecoveryPaused(_) => log_stub("ToolRecoveryPaused"),
            // F39: RFC 020 Track 4 boot-level recovery audit projected to
            // `recovery_summaries` (one row per boot_id). The emitter
            // contract guarantees one summary per boot; ON CONFLICT DO
            // NOTHING keeps replay idempotent.
            RuntimeEvent::RecoverySummaryEmitted(e) => {
                let recorded_at = i64::try_from(e.summary_at_ms).map_err(|_| {
                    StoreError::Internal(format!(
                        "RecoverySummaryEmitted.summary_at_ms {} exceeds i64::MAX",
                        e.summary_at_ms
                    ))
                })?;
                let startup_ms = i64::try_from(e.startup_ms).map_err(|_| {
                    StoreError::Internal(format!(
                        "RecoverySummaryEmitted.startup_ms {} exceeds i64::MAX",
                        e.startup_ms
                    ))
                })?;
                sqlx::query(
                    "INSERT INTO recovery_summaries
                         (boot_id, tenant_id, workspace_id, project_id,
                          recovered_runs, recovered_tasks, recovered_sandboxes,
                          preserved_sandboxes, orphaned_sandboxes_cleaned,
                          decision_cache_entries, stale_pending_cleared,
                          tool_result_cache_entries, memory_projection_entries,
                          graph_nodes_recovered, graph_edges_recovered,
                          webhook_dedup_entries, trigger_projections,
                          startup_ms, summary_at_ms)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11,
                             $12, $13, $14, $15, $16, $17, $18, $19)
                     ON CONFLICT (boot_id) DO NOTHING",
                )
                .bind(&e.boot_id)
                .bind(e.sentinel_project.tenant_id.as_str())
                .bind(e.sentinel_project.workspace_id.as_str())
                .bind(e.sentinel_project.project_id.as_str())
                // All count fields are `u32`; `i64::from` is an
                // infallible widening conversion (no silent truncation)
                // and keeps the checked-cast discipline symmetric with
                // the `u64 -> i64` `try_from` calls above.
                .bind(i64::from(e.recovered_runs))
                .bind(i64::from(e.recovered_tasks))
                .bind(i64::from(e.recovered_sandboxes))
                .bind(i64::from(e.preserved_sandboxes))
                .bind(i64::from(e.orphaned_sandboxes_cleaned))
                .bind(i64::from(e.decision_cache_entries))
                .bind(i64::from(e.stale_pending_cleared))
                .bind(i64::from(e.tool_result_cache_entries))
                .bind(i64::from(e.memory_projection_entries))
                .bind(i64::from(e.graph_nodes_recovered))
                .bind(i64::from(e.graph_edges_recovered))
                .bind(i64::from(e.webhook_dedup_entries))
                .bind(i64::from(e.trigger_projections))
                .bind(startup_ms)
                .bind(recorded_at)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

            // PR BP-2: project tool-call approval events into the
            // `tool_call_approvals` table. ToolCallProposed inserts a
            // new pending row; Amended/Approved/Rejected update it.
            RuntimeEvent::ToolCallProposed(e) => {
                let match_policy_json = serde_json::to_value(&e.match_policy)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                let display_summary_opt: Option<&str> = if e.display_summary.is_empty() {
                    None
                } else {
                    Some(e.display_summary.as_str())
                };
                sqlx::query(
                    "INSERT INTO tool_call_approvals (
                         call_id, session_id, run_id, tenant_id, workspace_id, project_id,
                         tool_name, original_tool_args, amended_tool_args, approved_tool_args,
                         display_summary, match_policy, state, operator_id, scope, reason,
                         proposed_at_ms, approved_at_ms, rejected_at_ms, last_amended_at_ms,
                         version, created_at, updated_at
                     )
                     VALUES (
                         $1, $2, $3, $4, $5, $6,
                         $7, $8, NULL, NULL,
                         $9, $10, 'pending', NULL, NULL, NULL,
                         $11, NULL, NULL, NULL,
                         1, $12, $12
                     )
                     ON CONFLICT (call_id) DO NOTHING",
                )
                .bind(e.call_id.as_str())
                .bind(e.session_id.as_str())
                .bind(e.run_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(&e.tool_name)
                .bind(&e.tool_args)
                .bind(display_summary_opt)
                .bind(&match_policy_json)
                .bind(e.proposed_at_ms as i64)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::ToolCallAmended(e) => {
                sqlx::query(
                    "UPDATE tool_call_approvals
                     SET amended_tool_args = $1,
                         last_amended_at_ms = $2,
                         version = version + 1,
                         updated_at = $3
                     WHERE call_id = $4",
                )
                .bind(&e.new_tool_args)
                .bind(e.amended_at_ms as i64)
                .bind(now)
                .bind(e.call_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::ToolCallApproved(e) => {
                let scope_json = serde_json::to_value(&e.scope)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                sqlx::query(
                    "UPDATE tool_call_approvals
                     SET state = 'approved',
                         operator_id = $1,
                         scope = $2,
                         approved_tool_args = $3,
                         approved_at_ms = $4,
                         version = version + 1,
                         updated_at = $5
                     WHERE call_id = $6",
                )
                .bind(e.operator_id.as_str())
                .bind(&scope_json)
                .bind(e.approved_tool_args.as_ref())
                .bind(e.approved_at_ms as i64)
                .bind(now)
                .bind(e.call_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::ToolCallRejected(e) => {
                sqlx::query(
                    "UPDATE tool_call_approvals
                     SET state = 'rejected',
                         operator_id = $1,
                         reason = $2,
                         rejected_at_ms = $3,
                         version = version + 1,
                         updated_at = $4
                     WHERE call_id = $5",
                )
                .bind(e.operator_id.as_str())
                .bind(e.reason.as_deref())
                .bind(e.rejected_at_ms as i64)
                .bind(now)
                .bind(e.call_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }

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

            RuntimeEvent::WorkspaceArchived(e) => {
                // `tenant_id` is included in the WHERE clause as
                // defense-in-depth: the service layer already refuses
                // cross-tenant archives before emitting, but if a replay
                // ever presents a mismatched event we'd rather no-op than
                // archive another tenant's row.
                sqlx::query(
                    "UPDATE workspaces
                        SET archived_at = $1, updated_at = $1
                      WHERE workspace_id = $2 AND tenant_id = $3",
                )
                .bind(e.archived_at as i64)
                .bind(e.workspace_id.as_str())
                .bind(e.tenant_id.as_str())
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
                     VALUES ($1, $2, $3, $4, $5, $6, $6)
                     ON CONFLICT (policy_id) DO UPDATE
                     SET name = EXCLUDED.name,
                         rules = EXCLUDED.rules,
                         enabled = EXCLUDED.enabled,
                         updated_at = EXCLUDED.updated_at",
                )
                .bind(&e.policy_id)
                .bind(e.tenant_id.as_str())
                .bind(&e.name)
                .bind(rules)
                .bind(e.enabled)
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

                // F29 CD-2: fold the call into session/project/workspace
                // cost rollups. This is the production path — the
                // InMemoryStore derives `SessionCostUpdated` from the
                // same event for the in-process read model, but that
                // derived event never reaches the durable log. Projecting
                // directly here keeps the pg tables in sync with InMemory.
                //
                // Calls with a resolvable session_id contribute to
                // session/project/workspace cost. We mirror the in-
                // memory `apply_projection` logic: prefer the event's
                // own session_id, fall back to the run's session_id,
                // skip rollup when neither is available. No additional
                // success/status filter is applied here — a zero-cost
                // failed call still bumps `provider_calls` so the
                // operator panel reflects actual attempt counts.
                let effective_session_id: Option<String> = if let Some(sid) = &e.session_id {
                    Some(sid.as_str().to_owned())
                } else if let Some(rid) = &e.run_id {
                    sqlx::query_scalar::<_, Option<String>>(
                        "SELECT session_id FROM runs WHERE run_id = $1",
                    )
                    .bind(rid.as_str())
                    .fetch_optional(&mut **tx)
                    .await
                    .map_err(|err| StoreError::Internal(err.to_string()))?
                    .flatten()
                } else {
                    None
                };
                if let Some(sid) = effective_session_id {
                    // ProviderCallCompleted has no top-level `tenant_id`
                    // — the tenant is only carried by `e.project`, so
                    // that's our source of truth here.
                    upsert_cost_rollups_pg(
                        tx,
                        &sid,
                        e.project.tenant_id.as_str(),
                        &e.project,
                        e.cost_micros.unwrap_or(0),
                        e.input_tokens.unwrap_or(0) as u64,
                        e.output_tokens.unwrap_or(0) as u64,
                        e.completed_at,
                    )
                    .await?;
                }
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
            // F39: RFC 019 / RFC 020 decision-cache projection. The
            // in-memory cache is still rebuilt from the event log at boot
            // (see cairn-app warmup); these tables give operator tooling
            // a queryable read model without walking the full stream.
            // `decision_key` + `outcome` are serialized as JSON text for
            // pg/sqlite portability (no JSONB operators).
            RuntimeEvent::DecisionRecorded(e) => {
                let decision_key_json = serde_json::to_string(&e.decision_key)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                let outcome_kind = decision_outcome_kind(&e.outcome);
                let expires_at = i64::try_from(e.expires_at).map_err(|_| {
                    StoreError::Internal(format!(
                        "DecisionRecorded.expires_at {} exceeds i64::MAX",
                        e.expires_at
                    ))
                })?;
                let decided_at = i64::try_from(e.decided_at).map_err(|_| {
                    StoreError::Internal(format!(
                        "DecisionRecorded.decided_at {} exceeds i64::MAX",
                        e.decided_at
                    ))
                })?;
                sqlx::query(
                    "INSERT INTO decision_records
                         (decision_id, tenant_id, workspace_id, project_id,
                          decision_key_json, outcome_kind, cached,
                          expires_at, decided_at, event_json, recorded_at_ms)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
                     ON CONFLICT (decision_id) DO NOTHING",
                )
                .bind(e.decision_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(decision_key_json)
                .bind(outcome_kind)
                .bind(e.cached)
                .bind(expires_at)
                .bind(decided_at)
                .bind(&e.event_json)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            // F47 PR2: persist the completion summary + verification
            // sidecar onto the existing runs row. UPDATE-only — the
            // event presumes the run row already exists (the normal
            // completion path runs RunCreated → RunStateChanged →
            // complete() → RunCompletionAnnotated). Silent no-op on
            // missing row mirrors the RunStateChanged handler above:
            // replay of an annotation with no corresponding RunCreated
            // in the same log is a malformed log, not something the
            // projection should hard-fail on. `completion_verification_json`
            // stores serde-JSON as TEXT rather than JSONB so the
            // portable cross-backend contract (per the no-DB-specific-
            // features memory) holds; the value is written and read
            // wholesale, never queried with JSONB operators.
            RuntimeEvent::RunCompletionAnnotated(e) => {
                let verification_json = serde_json::to_string(&e.verification)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                let annotated_at = i64::try_from(e.occurred_at_ms).map_err(|_| {
                    StoreError::Internal(format!(
                        "RunCompletionAnnotated.occurred_at_ms {} exceeds i64::MAX",
                        e.occurred_at_ms
                    ))
                })?;
                sqlx::query(
                    "UPDATE runs
                        SET completion_summary              = $1,
                            completion_verification_json    = $2,
                            completion_annotated_at_ms      = $3,
                            version                         = version + 1,
                            updated_at                      = $4
                      WHERE run_id = $5",
                )
                .bind(&e.summary)
                .bind(verification_json)
                .bind(annotated_at)
                .bind(now)
                .bind(e.run_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::DecisionCacheWarmup(e) => {
                let warmed_at = i64::try_from(e.warmed_at).map_err(|_| {
                    StoreError::Internal(format!(
                        "DecisionCacheWarmup.warmed_at {} exceeds i64::MAX",
                        e.warmed_at
                    ))
                })?;
                sqlx::query(
                    "INSERT INTO decision_cache_warmups
                         (warmed_at, cached, expired_and_dropped)
                     VALUES ($1, $2, $3)
                     ON CONFLICT (warmed_at) DO NOTHING",
                )
                .bind(warmed_at)
                .bind(i64::from(e.cached))
                .bind(i64::from(e.expired_and_dropped))
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
        }

        Ok(())
    }
}

fn tool_invocation_terminal_state_str(
    outcome: ToolInvocationOutcomeKind,
) -> Result<String, StoreError> {
    enum_to_str(&outcome.terminal_state())
}

/// F29 CD-2: fold a (session_id, project, delta) tuple into
/// `session_costs`, `project_costs`, and `workspace_costs`. Called from
/// both the `SessionCostUpdated` projection and the
/// `ProviderCallCompleted` projection so the rollup tables stay in sync
/// with InMemory regardless of which event shape the runtime uses.
///
/// `updated_at_ms` uses `GREATEST(existing, incoming)` so an out-of-
/// order replay can never move the column backwards. Overflow on any
/// `u64 -> i64` cast is a hard error, not silent wrap — wall-clock ms
/// stays well under `i64::MAX` (year 292 million) and cost/token totals
/// reaching 9.2 exabillion micros is a serious anomaly worth halting on.
#[allow(clippy::too_many_arguments)]
async fn upsert_cost_rollups_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    session_id: &str,
    tenant_id: &str,
    project: &cairn_domain::ProjectKey,
    delta_cost_micros: u64,
    delta_tokens_in: u64,
    delta_tokens_out: u64,
    updated_at_ms: u64,
) -> Result<(), StoreError> {
    fn to_i64(field: &str, v: u64) -> Result<i64, StoreError> {
        i64::try_from(v).map_err(|_| {
            StoreError::Internal(format!("session cost {field} value {v} exceeds i64::MAX"))
        })
    }
    let delta_cost = to_i64("delta_cost_micros", delta_cost_micros)?;
    let delta_in = to_i64("delta_tokens_in", delta_tokens_in)?;
    let delta_out = to_i64("delta_tokens_out", delta_tokens_out)?;
    let updated_at = to_i64("updated_at_ms", updated_at_ms)?;

    sqlx::query(
        "INSERT INTO session_costs
             (session_id, tenant_id, workspace_id, project_id,
              total_cost_micros, total_tokens_in, total_tokens_out,
              provider_calls, updated_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, $8)
         ON CONFLICT (session_id) DO UPDATE SET
             total_cost_micros = session_costs.total_cost_micros + EXCLUDED.total_cost_micros,
             total_tokens_in   = session_costs.total_tokens_in   + EXCLUDED.total_tokens_in,
             total_tokens_out  = session_costs.total_tokens_out  + EXCLUDED.total_tokens_out,
             provider_calls    = session_costs.provider_calls    + 1,
             updated_at_ms     = GREATEST(session_costs.updated_at_ms, EXCLUDED.updated_at_ms)",
    )
    .bind(session_id)
    .bind(tenant_id)
    .bind(project.workspace_id.as_str())
    .bind(project.project_id.as_str())
    .bind(delta_cost)
    .bind(delta_in)
    .bind(delta_out)
    .bind(updated_at)
    .execute(&mut **tx)
    .await
    .map_err(|err| StoreError::Internal(err.to_string()))?;

    sqlx::query(
        "INSERT INTO project_costs
             (tenant_id, workspace_id, project_id,
              total_cost_micros, total_tokens_in, total_tokens_out,
              provider_calls, updated_at_ms)
         VALUES ($1, $2, $3, $4, $5, $6, 1, $7)
         ON CONFLICT (tenant_id, workspace_id, project_id) DO UPDATE SET
             total_cost_micros = project_costs.total_cost_micros + EXCLUDED.total_cost_micros,
             total_tokens_in   = project_costs.total_tokens_in   + EXCLUDED.total_tokens_in,
             total_tokens_out  = project_costs.total_tokens_out  + EXCLUDED.total_tokens_out,
             provider_calls    = project_costs.provider_calls    + 1,
             updated_at_ms     = GREATEST(project_costs.updated_at_ms, EXCLUDED.updated_at_ms)",
    )
    .bind(tenant_id)
    .bind(project.workspace_id.as_str())
    .bind(project.project_id.as_str())
    .bind(delta_cost)
    .bind(delta_in)
    .bind(delta_out)
    .bind(updated_at)
    .execute(&mut **tx)
    .await
    .map_err(|err| StoreError::Internal(err.to_string()))?;

    sqlx::query(
        "INSERT INTO workspace_costs
             (tenant_id, workspace_id,
              total_cost_micros, total_tokens_in, total_tokens_out,
              provider_calls, updated_at_ms)
         VALUES ($1, $2, $3, $4, $5, 1, $6)
         ON CONFLICT (tenant_id, workspace_id) DO UPDATE SET
             total_cost_micros = workspace_costs.total_cost_micros + EXCLUDED.total_cost_micros,
             total_tokens_in   = workspace_costs.total_tokens_in   + EXCLUDED.total_tokens_in,
             total_tokens_out  = workspace_costs.total_tokens_out  + EXCLUDED.total_tokens_out,
             provider_calls    = workspace_costs.provider_calls    + 1,
             updated_at_ms     = GREATEST(workspace_costs.updated_at_ms, EXCLUDED.updated_at_ms)",
    )
    .bind(tenant_id)
    .bind(project.workspace_id.as_str())
    .bind(delta_cost)
    .bind(delta_in)
    .bind(delta_out)
    .bind(updated_at)
    .execute(&mut **tx)
    .await
    .map_err(|err| StoreError::Internal(err.to_string()))?;

    Ok(())
}

/// F39: short-form outcome discriminant for `decision_records.outcome_kind`.
/// Stored as a stable string so operator queries can filter without
/// parsing the full `decision_key_json` blob.
fn decision_outcome_kind(outcome: &cairn_domain::decisions::DecisionOutcome) -> &'static str {
    match outcome {
        cairn_domain::decisions::DecisionOutcome::Allowed => "allowed",
        cairn_domain::decisions::DecisionOutcome::Denied { .. } => "denied",
    }
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
