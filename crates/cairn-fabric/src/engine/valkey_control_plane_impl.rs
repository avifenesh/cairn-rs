//! [`ControlPlaneBackend`] implementation for [`ValkeyEngine`].
//!
//! Holds the FF key-builder / partition / FCALL logic lifted out of
//! `FabricBudgetService`, `FabricQuotaService`, and
//! `FabricRotationService`. Those services now delegate to this impl
//! through the [`ControlPlaneBackend`] trait; the FF imports
//! (`flowfabric::core::keys`, `flowfabric::core::partition`, `flowfabric::core::contracts`) live
//! here only.
//!
//! When FF 0.3 ships `describe_*` + control-plane primitives upstream
//! (FlowFabric#58), this file collapses to a thin passthrough — the
//! service layer stays untouched because it already talks to the
//! trait, not to FF.

use std::collections::HashMap;

use async_trait::async_trait;
use flowfabric::core::contracts::ReportUsageResult;
use flowfabric::core::keys::{
    budget_policies_index, budget_resets_key, quota_policies_index, usage_dedup_key,
    BudgetKeyContext, ExecKeyContext, FlowIndexKeys, FlowKeyContext, IndexKeys, QuotaKeyContext,
};
use flowfabric::core::partition::{
    budget_partition, execution_partition, flow_partition, quota_partition, Partition,
    PartitionFamily,
};
use flowfabric::core::types::{
    AttemptId, AttemptIndex, BudgetId, ExecutionId, LeaseId, QuotaPolicyId, SignalId, SuspensionId,
    TimestampMs, WaitpointId,
};
use flowfabric::sdk::task::parse_report_usage_result;

use crate::error::FabricError;
use crate::fcall;
use crate::helpers::{
    check_fcall_success, fcall_error_code, is_duplicate_result, parse_eligibility_result,
    parse_fail_outcome, parse_stage_result_revision, FailOutcome,
};

use super::control_plane::ControlPlaneBackend;
use super::control_plane_types::{
    AddExecutionToFlowInput, ApplyDependencyToChildInput, BudgetSpendOutcome, BudgetStatusSnapshot,
    CancelFlowInput, CancelRunInput, ClaimGrantOutcome, CompleteRunInput, CreateFlowInput,
    CreateRunExecutionInput, DeliverApprovalSignalInput, EligibilityResult, ExecutionCreated,
    FailExecutionOutcome, FailRunInput, FlowCancelOutcome, IssueGrantAndClaimInput, QuotaAdmission,
    RenewLeaseInput, ResumeRunInput, RotationFailure, RotationOutcome, StageDependencyEdgeInput,
    StageDependencyOutcome, SubmitTaskInput, SuspendRunInput,
};
use super::valkey_impl::ValkeyEngine;

/// FF Lua code returned by `ff_cancel_flow` when the flow is already in
/// a terminal `public_flow_state` (completed / cancelled). Cairn's
/// session-archive path treats this as success — the operator-visible
/// `cairn.archived` tag still needs to land.
const FLOW_ALREADY_TERMINAL: &str = "flow_already_terminal";

const ROTATION_DETAIL_LUA_REJECTED: &str = "lua_rejected";
const ROTATION_DETAIL_TRANSPORT_ERROR: &str = "transport_error";
const ROTATION_DETAIL_UNPARSEABLE_ENVELOPE: &str = "unparseable_envelope";

#[async_trait]
impl ControlPlaneBackend for ValkeyEngine {
    // ── Budget ──────────────────────────────────────────────────────────

    async fn create_budget(
        &self,
        scope_type: &str,
        scope_id: &str,
        dimensions: &[&str],
        hard_limits: &[u64],
        soft_limits: &[u64],
        reset_interval_ms: u64,
        enforcement_mode: &str,
    ) -> Result<BudgetId, FabricError> {
        if dimensions.len() != hard_limits.len() || dimensions.len() != soft_limits.len() {
            return Err(FabricError::Validation {
                reason: "dimensions, hard_limits, soft_limits must have equal length".to_owned(),
            });
        }

        let budget_id = BudgetId::new();
        let partition = budget_partition(&budget_id, &self.runtime().partition_config);
        let ctx = BudgetKeyContext::new(&partition, &budget_id);
        let resets_zset = budget_resets_key(&partition.hash_tag());
        let policies_index = budget_policies_index(&partition.hash_tag());
        let now = TimestampMs::now();

        let (keys, argv) = fcall::budget::build_create_budget(
            &ctx,
            &resets_zset,
            &policies_index,
            &budget_id,
            scope_type,
            scope_id,
            enforcement_mode,
            "block",
            "log",
            reset_interval_ms,
            now,
            dimensions,
            hard_limits,
            soft_limits,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_CREATE_BUDGET, &key_refs, &argv_refs)
            .await?;
        check_fcall_success(&raw, fcall::names::FF_CREATE_BUDGET)?;

        Ok(budget_id)
    }

    async fn record_spend(
        &self,
        budget_id: &BudgetId,
        execution_id: &ExecutionId,
        dimension_deltas: &[(&str, u64)],
        idempotency_key: &str,
    ) -> Result<BudgetSpendOutcome, FabricError> {
        if dimension_deltas.is_empty() {
            return Err(FabricError::Validation {
                reason: "record_spend: at least one dimension_delta is required".to_owned(),
            });
        }
        let _ = execution_id; // part of the idempotency_key caller-side; kept for
                              // future backends that want to log per-execution.

        let partition = budget_partition(budget_id, &self.runtime().partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);
        let now = TimestampMs::now();

        // Prefix with the budget's `{b:N}` hash tag so FF's SET lands
        // on the same slot as the budget itself.
        let dedup_key = usage_dedup_key(ctx.hash_tag(), idempotency_key);

        let (keys, argv) =
            fcall::budget::build_report_usage(&ctx, dimension_deltas, now, &dedup_key);
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_REPORT_USAGE_AND_CHECK,
                &key_refs,
                &argv_refs,
            )
            .await?;

        let ff_outcome: ReportUsageResult =
            parse_report_usage_result(&raw).map_err(|e| FabricError::Internal(e.to_string()))?;
        Ok(map_report_usage_result(ff_outcome))
    }

    async fn release_budget(&self, budget_id: &BudgetId) -> Result<(), FabricError> {
        let partition = budget_partition(budget_id, &self.runtime().partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);
        let resets_zset = budget_resets_key(&partition.hash_tag());
        let now = TimestampMs::now();

        let keys: Vec<String> = vec![ctx.definition(), ctx.usage(), resets_zset];
        let argv: Vec<String> = vec![budget_id.to_string(), now.to_string()];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_RESET_BUDGET, &key_refs, &argv_refs)
            .await?;
        check_fcall_success(&raw, fcall::names::FF_RESET_BUDGET)?;

        Ok(())
    }

    async fn get_budget_status(
        &self,
        budget_id: &BudgetId,
    ) -> Result<Option<BudgetStatusSnapshot>, FabricError> {
        let partition = budget_partition(budget_id, &self.runtime().partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);

        let def_fields: HashMap<String, String> = self
            .runtime()
            .client
            .hgetall(&ctx.definition())
            .await
            .map_err(|e| FabricError::Internal(format!("HGETALL budget def: {e}")))?;

        if def_fields.is_empty() {
            return Ok(None);
        }

        let usage_fields: HashMap<String, String> = self
            .runtime()
            .client
            .hgetall(&ctx.usage())
            .await
            .unwrap_or_default();

        let limits_fields: HashMap<String, String> = self
            .runtime()
            .client
            .hgetall(&ctx.limits())
            .await
            .unwrap_or_default();

        let mut usage = HashMap::new();
        let mut hard_limits = HashMap::new();
        let mut soft_limits = HashMap::new();

        for (k, v) in &usage_fields {
            if let Ok(val) = v.parse::<u64>() {
                usage.insert(k.clone(), val);
            }
        }

        // FF writes budget_limits fields as "hard:<dim>" / "soft:<dim>"
        // (prefix). See flowfabric.lua budget.lua:61.
        for (k, v) in &limits_fields {
            if let Some(dim) = k.strip_prefix("hard:") {
                if let Ok(val) = v.parse::<u64>() {
                    hard_limits.insert(dim.to_owned(), val);
                }
            } else if let Some(dim) = k.strip_prefix("soft:") {
                if let Ok(val) = v.parse::<u64>() {
                    soft_limits.insert(dim.to_owned(), val);
                }
            }
        }

        let breach_count = def_fields
            .get("breach_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let soft_breach_count = def_fields
            .get("soft_breach_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        Ok(Some(BudgetStatusSnapshot {
            budget_id: budget_id.to_string(),
            scope_type: def_fields.get("scope_type").cloned().unwrap_or_default(),
            scope_id: def_fields.get("scope_id").cloned().unwrap_or_default(),
            enforcement_mode: def_fields
                .get("enforcement_mode")
                .cloned()
                .unwrap_or_default(),
            usage,
            hard_limits,
            soft_limits,
            breach_count,
            soft_breach_count,
        }))
    }

    // ── Quota ───────────────────────────────────────────────────────────

    async fn create_quota_policy(
        &self,
        scope_type: &str,
        scope_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        let qid = QuotaPolicyId::new();
        let partition = quota_partition(&qid, &self.runtime().partition_config);
        let ctx = QuotaKeyContext::new(&partition, &qid);
        let now = TimestampMs::now();

        let dimension = "default";
        let policies_index = quota_policies_index(&partition.hash_tag());

        let (keys, args) = fcall::quota::build_create_quota_policy(
            &ctx,
            &policies_index,
            &qid,
            window_seconds,
            max_requests_per_window,
            max_concurrent,
            now,
            dimension,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_CREATE_QUOTA_POLICY, &key_refs, &arg_refs)
            .await?;
        check_fcall_success(&raw, fcall::names::FF_CREATE_QUOTA_POLICY)?;

        // Stamp scope metadata on the definition hash so admin reads
        // can surface it without going back to the caller.
        let def_key = ctx.definition();
        self.runtime()
            .client
            .hset(&def_key, "scope_type", scope_type)
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET scope_type: {e}")))?;
        self.runtime()
            .client
            .hset(&def_key, "scope_id", scope_id)
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET scope_id: {e}")))?;

        Ok(qid)
    }

    async fn check_admission(
        &self,
        quota_policy_id: &QuotaPolicyId,
        execution_id: &ExecutionId,
        window_seconds: u64,
        rate_limit: u64,
        concurrency_cap: u64,
    ) -> Result<QuotaAdmission, FabricError> {
        let partition = quota_partition(quota_policy_id, &self.runtime().partition_config);
        let ctx = QuotaKeyContext::new(&partition, quota_policy_id);
        let now = TimestampMs::now();
        let dimension = "default";

        let (keys, args) = fcall::quota::build_check_admission(
            &ctx,
            execution_id,
            now,
            window_seconds,
            rate_limit,
            concurrency_cap,
            dimension,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_CHECK_ADMISSION_AND_RECORD,
                &key_refs,
                &arg_refs,
            )
            .await?;

        parse_admission_result(&raw)
    }

    // ── Rotation ────────────────────────────────────────────────────────

    async fn rotate_waitpoint_hmac(
        &self,
        new_kid: &str,
        new_secret_hex: &str,
        grace_ms: u64,
    ) -> RotationOutcome {
        let num_partitions = self.runtime().partition_config.num_flow_partitions;
        let mut rotated = 0u16;
        let mut noop = 0u16;
        let mut failed: Vec<RotationFailure> = Vec::new();

        for index in 0..num_partitions {
            let partition = Partition {
                family: PartitionFamily::Execution,
                index,
            };
            let idx = IndexKeys::new(&partition);
            let (keys, args) = fcall::rotation::build_rotate_waitpoint_hmac_secret(
                &idx,
                new_kid,
                new_secret_hex,
                grace_ms,
            );
            let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

            match self
                .runtime()
                .fcall(
                    fcall::names::FF_ROTATE_WAITPOINT_HMAC_SECRET,
                    &key_refs,
                    &arg_refs,
                )
                .await
            {
                Ok(raw) => {
                    if let Some(code) = fcall_error_code(&raw) {
                        tracing::debug!(
                            partition = index,
                            code = %code,
                            "waitpoint hmac rotation rejected by FCALL"
                        );
                        failed.push(RotationFailure {
                            partition_index: index,
                            code: Some(code),
                            detail: ROTATION_DETAIL_LUA_REJECTED.to_owned(),
                        });
                        continue;
                    }
                    if let Err(err) =
                        check_fcall_success(&raw, fcall::names::FF_ROTATE_WAITPOINT_HMAC_SECRET)
                    {
                        tracing::debug!(
                            partition = index,
                            fabric_err = %err,
                            "waitpoint hmac rotation rejected without typed code"
                        );
                        failed.push(RotationFailure {
                            partition_index: index,
                            code: None,
                            detail: ROTATION_DETAIL_LUA_REJECTED.to_owned(),
                        });
                        continue;
                    }
                    match classify_rotation_ok_variant(&raw) {
                        Ok(RotationOkVariant::Rotated) => rotated += 1,
                        Ok(RotationOkVariant::Noop) => noop += 1,
                        Err(e) => {
                            tracing::debug!(
                                partition = index,
                                parse_err = %e,
                                "waitpoint hmac rotation envelope unparseable"
                            );
                            failed.push(RotationFailure {
                                partition_index: index,
                                code: None,
                                detail: ROTATION_DETAIL_UNPARSEABLE_ENVELOPE.to_owned(),
                            });
                        }
                    }
                }
                Err(err) => {
                    tracing::debug!(
                        partition = index,
                        fabric_err = %err,
                        "waitpoint hmac rotation transport error"
                    );
                    failed.push(RotationFailure {
                        partition_index: index,
                        code: None,
                        detail: ROTATION_DETAIL_TRANSPORT_ERROR.to_owned(),
                    });
                }
            }
        }

        RotationOutcome {
            rotated,
            noop,
            failed,
            new_kid: new_kid.to_owned(),
        }
    }

    // ── Run lifecycle (Phase D PR 2a) ────────────────────────────────────

    async fn create_run_execution(
        &self,
        input: CreateRunExecutionInput,
    ) -> Result<ExecutionCreated, FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let tags_json = serde_json::to_string(&input.tags).unwrap_or_else(|_| "{}".to_owned());

        let (keys, args) = fcall::execution::build_create_execution(
            &ctx,
            &idx,
            &input.lane_id,
            &input.execution_id,
            &input.namespace,
            crate::constants::EXECUTION_KIND_RUN,
            "0",
            &input.policy_json,
            &tags_json,
            partition.index,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_CREATE_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_execution: {e}")))?;

        // A DUPLICATE reply is the idempotent-replay success path and
        // must NOT error. Every other non-OK envelope (typed FF error
        // or malformed reply) surfaces through `check_fcall_success`
        // so callers never see `newly_created: true` on a rejected
        // create — which would otherwise trigger a spurious
        // `BridgeEvent::ExecutionCreated` emission.
        let duplicate = is_duplicate_result(&raw);
        if !duplicate {
            check_fcall_success(&raw, fcall::names::FF_CREATE_EXECUTION)?;
        }
        Ok(ExecutionCreated {
            newly_created: !duplicate,
        })
    }

    async fn complete_run_execution(&self, input: CompleteRunInput) -> Result<(), FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let (keys, args) = fcall::execution::build_complete_execution(
            &ctx,
            &idx,
            input.lease.attempt_index,
            &input.lease.worker_instance_id,
            &input.lease.lane_id,
            &input.execution_id,
            &input.lease.lease_id,
            &input.lease.lease_epoch,
            &input.lease.attempt_id,
            &input.lease.source,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_COMPLETE_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_complete_execution: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_COMPLETE_EXECUTION)?;
        Ok(())
    }

    async fn fail_run_execution(
        &self,
        input: FailRunInput,
    ) -> Result<FailExecutionOutcome, FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        // Read FF's retry policy GET key (populated during create). Fail
        // loud on transport errors: silent default here would disable
        // retries for this execution.
        let retry_policy_json: String = if input.retry_policy_json.is_empty() {
            self.runtime()
                .client
                .get(&ctx.policy())
                .await
                .map_err(|e| FabricError::Valkey(format!("GET retry_policy: {e}")))?
                .unwrap_or_default()
        } else {
            input.retry_policy_json.clone()
        };

        let (keys, args) = fcall::execution::build_fail_execution(
            &ctx,
            &idx,
            input.lease.attempt_index,
            &input.lease.worker_instance_id,
            &input.lease.lane_id,
            &input.execution_id,
            &input.lease.lease_id,
            &input.lease.lease_epoch,
            &input.lease.attempt_id,
            &input.reason,
            &input.category,
            &retry_policy_json,
            &input.lease.source,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_FAIL_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_fail_execution: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_FAIL_EXECUTION)?;
        Ok(match parse_fail_outcome(&raw) {
            FailOutcome::RetryScheduled => FailExecutionOutcome::RetryScheduled,
            FailOutcome::TerminalFailed => FailExecutionOutcome::TerminalFailed,
        })
    }

    async fn cancel_run_execution(&self, input: CancelRunInput) -> Result<(), FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let wp_id = input.current_waitpoint.unwrap_or_default();

        let (keys, args) = fcall::execution::build_cancel_execution(
            &ctx,
            &idx,
            input.lease.attempt_index,
            &input.lease.worker_instance_id,
            &input.lease.lane_id,
            &wp_id,
            &input.execution_id,
            crate::constants::CANCEL_SOURCE_OVERRIDE,
            crate::constants::CANCEL_SOURCE_OVERRIDE,
            &input.lease.lease_id,
            &input.lease.lease_epoch,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_CANCEL_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_cancel_execution: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_CANCEL_EXECUTION)?;
        Ok(())
    }

    async fn suspend_run_execution(&self, input: SuspendRunInput) -> Result<(), FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let suspension_id = SuspensionId::new();
        let waitpoint_id = WaitpointId::new();
        let waitpoint_key = format!("wpk:{waitpoint_id}");

        let (keys, args) = fcall::suspension::build_suspend_execution(
            &ctx,
            &idx,
            input.lease.attempt_index,
            &input.lease.worker_instance_id,
            &input.lease.lane_id,
            &waitpoint_id,
            &input.execution_id,
            &input.lease.attempt_id,
            &input.lease.lease_id,
            &input.lease.lease_epoch,
            &suspension_id,
            &waitpoint_key,
            &input.reason_code,
            &input.timeout_at,
            &input.resume_condition_json,
            &input.resume_policy_json,
            &input.timeout_behavior,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_SUSPEND_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_suspend_execution: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_SUSPEND_EXECUTION)?;
        Ok(())
    }

    async fn resume_run_execution(&self, input: ResumeRunInput) -> Result<(), FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let wp_id = input.waitpoint_id.unwrap_or_default();

        let (keys, args) = fcall::suspension::build_resume_execution(
            &ctx,
            &idx,
            &input.lane_id,
            &wp_id,
            &input.execution_id,
            &input.resume_source,
            "0",
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_RESUME_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_resume_execution: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_RESUME_EXECUTION)?;
        Ok(())
    }

    async fn deliver_approval_signal(
        &self,
        input: DeliverApprovalSignalInput,
    ) -> Result<(), FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let signal_id = SignalId::new();
        let now = TimestampMs::now();

        let idem_str = input.idempotency_suffix.clone();
        let idem_key = ctx.signal_dedup(&input.waitpoint_id, &idem_str);

        // HMAC waitpoint token — FF owns it, cairn never caches. Missing
        // token surfaces as `Validation` up the stack.
        let waitpoint_token = crate::signal_bridge::read_waitpoint_token(
            &self.runtime().client,
            &ctx,
            &input.waitpoint_id,
        )
        .await?;

        let signal_maxlen = input.maxlen.to_string();
        let max_signals = input.max_signals_per_execution.to_string();

        let (keys, args) = fcall::suspension::build_deliver_signal(
            &ctx,
            &idx,
            &input.lane_id,
            &signal_id,
            &input.waitpoint_id,
            idem_key,
            &input.execution_id,
            input.signal_name,
            "approval".to_owned(),
            crate::constants::SOURCE_TYPE_APPROVAL_OPERATOR.to_owned(),
            crate::constants::SOURCE_IDENTITY.to_owned(),
            String::new(),
            idem_str,
            now,
            input.signal_dedup_ttl_ms,
            &signal_maxlen,
            &max_signals,
            waitpoint_token.as_str(),
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_DELIVER_SIGNAL, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_deliver_signal: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_DELIVER_SIGNAL)?;
        Ok(())
    }

    // ── Session lifecycle (Phase D PR 2a) ────────────────────────────────

    async fn create_flow(&self, input: CreateFlowInput) -> Result<(), FabricError> {
        let partition = flow_partition(&input.flow_id, &self.runtime().partition_config);
        let fctx = FlowKeyContext::new(&partition, &input.flow_id);
        let now = TimestampMs::now();

        let (keys, args) = fcall::session::build_create_flow(
            &fctx,
            &partition,
            &input.flow_id,
            &input.flow_kind,
            &input.namespace,
            now,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_CREATE_FLOW, &key_refs, &arg_refs)
            .await?;
        check_fcall_success(&raw, fcall::names::FF_CREATE_FLOW)?;
        Ok(())
    }

    async fn cancel_flow(&self, input: CancelFlowInput) -> Result<FlowCancelOutcome, FabricError> {
        let partition = flow_partition(&input.flow_id, &self.runtime().partition_config);
        let fctx = FlowKeyContext::new(&partition, &input.flow_id);
        let now = TimestampMs::now();

        let (keys, args) = fcall::session::build_cancel_flow(
            &fctx,
            &input.flow_id,
            &input.reason,
            &input.cancel_mode,
            now,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_CANCEL_FLOW, &key_refs, &arg_refs)
            .await?;

        // `flow_already_terminal` is acceptable — the flow may already
        // be completed/cancelled, and cairn still wants to stamp its
        // archive marker. Dispatch on the typed Lua error code (via
        // `fcall_error_code`) instead of string-matching the formatted
        // `FabricError` message; the latter would silently break on
        // any future tweak to the `Display` impl.
        if let Some(code) = fcall_error_code(&raw) {
            if code == FLOW_ALREADY_TERMINAL {
                return Ok(FlowCancelOutcome::AlreadyTerminal);
            }
            // Non-accepted typed code — surface via the shared
            // envelope-to-error path so the variant matches the rest
            // of cairn's FCALL error handling.
            check_fcall_success(&raw, fcall::names::FF_CANCEL_FLOW)?;
            // Unreachable: check_fcall_success must return Err above
            // because fcall_error_code only fires on typed errors.
            unreachable!("fcall_error_code returned Some but check_fcall_success accepted");
        }
        check_fcall_success(&raw, fcall::names::FF_CANCEL_FLOW)?;
        Ok(FlowCancelOutcome::Cancelled)
    }

    // ── Claim (Phase D PR 2a) ────────────────────────────────────────────

    async fn issue_grant_and_claim(
        &self,
        input: IssueGrantAndClaimInput,
    ) -> Result<ClaimGrantOutcome, FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        // ── Step 1: issue claim grant ─────────────────────────────────────
        let grant_ttl = self.runtime().config.grant_ttl_ms;
        let grant_keys: Vec<String> = vec![
            ctx.core(),
            ctx.claim_grant(),
            idx.lane_eligible(&input.lane_id),
        ];
        let grant_args: Vec<String> = vec![
            input.execution_id.to_string(),
            self.runtime().config.worker_id.to_string(),
            self.runtime().config.worker_instance_id.to_string(),
            input.lane_id.to_string(),
            String::new(),
            grant_ttl.to_string(),
            String::new(),
            String::new(),
        ];
        let grant_key_refs: Vec<&str> = grant_keys.iter().map(|s| s.as_str()).collect();
        let grant_arg_refs: Vec<&str> = grant_args.iter().map(|s| s.as_str()).collect();

        let raw_grant: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_ISSUE_CLAIM_GRANT,
                &grant_key_refs,
                &grant_arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_issue_claim_grant: {e}")))?;
        check_fcall_success(&raw_grant, fcall::names::FF_ISSUE_CLAIM_GRANT)?;

        // ── Step 2: claim execution ───────────────────────────────────────
        let total_str: Option<String> = self
            .runtime()
            .client
            .hget(&ctx.core(), "total_attempt_count")
            .await
            .unwrap_or(None);
        let next_idx = total_str
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let att_idx = AttemptIndex::new(next_idx);

        let lease_id = LeaseId::new();
        let attempt_id = AttemptId::new();
        let renew_before_ms = input.lease_duration_ms / 3;

        let claim_keys: Vec<String> = vec![
            ctx.core(),
            ctx.claim_grant(),
            idx.lane_eligible(&input.lane_id),
            idx.lease_expiry(),
            idx.worker_leases(&self.runtime().config.worker_instance_id),
            ctx.attempt_hash(att_idx),
            ctx.attempt_usage(att_idx),
            ctx.attempt_policy(att_idx),
            ctx.attempts(),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lane_active(&input.lane_id),
            idx.attempt_timeout(),
            idx.execution_deadline(),
        ];
        let claim_args: Vec<String> = vec![
            input.execution_id.to_string(),
            self.runtime().config.worker_id.to_string(),
            self.runtime().config.worker_instance_id.to_string(),
            input.lane_id.to_string(),
            String::new(),
            lease_id.to_string(),
            input.lease_duration_ms.to_string(),
            renew_before_ms.to_string(),
            attempt_id.to_string(),
            "{}".to_owned(),
            String::new(),
            String::new(),
        ];
        let claim_key_refs: Vec<&str> = claim_keys.iter().map(|s| s.as_str()).collect();
        let claim_arg_refs: Vec<&str> = claim_args.iter().map(|s| s.as_str()).collect();

        let raw_claim: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_CLAIM_EXECUTION,
                &claim_key_refs,
                &claim_arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_claim_execution: {e}")))?;

        // Dispatch: if FF returns `use_claim_resumed_execution` the
        // execution is in `attempt_interrupted` (resumed from suspension)
        // and must go through `ff_claim_resumed_execution` — which
        // resumes the SAME attempt instead of allocating a new one. The
        // grant is still live: FF's dispatch guard runs BEFORE grant
        // consumption, so we do NOT re-issue it.
        if fcall_error_code(&raw_claim).as_deref() == Some(USE_CLAIM_RESUMED_EXECUTION) {
            return self
                .claim_resumed_execution(
                    &ctx,
                    &idx,
                    &input.execution_id,
                    &input.lane_id,
                    input.lease_duration_ms,
                )
                .await;
        }

        check_fcall_success(&raw_claim, fcall::names::FF_CLAIM_EXECUTION)?;
        let lease_epoch = parse_claim_lease_epoch(&raw_claim)?;

        Ok(ClaimGrantOutcome {
            lease_id,
            lease_epoch,
            attempt_index: att_idx,
        })
    }

    // ── Task lifecycle (Phase D PR 2b) ──────────────────────────────────

    async fn submit_task_execution(
        &self,
        input: SubmitTaskInput,
    ) -> Result<ExecutionCreated, FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let tags_json = serde_json::to_string(&input.tags).unwrap_or_else(|_| "{}".to_owned());

        // Historical default retry policy. Callers may override by
        // passing a non-empty `policy_json`.
        let policy_json = if input.policy_json.is_empty() {
            serde_json::json!({
                "max_retries": 2,
                "backoff": {
                    "type": "exponential",
                    "initial_delay_ms": 1000,
                    "max_delay_ms": 30000,
                    "multiplier": 2
                }
            })
            .to_string()
        } else {
            input.policy_json.clone()
        };

        let priority_str = input.priority.to_string();
        let (keys, args) = fcall::execution::build_create_execution(
            &ctx,
            &idx,
            &input.lane_id,
            &input.execution_id,
            &input.namespace,
            crate::constants::EXECUTION_KIND_TASK,
            &priority_str,
            &policy_json,
            &tags_json,
            partition.index,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_CREATE_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_execution: {e}")))?;

        let duplicate = is_duplicate_result(&raw);
        if !duplicate {
            check_fcall_success(&raw, fcall::names::FF_CREATE_EXECUTION)?;
        }
        Ok(ExecutionCreated {
            newly_created: !duplicate,
        })
    }

    async fn add_execution_to_flow(
        &self,
        input: AddExecutionToFlowInput,
    ) -> Result<(), FabricError> {
        let flow_partition = flow_partition(&input.flow_id, &self.runtime().partition_config);
        let fctx = FlowKeyContext::new(&flow_partition, &input.flow_id);
        let flow_idx = FlowIndexKeys::new(&flow_partition);
        let exec_partition =
            execution_partition(&input.execution_id, &self.runtime().partition_config);
        let exec_ctx = ExecKeyContext::new(&exec_partition, &input.execution_id);

        let now = TimestampMs::now();

        // 1. Ensure the flow exists. Idempotent — FF replies
        // `ok_already_satisfied` on duplicate.
        let (create_keys, create_args) = fcall::session::build_create_flow(
            &fctx,
            &flow_partition,
            &input.flow_id,
            &input.flow_kind,
            &input.namespace,
            now,
        );
        let create_key_refs: Vec<&str> = create_keys.iter().map(|s| s.as_str()).collect();
        let create_arg_refs: Vec<&str> = create_args.iter().map(|s| s.as_str()).collect();
        let create_raw: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_CREATE_FLOW,
                &create_key_refs,
                &create_arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_flow: {e}")))?;
        check_fcall_success(&create_raw, fcall::names::FF_CREATE_FLOW)?;

        // 2. Bind the execution as a flow member.
        let now_ms = now.0 as u64;
        let (keys, args) = fcall::flow_edges::build_add_execution_to_flow(
            &fctx,
            &flow_idx,
            &exec_ctx,
            &input.flow_id,
            &input.execution_id,
            now_ms,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_ADD_EXECUTION_TO_FLOW, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_add_execution_to_flow: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_ADD_EXECUTION_TO_FLOW)?;
        Ok(())
    }

    async fn stage_dependency_edge(
        &self,
        input: StageDependencyEdgeInput,
    ) -> Result<StageDependencyOutcome, FabricError> {
        let partition = flow_partition(&input.flow_id, &self.runtime().partition_config);
        let fctx = FlowKeyContext::new(&partition, &input.flow_id);
        let now_ms = TimestampMs::now().0 as u64;

        let (keys, args) = fcall::flow_edges::build_stage_dependency_edge(
            &fctx,
            &input.flow_id,
            &input.edge_id,
            &input.upstream_execution_id,
            &input.downstream_execution_id,
            &input.dependency_kind,
            &input.data_passing_ref,
            input.expected_graph_revision,
            now_ms,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_STAGE_DEPENDENCY_EDGE, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_stage_dependency_edge: {e}")))?;

        // Typed-code dispatch. Any code not matched below falls
        // through to `check_fcall_success` which surfaces it as
        // `FabricError::Internal` — services see a hard error rather
        // than a silent "unknown variant" outcome.
        if let Some(code) = fcall_error_code(&raw) {
            return Ok(match code.as_str() {
                "stale_graph_revision" => StageDependencyOutcome::StaleGraphRevision,
                "cycle_detected" => StageDependencyOutcome::Cycle,
                "self_referencing_edge" => StageDependencyOutcome::SelfReferencing,
                "dependency_already_exists" => StageDependencyOutcome::AlreadyExists,
                "flow_not_found" => StageDependencyOutcome::FlowNotFound,
                "flow_already_terminal" => StageDependencyOutcome::FlowAlreadyTerminal,
                "execution_not_in_flow" => StageDependencyOutcome::ExecutionNotInFlow,
                _ => {
                    return Err(FabricError::Internal(format!(
                        "ff_stage_dependency_edge rejected: {code}"
                    )));
                }
            });
        }

        let new_graph_revision = parse_stage_result_revision(&raw).ok_or_else(|| {
            FabricError::Internal("ff_stage_dependency_edge: malformed OK envelope".into())
        })?;
        Ok(StageDependencyOutcome::Staged { new_graph_revision })
    }

    async fn apply_dependency_to_child(
        &self,
        input: ApplyDependencyToChildInput,
    ) -> Result<(), FabricError> {
        let child_partition = execution_partition(
            &input.downstream_execution_id,
            &self.runtime().partition_config,
        );
        let child_exec_ctx = ExecKeyContext::new(&child_partition, &input.downstream_execution_id);
        let child_idx = IndexKeys::new(&child_partition);
        let now_ms = TimestampMs::now().0 as u64;

        let (keys, args) = fcall::flow_edges::build_apply_dependency_to_child(
            &child_exec_ctx,
            &child_idx.lane_eligible(&input.lane_id),
            &child_idx.lane_blocked_dependencies(&input.lane_id),
            &input.edge_id,
            &input.flow_id,
            &input.upstream_execution_id,
            input.graph_revision,
            &input.dependency_kind,
            &input.data_passing_ref,
            now_ms,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_APPLY_DEPENDENCY_TO_CHILD,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_apply_dependency_to_child: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_APPLY_DEPENDENCY_TO_CHILD)?;
        Ok(())
    }

    async fn evaluate_flow_eligibility(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<EligibilityResult, FabricError> {
        let partition = execution_partition(execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, execution_id);

        let (keys, args) = fcall::flow_edges::build_evaluate_flow_eligibility(&ctx);
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_EVALUATE_FLOW_ELIGIBILITY,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_evaluate_flow_eligibility: {e}")))?;

        // Guard against typed Lua error codes (e.g. `execution_not_found`).
        // Without this, `parse_eligibility_result` would extract the error
        // code slot as if it were an eligibility state string and silently
        // return "Other(execution_not_found)" — masking the real failure.
        check_fcall_success(&raw, fcall::names::FF_EVALUATE_FLOW_ELIGIBILITY)?;

        let state = parse_eligibility_result(&raw).ok_or_else(|| {
            FabricError::Internal("ff_evaluate_flow_eligibility: malformed OK envelope".into())
        })?;

        Ok(match state.as_str() {
            "eligible" => EligibilityResult::Eligible,
            "blocked_by_dependencies" => EligibilityResult::BlockedByDependencies,
            other => EligibilityResult::Other(other.to_owned()),
        })
    }

    async fn renew_task_lease(&self, input: RenewLeaseInput) -> Result<(), FabricError> {
        let partition = execution_partition(&input.execution_id, &self.runtime().partition_config);
        let ctx = ExecKeyContext::new(&partition, &input.execution_id);
        let idx = IndexKeys::new(&partition);

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lease_expiry(),
        ];
        let args: Vec<String> = vec![
            input.execution_id.to_string(),
            input.lease.attempt_index.to_string(),
            input.lease.attempt_id.clone(),
            input.lease.lease_id.clone(),
            input.lease.lease_epoch.clone(),
            input.lease_extension_ms.to_string(),
            crate::constants::DEFAULT_LEASE_HISTORY_GRACE_MS.to_owned(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(fcall::names::FF_RENEW_LEASE, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_renew_lease: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_RENEW_LEASE)?;
        Ok(())
    }
}

// ── Claim-resumed (Phase D PR 2a) ───────────────────────────────────────

/// FF's typed dispatch error: the execution's `attempt_state` is
/// `attempt_interrupted` (resume-from-suspension claim), so the caller
/// must use `ff_claim_resumed_execution`. See `lua/execution.lua`.
const USE_CLAIM_RESUMED_EXECUTION: &str = "use_claim_resumed_execution";

impl ValkeyEngine {
    async fn claim_resumed_execution(
        &self,
        ctx: &ExecKeyContext,
        idx: &IndexKeys,
        eid: &ExecutionId,
        lane_id: &flowfabric::core::types::LaneId,
        lease_duration_ms: u64,
    ) -> Result<ClaimGrantOutcome, FabricError> {
        // FF requires the existing attempt_hash as KEYS[6]; re-read the
        // attempt index from exec_core so the key points at the live
        // attempt.
        let att_idx_str: Option<String> = self
            .runtime()
            .client
            .hget(&ctx.core(), "current_attempt_index")
            .await
            .unwrap_or(None);
        let att_idx = AttemptIndex::new(
            att_idx_str
                .as_deref()
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0),
        );

        let lease_id = LeaseId::new();

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.claim_grant(),
            idx.lane_eligible(lane_id),
            idx.lease_expiry(),
            idx.worker_leases(&self.runtime().config.worker_instance_id),
            ctx.attempt_hash(att_idx),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lane_active(lane_id),
            idx.attempt_timeout(),
            idx.execution_deadline(),
        ];
        let args: Vec<String> = vec![
            eid.to_string(),
            self.runtime().config.worker_id.to_string(),
            self.runtime().config.worker_instance_id.to_string(),
            lane_id.to_string(),
            String::new(),
            lease_id.to_string(),
            lease_duration_ms.to_string(),
            String::new(),
        ];
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime()
            .fcall(
                fcall::names::FF_CLAIM_RESUMED_EXECUTION,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_claim_resumed_execution: {e}")))?;
        check_fcall_success(&raw, fcall::names::FF_CLAIM_RESUMED_EXECUTION)?;

        let lease_epoch = parse_claim_lease_epoch(&raw)?;
        Ok(ClaimGrantOutcome {
            lease_id,
            lease_epoch,
            attempt_index: att_idx,
        })
    }
}

/// Parse the `lease_epoch` slot of `ff_claim_execution`'s reply:
/// `{1, "OK", <lease_id>, <lease_epoch>}`. Previously lived inside
/// `services::claim_common`; lifted here with the rest of the claim
/// machinery.
fn parse_claim_lease_epoch(
    raw: &ferriskey::Value,
) -> Result<flowfabric::core::types::LeaseEpoch, FabricError> {
    if let ferriskey::Value::Array(arr) = raw {
        if let Some(Ok(ferriskey::Value::BulkString(b))) = arr.get(3) {
            if let Ok(n) = String::from_utf8_lossy(b).parse::<u64>() {
                return Ok(flowfabric::core::types::LeaseEpoch::new(n));
            }
        }
        if let Some(Ok(ferriskey::Value::Int(n))) = arr.get(3) {
            return Ok(flowfabric::core::types::LeaseEpoch::new(*n as u64));
        }
    }
    Err(FabricError::Internal(
        "ff_claim_execution: missing lease_epoch in response".to_owned(),
    ))
}

// ── Helpers (free functions, kept file-local) ──────────────────────────

fn map_report_usage_result(r: ReportUsageResult) -> BudgetSpendOutcome {
    match r {
        ReportUsageResult::Ok => BudgetSpendOutcome::Ok,
        ReportUsageResult::AlreadyApplied => BudgetSpendOutcome::AlreadyApplied,
        ReportUsageResult::SoftBreach {
            dimension,
            current_usage,
            soft_limit,
        } => BudgetSpendOutcome::SoftBreach {
            dimension,
            current_usage,
            soft_limit,
        },
        ReportUsageResult::HardBreach {
            dimension,
            current_usage,
            hard_limit,
        } => BudgetSpendOutcome::HardBreach {
            dimension,
            current_usage,
            hard_limit,
        },
        // `ReportUsageResult` is `#[non_exhaustive]`. All known FF 0.9
        // variants are handled above; a new variant added upstream
        // (e.g. RFC-015 quota-scoped breach) should fail loud so cairn
        // audits the mapping rather than silently dropping the result.
        other => panic!("unhandled ReportUsageResult variant (post-FF-0.9 addition): {other:?}"),
    }
}

fn parse_admission_result(raw: &ferriskey::Value) -> Result<QuotaAdmission, FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => {
            return Err(FabricError::Internal(
                "ff_check_admission_and_record: expected Array".to_owned(),
            ))
        }
    };

    let status = match arr.first() {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        _ => {
            return Err(FabricError::Internal(
                "ff_check_admission_and_record: missing status".to_owned(),
            ))
        }
    };

    match status.as_str() {
        "ADMITTED" => Ok(QuotaAdmission::Admitted),
        "ALREADY_ADMITTED" => Ok(QuotaAdmission::AlreadyAdmitted),
        "RATE_EXCEEDED" => {
            let retry_str = match arr.get(1) {
                Some(Ok(ferriskey::Value::BulkString(b))) => {
                    String::from_utf8_lossy(b).into_owned()
                }
                Some(Ok(ferriskey::Value::Int(n))) => n.to_string(),
                _ => "0".to_owned(),
            };
            let retry_after_ms = retry_str.parse().unwrap_or(0);
            Ok(QuotaAdmission::RateExceeded { retry_after_ms })
        }
        "CONCURRENCY_EXCEEDED" => Ok(QuotaAdmission::ConcurrencyExceeded),
        _ => Err(FabricError::Internal(format!(
            "ff_check_admission_and_record: unknown status: {status}"
        ))),
    }
}

enum RotationOkVariant {
    Rotated,
    Noop,
}

/// Parse an `ok(...)` rotate envelope: `[Int(1), "OK", variant, ...args]`.
fn classify_rotation_ok_variant(raw: &ferriskey::Value) -> Result<RotationOkVariant, FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => return Err(FabricError::Internal("rotate envelope not an array".into())),
    };
    let variant = match arr.get(2) {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        _ => {
            return Err(FabricError::Internal(
                "rotate envelope missing variant discriminator".into(),
            ))
        }
    };
    match variant.as_str() {
        "rotated" => Ok(RotationOkVariant::Rotated),
        "noop" => Ok(RotationOkVariant::Noop),
        other => Err(FabricError::Internal(format!(
            "unexpected rotation variant: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Envelope shape tests for parse_admission_result.

    #[test]
    fn admission_admitted() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "ADMITTED".to_owned(),
        ))]);
        assert_eq!(
            parse_admission_result(&raw).unwrap(),
            QuotaAdmission::Admitted
        );
    }

    #[test]
    fn admission_already_admitted() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "ALREADY_ADMITTED".to_owned(),
        ))]);
        assert_eq!(
            parse_admission_result(&raw).unwrap(),
            QuotaAdmission::AlreadyAdmitted
        );
    }

    #[test]
    fn admission_rate_exceeded_carries_retry() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::SimpleString("RATE_EXCEEDED".to_owned())),
            Ok(ferriskey::Value::Int(5000)),
        ]);
        assert_eq!(
            parse_admission_result(&raw).unwrap(),
            QuotaAdmission::RateExceeded {
                retry_after_ms: 5000
            }
        );
    }

    #[test]
    fn admission_concurrency_exceeded() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "CONCURRENCY_EXCEEDED".to_owned(),
        ))]);
        assert_eq!(
            parse_admission_result(&raw).unwrap(),
            QuotaAdmission::ConcurrencyExceeded
        );
    }

    #[test]
    fn admission_unknown_errors() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "GARBAGE".to_owned(),
        ))]);
        assert!(parse_admission_result(&raw).is_err());
    }

    #[test]
    fn admission_non_array_errors() {
        let raw = ferriskey::Value::SimpleString("not array".to_owned());
        assert!(parse_admission_result(&raw).is_err());
    }

    // Rotation variant classifier tests.

    #[test]
    fn rotation_ok_rotated() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(b"OK".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"rotated".to_vec().into())),
        ]);
        assert!(matches!(
            classify_rotation_ok_variant(&raw),
            Ok(RotationOkVariant::Rotated)
        ));
    }

    #[test]
    fn rotation_ok_noop() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(b"OK".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"noop".to_vec().into())),
        ]);
        assert!(matches!(
            classify_rotation_ok_variant(&raw),
            Ok(RotationOkVariant::Noop)
        ));
    }

    #[test]
    fn rotation_unknown_variant_errors() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::BulkString(b"OK".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"unknown".to_vec().into())),
        ]);
        assert!(classify_rotation_ok_variant(&raw).is_err());
    }

    // ReportUsageResult mapper.

    #[test]
    fn maps_ff_ok_to_cairn_ok() {
        assert_eq!(
            map_report_usage_result(ReportUsageResult::Ok),
            BudgetSpendOutcome::Ok
        );
    }

    #[test]
    fn maps_ff_already_applied() {
        assert_eq!(
            map_report_usage_result(ReportUsageResult::AlreadyApplied),
            BudgetSpendOutcome::AlreadyApplied
        );
    }

    #[test]
    fn maps_ff_hard_breach_preserves_fields() {
        let got = map_report_usage_result(ReportUsageResult::HardBreach {
            dimension: "tokens".into(),
            current_usage: 110,
            hard_limit: 100,
        });
        match got {
            BudgetSpendOutcome::HardBreach {
                dimension,
                current_usage,
                hard_limit,
            } => {
                assert_eq!(dimension, "tokens");
                assert_eq!(current_usage, 110);
                assert_eq!(hard_limit, 100);
            }
            other => panic!("expected HardBreach, got {other:?}"),
        }
    }

    #[test]
    fn maps_ff_soft_breach_preserves_fields() {
        let got = map_report_usage_result(ReportUsageResult::SoftBreach {
            dimension: "cost".into(),
            current_usage: 85,
            soft_limit: 80,
        });
        match got {
            BudgetSpendOutcome::SoftBreach {
                dimension,
                current_usage,
                soft_limit,
            } => {
                assert_eq!(dimension, "cost");
                assert_eq!(current_usage, 85);
                assert_eq!(soft_limit, 80);
            }
            other => panic!("expected SoftBreach, got {other:?}"),
        }
    }
}
