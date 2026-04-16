use ff_core::keys::{ExecKeyContext, IndexKeys};
use ff_core::types::{
    AttemptIndex, ExecutionId, LaneId, SignalId, SuspensionId, TimestampMs, WaitpointId,
    WorkerInstanceId,
};

#[allow(clippy::too_many_arguments)]
pub fn build_suspend_execution(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    att_idx: AttemptIndex,
    worker_instance_id: &WorkerInstanceId,
    lane_id: &LaneId,
    waitpoint_id: &WaitpointId,
    eid: &ExecutionId,
    attempt_id: &str,
    lease_id: &str,
    lease_epoch: &str,
    suspension_id: &SuspensionId,
    waitpoint_key: &str,
    reason_code: &str,
    timeout_at: &str,
    resume_condition_json: &str,
    resume_policy_json: &str,
    timeout_behavior: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.attempt_hash(att_idx),
        ctx.lease_current(),
        ctx.lease_history(),
        idx.lease_expiry(),
        idx.worker_leases(worker_instance_id),
        ctx.suspension_current(),
        ctx.waitpoint(waitpoint_id),
        ctx.waitpoint_signals(waitpoint_id),
        idx.suspension_timeout(),
        idx.pending_waitpoint_expiry(),
        idx.lane_active(lane_id),
        idx.lane_suspended(lane_id),
        ctx.waitpoints(),
        ctx.waitpoint_condition(waitpoint_id),
        idx.attempt_timeout(),
    ];
    let args = vec![
        eid.to_string(),
        att_idx.to_string(),
        attempt_id.to_owned(),
        lease_id.to_owned(),
        lease_epoch.to_owned(),
        suspension_id.to_string(),
        waitpoint_id.to_string(),
        waitpoint_key.to_owned(),
        reason_code.to_owned(),
        crate::constants::SOURCE_IDENTITY.to_owned(),
        timeout_at.to_owned(),
        resume_condition_json.to_owned(),
        resume_policy_json.to_owned(),
        String::new(), // continuation_metadata_pointer (unused in v1)
        String::new(), // use_pending_waitpoint (unused in v1)
        timeout_behavior.to_owned(),
        crate::constants::DEFAULT_LEASE_HISTORY_MAXLEN.to_owned(),
    ];
    (keys, args)
}

pub fn build_resume_execution(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    lane_id: &LaneId,
    wp_id: &WaitpointId,
    eid: &ExecutionId,
    trigger_type: &str,
    resume_delay_ms: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.suspension_current(),
        ctx.waitpoint(wp_id),
        ctx.waitpoint_signals(wp_id),
        idx.suspension_timeout(),
        idx.lane_eligible(lane_id),
        idx.lane_delayed(lane_id),
        idx.lane_suspended(lane_id),
    ];
    let args = vec![
        eid.to_string(),
        trigger_type.to_owned(),
        resume_delay_ms.to_owned(),
    ];
    (keys, args)
}

#[allow(clippy::too_many_arguments)]
pub fn build_deliver_signal(
    ctx: &ExecKeyContext,
    idx: &IndexKeys,
    lane_id: &LaneId,
    signal_id: &SignalId,
    waitpoint_id: &WaitpointId,
    idem_key: String,
    eid: &ExecutionId,
    signal_name: String,
    signal_category: String,
    source_type: String,
    source_identity: String,
    payload_str: String,
    idempotency_key: String,
    now: TimestampMs,
    dedup_ttl_ms: u64,
    signal_maxlen: &str,
    max_signals_per_execution: &str,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        ctx.core(),
        ctx.waitpoint_condition(waitpoint_id),
        ctx.waitpoint_signals(waitpoint_id),
        ctx.exec_signals(),
        ctx.signal(signal_id),
        ctx.signal_payload(signal_id),
        idem_key,
        ctx.waitpoint(waitpoint_id),
        ctx.suspension_current(),
        idx.lane_eligible(lane_id),
        idx.lane_suspended(lane_id),
        idx.lane_delayed(lane_id),
        idx.suspension_timeout(),
    ];
    let args = vec![
        signal_id.to_string(),
        eid.to_string(),
        waitpoint_id.to_string(),
        signal_name,
        signal_category,
        source_type,
        source_identity,
        payload_str,
        "json".to_owned(),
        idempotency_key,
        String::new(),
        "waitpoint".to_owned(),
        now.to_string(),
        dedup_ttl_ms.to_string(),
        "0".to_owned(),
        signal_maxlen.to_owned(),
        max_signals_per_execution.to_owned(),
    ];
    (keys, args)
}

pub const SUSPEND_EXECUTION_KEYS: usize = 16;
pub const SUSPEND_EXECUTION_ARGS: usize = 17;
pub const RESUME_EXECUTION_KEYS: usize = 8;
pub const RESUME_EXECUTION_ARGS: usize = 3;
pub const DELIVER_SIGNAL_KEYS: usize = 13;
pub const DELIVER_SIGNAL_ARGS: usize = 17;

#[cfg(test)]
mod tests {
    use super::*;
    use ff_core::partition::{execution_partition, PartitionConfig};

    fn test_ctx() -> (ExecKeyContext, IndexKeys, ExecutionId) {
        let eid = ExecutionId::from_uuid(uuid::Uuid::nil());
        let pc = PartitionConfig::default();
        let partition = execution_partition(&eid, &pc);
        (
            ExecKeyContext::new(&partition, &eid),
            IndexKeys::new(&partition),
            eid,
        )
    }

    #[test]
    fn suspend_execution_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("t");
        let wid = WorkerInstanceId::new("w");
        let wp = WaitpointId::default();
        let sid = SuspensionId::new();
        let (keys, args) = build_suspend_execution(
            &ctx,
            &idx,
            AttemptIndex::new(0),
            &wid,
            &lid,
            &wp,
            &eid,
            "",
            "",
            "1",
            &sid,
            "wpk:x",
            "test",
            "",
            "{}",
            "{}",
            "fail",
        );
        assert_eq!(keys.len(), SUSPEND_EXECUTION_KEYS);
        assert_eq!(args.len(), SUSPEND_EXECUTION_ARGS);
    }

    #[test]
    fn resume_execution_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("t");
        let wp = WaitpointId::default();
        let (keys, args) = build_resume_execution(&ctx, &idx, &lid, &wp, &eid, "operator", "0");
        assert_eq!(keys.len(), RESUME_EXECUTION_KEYS);
        assert_eq!(args.len(), RESUME_EXECUTION_ARGS);
    }

    #[test]
    fn deliver_signal_counts() {
        let (ctx, idx, eid) = test_ctx();
        let lid = LaneId::new("t");
        let sig = SignalId::new();
        let wp = WaitpointId::default();
        let (keys, args) = build_deliver_signal(
            &ctx,
            &idx,
            &lid,
            &sig,
            &wp,
            "idem".into(),
            &eid,
            "sig".into(),
            "cat".into(),
            "src".into(),
            "id".into(),
            "".into(),
            "idem".into(),
            TimestampMs::now(),
            86400000,
            "1000",
            "10000",
        );
        assert_eq!(keys.len(), DELIVER_SIGNAL_KEYS);
        assert_eq!(args.len(), DELIVER_SIGNAL_ARGS);
    }
}
