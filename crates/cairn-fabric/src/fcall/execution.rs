use flowfabric::core::keys::{ExecKeyContext, IndexKeys};
use flowfabric::core::types::{
    AttemptIndex, ExecutionId, LaneId, Namespace, WaitpointId, WorkerInstanceId,
};

pub fn build_create_execution(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    lane_id: &LaneId,
    eid: &ExecutionId,
    namespace: &Namespace,
    execution_kind: &str,
    priority: &str,
    policy_json: &str,
    tags_json: &str,
    partition_index: u16,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.payload(),
        ctx.policy(),
        ctx.tags(),
        idx.lane_eligible(lane_id),
        ctx.noop(),
        idx.execution_deadline(),
        idx.all_executions(),
    ];
    let args = vec![
        eid.to_string(),
        namespace.to_string(),
        lane_id.to_string(),
        execution_kind.to_owned(),
        priority.to_owned(),
        "cairn".to_owned(),
        policy_json.to_owned(),
        String::new(),
        String::new(),
        "0".to_owned(),
        tags_json.to_owned(),
        String::new(),
        partition_index.to_string(),
    ];
    (keys, args)
}

pub fn build_complete_execution(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    att_idx: AttemptIndex,
    worker_instance_id: &WorkerInstanceId,
    lane_id: &LaneId,
    eid: &ExecutionId,
    lease_id: &str,
    lease_epoch: &str,
    attempt_id: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.attempt_hash(att_idx),
        idx.lease_expiry(),
        idx.worker_leases(worker_instance_id),
        idx.lane_terminal(lane_id),
        ctx.lease_current(),
        ctx.lease_history(),
        idx.lane_active(lane_id),
        ctx.stream_meta(att_idx),
        ctx.result(),
        idx.attempt_timeout(),
        idx.execution_deadline(),
    ];
    let args = vec![
        eid.to_string(),
        lease_id.to_owned(),
        lease_epoch.to_owned(),
        attempt_id.to_owned(),
        String::new(),
    ];
    (keys, args)
}

pub fn build_fail_execution(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    att_idx: AttemptIndex,
    worker_instance_id: &WorkerInstanceId,
    lane_id: &LaneId,
    eid: &ExecutionId,
    lease_id: &str,
    lease_epoch: &str,
    attempt_id: &str,
    reason: &str,
    category: &str,
    retry_policy_json: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.attempt_hash(att_idx),
        idx.lease_expiry(),
        idx.worker_leases(worker_instance_id),
        idx.lane_terminal(lane_id),
        idx.lane_delayed(lane_id),
        ctx.lease_current(),
        ctx.lease_history(),
        idx.lane_active(lane_id),
        ctx.stream_meta(att_idx),
        idx.attempt_timeout(),
        idx.execution_deadline(),
    ];
    let args = vec![
        eid.to_string(),
        lease_id.to_owned(),
        lease_epoch.to_owned(),
        attempt_id.to_owned(),
        reason.to_owned(),
        category.to_owned(),
        retry_policy_json.to_owned(),
    ];
    (keys, args)
}

pub fn build_cancel_execution(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    att_idx: AttemptIndex,
    worker_instance_id: &WorkerInstanceId,
    lane_id: &LaneId,
    wp_id: &WaitpointId,
    eid: &ExecutionId,
    reason: &str,
    source: &str,
    lease_id: &str,
    lease_epoch: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.attempt_hash(att_idx),
        ctx.stream_meta(att_idx),
        ctx.lease_current(),
        ctx.lease_history(),
        idx.lease_expiry(),
        idx.worker_leases(worker_instance_id),
        ctx.suspension_current(),
        ctx.waitpoint(wp_id),
        ctx.waitpoint_condition(wp_id),
        idx.suspension_timeout(),
        idx.lane_terminal(lane_id),
        idx.attempt_timeout(),
        idx.execution_deadline(),
        idx.lane_eligible(lane_id),
        idx.lane_delayed(lane_id),
        idx.lane_blocked_dependencies(lane_id),
        idx.lane_blocked_budget(lane_id),
        idx.lane_blocked_quota(lane_id),
        idx.lane_blocked_route(lane_id),
        idx.lane_blocked_operator(lane_id),
    ];
    let args = vec![
        eid.to_string(),
        reason.to_owned(),
        source.to_owned(),
        lease_id.to_owned(),
        lease_epoch.to_owned(),
    ];
    (keys, args)
}

pub const CREATE_EXECUTION_KEYS: usize = 8;
pub const CREATE_EXECUTION_ARGS: usize = 13;
pub const COMPLETE_EXECUTION_KEYS: usize = 12;
pub const COMPLETE_EXECUTION_ARGS: usize = 5;
pub const FAIL_EXECUTION_KEYS: usize = 12;
pub const FAIL_EXECUTION_ARGS: usize = 7;
pub const CANCEL_EXECUTION_KEYS: usize = 21;
pub const CANCEL_EXECUTION_ARGS: usize = 5;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_eid;
    use flowfabric::core::partition::{execution_partition, PartitionConfig};

    fn test_ctx() -> (ExecKeyContext, IndexKeys, ExecutionId) {
        let eid = test_eid("execution");
        let pc = PartitionConfig::default();
        let partition = execution_partition(&eid, &pc);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);
        (ctx, idx, eid)
    }

    #[test]
    fn create_execution_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("test");
        let ns = Namespace::new("ns");
        let (keys, args) =
            build_create_execution(&ctx, &idx, &lid, &eid, &ns, "run", "0", "{}", "{}", 0);
        assert_eq!(keys.len(), CREATE_EXECUTION_KEYS);
        assert_eq!(args.len(), CREATE_EXECUTION_ARGS);
    }

    #[test]
    fn complete_execution_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("test");
        let wid = WorkerInstanceId::new("w");
        let (keys, args) = build_complete_execution(
            &ctx,
            &idx,
            AttemptIndex::new(0),
            &wid,
            &lid,
            &eid,
            "",
            "1",
            "",
        );
        assert_eq!(keys.len(), COMPLETE_EXECUTION_KEYS);
        assert_eq!(args.len(), COMPLETE_EXECUTION_ARGS);
    }

    #[test]
    fn fail_execution_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("test");
        let wid = WorkerInstanceId::new("w");
        let (keys, args) = build_fail_execution(
            &ctx,
            &idx,
            AttemptIndex::new(0),
            &wid,
            &lid,
            &eid,
            "",
            "1",
            "",
            "err",
            "exec",
            "{}",
        );
        assert_eq!(keys.len(), FAIL_EXECUTION_KEYS);
        assert_eq!(args.len(), FAIL_EXECUTION_ARGS);
    }

    #[test]
    fn cancel_execution_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("test");
        let wid = WorkerInstanceId::new("w");
        let wp = WaitpointId::default();
        let (keys, args) = build_cancel_execution(
            &ctx,
            &idx,
            AttemptIndex::new(0),
            &wid,
            &lid,
            &wp,
            &eid,
            "cancel",
            "operator_override",
            "",
            "1",
        );
        assert_eq!(keys.len(), CANCEL_EXECUTION_KEYS);
        assert_eq!(args.len(), CANCEL_EXECUTION_ARGS);
    }
}
