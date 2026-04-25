use flowfabric::core::keys::QuotaKeyContext;
use flowfabric::core::types::{ExecutionId, QuotaPolicyId, TimestampMs};

pub fn build_create_quota_policy(
    ctx: &QuotaKeyContext,
    policies_index: &str,
    qid: &QuotaPolicyId,
    window_seconds: u64,
    max_requests_per_window: u64,
    max_concurrent: u64,
    now: TimestampMs,
    dimension: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.definition(),
        ctx.window(dimension),
        ctx.concurrency(),
        ctx.admitted_set(),
        policies_index.to_owned(),
    ];
    let args = vec![
        qid.to_string(),
        window_seconds.to_string(),
        max_requests_per_window.to_string(),
        max_concurrent.to_string(),
        now.to_string(),
    ];
    (keys, args)
}

pub fn build_check_admission(
    ctx: &QuotaKeyContext,
    execution_id: &ExecutionId,
    now: TimestampMs,
    window_seconds: u64,
    rate_limit: u64,
    concurrency_cap: u64,
    dimension: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.window(dimension),
        ctx.concurrency(),
        ctx.definition(),
        ctx.admitted(execution_id),
        ctx.admitted_set(),
    ];
    let args = vec![
        now.to_string(),
        window_seconds.to_string(),
        rate_limit.to_string(),
        concurrency_cap.to_string(),
        execution_id.to_string(),
        "0".to_owned(),
    ];
    (keys, args)
}

pub const CREATE_QUOTA_POLICY_KEYS: usize = 5;
pub const CREATE_QUOTA_POLICY_ARGS: usize = 5;
pub const CHECK_ADMISSION_KEYS: usize = 5;
pub const CHECK_ADMISSION_ARGS: usize = 6;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_eid;
    use flowfabric::core::partition::{quota_partition, PartitionConfig};

    #[test]
    fn create_quota_policy_counts() {
        let qid = QuotaPolicyId::new();
        let pc = PartitionConfig::default();
        let partition = quota_partition(&qid, &pc);
        let ctx = QuotaKeyContext::new(&partition, &qid);
        let (keys, args) = build_create_quota_policy(
            &ctx,
            "ff:idx:policies",
            &qid,
            60,
            100,
            10,
            TimestampMs::now(),
            "default",
        );
        assert_eq!(keys.len(), CREATE_QUOTA_POLICY_KEYS);
        assert_eq!(args.len(), CREATE_QUOTA_POLICY_ARGS);
    }

    #[test]
    fn check_admission_counts() {
        let qid = QuotaPolicyId::new();
        let pc = PartitionConfig::default();
        let partition = quota_partition(&qid, &pc);
        let ctx = QuotaKeyContext::new(&partition, &qid);
        let eid = test_eid("check_admission");
        let (keys, args) =
            build_check_admission(&ctx, &eid, TimestampMs::now(), 60, 100, 10, "default");
        assert_eq!(keys.len(), CHECK_ADMISSION_KEYS);
        assert_eq!(args.len(), CHECK_ADMISSION_ARGS);
    }
}
