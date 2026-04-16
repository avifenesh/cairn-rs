use ff_core::keys::BudgetKeyContext;
use ff_core::types::{BudgetId, TimestampMs};

#[allow(clippy::too_many_arguments)]
pub fn build_create_budget(
    ctx: &BudgetKeyContext,
    resets_zset: &str,
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
    let keys = vec![
        ctx.definition(),
        ctx.limits(),
        ctx.usage(),
        resets_zset.to_owned(),
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

pub fn build_report_usage(
    ctx: &BudgetKeyContext,
    dimension_deltas: &[(&str, u64)],
    now: TimestampMs,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![ctx.usage(), ctx.limits(), ctx.definition()];
    let dim_count = dimension_deltas.len();
    let mut args: Vec<String> = Vec::with_capacity(2 + dim_count * 2);
    args.push(dim_count.to_string());
    for (dim, _) in dimension_deltas {
        args.push((*dim).to_owned());
    }
    for (_, delta) in dimension_deltas {
        args.push(delta.to_string());
    }
    args.push(now.to_string());
    (keys, args)
}

pub const CREATE_BUDGET_KEYS: usize = 4;
pub const REPORT_USAGE_KEYS: usize = 3;

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
        let (keys, args) =
            build_report_usage(&ctx, &[("tokens", 100), ("cost", 50)], TimestampMs::now());
        assert_eq!(keys.len(), REPORT_USAGE_KEYS);
        assert_eq!(args.len(), 1 + 2 + 2 + 1);
    }
}
