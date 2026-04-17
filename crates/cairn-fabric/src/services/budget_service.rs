use std::collections::HashMap;
use std::sync::Arc;

use crate::error::FabricError;
use ff_core::keys::{budget_resets_key, BudgetKeyContext};
use ff_core::partition::budget_partition;
use ff_core::types::{BudgetId, ExecutionId, TimestampMs};
use uuid::Uuid;

use crate::boot::FabricRuntime;

// B4 idempotency (prep): stable namespace UUID for spend-dedup keys.
// Mirrors the UUID v5 + null-byte-delimited scheme from id_map.rs.
// Changing these bytes orphans any in-flight idempotency record.
const SPEND_NAMESPACE: Uuid = Uuid::from_bytes([
    0xb7, 0x1a, 0x2e, 0x04, 0x9c, 0x85, 0x45, 0xc3, 0x88, 0xd9, 0x0f, 0x4a, 0x6b, 0x2c, 0x13, 0x77,
]);

const SPEND_NAMESPACE_VERSION: u8 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SpendResult {
    /// Spend applied (or retry suppressed via idempotency key).
    /// `was_retry` is true only when FF returned ALREADY_APPLIED — the budget
    /// was *not* double-decremented; the original record is authoritative.
    Ok {
        was_retry: bool,
    },
    SoftBreach {
        dimension: String,
        action: String,
    },
    HardBreach {
        dimension: String,
        action: String,
        current_usage: u64,
        hard_limit: u64,
    },
}

/// B4 idempotency key derivation (prep).
///
/// Stable across retries for the same (budget, execution, dimension set/amount).
/// Callers that repeat a spend with identical inputs produce an identical key,
/// so FF can dedup server-side when the contract accepts ARGV[last] = key.
///
/// Scheme: UUID v5 over `"v{ver}:spend:\0{budget}\0{execution}\0{sorted dim\0delta pairs}"`.
/// Null-byte delimiters match id_map.rs and eliminate colon-boundary collisions
/// (e.g. dim "a:b" vs dims "a"+"b").
pub(crate) fn compute_spend_idempotency_key(
    budget_id: &BudgetId,
    execution_id: &ExecutionId,
    dimension_deltas: &[(&str, u64)],
) -> String {
    let mut sorted: Vec<(&str, u64)> = dimension_deltas.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));

    let mut input = format!("v{SPEND_NAMESPACE_VERSION}:spend:\0{budget_id}\0{execution_id}");
    for (dim, delta) in &sorted {
        input.push('\0');
        input.push_str(dim);
        input.push('\0');
        input.push_str(&delta.to_string());
    }
    Uuid::new_v5(&SPEND_NAMESPACE, input.as_bytes()).to_string()
}

#[derive(Clone, Debug)]
pub struct BudgetStatus {
    pub budget_id: String,
    pub scope_type: String,
    pub scope_id: String,
    pub enforcement_mode: String,
    pub usage: HashMap<String, u64>,
    pub hard_limits: HashMap<String, u64>,
    pub soft_limits: HashMap<String, u64>,
    pub breach_count: u64,
    pub soft_breach_count: u64,
}

pub struct FabricBudgetService {
    runtime: Arc<FabricRuntime>,
}

impl FabricBudgetService {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }

    pub async fn create_budget(
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
        let partition = budget_partition(&budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, &budget_id);
        let resets_zset = budget_resets_key(&partition.hash_tag());
        let now = TimestampMs::now();

        let (keys, argv) = crate::fcall::budget::build_create_budget(
            &ctx,
            &resets_zset,
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

        let _: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_CREATE_BUDGET, &key_refs, &argv_refs)
            .await?;

        Ok(budget_id)
    }

    pub async fn create_run_budget(
        &self,
        run_id: &cairn_domain::RunId,
        token_limit: u64,
        cost_limit_microdollars: u64,
        api_call_limit: u64,
    ) -> Result<BudgetId, FabricError> {
        self.create_budget(
            "run",
            run_id.as_str(),
            &["tokens", "cost_microdollars", "api_calls"],
            &[token_limit, cost_limit_microdollars, api_call_limit],
            &[
                token_limit * 80 / 100,
                cost_limit_microdollars * 80 / 100,
                api_call_limit * 80 / 100,
            ],
            0,
            "enforce",
        )
        .await
    }

    pub async fn create_tenant_budget(
        &self,
        tenant_id: &cairn_domain::TenantId,
        token_limit: u64,
        cost_limit_microdollars: u64,
        api_call_limit: u64,
        reset_interval_ms: u64,
    ) -> Result<BudgetId, FabricError> {
        self.create_budget(
            "tenant",
            tenant_id.as_str(),
            &["tokens", "cost_microdollars", "api_calls"],
            &[token_limit, cost_limit_microdollars, api_call_limit],
            &[
                token_limit * 80 / 100,
                cost_limit_microdollars * 80 / 100,
                api_call_limit * 80 / 100,
            ],
            reset_interval_ms,
            "enforce",
        )
        .await
    }

    pub async fn release_budget(&self, budget_id: &BudgetId) -> Result<(), FabricError> {
        let partition = budget_partition(budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);
        let resets_zset = budget_resets_key(&partition.hash_tag());
        let now = TimestampMs::now();

        let keys: Vec<String> = vec![ctx.definition(), ctx.usage(), resets_zset];
        let argv: Vec<String> = vec![budget_id.to_string(), now.to_string()];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_RESET_BUDGET, &key_refs, &argv_refs)
            .await?;

        Ok(())
    }

    /// Record spend against a budget.
    ///
    /// # B4 idempotency (two-phase rollout)
    ///
    /// FF's `ff_report_usage_and_check` does not yet accept an idempotency-key
    /// ARGV slot. Until it does, calling this twice with the same inputs will
    /// double-decrement the budget. Phase 1 (this commit) computes a
    /// deterministic key using the same UUID v5 + null-byte scheme as
    /// `id_map.rs` but does **not** send it. Phase 2 lands once FF ships: we
    /// push the key as `ARGV[last]` and FF replies `ALREADY_APPLIED` on retry;
    /// `parse_spend_result` already understands that arm and returns
    /// `SpendResult::Ok { was_retry: true }` so callers can log
    /// retry-suppression without a behavior change today.
    ///
    /// When the caller has no `ExecutionId` yet (e.g. pre-claim spending in
    /// tests), it passes `None` and we degrade to per-budget-only keys. Real
    /// callers from run_service will always thread the ExecutionId from
    /// `id_map::run_to_execution_id`.
    pub async fn record_spend(
        &self,
        budget_id: &BudgetId,
        dimension_deltas: &[(&str, u64)],
    ) -> Result<SpendResult, FabricError> {
        self.record_spend_with_execution(budget_id, None, dimension_deltas)
            .await
    }

    /// Same as `record_spend` but with a caller-provided ExecutionId for
    /// idempotency-key derivation. Today the key is computed and discarded;
    /// once FF accepts the ARGV slot we pass it through unchanged.
    pub async fn record_spend_with_execution(
        &self,
        budget_id: &BudgetId,
        execution_id: Option<&ExecutionId>,
        dimension_deltas: &[(&str, u64)],
    ) -> Result<SpendResult, FabricError> {
        let partition = budget_partition(budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);
        let now = TimestampMs::now();

        let (keys, mut argv) =
            crate::fcall::budget::build_report_usage(&ctx, dimension_deltas, now);

        // B4 prep: compute the idempotency key on every call so the derivation
        // is exercised in production telemetry paths before we flip the flag.
        // When execution_id is absent we still compute a key — it just isn't
        // retry-stable across process boundaries.
        let _idempotency_key = match execution_id {
            Some(eid) => compute_spend_idempotency_key(budget_id, eid, dimension_deltas),
            None => compute_spend_idempotency_key(
                budget_id,
                &ExecutionId::from_uuid(Uuid::nil()),
                dimension_deltas,
            ),
        };

        #[cfg(feature = "fabric-b4-idempotency")]
        {
            // TODO(B4): pass as ARGV[last] when FF ff_report_usage_and_check
            // accepts it. Until FF ships, Lua rejects the extra arg with
            // "wrong number of arguments". Enabling this feature without the
            // matching FF Lua change WILL fail every spend — do not flip in
            // production until both sides are cut over together.
            argv.push(_idempotency_key);
        }
        // In the default (feature-off) build the key would otherwise be dead.
        // This debug_assert proves the derivation actually ran without adding
        // runtime telemetry noise — keeps the prep honest for every caller.
        #[cfg(not(feature = "fabric-b4-idempotency"))]
        debug_assert!(
            !_idempotency_key.is_empty(),
            "B4 prep: idempotency key derivation returned empty string"
        );
        // Silence unused-mut warning when feature is off.
        let _ = &mut argv;

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let argv_refs: Vec<&str> = argv.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_REPORT_USAGE_AND_CHECK,
                &key_refs,
                &argv_refs,
            )
            .await?;

        parse_spend_result(&raw)
    }

    pub async fn get_budget_status(
        &self,
        budget_id: &BudgetId,
    ) -> Result<BudgetStatus, FabricError> {
        let partition = budget_partition(budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);

        let def_fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.definition())
            .await
            .map_err(|e| FabricError::Internal(format!("HGETALL budget def: {e}")))?;

        if def_fields.is_empty() {
            return Err(FabricError::NotFound {
                entity: "budget",
                id: budget_id.to_string(),
            });
        }

        let usage_fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.usage())
            .await
            .unwrap_or_default();

        let limits_fields: HashMap<String, String> = self
            .runtime
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

        for (k, v) in &limits_fields {
            if let Some(dim) = k.strip_suffix(":hard") {
                if let Ok(val) = v.parse::<u64>() {
                    hard_limits.insert(dim.to_owned(), val);
                }
            } else if let Some(dim) = k.strip_suffix(":soft") {
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

        Ok(BudgetStatus {
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
        })
    }
}

fn parse_spend_result(raw: &ferriskey::Value) -> Result<SpendResult, FabricError> {
    let arr = match raw {
        ferriskey::Value::Array(arr) => arr,
        _ => {
            return Err(FabricError::Internal(
                "ff_report_usage_and_check: expected Array".to_owned(),
            ))
        }
    };

    let status = match arr.first() {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        _ => {
            return Err(FabricError::Internal(
                "ff_report_usage_and_check: missing status".to_owned(),
            ))
        }
    };

    match status.as_str() {
        "OK" => Ok(SpendResult::Ok { was_retry: false }),
        // B4 idempotency: FF returns ALREADY_APPLIED when the idempotency key
        // matches a previously-recorded spend. The budget was NOT
        // decremented again — treat as success and flag the retry so callers
        // can suppress duplicate telemetry/events without producing a ghost
        // breach.
        "ALREADY_APPLIED" | "already_applied" => Ok(SpendResult::Ok { was_retry: true }),
        "SOFT_BREACH" => {
            let dimension = field_str(arr, 1);
            let action = field_str(arr, 2);
            Ok(SpendResult::SoftBreach { dimension, action })
        }
        "HARD_BREACH" => {
            let dimension = field_str(arr, 1);
            let action = field_str(arr, 2);
            let current_usage = field_str(arr, 3).parse().unwrap_or(0);
            let hard_limit = field_str(arr, 4).parse().unwrap_or(0);
            Ok(SpendResult::HardBreach {
                dimension,
                action,
                current_usage,
                hard_limit,
            })
        }
        _ => Err(FabricError::Internal(format!(
            "ff_report_usage_and_check: unknown status: {status}"
        ))),
    }
}

fn field_str(arr: &[Result<ferriskey::Value, ferriskey::Error>], index: usize) -> String {
    match arr.get(index) {
        Some(Ok(ferriskey::Value::BulkString(b))) => String::from_utf8_lossy(b).into_owned(),
        Some(Ok(ferriskey::Value::SimpleString(s))) => s.clone(),
        Some(Ok(ferriskey::Value::Int(n))) => n.to_string(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spend_result_ok_variant() {
        let raw =
            ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString("OK".to_owned()))]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(result, SpendResult::Ok { was_retry: false });
    }

    #[test]
    fn spend_result_already_applied_is_retry_success() {
        // B4 idempotency prep: simulated FF response when the idempotency
        // key matches a prior spend. Budget is NOT double-decremented; we
        // surface this as a successful Ok with was_retry=true so callers can
        // log retry-suppression and skip duplicate telemetry.
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "ALREADY_APPLIED".to_owned(),
        ))]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(result, SpendResult::Ok { was_retry: true });
    }

    #[test]
    fn spend_result_already_applied_lowercase_also_handled() {
        // Defensive: FF Lua may emit either upper- or lower-case spelling.
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::BulkString(
            b"already_applied".to_vec().into(),
        ))]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(result, SpendResult::Ok { was_retry: true });
    }

    #[test]
    fn idempotency_key_stable_for_same_inputs() {
        let bid = BudgetId::new();
        let eid = ExecutionId::new();
        let k1 = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        assert_eq!(k1, k2);
        // UUID v5 is 36 chars with hyphens.
        assert_eq!(k1.len(), 36);
    }

    #[test]
    fn idempotency_key_differs_when_inputs_change() {
        let bid = BudgetId::new();
        let eid = ExecutionId::new();
        let k_tokens = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        let k_cost = compute_spend_idempotency_key(&bid, &eid, &[("cost", 50)]);
        let k_amount = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 51)]);
        assert_ne!(k_tokens, k_cost);
        assert_ne!(k_tokens, k_amount);
    }

    #[test]
    fn idempotency_key_order_independent_for_same_dimension_set() {
        let bid = BudgetId::new();
        let eid = ExecutionId::new();
        let k_ab = compute_spend_idempotency_key(&bid, &eid, &[("a", 1), ("b", 2)]);
        let k_ba = compute_spend_idempotency_key(&bid, &eid, &[("b", 2), ("a", 1)]);
        assert_eq!(k_ab, k_ba);
    }

    #[test]
    fn idempotency_key_isolates_execution() {
        let bid = BudgetId::new();
        let eid1 = ExecutionId::new();
        let eid2 = ExecutionId::new();
        let k1 = compute_spend_idempotency_key(&bid, &eid1, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid2, &[("tokens", 50)]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn idempotency_key_isolates_budget() {
        let b1 = BudgetId::new();
        let b2 = BudgetId::new();
        let eid = ExecutionId::new();
        let k1 = compute_spend_idempotency_key(&b1, &eid, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&b2, &eid, &[("tokens", 50)]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn idempotency_key_no_delimiter_collision() {
        // Null-byte delimiters prevent "a:b"+"c" vs "a"+"b:c" style collisions,
        // same pattern id_map.rs guards against.
        let bid = BudgetId::new();
        let eid = ExecutionId::new();
        let k1 = compute_spend_idempotency_key(&bid, &eid, &[("a:b", 1), ("c", 2)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid, &[("a", 1), ("b:c", 2)]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn spend_result_soft_breach() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::SimpleString("SOFT_BREACH".to_owned())),
            Ok(ferriskey::Value::BulkString(b"tokens".to_vec().into())),
            Ok(ferriskey::Value::BulkString(b"log".to_vec().into())),
        ]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(
            result,
            SpendResult::SoftBreach {
                dimension: "tokens".to_owned(),
                action: "log".to_owned(),
            }
        );
    }

    #[test]
    fn spend_result_hard_breach() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::SimpleString("HARD_BREACH".to_owned())),
            Ok(ferriskey::Value::BulkString(
                b"cost_microdollars".to_vec().into(),
            )),
            Ok(ferriskey::Value::BulkString(b"block".to_vec().into())),
            Ok(ferriskey::Value::Int(5000)),
            Ok(ferriskey::Value::Int(4000)),
        ]);
        let result = parse_spend_result(&raw).unwrap();
        assert_eq!(
            result,
            SpendResult::HardBreach {
                dimension: "cost_microdollars".to_owned(),
                action: "block".to_owned(),
                current_usage: 5000,
                hard_limit: 4000,
            }
        );
    }

    #[test]
    fn spend_result_unknown_status_errors() {
        let raw = ferriskey::Value::Array(vec![Ok(ferriskey::Value::SimpleString(
            "GARBAGE".to_owned(),
        ))]);
        let result = parse_spend_result(&raw);
        assert!(result.is_err());
    }

    #[test]
    fn field_str_extracts_bulk_string() {
        let arr = vec![
            Ok(ferriskey::Value::BulkString(b"hello".to_vec().into())),
            Ok(ferriskey::Value::Int(42)),
        ];
        assert_eq!(field_str(&arr, 0), "hello");
        assert_eq!(field_str(&arr, 1), "42");
        assert_eq!(field_str(&arr, 99), "");
    }

    #[test]
    fn budget_status_default_values() {
        let status = BudgetStatus {
            budget_id: "test".to_owned(),
            scope_type: "run".to_owned(),
            scope_id: "run_1".to_owned(),
            enforcement_mode: "enforce".to_owned(),
            usage: HashMap::new(),
            hard_limits: HashMap::new(),
            soft_limits: HashMap::new(),
            breach_count: 0,
            soft_breach_count: 0,
        };
        assert_eq!(status.scope_type, "run");
        assert!(status.usage.is_empty());
    }

    #[test]
    fn standard_dimensions() {
        let dims = ["tokens", "cost_microdollars", "api_calls"];
        assert_eq!(dims.len(), 3);
        assert!(dims.contains(&"tokens"));
        assert!(dims.contains(&"cost_microdollars"));
        assert!(dims.contains(&"api_calls"));
    }
}
