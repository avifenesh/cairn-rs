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
            RuntimeEvent::PromptAssetCreated(e) => {
                sqlx::query(
                    "INSERT INTO prompt_assets
                         (prompt_asset_id, tenant_id, workspace_id, project_id, name, kind,
                          scope, status, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, NULL, 'draft', ?, ?)
                     ON CONFLICT(prompt_asset_id) DO NOTHING",
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
                // SQLite has no `SELECT ... FOR UPDATE`. sqlx opens
                // transactions as DEFERRED by default, so concurrent
                // appenders to the same asset could in principle both
                // compute MAX+1 and insert the same version_number. In
                // local-mode there is a single append path serialized
                // by `SqliteEventLog`, which makes this safe in
                // practice. The defensive `UNIQUE(prompt_asset_id,
                // version_number)` constraint in schema.rs (and
                // Postgres V023) converts any future concurrent
                // allocation bug into a hard error instead of silent
                // duplicate rows.
                let version_number: i64 = sqlx::query_scalar(
                    "SELECT COALESCE(MAX(version_number), 0) + 1
                     FROM prompt_versions
                     WHERE prompt_asset_id = ?",
                )
                .bind(e.prompt_asset_id.as_str())
                .fetch_one(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;

                sqlx::query(
                    "INSERT INTO prompt_versions
                         (prompt_version_id, prompt_asset_id, tenant_id, workspace_id, project_id,
                          version_number, content_hash, content, format, created_by, created_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, NULL, NULL, NULL, ?)
                     ON CONFLICT(prompt_version_id) DO NOTHING",
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
            RuntimeEvent::ApprovalPolicyCreated(_) => log_stub("ApprovalPolicyCreated"),
            RuntimeEvent::PromptReleaseCreated(e) => {
                sqlx::query(
                    "INSERT INTO prompt_releases
                         (prompt_release_id, prompt_asset_id, prompt_version_id,
                          tenant_id, workspace_id, project_id,
                          release_tag, state, rollout_target, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?, 'draft', NULL, ?, ?)
                     ON CONFLICT(prompt_release_id) DO NOTHING",
                )
                .bind(e.prompt_release_id.as_str())
                .bind(e.prompt_asset_id.as_str())
                .bind(e.prompt_version_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(e.release_tag.as_deref())
                .bind(e.created_at as i64)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::PromptReleaseTransitioned(e) => {
                sqlx::query(
                    "UPDATE prompt_releases
                     SET state = ?, updated_at = ?
                     WHERE prompt_release_id = ?",
                )
                .bind(&e.to_state)
                .bind(now)
                .bind(e.prompt_release_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            // RFC 001 gradual rollout — state tracked via prompt_releases;
            // no dedicated projection table.
            RuntimeEvent::PromptRolloutStarted(_) => {}
            RuntimeEvent::TenantCreated(e) => {
                sqlx::query(
                    "INSERT INTO tenants (tenant_id, name, created_at, updated_at)
                     VALUES (?, ?, ?, ?)
                     ON CONFLICT(tenant_id) DO NOTHING",
                )
                .bind(e.tenant_id.as_str())
                .bind(&e.name)
                .bind(e.created_at as i64)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::WorkspaceCreated(e) => {
                sqlx::query(
                    "INSERT INTO workspaces (workspace_id, tenant_id, name, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?)
                     ON CONFLICT(workspace_id) DO NOTHING",
                )
                .bind(e.workspace_id.as_str())
                .bind(e.tenant_id.as_str())
                .bind(&e.name)
                .bind(e.created_at as i64)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::WorkspaceArchived(e) => {
                // `tenant_id` in the WHERE clause is defense-in-depth —
                // the service layer already rejects cross-tenant archives
                // before emitting, but a replay or injected event with a
                // mismatched tenant_id should no-op rather than touch
                // another tenant's row.
                sqlx::query(
                    "UPDATE workspaces
                        SET archived_at = ?, updated_at = ?
                      WHERE workspace_id = ? AND tenant_id = ?",
                )
                .bind(e.archived_at as i64)
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
                     VALUES (?, ?, ?, ?, ?, ?)
                     ON CONFLICT(project_id) DO NOTHING",
                )
                .bind(e.project.project_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(&e.name)
                .bind(e.created_at as i64)
                .bind(e.created_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::RouteDecisionMade(e) => {
                let operation_kind = enum_to_str(&e.operation_kind)?;
                let final_status = enum_to_str(&e.final_status)?;
                // selector_context is not carried by the event (mirrors PG).
                let selector_ctx: Option<String> = None;
                sqlx::query(
                    "INSERT INTO route_decisions
                         (route_decision_id, tenant_id, workspace_id, project_id,
                          operation_kind, route_policy_id, terminal_route_attempt_id,
                          selected_provider_binding_id, selected_route_attempt_id,
                          selector_context, attempt_count, fallback_used, final_status,
                          created_at)
                     VALUES (?, ?, ?, ?, ?, NULL, NULL, ?, NULL, ?, ?, ?, ?, ?)
                     ON CONFLICT(route_decision_id) DO NOTHING",
                )
                .bind(e.route_decision_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(operation_kind)
                .bind(
                    e.selected_provider_binding_id
                        .as_ref()
                        .map(|id| id.as_str()),
                )
                .bind(selector_ctx)
                .bind(e.attempt_count as i64)
                .bind(i64::from(e.fallback_used))
                .bind(final_status)
                .bind(e.decided_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::ProviderCallCompleted(e) => {
                let operation_kind = enum_to_str(&e.operation_kind)?;
                let status = enum_to_str(&e.status)?;
                let error_class = e.error_class.as_ref().map(enum_to_str).transpose()?;
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
                         (?, ?, ?, ?, ?, ?, ?, ?, ?, '', ?,
                          ?, ?, ?, ?, ?, ?, ?, ?, ?,
                          ?, ?, ?, ?)
                     ON CONFLICT(provider_call_id) DO NOTHING",
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
                .bind(e.fallback_position as i64)
                .bind(status)
                .bind(latency_ms)
                .bind(e.input_tokens.map(|v| v as i64))
                .bind(e.output_tokens.map(|v| v as i64))
                .bind(e.cost_micros.map(|v| v as i64))
                .bind(error_class)
                .bind(e.raw_error_message.as_deref())
                .bind(e.retry_count as i64)
                .bind(e.completed_at as i64)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
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
            RuntimeEvent::WorkspaceMemberAdded(e) => {
                let role = enum_to_str(&e.role)?;
                sqlx::query(
                    "INSERT INTO workspace_members (workspace_id, operator_id, role, added_at_ms)
                     VALUES (?, ?, ?, ?)
                     ON CONFLICT(workspace_id, operator_id) DO UPDATE SET role = excluded.role",
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
                    "DELETE FROM workspace_members WHERE workspace_id = ? AND operator_id = ?",
                )
                .bind(e.workspace_key.workspace_id.as_str())
                .bind(e.member_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
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
            RuntimeEvent::RoutePolicyCreated(e) => {
                // `rules` is a JSON string (SQLite has no JSONB); serialised
                // on write and parsed wholesale on read by the service layer.
                // `enabled` is projected from the event's `enabled` field —
                // bool maps to SQLite INTEGER (0/1) via sqlx. Symmetric with
                // PG where the column is BOOLEAN.
                let rules = serde_json::to_string(&e.rules)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                sqlx::query(
                    "INSERT INTO route_policies (policy_id, tenant_id, name, rules, enabled, created_at, updated_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?)
                     ON CONFLICT(policy_id) DO UPDATE
                     SET name = excluded.name,
                         rules = excluded.rules,
                         enabled = excluded.enabled,
                         updated_at = excluded.updated_at",
                )
                .bind(&e.policy_id)
                .bind(e.tenant_id.as_str())
                .bind(&e.name)
                .bind(rules)
                .bind(e.enabled)
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            // PG's projection does not consume RoutePolicyUpdated — the event
            // is kept for audit, and the rules set is advanced by the next
            // RoutePolicyCreated upsert. SQLite mirrors that shape.
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
            // RFC 020 decision-cache survival (PR #85): no projection
            // table; cairn-app rebuilds the in-memory cache from the
            // event log on startup.
            RuntimeEvent::DecisionRecorded(_) => log_stub("DecisionRecorded"),
            RuntimeEvent::DecisionCacheWarmup(_) => log_stub("DecisionCacheWarmup"),
            // RFC 020 Track 4: boot-level recovery audit event.
            RuntimeEvent::RecoverySummaryEmitted(_) => log_stub("RecoverySummaryEmitted"),

            // PR BP-1: tool-call approval foundation events — no
            // projection table yet; a later PR in the wave wires these
            // into the approvals / tool-call projections.
            // PR BP-2: project tool-call approval events into the
            // `tool_call_approvals` table. JSON fields are stored as
            // TEXT because SQLite has no native JSONB.
            RuntimeEvent::ToolCallProposed(e) => {
                let tool_args_text = serde_json::to_string(&e.tool_args)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                let match_policy_text = serde_json::to_string(&e.match_policy)
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
                         ?, ?, ?, ?, ?, ?,
                         ?, ?, NULL, NULL,
                         ?, ?, 'pending', NULL, NULL, NULL,
                         ?, NULL, NULL, NULL,
                         1, ?, ?
                     )
                     ON CONFLICT(call_id) DO NOTHING",
                )
                .bind(e.call_id.as_str())
                .bind(e.session_id.as_str())
                .bind(e.run_id.as_str())
                .bind(e.project.tenant_id.as_str())
                .bind(e.project.workspace_id.as_str())
                .bind(e.project.project_id.as_str())
                .bind(&e.tool_name)
                .bind(tool_args_text)
                .bind(display_summary_opt)
                .bind(match_policy_text)
                .bind(e.proposed_at_ms as i64)
                .bind(now)
                .bind(now)
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::ToolCallAmended(e) => {
                let new_args_text = serde_json::to_string(&e.new_tool_args)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                sqlx::query(
                    "UPDATE tool_call_approvals
                     SET amended_tool_args = ?,
                         last_amended_at_ms = ?,
                         version = version + 1,
                         updated_at = ?
                     WHERE call_id = ?",
                )
                .bind(new_args_text)
                .bind(e.amended_at_ms as i64)
                .bind(now)
                .bind(e.call_id.as_str())
                .execute(&mut **tx)
                .await
                .map_err(|err| StoreError::Internal(err.to_string()))?;
            }
            RuntimeEvent::ToolCallApproved(e) => {
                let scope_text = serde_json::to_string(&e.scope)
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                let approved_args_text: Option<String> = e
                    .approved_tool_args
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|err| StoreError::Serialization(err.to_string()))?;
                sqlx::query(
                    "UPDATE tool_call_approvals
                     SET state = 'approved',
                         operator_id = ?,
                         scope = ?,
                         approved_tool_args = ?,
                         approved_at_ms = ?,
                         version = version + 1,
                         updated_at = ?
                     WHERE call_id = ?",
                )
                .bind(e.operator_id.as_str())
                .bind(scope_text)
                .bind(approved_args_text)
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
                         operator_id = ?,
                         reason = ?,
                         rejected_at_ms = ?,
                         version = version + 1,
                         updated_at = ?
                     WHERE call_id = ?",
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
