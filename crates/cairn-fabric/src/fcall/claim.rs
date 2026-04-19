use ff_core::keys::{ExecKeyContext, IndexKeys};
use ff_core::types::{
    AttemptId, AttemptIndex, ExecutionId, LaneId, LeaseId, WorkerId, WorkerInstanceId,
};

pub fn build_issue_claim_grant(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    lane_id: &LaneId,
    eid: &ExecutionId,
    worker_id: &WorkerId,
    worker_instance_id: &WorkerInstanceId,
    grant_ttl_ms: u64,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![ctx.core(), ctx.claim_grant(), idx.lane_eligible(lane_id)];
    let args = vec![
        eid.to_string(),
        worker_id.to_string(),
        worker_instance_id.to_string(),
        lane_id.to_string(),
        String::new(),
        grant_ttl_ms.to_string(),
        String::new(),
        String::new(),
    ];
    (keys, args)
}

pub fn build_claim_execution(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    att_idx: AttemptIndex,
    worker_instance_id: &WorkerInstanceId,
    lane_id: &LaneId,
    eid: &ExecutionId,
    worker_id: &WorkerId,
    lease_id: &LeaseId,
    lease_duration_ms: u64,
    attempt_id: &AttemptId,
) -> (Vec<String>, Vec<String>) {
    let renew_before_ms = lease_duration_ms / 3;
    let keys = vec![
        ctx.core(),
        ctx.claim_grant(),
        idx.lane_eligible(lane_id),
        idx.lease_expiry(),
        idx.worker_leases(worker_instance_id),
        ctx.attempt_hash(att_idx),
        ctx.attempt_usage(att_idx),
        ctx.attempt_policy(att_idx),
        ctx.attempts(),
        ctx.lease_current(),
        ctx.lease_history(),
        idx.lane_active(lane_id),
        idx.attempt_timeout(),
        idx.execution_deadline(),
    ];
    let args = vec![
        eid.to_string(),
        worker_id.to_string(),
        worker_instance_id.to_string(),
        lane_id.to_string(),
        String::new(),
        lease_id.to_string(),
        lease_duration_ms.to_string(),
        renew_before_ms.to_string(),
        attempt_id.to_string(),
        "{}".to_owned(),
        String::new(),
        String::new(),
    ];
    (keys, args)
}

pub fn build_renew_lease(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    eid: &ExecutionId,
    att_idx: AttemptIndex,
    attempt_id: &str,
    lease_id: &str,
    lease_epoch: &str,
    lease_extension_ms: u64,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.lease_current(),
        ctx.lease_history(),
        idx.lease_expiry(),
    ];
    let args = vec![
        eid.to_string(),
        att_idx.to_string(),
        attempt_id.to_owned(),
        lease_id.to_owned(),
        lease_epoch.to_owned(),
        lease_extension_ms.to_string(),
        crate::constants::DEFAULT_LEASE_HISTORY_GRACE_MS.to_owned(),
    ];
    (keys, args)
}

pub const ISSUE_CLAIM_GRANT_KEYS: usize = 3;
pub const ISSUE_CLAIM_GRANT_ARGS: usize = 8;
pub const CLAIM_EXECUTION_KEYS: usize = 14;
pub const CLAIM_EXECUTION_ARGS: usize = 12;
pub const RENEW_LEASE_KEYS: usize = 4;
pub const RENEW_LEASE_ARGS: usize = 7;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_eid;
    use ff_core::partition::{execution_partition, PartitionConfig};

    fn test_ctx() -> (ExecKeyContext, IndexKeys, ExecutionId) {
        let eid = test_eid("claim");
        let pc = PartitionConfig::default();
        let partition = execution_partition(&eid, &pc);
        (
            ExecKeyContext::new(&partition, &eid),
            IndexKeys::new(&partition),
            eid,
        )
    }

    #[test]
    fn issue_claim_grant_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("t");
        let wid = WorkerId::new("w");
        let wiid = WorkerInstanceId::new("i");
        let (keys, args) = build_issue_claim_grant(&ctx, &idx, &lid, &eid, &wid, &wiid, 5000);
        assert_eq!(keys.len(), ISSUE_CLAIM_GRANT_KEYS);
        assert_eq!(args.len(), ISSUE_CLAIM_GRANT_ARGS);
    }

    #[test]
    fn claim_execution_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("t");
        let wid = WorkerId::new("w");
        let wiid = WorkerInstanceId::new("i");
        let lease = LeaseId::new();
        let att = AttemptId::new();
        let (keys, args) = build_claim_execution(
            &ctx,
            &idx,
            AttemptIndex::new(0),
            &wiid,
            &lid,
            &eid,
            &wid,
            &lease,
            30000,
            &att,
        );
        assert_eq!(keys.len(), CLAIM_EXECUTION_KEYS);
        assert_eq!(args.len(), CLAIM_EXECUTION_ARGS);
    }

    #[test]
    fn renew_lease_counts() {
        let (ctx, idx, eid) = test_ctx();
        let (keys, args) =
            build_renew_lease(&ctx, &idx, &eid, AttemptIndex::new(0), "", "", "1", 30000);
        assert_eq!(keys.len(), RENEW_LEASE_KEYS);
        assert_eq!(args.len(), RENEW_LEASE_ARGS);
    }
}
