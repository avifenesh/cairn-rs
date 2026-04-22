//! Cairn-side abstraction over FlowFabric's read-side state.
//!
//! # Why this exists
//!
//! Cairn services used to read FF's Valkey state directly via
//! `.client.hgetall(&ctx.core())` — pinning cairn to FF's storage
//! engine (Valkey), key layout (`ExecKeyContext::core()`), and hash
//! field names (`public_state`, `dependency_kind`, etc.). A field
//! rename or storage swap in FF would silently break cairn.
//!
//! The [`Engine`] trait confines every cairn-side read of FF state
//! to one trait boundary. Services call `engine.describe_execution(&eid)`
//! and get a typed [`ExecutionSnapshot`] back; they never see Valkey
//! keys or hash fields.
//!
//! The single implementation, [`valkey_impl::ValkeyEngine`], holds
//! the `ferriskey::Client` handle and performs the direct HGETALL /
//! SMEMBERS reads. When FF 0.3 ships the upstream `describe_*`
//! primitives ([FlowFabric#58](https://github.com/avifenesh/FlowFabric/issues/58)),
//! `ValkeyEngine` becomes a thin passthrough and the typed snapshot
//! structs will be replaced by re-exports from the `ff` umbrella
//! crate.
//!
//! # Scope (what this trait does and doesn't cover)
//!
//! **In scope**: reads of FF-owned state (executions, flows,
//! dependency edges). Reads that fed `TaskRecord` / `RunRecord` /
//! `SessionRecord` construction.
//!
//! **Not in scope (yet)**:
//! - FCALL ARGV pre-reads (Phase D). The ~12 `hget ctx.core(),
//!   "current_attempt_id"` sites before FCALLs stay for now.
//! - Typed error model (Phase E). FCALL errors continue arriving as
//!   `ferriskey::Value` envelopes parsed by `helpers::*`; typed
//!   [`EngineError`](crate::error::FabricError) absorbs them later.
//! - Cairn-owned state (worker/quota/budget keyspaces). Those
//!   `HSET`s are cairn's own data, not a layering violation.
//! - `instance_tag_backfill` one-shot scanner. It operates on raw
//!   `ff:exec:*:tags` scan keys, not typed [`ExecutionId`]s;
//!   routing it through the trait would require a raw-key
//!   escape-hatch that re-exposes the Valkey layout. The backfill
//!   is a migration utility with a finite lifetime — it keeps its
//!   direct `HSET` until its sunset.
//!
//! **Phase C (shipped)**: Tag writes. Cairn services no longer call
//! `client.hset(&fctx.core(), "cairn.*", …)` directly —
//! [`Engine::set_flow_tag`] and [`Engine::set_flow_tags`] own the
//! flow-core namespace writes, and [`Engine::set_execution_tag`]
//! owns the execution-tags-hash writes. Enforced by the workspace
//! `clippy.toml` disallowed-methods lint on `ferriskey::Client::hset`
//! outside `engine/valkey_impl.rs`.

pub mod snapshots;
pub mod valkey_impl;

use std::collections::BTreeMap;

use async_trait::async_trait;
use ff_core::types::{EdgeId, ExecutionId, FlowId};

use crate::error::FabricError;

pub use snapshots::{
    AttemptSummary, EdgeSnapshot, EdgeState, ExecutionSnapshot, FlowSnapshot, LeaseSummary,
};
pub use valkey_impl::ValkeyEngine;

/// Cairn-side read abstraction over FF state.
///
/// Every method that returns `Option<_>` uses `None` for "not present
/// in FF" and `Err` only for transport / serialisation / malformed
/// data. Callers that need a typed not-found error wrap the `None`
/// with their cairn-specific entity name (e.g.
/// `.ok_or(FabricError::NotFound { entity: "task", id })`).
///
/// ## Why `describe_edge` takes a `flow_id`
///
/// FF's edge hash key is `ff:flow:{fp:N}:<flow_id>:edge:<edge_id>` —
/// the flow id is part of the key. Cairn cannot locate an edge from
/// `edge_id` alone without an FF-side edge→flow index, which doesn't
/// exist today (see FlowFabric#58). Callers that know the flow
/// (typically because they just issued a `stage_dependency_edge`
/// FCALL on it) pass the flow_id explicitly. When FF 0.3 ships an
/// `edge_id`-only lookup this parameter becomes optional.
#[async_trait]
pub trait Engine: Send + Sync {
    /// Read a single execution's snapshot. Returns `Ok(None)` when
    /// the execution is not in FF (typically because cairn minted an
    /// id for an entity that was never submitted, or because the
    /// entity was purged).
    async fn describe_execution(
        &self,
        id: &ExecutionId,
    ) -> Result<Option<ExecutionSnapshot>, FabricError>;

    /// Read a flow's snapshot. Returns `Ok(None)` when the flow does
    /// not exist in FF.
    async fn describe_flow(&self, id: &FlowId) -> Result<Option<FlowSnapshot>, FabricError>;

    /// Read a dependency edge's snapshot. The caller must supply
    /// `flow_id` because FF's edge storage is flow-scoped — see
    /// the type-level docs above. Returns `Ok(None)` when the edge
    /// does not exist on the given flow.
    async fn describe_edge(
        &self,
        flow_id: &FlowId,
        edge_id: &EdgeId,
    ) -> Result<Option<EdgeSnapshot>, FabricError>;

    /// Enumerate dependency edges where `execution_id` is the
    /// downstream endpoint. Empty vec means the execution has no
    /// incoming edges (either never had dependencies declared, or
    /// all are resolved).
    async fn list_incoming_edges(
        &self,
        execution_id: &ExecutionId,
    ) -> Result<Vec<EdgeSnapshot>, FabricError>;

    /// Fetch a single tag value from an execution's tag hash.
    ///
    /// Targeted read — cheaper than
    /// [`Self::describe_execution`](Engine::describe_execution) when
    /// the caller only needs one field (e.g. the `cairn.task_id`
    /// back-reference stamped on an upstream execution). Avoids the
    /// N+1 amplification that full snapshot reads would cause in
    /// loops like `check_dependencies`'s per-blocker resolve.
    ///
    /// Returns `Ok(None)` if the execution's tag hash doesn't exist
    /// or the tag is absent. Empty-string values are normalised to
    /// `None`.
    async fn get_execution_tag(
        &self,
        id: &ExecutionId,
        key: &str,
    ) -> Result<Option<String>, FabricError>;

    /// Set a single tag on an execution's tag hash.
    ///
    /// Namespace-guarded: `key` must match `^[a-z][a-z0-9_]*\.` —
    /// one lowercase alpha-underscore prefix, then a `.`, then
    /// anything. Cairn owns the `cairn.*` namespace; FF's own hash
    /// fields have no `.`, so the rule is a mechanical guard
    /// against accidental collision with FF-managed fields. Keys
    /// that fail the rule return [`FabricError::Validation`].
    ///
    /// Callers never see the Valkey hash layout — the impl
    /// constructs the execution's tag key internally from the
    /// `ExecutionId` + partition config.
    async fn set_execution_tag(
        &self,
        id: &ExecutionId,
        key: &str,
        value: &str,
    ) -> Result<(), FabricError>;

    /// Set a single tag on a flow's core hash.
    ///
    /// Namespace-guarded: see [`Self::set_execution_tag`] for the
    /// rule. Callers never see the Valkey hash layout — the impl
    /// constructs the flow's core key internally.
    async fn set_flow_tag(&self, id: &FlowId, key: &str, value: &str) -> Result<(), FabricError>;

    /// Bulk-set flow tags in a single round-trip.
    ///
    /// Validation is **all-or-nothing**: if any key in `tags`
    /// fails the namespace rule ([`Self::set_execution_tag`]) the
    /// entire batch is rejected with [`FabricError::Validation`]
    /// and no write is issued. This preserves the "no partial
    /// writes" guarantee cairn's session creation path relies on
    /// (both `cairn.project` and `cairn.session_id` must be
    /// present before the bridge event fires).
    ///
    /// An empty map is a no-op that returns `Ok(())`.
    async fn set_flow_tags(
        &self,
        id: &FlowId,
        tags: &BTreeMap<String, String>,
    ) -> Result<(), FabricError>;
}
