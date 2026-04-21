//! Valkey-backed [`Engine`] implementation.
//!
//! This is the **one file** in cairn that knows FF is Valkey-backed.
//! It holds a `ferriskey::Client` handle, performs direct HGETALL /
//! SMEMBERS reads against FF's key layout, and parses raw
//! `HashMap<String, String>` responses into typed snapshot structs.
//!
//! When FF 0.3 ships the upstream `describe_*` primitives this file
//! shrinks to ~30 lines of delegation; the key-layout imports
//! (`ExecKeyContext`, `FlowKeyContext`) disappear, and every cairn
//! service continues talking to the [`Engine`] trait without source
//! changes.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use ff_core::keys::{ExecKeyContext, FlowKeyContext};
use ff_core::partition::{execution_partition, flow_partition};
use ff_core::types::{
    AttemptId, AttemptIndex, EdgeId, ExecutionId, FlowId, LeaseEpoch, LeaseId, Namespace,
    TimestampMs, WaitpointId,
};

use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::helpers::parse_string_array;

use super::snapshots::{
    AttemptSummary, EdgeSnapshot, EdgeState, ExecutionSnapshot, FlowSnapshot, LeaseSummary,
};
use super::Engine;

/// [`Engine`] implementation that reads FF state directly from Valkey.
pub struct ValkeyEngine {
    runtime: Arc<FabricRuntime>,
}

impl ValkeyEngine {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }
}

#[async_trait]
impl Engine for ValkeyEngine {
    async fn describe_execution(
        &self,
        id: &ExecutionId,
    ) -> Result<Option<ExecutionSnapshot>, FabricError> {
        let partition = execution_partition(id, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, id);

        let core: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL exec_core: {e}")))?;

        if core.is_empty() {
            return Ok(None);
        }

        // Tags fetch is best-effort — an execution can legitimately
        // have no tag hash yet if it was created without any tag
        // writes. `unwrap_or_else` matches the behaviour services
        // had before this trait existed.
        let tags: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.tags())
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(execution_id = %id, error = %e, "valkey HGETALL exec_tags failed");
                HashMap::new()
            });

        Ok(Some(parse_execution_snapshot(id.clone(), core, tags)?))
    }

    async fn describe_flow(&self, id: &FlowId) -> Result<Option<FlowSnapshot>, FabricError> {
        let partition = flow_partition(id, &self.runtime.partition_config);
        let fctx = FlowKeyContext::new(&partition, id);

        let core: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&fctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL flow_core: {e}")))?;

        if core.is_empty() {
            return Ok(None);
        }

        // Summary hash is a FF-maintained read projection; not all
        // fields live on core. Keep the two-HGETALL pattern until FF
        // 0.3 ships describe_flow server-side (FF#58).
        let summary: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&fctx.summary())
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(flow_id = %id, error = %e, "valkey HGETALL flow summary failed");
                HashMap::new()
            });

        Ok(Some(parse_flow_snapshot(id.clone(), core, summary)?))
    }

    async fn describe_edge(
        &self,
        flow_id: &FlowId,
        edge_id: &EdgeId,
    ) -> Result<Option<EdgeSnapshot>, FabricError> {
        let partition = flow_partition(flow_id, &self.runtime.partition_config);
        let fctx = FlowKeyContext::new(&partition, flow_id);
        let edge_key = fctx.edge(edge_id);

        let fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&edge_key)
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL edge_hash: {e}")))?;

        if fields.is_empty() {
            return Ok(None);
        }

        Ok(Some(parse_edge_snapshot(edge_id.clone(), &fields)?))
    }

    async fn list_incoming_edges(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<Vec<EdgeSnapshot>, FabricError> {
        let partition = execution_partition(execution_id, &self.runtime.partition_config);
        let exec_ctx = ExecKeyContext::new(&partition, execution_id);

        // SMEMBERS on the child's deps_all_edges set (raw cmd —
        // ferriskey doesn't expose a typed smembers() helper).
        let smembers_raw: ferriskey::Value = self
            .runtime
            .client
            .cmd("SMEMBERS")
            .arg(exec_ctx.deps_all_edges())
            .execute()
            .await
            .map_err(|e| FabricError::Internal(format!("smembers deps_all_edges: {e}")))?;
        let edge_ids: Vec<String> = parse_string_array(&smembers_raw);

        let mut edges = Vec::with_capacity(edge_ids.len());
        for edge_id_str in &edge_ids {
            let edge_id = EdgeId::parse(edge_id_str)
                .map_err(|e| FabricError::Internal(format!("parse edge_id {edge_id_str}: {e}")))?;
            let dep_key = exec_ctx.dep_edge(&edge_id);
            // HGETALL the child-side dep record. This is the same
            // hash that apply_dependency_to_child populates on
            // `{p:N}` — it mirrors `dependency_kind`, `data_passing_ref`,
            // and the current `state` so we don't need to cross-slot
            // to the flow-side edge hash for dependency reads.
            let fields: HashMap<String, String> = self
                .runtime
                .client
                .hgetall(&dep_key)
                .await
                .map_err(|e| FabricError::Internal(format!("hgetall dep_edge: {e}")))?;
            if fields.is_empty() {
                continue;
            }
            edges.push(parse_edge_snapshot(edge_id, &fields)?);
        }

        Ok(edges)
    }
}

// ─── Parsers ─────────────────────────────────────────────────────────────

fn parse_execution_snapshot(
    execution_id: ExecutionId,
    core: HashMap<String, String>,
    tags: HashMap<String, String>,
) -> Result<ExecutionSnapshot, FabricError> {
    let lane_id = core
        .get("lane_id")
        .filter(|s| !s.is_empty())
        .map(ff_core::types::LaneId::new)
        .unwrap_or_else(|| ff_core::types::LaneId::new("cairn"));
    let namespace = core
        .get("namespace")
        .filter(|s| !s.is_empty())
        .map(Namespace::new)
        .unwrap_or_else(|| Namespace::new("default"));

    let public_state = core.get("public_state").cloned().unwrap_or_default();
    let blocking_reason = core
        .get("blocking_reason")
        .filter(|s| !s.is_empty())
        .cloned();
    let blocking_detail = core
        .get("blocking_detail")
        .filter(|s| !s.is_empty())
        .cloned();

    let current_attempt = match (
        core.get("current_attempt_id").filter(|s| !s.is_empty()),
        core.get("current_attempt_index")
            .and_then(|s| s.parse().ok()),
    ) {
        (Some(id_str), Some(idx)) => {
            let id = AttemptId::parse(id_str)
                .map_err(|e| FabricError::Internal(format!("parse current_attempt_id: {e}")))?;
            Some(AttemptSummary {
                id,
                index: AttemptIndex::new(idx),
            })
        }
        _ => None,
    };

    let current_lease = match core.get("current_lease_id").filter(|s| !s.is_empty()) {
        Some(lease_id_str) => {
            let lease_id = LeaseId::parse(lease_id_str)
                .map_err(|e| FabricError::Internal(format!("parse current_lease_id: {e}")))?;
            let epoch = LeaseEpoch::new(
                core.get("current_lease_epoch")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            );
            let attempt_index = AttemptIndex::new(
                core.get("current_attempt_index")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            );
            let owner = core
                .get("current_worker_instance_id")
                .cloned()
                .unwrap_or_default();
            let expires_at = TimestampMs::from_millis(
                core.get("lease_expires_at")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            );
            Some(LeaseSummary {
                lease_id,
                epoch,
                attempt_index,
                owner,
                expires_at,
            })
        }
        None => None,
    };

    let current_waitpoint = core
        .get("current_waitpoint_id")
        .filter(|s| !s.is_empty())
        .map(|s| WaitpointId::parse(s))
        .transpose()
        .map_err(|e| FabricError::Internal(format!("parse current_waitpoint_id: {e}")))?;

    let created_at = TimestampMs::from_millis(
        core.get("created_at")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    );
    let last_mutation_at = TimestampMs::from_millis(
        core.get("last_mutation_at")
            .and_then(|s| s.parse().ok())
            .unwrap_or(created_at.0),
    );
    let total_attempt_count = core
        .get("total_attempt_count")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let current_lease_epoch = core
        .get("current_lease_epoch")
        .and_then(|s| s.parse().ok())
        .map(LeaseEpoch::new);

    let tags: BTreeMap<String, String> = tags.into_iter().collect();

    Ok(ExecutionSnapshot {
        execution_id,
        lane_id,
        namespace,
        public_state,
        blocking_reason,
        blocking_detail,
        current_attempt,
        current_lease,
        current_waitpoint,
        created_at,
        last_mutation_at,
        total_attempt_count,
        current_lease_epoch,
        tags,
    })
}

fn parse_flow_snapshot(
    flow_id: FlowId,
    core: HashMap<String, String>,
    summary: HashMap<String, String>,
) -> Result<FlowSnapshot, FabricError> {
    let kind = core.get("flow_kind").cloned().unwrap_or_default();
    let namespace = core
        .get("namespace")
        .filter(|s| !s.is_empty())
        .map(Namespace::new)
        .unwrap_or_else(|| Namespace::new("default"));
    let node_count = core
        .get("node_count")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let edge_count = core
        .get("edge_count")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let graph_revision = core
        .get("graph_revision")
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // public_flow_state: prefer summary (FF's read-optimised
    // projection), fall back to core. Matches the existing
    // session_service code path.
    let public_flow_state = summary
        .get("public_state")
        .or_else(|| core.get("public_flow_state"))
        .or_else(|| core.get("state"))
        .cloned()
        .unwrap_or_default();

    let created_at = TimestampMs::from_millis(
        core.get("created_at")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    );
    let last_mutation_at = TimestampMs::from_millis(
        summary
            .get("last_mutation_at")
            .or_else(|| core.get("last_mutation_at"))
            .and_then(|s| s.parse().ok())
            .unwrap_or(created_at.0),
    );

    // Only the `cairn.*`-prefixed keys from `core` are tags — FF's
    // own fields (flow_kind, namespace, counts, state, timestamps)
    // stay as typed fields on the snapshot.
    let tags: BTreeMap<String, String> = core
        .into_iter()
        .filter(|(k, _)| k.starts_with("cairn."))
        .collect();

    Ok(FlowSnapshot {
        flow_id,
        kind,
        namespace,
        node_count,
        edge_count,
        graph_revision,
        public_flow_state,
        created_at,
        last_mutation_at,
        tags,
    })
}

fn parse_edge_snapshot(
    edge_id: EdgeId,
    fields: &HashMap<String, String>,
) -> Result<EdgeSnapshot, FabricError> {
    let flow_id_str = fields
        .get("flow_id")
        .ok_or_else(|| FabricError::Internal("edge missing flow_id field".to_owned()))?;
    let flow_id = FlowId::parse(flow_id_str)
        .map_err(|e| FabricError::Internal(format!("parse edge.flow_id: {e}")))?;

    let upstream_eid_str = fields
        .get("upstream_execution_id")
        .ok_or_else(|| FabricError::Internal("edge missing upstream_execution_id".to_owned()))?;
    let upstream_execution_id = ExecutionId::parse(upstream_eid_str)
        .map_err(|e| FabricError::Internal(format!("parse edge.upstream_eid: {e}")))?;

    // downstream_execution_id is optional on child-side dep_hash
    // (apply_dependency_to_child writes it, but some older edges
    // might not have it). Fall back to parsing from the requesting
    // context if absent — though in practice the dep_hash always
    // has it in FF 0.2.
    let downstream_execution_id = match fields
        .get("downstream_execution_id")
        .filter(|s| !s.is_empty())
    {
        Some(s) => ExecutionId::parse(s)
            .map_err(|e| FabricError::Internal(format!("parse edge.downstream_eid: {e}")))?,
        None => {
            return Err(FabricError::Internal(
                "edge missing downstream_execution_id field".to_owned(),
            ));
        }
    };

    let kind = fields
        .get("dependency_kind")
        .filter(|s| !s.is_empty())
        .cloned()
        .unwrap_or_else(|| "success_only".to_owned());
    let data_passing_ref = match fields.get("data_passing_ref").map(String::as_str) {
        None | Some("") => None,
        Some(s) => Some(s.to_owned()),
    };

    // `state` lives on child-side dep_hash (written by
    // apply_dependency_to_child, mutated by resolve_dependency).
    // Edge hash on flow side uses `edge_state` (set to "pending" at
    // stage time). Accept either for forward-compat.
    let state_str = fields
        .get("state")
        .or_else(|| fields.get("edge_state"))
        .map(String::as_str)
        .unwrap_or_default();
    let state = match state_str {
        "unsatisfied" | "pending" => EdgeState::Unsatisfied,
        "satisfied" => EdgeState::Satisfied,
        "impossible" => EdgeState::Impossible,
        _ => EdgeState::Unknown,
    };

    let created_at = TimestampMs::from_millis(
        fields
            .get("created_at")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    );

    Ok(EdgeSnapshot {
        edge_id,
        flow_id,
        upstream_execution_id,
        downstream_execution_id,
        kind,
        data_passing_ref,
        state,
        created_at,
    })
}
