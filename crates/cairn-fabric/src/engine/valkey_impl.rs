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
use flowfabric::core::keys::{self, ExecKeyContext, FlowKeyContext};
use flowfabric::core::partition::{execution_partition, flow_partition};
use flowfabric::core::types::{
    AttemptId, AttemptIndex, EdgeId, ExecutionId, FlowId, LeaseEpoch, LeaseId, Namespace,
    TimestampMs, WaitpointId, WorkerId, WorkerInstanceId,
};

use crate::boot::FabricRuntime;
use crate::error::FabricError;
use crate::helpers::{now_ms, parse_string_array};

use super::control_plane_types::WorkerRegistration;
use super::snapshots::{
    AttemptSummary, EdgeSnapshot, EdgeState, ExecutionSnapshot, FlowSnapshot, LeaseSummary,
};
use super::Engine;

/// [`Engine`] implementation that reads FF state directly from Valkey.
///
/// Also carries the [`ControlPlaneBackend`] impl (see
/// [`super::valkey_control_plane_impl`]) — one struct, two traits, so
/// callers can hold a single `Arc<ValkeyEngine>` and cast it to either
/// trait object.
///
/// [`ControlPlaneBackend`]: super::control_plane::ControlPlaneBackend
pub struct ValkeyEngine {
    runtime: Arc<FabricRuntime>,
}

impl ValkeyEngine {
    pub fn new(runtime: Arc<FabricRuntime>) -> Self {
        Self { runtime }
    }

    /// Internal accessor for [`super::valkey_control_plane_impl`].
    /// Not exposed outside the engine module.
    pub(super) fn runtime(&self) -> &Arc<FabricRuntime> {
        &self.runtime
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

    async fn get_execution_tag(
        &self,
        id: &ExecutionId,
        key: &str,
    ) -> Result<Option<String>, FabricError> {
        let partition = execution_partition(id, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, id);
        let value: Option<String> = self
            .runtime
            .client
            .hget(&ctx.tags(), key)
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGET exec_tags.{key}: {e}")))?;
        Ok(value.filter(|s| !s.is_empty()))
    }

    async fn set_execution_tag(
        &self,
        id: &ExecutionId,
        key: &str,
        value: &str,
    ) -> Result<(), FabricError> {
        validate_tag_namespace(key)?;
        let partition = execution_partition(id, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, id);
        let _: i64 = self
            .runtime
            .client
            .hset(&ctx.tags(), key, value)
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HSET exec_tags.{key}: {e}")))?;
        Ok(())
    }

    async fn set_flow_tag(&self, id: &FlowId, key: &str, value: &str) -> Result<(), FabricError> {
        validate_tag_namespace(key)?;
        let partition = flow_partition(id, &self.runtime.partition_config);
        let fctx = FlowKeyContext::new(&partition, id);
        let _: i64 = self
            .runtime
            .client
            .hset(&fctx.core(), key, value)
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HSET flow_core.{key}: {e}")))?;
        Ok(())
    }

    async fn set_flow_tags(
        &self,
        id: &FlowId,
        tags: &BTreeMap<String, String>,
    ) -> Result<(), FabricError> {
        if tags.is_empty() {
            return Ok(());
        }
        // All-or-nothing validation: reject the whole batch if any
        // key is malformed. Matches the "no partial writes" contract
        // the trait docs promise — session creation depends on both
        // `cairn.project` and `cairn.session_id` being present
        // before the `SessionCreated` bridge event fires.
        for key in tags.keys() {
            validate_tag_namespace(key)?;
        }

        let partition = flow_partition(id, &self.runtime.partition_config);
        let fctx = FlowKeyContext::new(&partition, id);
        let flow_core = fctx.core();

        // Variadic HSET: one round-trip for the whole batch.
        // Redis ≥ 4.0 / Valkey support `HSET key field value
        // [field value ...]`. ferriskey's typed `.hset()` helper is
        // single-pair only, so we drop to the raw `CommandBuilder`
        // to pass the full batch in one command. `CommandBuilder`
        // is a consuming builder (`fn arg(mut self, …) -> Self`),
        // so we re-bind on each iteration.
        let mut cmd = self.runtime.client.cmd("HSET").arg(flow_core.as_str());
        for (k, v) in tags {
            cmd = cmd.arg(k.as_str()).arg(v.as_str());
        }
        cmd.execute::<i64>().await.map_err(|e| {
            FabricError::Internal(format!("valkey HSET flow_core (bulk {}): {e}", tags.len()))
        })?;
        Ok(())
    }

    // ── Worker registry (Phase D PR 1) ──────────────────────────────────

    async fn register_worker(
        &self,
        worker_id: &WorkerId,
        instance_id: &WorkerInstanceId,
        capabilities: &[String],
    ) -> Result<WorkerRegistration, FabricError> {
        let worker_key = keys::worker_key(instance_id);
        let now = now_ms();
        let now_str = now.to_string();

        self.runtime
            .client
            .cmd("HSET")
            .arg(&worker_key)
            .arg("worker_id")
            .arg(worker_id.to_string())
            .arg("instance_id")
            .arg(instance_id.to_string())
            .arg("capabilities")
            .arg(capabilities.join(","))
            .arg("last_heartbeat_ms")
            .arg(&now_str)
            .arg("is_alive")
            .arg("true")
            .arg("registered_at_ms")
            .arg(&now_str)
            .execute::<u64>()
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET {worker_key}: {e}")))?;

        // TTL-based expiry: dead workers auto-expire if heartbeats stop.
        // `mark_worker_dead` is the explicit opt-out; this is the safety net.
        let ttl_ms = self.runtime.config.lease_ttl_ms * 3;
        // Valkey's PEXPIRE returns 1 (true) on success, 0 (false) when
        // the key does not exist. ferriskey typechecks the reply as
        // boolean — not u64 — so we must decode into `bool` here.
        let _: bool = self
            .runtime
            .client
            .cmd("PEXPIRE")
            .arg(&worker_key)
            .arg(ttl_ms.to_string())
            .execute()
            .await
            .map_err(|e| FabricError::Valkey(format!("PEXPIRE {worker_key}: {e}")))?;

        let workers_index = keys::workers_index_key();
        self.runtime
            .client
            .cmd("SADD")
            .arg(workers_index)
            .arg(instance_id.to_string())
            .execute::<u64>()
            .await
            .map_err(|e| FabricError::Valkey(format!("SADD workers index: {e}")))?;

        for cap in capabilities {
            if let Some((k, v)) = cap.split_once('=') {
                let cap_key = keys::workers_capability_key(k, v);
                self.runtime
                    .client
                    .cmd("SADD")
                    .arg(cap_key)
                    .arg(instance_id.to_string())
                    .execute::<u64>()
                    .await
                    .map_err(|e| FabricError::Valkey(format!("SADD cap index: {e}")))?;
            }
        }

        Ok(WorkerRegistration {
            worker_id: worker_id.clone(),
            instance_id: instance_id.clone(),
            capabilities: capabilities.to_vec(),
            registered_at_ms: now,
        })
    }

    async fn heartbeat_worker(&self, instance_id: &WorkerInstanceId) -> Result<(), FabricError> {
        let worker_key = keys::worker_key(instance_id);
        let now = now_ms().to_string();
        let _: i64 = self
            .runtime
            .client
            .hset(&worker_key, "last_heartbeat_ms", &now)
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET heartbeat: {e}")))?;

        let ttl_ms = self.runtime.config.lease_ttl_ms * 3;
        let _: bool = self
            .runtime
            .client
            .cmd("PEXPIRE")
            .arg(&worker_key)
            .arg(ttl_ms.to_string())
            .execute()
            .await
            .map_err(|e| FabricError::Valkey(format!("PEXPIRE heartbeat: {e}")))?;

        Ok(())
    }

    async fn mark_worker_dead(&self, instance_id: &WorkerInstanceId) -> Result<(), FabricError> {
        let worker_key = keys::worker_key(instance_id);
        let _: i64 = self
            .runtime
            .client
            .hset(&worker_key, "is_alive", "false")
            .await
            .map_err(|e| FabricError::Valkey(format!("HSET is_alive: {e}")))?;
        Ok(())
    }

    async fn list_expired_leases(
        &self,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<super::control_plane_types::ExpiredLease>, FabricError> {
        use flowfabric::core::keys::IndexKeys;
        use flowfabric::core::partition::{Partition, PartitionFamily};

        // Exec and flow share a slot under the `num_flow_partitions`
        // budget post-RFC-011; the execution lease_expiry index is
        // stamped on the flow-partition fan-out.
        let num_partitions = self.runtime.partition_config.num_flow_partitions;
        let mut out: Vec<super::control_plane_types::ExpiredLease> = Vec::new();
        let remaining_limit = limit;
        let score_max = now_ms.to_string();

        for index in 0..num_partitions {
            if out.len() >= remaining_limit {
                break;
            }
            let partition = Partition {
                family: PartitionFamily::Execution,
                index,
            };
            let idx = IndexKeys::new(&partition);
            let zset_key = idx.lease_expiry();
            let batch_cap = (remaining_limit - out.len()).min(512);

            // ZRANGEBYSCORE <key> 0 <now_ms> WITHSCORES LIMIT 0 <cap>
            let raw: ferriskey::Value = self
                .runtime
                .client
                .cmd("ZRANGEBYSCORE")
                .arg(zset_key.as_str())
                .arg("0")
                .arg(score_max.as_str())
                .arg("WITHSCORES")
                .arg("LIMIT")
                .arg("0")
                .arg(batch_cap.to_string().as_str())
                .execute()
                .await
                .map_err(|e| FabricError::Valkey(format!("ZRANGEBYSCORE lease_expiry: {e}")))?;

            // Reply shape: Array of alternating [member, score, member, score, ...].
            if let ferriskey::Value::Array(items) = raw {
                let mut it = items.into_iter();
                while let (Some(m), Some(s)) = (it.next(), it.next()) {
                    let Ok(m) = m else { continue };
                    let Ok(s) = s else { continue };
                    let Some(member) = crate::helpers::value_to_string(&m) else {
                        continue;
                    };
                    let Some(score) = crate::helpers::value_to_string(&s) else {
                        continue;
                    };
                    let Ok(eid) = ExecutionId::parse(&member) else {
                        // Malformed member — skip rather than fail the whole scan.
                        continue;
                    };
                    // Skip malformed scores rather than coercing to 0: a 0
                    // fallback would surface the row as "expired at epoch",
                    // a false-positive that would confuse operator dashboards.
                    let Ok(expires_at_ms) = score.parse::<u64>() else {
                        tracing::warn!(
                            execution_id = %eid,
                            raw_score = %score,
                            "lease_expiry score unparseable; skipping row",
                        );
                        continue;
                    };
                    out.push(super::control_plane_types::ExpiredLease {
                        execution_id: eid,
                        expires_at_ms,
                    });
                    if out.len() >= remaining_limit {
                        break;
                    }
                }
            }
        }

        Ok(out)
    }
}

/// Namespace guard for cairn tag writes.
///
/// Rejects keys that don't match `^[a-z][a-z0-9_]*\.` (one lowercase
/// alpha-underscore segment followed by a `.`). Cairn writes
/// `cairn.*`; FF's own hash fields have no `.`, so this rule keeps
/// the two namespaces from colliding without needing a closed list
/// of allowed prefixes.
fn validate_tag_namespace(key: &str) -> Result<(), FabricError> {
    let (prefix, rest) = match key.split_once('.') {
        Some((p, r)) => (p, r),
        None => {
            return Err(FabricError::Validation {
                reason: format!(
                    "tag key {key:?} is missing a namespace prefix (expected `<ns>.<field>`)"
                ),
            });
        }
    };
    if prefix.is_empty() {
        return Err(FabricError::Validation {
            reason: format!("tag key {key:?} has an empty namespace prefix"),
        });
    }
    let mut chars = prefix.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_lowercase() {
        return Err(FabricError::Validation {
            reason: format!(
                "tag key {key:?} namespace prefix must start with a lowercase ASCII letter"
            ),
        });
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(FabricError::Validation {
                reason: format!("tag key {key:?} namespace prefix must match [a-z][a-z0-9_]*"),
            });
        }
    }
    // `rest` (everything after the first `.`) is intentionally
    // unconstrained — callers use dotted sub-paths
    // (`cairn.task_id`, `cairn.run.parent_id`) and the namespace
    // guard only needs to prove the prefix is cairn-owned.
    let _ = rest;
    Ok(())
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
        .map(flowfabric::core::types::LaneId::new)
        .unwrap_or_else(|| flowfabric::core::types::LaneId::new("cairn"));
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
            let worker_instance_id = flowfabric::core::types::WorkerInstanceId::new(
                core.get("current_worker_instance_id")
                    .cloned()
                    .unwrap_or_default(),
            );
            let expires_at = TimestampMs::from_millis(
                core.get("lease_expires_at")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0),
            );
            // Upstream LeaseSummary (FF#278) is `#[non_exhaustive]` —
            // use the `new` + `with_*` builder chain rather than struct
            // literal. `last_heartbeat_at` is the FF#278-added field;
            // cairn surfaces it from `lease_last_renewed_at` on
            // exec_core when populated (backends that don't emit
            // per-renewal ticks leave the field empty, which we map to
            // `None`).
            let mut summary = LeaseSummary::new(epoch, worker_instance_id, expires_at)
                .with_lease_id(lease_id)
                .with_attempt_index(attempt_index);
            if let Some(ts) = core
                .get("lease_last_renewed_at")
                .and_then(|s| s.parse::<i64>().ok())
                .filter(|ms| *ms > 0)
            {
                summary = summary.with_last_heartbeat_at(TimestampMs::from_millis(ts));
            }
            Some(summary)
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

    // `downstream_execution_id` is always written by FF's Lua
    // (both `ff_stage_dependency_edge` on the flow-side edge hash
    // and `ff_apply_dependency_to_child` on the child-side dep hash).
    // Its absence means schema drift — surface as Internal so the
    // failure is loud, not silently papered over.
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

#[cfg(test)]
mod tests {
    use super::validate_tag_namespace;

    #[test]
    fn accepts_cairn_namespace() {
        validate_tag_namespace("cairn.project").expect("cairn.project");
        validate_tag_namespace("cairn.session_id").expect("cairn.session_id");
        validate_tag_namespace("cairn.run.parent_id").expect("dotted sub-path ok");
        validate_tag_namespace("a1.x").expect("alnum prefix ok");
        validate_tag_namespace("ns_with_underscore.field").expect("underscore ok");
    }

    #[test]
    fn rejects_missing_dot() {
        let err = validate_tag_namespace("notanamespace").unwrap_err();
        assert!(format!("{err:?}").contains("missing a namespace prefix"));
    }

    #[test]
    fn rejects_uppercase_prefix() {
        let err = validate_tag_namespace("Cairn.foo").unwrap_err();
        assert!(format!("{err:?}").contains("lowercase"));
    }

    #[test]
    fn rejects_leading_digit() {
        let err = validate_tag_namespace("1cairn.foo").unwrap_err();
        assert!(format!("{err:?}").contains("lowercase"));
    }

    #[test]
    fn rejects_empty_prefix() {
        let err = validate_tag_namespace(".field").unwrap_err();
        assert!(format!("{err:?}").contains("empty namespace"));
    }

    #[test]
    fn rejects_hyphen_in_prefix() {
        let err = validate_tag_namespace("cairn-scope.foo").unwrap_err();
        assert!(format!("{err:?}").contains("[a-z][a-z0-9_]*"));
    }
}
