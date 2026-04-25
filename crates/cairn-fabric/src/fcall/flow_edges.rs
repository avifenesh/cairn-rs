//! Builders for the FF flow-edge FCALLs (RFC-007 / Batch C).
//!
//! Four functions wire dependency edges between executions in a flow:
//!
//! - `ff_add_execution_to_flow` — on `{fp:N}` — add an execution to the
//!   flow's members set. Required before any edge can reference the
//!   execution as an endpoint. Idempotent (`ok_already_satisfied`).
//! - `ff_stage_dependency_edge` — on `{fp:N}` — declare an edge.
//!   Returns `stale_graph_revision` under concurrent declarers so
//!   callers retry with a fresh revision.
//! - `ff_apply_dependency_to_child` — on the child's `{p:N}` (aliased to
//!   the same `{fp:N}` tag for session-bound executions — see
//!   `PartitionFamily` docs) — register the edge on the child's
//!   deps_meta and, if the child is runnable, block it.
//! - `ff_evaluate_flow_eligibility` — read-only, on child's partition.
//!   Returns `"eligible"` / `"blocked_by_dependencies"` / etc.
//!
//! Partition invariant: because cairn mints session-bound exec ids via
//! `ExecutionId::deterministic_for_flow`, the child's execution
//! partition shares its hash tag with the flow partition, so all four
//! FCALLs are CROSSSLOT-safe as long as both endpoints belong to the
//! same flow.

use flowfabric::core::keys::{ExecKeyContext, FlowIndexKeys, FlowKeyContext};
use flowfabric::core::types::{EdgeId, ExecutionId, FlowId};

// ── ff_add_execution_to_flow ────────────────────────────────────────────────

pub const ADD_EXECUTION_TO_FLOW_KEYS: usize = 4;
pub const ADD_EXECUTION_TO_FLOW_ARGS: usize = 3;

pub fn build_add_execution_to_flow(
    fctx: &FlowKeyContext,
    flow_idx: &FlowIndexKeys,
    exec_ctx: &ExecKeyContext,
    fid: &FlowId,
    eid: &ExecutionId,
    now_ms: u64,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        fctx.core(),
        fctx.members(),
        flow_idx.flow_index(),
        exec_ctx.core(),
    ];
    let args = vec![fid.to_string(), eid.to_string(), now_ms.to_string()];
    (keys, args)
}

// ── ff_stage_dependency_edge ────────────────────────────────────────────────

pub const STAGE_DEPENDENCY_EDGE_KEYS: usize = 6;
pub const STAGE_DEPENDENCY_EDGE_ARGS: usize = 8;

#[allow(clippy::too_many_arguments)]
pub fn build_stage_dependency_edge(
    fctx: &FlowKeyContext,
    fid: &FlowId,
    edge_id: &EdgeId,
    upstream_eid: &ExecutionId,
    downstream_eid: &ExecutionId,
    dependency_kind: &str,
    data_passing_ref: &str,
    expected_graph_revision: u64,
    now_ms: u64,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        fctx.core(),
        fctx.members(),
        fctx.edge(edge_id),
        fctx.outgoing(upstream_eid),
        fctx.incoming(downstream_eid),
        // `grant_hash` is stored on the flow partition too; we use the
        // edge_id as the grant key slot since the cairn path doesn't
        // mint separate grants. FF's Lua accepts any key here — it
        // only writes to it on certain policy-gated paths we don't
        // exercise.
        fctx.grant(edge_id.to_string().as_str()),
    ];
    let args = vec![
        fid.to_string(),
        edge_id.to_string(),
        upstream_eid.to_string(),
        downstream_eid.to_string(),
        dependency_kind.to_owned(),
        data_passing_ref.to_owned(),
        expected_graph_revision.to_string(),
        now_ms.to_string(),
    ];
    (keys, args)
}

// ── ff_apply_dependency_to_child ────────────────────────────────────────────

pub const APPLY_DEPENDENCY_TO_CHILD_KEYS: usize = 7;
pub const APPLY_DEPENDENCY_TO_CHILD_ARGS: usize = 7;

#[allow(clippy::too_many_arguments)]
pub fn build_apply_dependency_to_child(
    child_exec_ctx: &ExecKeyContext,
    lane_eligible: &str,
    lane_blocked_deps: &str,
    edge_id: &EdgeId,
    fid: &FlowId,
    upstream_eid: &ExecutionId,
    graph_revision: u64,
    dependency_kind: &str,
    data_passing_ref: &str,
    now_ms: u64,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![
        child_exec_ctx.core(),
        child_exec_ctx.deps_meta(),
        child_exec_ctx.deps_unresolved(),
        child_exec_ctx.dep_edge(edge_id),
        lane_eligible.to_owned(),
        lane_blocked_deps.to_owned(),
        child_exec_ctx.deps_all_edges(),
    ];
    let args = vec![
        fid.to_string(),
        edge_id.to_string(),
        upstream_eid.to_string(),
        graph_revision.to_string(),
        dependency_kind.to_owned(),
        data_passing_ref.to_owned(),
        now_ms.to_string(),
    ];
    (keys, args)
}

// ── ff_evaluate_flow_eligibility ────────────────────────────────────────────

pub const EVALUATE_FLOW_ELIGIBILITY_KEYS: usize = 2;
pub const EVALUATE_FLOW_ELIGIBILITY_ARGS: usize = 0;

pub fn build_evaluate_flow_eligibility(
    child_exec_ctx: &ExecKeyContext,
) -> (Vec<String>, Vec<String>) {
    let keys = vec![child_exec_ctx.core(), child_exec_ctx.deps_meta()];
    (keys, vec![])
}

#[cfg(test)]
mod tests {
    use super::*;
    use flowfabric::core::partition::{Partition, PartitionFamily};

    fn test_partition() -> Partition {
        Partition {
            family: PartitionFamily::Flow,
            index: 7,
        }
    }

    #[test]
    fn add_execution_to_flow_key_count() {
        let part = test_partition();
        let fid = FlowId::new();
        // `for_flow` mints an exec id co-located with the flow.
        let eid = ExecutionId::for_flow(&fid, &Default::default());
        let fctx = FlowKeyContext::new(&part, &fid);
        let flow_idx = FlowIndexKeys::new(&part);
        let exec_ctx = ExecKeyContext::new(&part, &eid);
        let (keys, args) =
            build_add_execution_to_flow(&fctx, &flow_idx, &exec_ctx, &fid, &eid, 1_000);
        assert_eq!(keys.len(), ADD_EXECUTION_TO_FLOW_KEYS);
        assert_eq!(args.len(), ADD_EXECUTION_TO_FLOW_ARGS);
    }

    #[test]
    fn stage_dependency_edge_key_count() {
        let part = test_partition();
        let fid = FlowId::new();
        let up = ExecutionId::for_flow(&fid, &Default::default());
        let down = ExecutionId::for_flow(&fid, &Default::default());
        let edge = EdgeId::new();
        let fctx = FlowKeyContext::new(&part, &fid);
        let (keys, args) = build_stage_dependency_edge(
            &fctx,
            &fid,
            &edge,
            &up,
            &down,
            "success_only",
            "",
            0,
            1_000,
        );
        assert_eq!(keys.len(), STAGE_DEPENDENCY_EDGE_KEYS);
        assert_eq!(args.len(), STAGE_DEPENDENCY_EDGE_ARGS);
    }

    #[test]
    fn apply_dependency_to_child_key_count() {
        let part = test_partition();
        let fid = FlowId::new();
        let up = ExecutionId::for_flow(&fid, &Default::default());
        let down = ExecutionId::for_flow(&fid, &Default::default());
        let edge = EdgeId::new();
        let exec_ctx = ExecKeyContext::new(&part, &down);
        let (keys, args) = build_apply_dependency_to_child(
            &exec_ctx,
            "lane-eligible",
            "lane-blocked",
            &edge,
            &fid,
            &up,
            0,
            "success_only",
            "",
            1_000,
        );
        assert_eq!(keys.len(), APPLY_DEPENDENCY_TO_CHILD_KEYS);
        assert_eq!(args.len(), APPLY_DEPENDENCY_TO_CHILD_ARGS);
    }

    #[test]
    fn evaluate_flow_eligibility_key_count() {
        let part = test_partition();
        let fid = FlowId::new();
        let eid = ExecutionId::for_flow(&fid, &Default::default());
        let exec_ctx = ExecKeyContext::new(&part, &eid);
        let (keys, args) = build_evaluate_flow_eligibility(&exec_ctx);
        assert_eq!(keys.len(), EVALUATE_FLOW_ELIGIBILITY_KEYS);
        assert_eq!(args.len(), EVALUATE_FLOW_ELIGIBILITY_ARGS);
    }
}
