//! Typed snapshot shapes for cairn-side reads of FF state.
//!
//! These types are the contract between services and the [`Engine`]
//! trait. They intentionally mirror the primitives proposed upstream
//! in [FlowFabric#58](https://github.com/avifenesh/FlowFabric/issues/58)
//! so that when FF 0.3 ships `describe_execution` / `describe_flow` /
//! `describe_edge`, the cairn-side `valkey_impl` becomes a thin
//! passthrough and these types are replaced by re-exports from the
//! `ff` umbrella crate.
//!
//! # Design notes
//!
//! - **`blocking_reason` stays an opaque `Option<String>`** because
//!   FF's Lua emits a small open set of values (`waiting_for_children`,
//!   `blocked_by_dependencies`, etc.) that FF doesn't type upstream.
//!   Cairn's `state_map::adjust_*_state_for_blocking_reason` does the
//!   translation to cairn state variants.
//! - **`tags` is a `BTreeMap<String, String>`** (not `HashMap`) for
//!   stable iteration order in logs and deterministic serialisation.
//! - **`LeaseSummary` carries `lease_id`, `epoch`, `attempt_index`**
//!   in addition to `owner`/`expires_at` because release/heartbeat
//!   FCALL paths feed these back in as ARGV (until FF 0.4 drops the
//!   requirement).

use std::collections::BTreeMap;

use flowfabric::core::types::{
    AttemptId, AttemptIndex, EdgeId, ExecutionId, FlowId, LaneId, LeaseEpoch, LeaseId, Namespace,
    TimestampMs, WaitpointId,
};

/// Snapshot of a single FF execution, populated from cairn-visible
/// fields of `exec_core` + `exec_tags`.
#[derive(Clone, Debug)]
pub struct ExecutionSnapshot {
    pub execution_id: ExecutionId,
    /// Lane this execution routes to.
    pub lane_id: LaneId,
    /// Namespace (cairn tenant_id → FF Namespace).
    pub namespace: Namespace,
    /// Raw FF public state string. Parsed to `PublicState` by the
    /// service via `state_map::parse_public_state`; kept raw here so
    /// the snapshot type doesn't silently lose forward-compatible FF
    /// state additions.
    pub public_state: String,
    /// FF-classified reason for a non-ready state. Empty `None` on
    /// runnable executions. Opaque string — cairn's `state_map` does
    /// the translation to cairn states.
    pub blocking_reason: Option<String>,
    pub blocking_detail: Option<String>,
    /// Current (active) attempt summary, if any.
    pub current_attempt: Option<AttemptSummary>,
    /// Current (active) lease summary, if any.
    pub current_lease: Option<LeaseSummary>,
    /// Current waitpoint id, if the execution is suspended waiting
    /// for a signal.
    pub current_waitpoint: Option<WaitpointId>,
    pub created_at: TimestampMs,
    pub last_mutation_at: TimestampMs,
    pub total_attempt_count: u32,
    /// Current (or last-known, if no active lease) epoch counter.
    /// Monotonic across the execution's lifetime — survives lease
    /// release / re-claim cycles. Cairn uses this as the `version`
    /// field on `RunRecord` / `TaskRecord` for optimistic-concurrency
    /// checks.
    pub current_lease_epoch: Option<LeaseEpoch>,
    /// Caller-owned metadata (the `cairn.*` tag namespace cairn
    /// writes to `exec_tags`).
    pub tags: BTreeMap<String, String>,
}

/// Snapshot of the current attempt on an execution. Missing fields
/// means no attempt has been issued yet.
#[derive(Clone, Debug)]
pub struct AttemptSummary {
    pub id: AttemptId,
    pub index: AttemptIndex,
}

/// Snapshot of the active lease on an execution. `None` on
/// [`ExecutionSnapshot::current_lease`] means no active lease.
///
/// Carries more than just `owner` + `expires_at` because release /
/// heartbeat / complete FCALL paths take `lease_id`, `epoch`, and
/// `attempt_index` as ARGV today — cairn has to feed those values
/// back to FF. When FF 0.4 drops those ARGV requirements
/// (FlowFabric#58), `lease_id` + `attempt_index` become optional
/// fields here.
#[derive(Clone, Debug)]
pub struct LeaseSummary {
    pub lease_id: LeaseId,
    pub epoch: LeaseEpoch,
    pub attempt_index: AttemptIndex,
    pub owner: String,
    pub expires_at: TimestampMs,
}

/// Snapshot of a flow (cairn session).
///
/// Field names mirror FF's `flow_core` HSET body: `flow_kind`,
/// `namespace`, `graph_revision`, `node_count`, `edge_count`,
/// `public_flow_state`, `created_at`, `last_mutation_at`. Cairn's
/// own tag keys (`cairn.project`, `cairn.session_id`, `cairn.archived`)
/// also live on `flow_core` and are surfaced via [`Self::tags`].
#[derive(Clone, Debug)]
pub struct FlowSnapshot {
    pub flow_id: FlowId,
    /// FF flow kind (`flow_kind` field). Cairn uses `"cairn_session"`.
    pub kind: String,
    pub namespace: Namespace,
    /// Count of executions in the flow (`node_count` field).
    pub node_count: u32,
    pub edge_count: u32,
    pub graph_revision: u64,
    /// Raw FF `public_flow_state` string (`"open"`, and FF-managed
    /// terminal variants). Kept raw so forward-compatible FF state
    /// additions don't get swallowed.
    pub public_flow_state: String,
    pub created_at: TimestampMs,
    pub last_mutation_at: TimestampMs,
    /// Caller-owned metadata written to `flow_core` as `cairn.*`
    /// keys. Values like `cairn.project`, `cairn.session_id`,
    /// `cairn.archived`.
    pub tags: BTreeMap<String, String>,
}

/// State of a dependency edge. Mirrors FF's edge state machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdgeState {
    /// Upstream has not yet completed in a way that satisfies this
    /// edge. Downstream remains blocked.
    Unsatisfied,
    /// Upstream completed with an outcome that satisfies this edge.
    Satisfied,
    /// Upstream terminated with an outcome that can never satisfy
    /// this edge (e.g. `success_only` + upstream failed). Downstream
    /// will be skipped.
    Impossible,
    /// Unknown value (forward-compatibility for FF-side additions).
    /// Services should treat this as non-satisfying.
    Unknown,
}

/// Snapshot of a dependency edge between two executions within a flow.
#[derive(Clone, Debug)]
pub struct EdgeSnapshot {
    pub edge_id: EdgeId,
    pub flow_id: FlowId,
    pub upstream_execution_id: ExecutionId,
    pub downstream_execution_id: ExecutionId,
    /// FF edge kind string. Currently only `"success_only"` exists.
    pub kind: String,
    /// Caller-supplied opaque reference stored on the edge. Empty
    /// FF-side strings are normalised to `None`.
    pub data_passing_ref: Option<String>,
    pub state: EdgeState,
    pub created_at: TimestampMs,
}
