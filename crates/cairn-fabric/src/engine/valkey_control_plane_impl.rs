//! [`ControlPlaneBackend`] implementation for [`ValkeyEngine`].
//!
//! Holds the FF key-builder / partition / FCALL logic lifted out of
//! `FabricBudgetService`, `FabricQuotaService`, and
//! `FabricRotationService`. Those services now delegate to this impl
//! through the [`ControlPlaneBackend`] trait; the FF imports
//! (`ff_core::keys`, `ff_core::partition`, `ff_core::contracts`) live
//! here only.
//!
//! When FF 0.3 ships `describe_*` + control-plane primitives upstream
//! (FlowFabric#58), this file collapses to a thin passthrough — the
//! service layer stays untouched because it already talks to the
//! trait, not to FF.

use std::collections::HashMap;

use async_trait::async_trait;
use ff_core::contracts::ReportUsageResult;
use ff_core::keys::{
    budget_policies_index, budget_resets_key, quota_policies_index, usage_dedup_key,
    BudgetKeyContext, IndexKeys, QuotaKeyContext,
};
use ff_core::partition::{budget_partition, quota_partition, Partition, PartitionFamily};
use ff_core::types::{BudgetId, ExecutionId, QuotaPolicyId, TimestampMs};
use ff_sdk::task::parse_report_usage_result;

use crate::error::FabricError;
use crate::fcall;
use crate::helpers::{check_fcall_success, fcall_error_code};

use super::control_plane::ControlPlaneBackend;
use super::control_plane_types::{
    BudgetSpendOutcome, BudgetStatusSnapshot, QuotaAdmission, RotationFailure, RotationOutcome,
};
use super::valkey_impl::ValkeyEngine;

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
