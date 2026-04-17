use ff_core::keys::BudgetKeyContext;
use ff_core::types::{BudgetId, TimestampMs};

#[allow(clippy::too_many_arguments)]
pub fn build_create_budget(
    ctx: &BudgetKeyContext,
    resets_zset: &str,
    policies_index: &str,
    budget_id: &BudgetId,
    scope_type: &str,
    scope_id: &str,
    enforcement_mode: &str,
    on_hard_limit: &str,
    on_soft_limit: &str,
    reset_interval_ms: u64,
    now: TimestampMs,
    dimensions: &[&str],
    hard_limits: &[u64],
    soft_limits: &[u64],
) -> (Vec<String>, Vec<String>) {
    // FF ff_create_budget @a098710 expects KEYS(5): def, limits, usage,
    // resets_zset, policies_index. See lua/budget.lua:18-25. policies_index
    // is SADD'd before the idempotency guard to heal pre-existing budget_def
    // records.
    let keys = vec![
        ctx.definition(),
        ctx.limits(),
        ctx.usage(),
        resets_zset.to_owned(),
        policies_index.to_owned(),
    ];
    let dim_count = dimensions.len();
    let mut args: Vec<String> = Vec::with_capacity(9 + dim_count * 3);
    args.push(budget_id.to_string());
    args.push(scope_type.to_owned());
    args.push(scope_id.to_owned());
    args.push(enforcement_mode.to_owned());
    args.push(on_hard_limit.to_owned());
    args.push(on_soft_limit.to_owned());
    args.push(reset_interval_ms.to_string());
    args.push(now.to_string());
    args.push(dim_count.to_string());
    for dim in dimensions {
        args.push((*dim).to_owned());
    }
    for &hard in hard_limits {
        args.push(hard.to_string());
    }
    for &soft in soft_limits {
        args.push(soft.to_string());
    }
    (keys, args)
}

/// Build KEYS and ARGV for `ff_report_usage_and_check`.
///
/// `dedup_key` is REQUIRED. FF reads `ARGV[2 * dim_count + 3]`; if the slot is
/// empty or missing, server-side dedup is silently disabled and a double-submit
/// will double-decrement the budget. Callers must pass a caller-prefixed key
/// (typically `"ff:usagededup:{hash_tag}:{uuid}"`) so FF's SET lands on the
/// same slot as the budget itself. Pass `""` only when you explicitly want to
/// disable dedup (integration tests, one-off admin spends) — never for
/// production run-service spend paths.
pub fn build_report_usage(
    ctx: &BudgetKeyContext,
    dimension_deltas: &[(&str, u64)],
    now: TimestampMs,
    dedup_key: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![ctx.usage(), ctx.limits(), ctx.definition()];
    let dim_count = dimension_deltas.len();
    let mut args: Vec<String> = Vec::with_capacity(3 + dim_count * 2);
    args.push(dim_count.to_string());
    for (dim, _) in dimension_deltas {
        args.push((*dim).to_owned());
    }
    for (_, delta) in dimension_deltas {
        args.push(delta.to_string());
    }
    args.push(now.to_string());
    args.push(dedup_key.to_owned());
    (keys, args)
}

pub const CREATE_BUDGET_KEYS: usize = 5;
pub const REPORT_USAGE_KEYS: usize = 3;
pub const RESET_BUDGET_KEYS: usize = 3;
pub const RESET_BUDGET_ARGS: usize = 2;

/// ARGV layout for `build_report_usage`: dim_count + N dims + N deltas + now + dedup_key.
pub const REPORT_USAGE_FIXED_ARGS: usize = 3;

#[cfg(test)]
mod tests {
    use super::*;
    use ff_core::partition::{budget_partition, PartitionConfig};

    #[test]
    fn create_budget_counts() {
        let bid = BudgetId::new();
        let pc = PartitionConfig::default();
        let partition = budget_partition(&bid, &pc);
        let ctx = BudgetKeyContext::new(&partition, &bid);
        let (keys, args) = build_create_budget(
            &ctx,
            "ff:resets",
            "ff:policies",
            &bid,
            "run",
            "r1",
            "enforce",
            "block",
            "log",
            0,
            TimestampMs::now(),
            &["tokens", "cost"],
            &[1000, 500],
            &[800, 400],
        );
        assert_eq!(keys.len(), CREATE_BUDGET_KEYS);
        assert_eq!(args.len(), 9 + 2 * 3);
    }

    #[test]
    fn report_usage_counts() {
        let bid = BudgetId::new();
        let pc = PartitionConfig::default();
        let partition = budget_partition(&bid, &pc);
        let ctx = BudgetKeyContext::new(&partition, &bid);
        let (keys, args) = build_report_usage(
            &ctx,
            &[("tokens", 100), ("cost", 50)],
            TimestampMs::now(),
            "ff:usagededup:{b:0}:test-key",
        );
        assert_eq!(keys.len(), REPORT_USAGE_KEYS);
        // dim_count + 2 dims + 2 deltas + now + dedup_key = 7
        assert_eq!(args.len(), REPORT_USAGE_FIXED_ARGS + 2 * 2);
        assert_eq!(args.last().unwrap(), "ff:usagededup:{b:0}:test-key");
    }

    #[test]
    fn report_usage_dedup_key_lands_in_last_slot() {
        // FF lua/budget.lua reads args[2 * dim_count + 3] as the dedup_key.
        // Pin the slot position so a reorder in the builder is caught.
        let bid = BudgetId::new();
        let pc = PartitionConfig::default();
        let partition = budget_partition(&bid, &pc);
        let ctx = BudgetKeyContext::new(&partition, &bid);
        let (_keys, args) = build_report_usage(
            &ctx,
            &[("tokens", 1)],
            TimestampMs::from_millis(1_700_000_000_000),
            "sentinel-dedup",
        );
        let dim_count: usize = args[0].parse().unwrap();
        assert_eq!(
            args[2 * dim_count + 2],
            "sentinel-dedup",
            "dedup_key must occupy ARGV[2*dim_count + 3] (zero-indexed: 2*dim_count + 2)",
        );
    }
}
