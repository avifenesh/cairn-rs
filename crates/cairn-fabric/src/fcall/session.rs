use flowfabric::core::keys::{FlowIndexKeys, FlowKeyContext};
use flowfabric::core::partition::Partition;
use flowfabric::core::types::{FlowId, Namespace, TimestampMs};

pub fn build_create_flow(
    fctx: &FlowKeyContext,
    partition: &Partition,
    fid: &FlowId,
    flow_kind: &str,
    namespace: &Namespace,
    now: TimestampMs,
) -> (Vec<String>, Vec<String>) {
    // FF ff_create_flow @a098710 expects KEYS(3): flow_core, members_set, flow_index.
    // See lua/flow.lua:69-74. flow_index is SADD'd before the idempotency guard to
    // heal pre-existing flow_core records.
    let idx = FlowIndexKeys::new(partition);
    let keys = vec![fctx.core(), fctx.members(), idx.flow_index()];
    let args = vec![
        fid.to_string(),
        flow_kind.to_owned(),
        namespace.to_string(),
        now.to_string(),
    ];
    (keys, args)
}

pub fn build_cancel_flow(
    fctx: &FlowKeyContext,
    fid: &FlowId,
    reason: &str,
    cancellation_policy: &str,
    now: TimestampMs,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![fctx.core(), fctx.members()];
    let args = vec![
        fid.to_string(),
        reason.to_owned(),
        cancellation_policy.to_owned(),
        now.to_string(),
    ];
    (keys, args)
}

pub const CREATE_FLOW_KEYS: usize = 3;
pub const CREATE_FLOW_ARGS: usize = 4;
pub const CANCEL_FLOW_KEYS: usize = 2;
pub const CANCEL_FLOW_ARGS: usize = 4;

#[cfg(test)]
mod tests {
    use super::*;
    use flowfabric::core::partition::{flow_partition, PartitionConfig};

    #[test]
    fn create_flow_counts() {
        let fid = FlowId::from_uuid(uuid::Uuid::nil());
        let pc = PartitionConfig::default();
        let partition = flow_partition(&fid, &pc);
        let fctx = FlowKeyContext::new(&partition, &fid);
        let ns = Namespace::new("ns");
        let (keys, args) = build_create_flow(
            &fctx,
            &partition,
            &fid,
            "cairn_session",
            &ns,
            TimestampMs::now(),
        );
        assert_eq!(keys.len(), CREATE_FLOW_KEYS);
        assert_eq!(args.len(), CREATE_FLOW_ARGS);
    }

    #[test]
    fn cancel_flow_counts() {
        let fid = FlowId::from_uuid(uuid::Uuid::nil());
        let pc = PartitionConfig::default();
        let partition = flow_partition(&fid, &pc);
        let fctx = FlowKeyContext::new(&partition, &fid);
        let (keys, args) =
            build_cancel_flow(&fctx, &fid, "archived", "cancel_all", TimestampMs::now());
        assert_eq!(keys.len(), CANCEL_FLOW_KEYS);
        assert_eq!(args.len(), CANCEL_FLOW_ARGS);
    }
}
