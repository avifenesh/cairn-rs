use std::collections::HashMap;
use std::sync::Arc;

use crate::error::FabricError;
use crate::helpers::check_fcall_success;
use ff_core::contracts::ReportUsageResult;
use ff_core::keys::{budget_policies_index, budget_resets_key, usage_dedup_key, BudgetKeyContext};
use ff_core::partition::budget_partition;
use ff_core::types::{BudgetId, ExecutionId, TimestampMs};
use ff_sdk::task::parse_report_usage_result;
use uuid::Uuid;

use crate::boot::FabricRuntime;

// Stable namespace UUID for spend-dedup keys. Mirrors the UUID v5 +
// null-byte-delimited scheme from id_map.rs. Changing these bytes orphans any
// in-flight idempotency record.
const SPEND_NAMESPACE: Uuid = Uuid::from_bytes([
    0xb7, 0x1a, 0x2e, 0x04, 0x9c, 0x85, 0x45, 0xc3, 0x88, 0xd9, 0x0f, 0x4a, 0x6b, 0x2c, 0x13, 0x77,
]);

const SPEND_NAMESPACE_VERSION: u8 = 1;

/// Derive a stable idempotency key for a spend call.
///
/// Stable across retries for the same (budget, execution, dimension set/amount).
/// Callers that repeat a spend with identical inputs produce an identical key,
/// so FF dedups server-side via the `dedup_key` ARGV slot of
/// `ff_report_usage_and_check`.
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
    sorted.sort_by_key(|r| r.0);

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

/// Budget service.
///
/// **Lean-bridge silence (intentional).** None of this service's methods emit
/// `BridgeEvent`s — budget state is FF-owned operational state with no
/// `BudgetReadModel` projection on the cairn-store side. `create_budget`,
/// `release_budget`, `record_spend`, and `get_budget_status` all route
/// through FF directly; admin reads go via `get_budget_status` (HGETALL on
/// FF's definition + usage hashes), not via a cairn projection.
///
/// `record_spend` is additionally volume-sensitive — it fires on every tool
/// call / LLM token charge. Even if a `BudgetSpendRecorded` projection
/// existed, the bridge-event channel would saturate first. Spend outcomes
/// are returned inline in `ReportUsageResult` for the caller.
///
/// If a future cairn surface projects budgets (e.g. history timeline, breach
/// replay), introduce BridgeEvent variants and revisit. Until then:
/// additions here must not emit.
///
/// See `docs/design/bridge-event-audit.md` §2.6 for the full rationale.
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
        let policies_index = budget_policies_index(&partition.hash_tag());
        let now = TimestampMs::now();

        let (keys, argv) = crate::fcall::budget::build_create_budget(
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
            .runtime
            .fcall(crate::fcall::names::FF_CREATE_BUDGET, &key_refs, &argv_refs)
            .await?;
        check_fcall_success(&raw, crate::fcall::names::FF_CREATE_BUDGET)?;

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

        let raw: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_RESET_BUDGET, &key_refs, &argv_refs)
            .await?;
        check_fcall_success(&raw, crate::fcall::names::FF_RESET_BUDGET)?;

        Ok(())
    }

    /// Record spend against a budget.
    ///
    /// Pass-through to `ff_report_usage_and_check`. The result is FF's
    /// [`ReportUsageResult`] — callers match on the variants directly; cairn
    /// does not re-derive or unify the wire shape.
    ///
    /// `execution_id` is REQUIRED. Two calls that share an idempotency key
    /// are treated by FF as the same spend (the second returns
    /// [`ReportUsageResult::AlreadyApplied`] and the budget is not
    /// double-decremented). The key's caller-identity component comes from
    /// the ExecutionId, so every distinct logical spend MUST present a
    /// distinct ExecutionId. Tests without a real execution must mint a
    /// throwaway via [`ExecutionId::deterministic_solo`] with a fresh
    /// UUID; silently falling back to a
    /// sentinel (`Uuid::nil`) would make two unrelated process-level
    /// retries collide into a single FF dedup slot and suppress a
    /// legitimate spend.
    pub async fn record_spend(
        &self,
        budget_id: &BudgetId,
        execution_id: &ExecutionId,
        dimension_deltas: &[(&str, u64)],
    ) -> Result<ReportUsageResult, FabricError> {
        if dimension_deltas.is_empty() {
            return Err(FabricError::Validation {
                reason: "record_spend: at least one dimension_delta is required".to_owned(),
            });
        }

        let partition = budget_partition(budget_id, &self.runtime.partition_config);
        let ctx = BudgetKeyContext::new(&partition, budget_id);
        let now = TimestampMs::now();

        let idempotency_key =
            compute_spend_idempotency_key(budget_id, execution_id, dimension_deltas);

        // Prefix with the budget's `{b:M}` hash tag so FF's SET lands on the
        // same slot as the budget itself — matches ff-sdk task.rs:699-702.
        let dedup_key = usage_dedup_key(ctx.hash_tag(), &idempotency_key);

        let (keys, argv) =
            crate::fcall::budget::build_report_usage(&ctx, dimension_deltas, now, &dedup_key);

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

        // FF owns the wire format for ff_report_usage_and_check; cairn just
        // surfaces the result. `parse_report_usage_result` is strict —
        // malformed u64 fields fail loudly instead of silently coercing to 0,
        // which matches the semantics we want (drift = error, not fake
        // zero-usage breach).
        parse_report_usage_result(&raw).map_err(|e| FabricError::Internal(e.to_string()))
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

        // FF writes budget_limits fields as "hard:<dim>" / "soft:<dim>"
        // (prefix), see FlowFabric lua/budget.lua:61. Earlier versions of
        // this code used "<dim>:hard" (suffix) and silently returned empty
        // limits — caught by test_budget_status_reflects_spend.
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

#[cfg(test)]
mod tests {
    use super::*;
    use ff_core::partition::PartitionConfig;
    use ff_core::types::LaneId;

    fn test_eid(seed: &str) -> ExecutionId {
        let uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, seed.as_bytes());
        ExecutionId::deterministic_solo(&LaneId::new("test"), &PartitionConfig::default(), uuid)
    }

    #[test]
    fn idempotency_key_stable_for_same_inputs() {
        let bid = BudgetId::new();
        let eid = test_eid("stable");
        let k1 = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        assert_eq!(k1, k2);
        // UUID v5 is 36 chars with hyphens.
        assert_eq!(k1.len(), 36);
    }

    #[test]
    fn idempotency_key_differs_when_inputs_change() {
        let bid = BudgetId::new();
        let eid = test_eid("differs");
        let k_tokens = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 50)]);
        let k_cost = compute_spend_idempotency_key(&bid, &eid, &[("cost", 50)]);
        let k_amount = compute_spend_idempotency_key(&bid, &eid, &[("tokens", 51)]);
        assert_ne!(k_tokens, k_cost);
        assert_ne!(k_tokens, k_amount);
    }

    #[test]
    fn idempotency_key_order_independent_for_same_dimension_set() {
        let bid = BudgetId::new();
        let eid = test_eid("order");
        let k_ab = compute_spend_idempotency_key(&bid, &eid, &[("a", 1), ("b", 2)]);
        let k_ba = compute_spend_idempotency_key(&bid, &eid, &[("b", 2), ("a", 1)]);
        assert_eq!(k_ab, k_ba);
    }

    #[test]
    fn idempotency_key_isolates_execution() {
        let bid = BudgetId::new();
        let eid1 = test_eid("exec1");
        let eid2 = test_eid("exec2");
        let k1 = compute_spend_idempotency_key(&bid, &eid1, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid2, &[("tokens", 50)]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn idempotency_key_isolates_budget() {
        let b1 = BudgetId::new();
        let b2 = BudgetId::new();
        let eid = test_eid("budget_iso");
        let k1 = compute_spend_idempotency_key(&b1, &eid, &[("tokens", 50)]);
        let k2 = compute_spend_idempotency_key(&b2, &eid, &[("tokens", 50)]);
        assert_ne!(k1, k2);
    }

    #[test]
    fn idempotency_key_no_delimiter_collision() {
        // Null-byte delimiters prevent "a:b"+"c" vs "a"+"b:c" style collisions,
        // same pattern id_map.rs guards against.
        let bid = BudgetId::new();
        let eid = test_eid("delim");
        let k1 = compute_spend_idempotency_key(&bid, &eid, &[("a:b", 1), ("c", 2)]);
        let k2 = compute_spend_idempotency_key(&bid, &eid, &[("a", 1), ("b:c", 2)]);
        assert_ne!(k1, k2);
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
