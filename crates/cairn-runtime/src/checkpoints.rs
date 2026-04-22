//! Checkpoint service boundary per RFC 005.
//!
//! Checkpoints are first-class durable recovery points for runs.
//! They are immutable once created; one may be marked latest.

use async_trait::async_trait;
use cairn_domain::{CheckpointId, CheckpointKind, CheckpointStrategy, ProjectKey, RunId};
use cairn_store::projections::CheckpointRecord;

use crate::error::RuntimeError;

/// Checkpoint service boundary.
///
/// Per RFC 005:
/// - checkpoints are immutable recovery records
/// - one checkpoint per run may be marked latest
/// - resume from explicit checkpoint state
#[async_trait]
pub trait CheckpointService: Send + Sync {
    /// Save a checkpoint for a run (marks it as latest, supersedes prior).
    async fn save(
        &self,
        project: &ProjectKey,
        run_id: &RunId,
        checkpoint_id: CheckpointId,
    ) -> Result<CheckpointRecord, RuntimeError>;

    /// RFC 020 Track 4 — save a typed (Intent or Result) checkpoint that
    /// captures the iteration's message history + planned tool-call IDs.
    ///
    /// Orchestrator contract: exactly two checkpoints per iteration —
    /// `Intent` after decide (before any dispatch) and `Result` after
    /// execute (all dispatches settled). Intent carries the deterministic
    /// `ToolCallId`s Track 3 mints; Result carries an empty `tool_call_ids`
    /// (the Intent checkpoint already owns the full planned list — see
    /// `project_track4_implementation_plan.md` §6 Q4).
    ///
    /// `message_history` is the full serialized snapshot (Gap 3
    /// resolution — v1 ships full snapshots, not diffs; the
    /// `message_history_size` field on the emitted `CheckpointRecorded`
    /// event records post-serialization byte count so operators can
    /// monitor checkpoint cost).
    async fn save_dual(
        &self,
        project: &ProjectKey,
        run_id: &RunId,
        checkpoint_id: CheckpointId,
        kind: CheckpointKind,
        message_history: serde_json::Value,
        tool_call_ids: Vec<String>,
    ) -> Result<CheckpointRecord, RuntimeError>;

    /// Get a checkpoint by ID.
    async fn get(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointRecord>, RuntimeError>;

    /// Get the latest checkpoint for a run (used by recovery).
    async fn latest_for_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<CheckpointRecord>, RuntimeError>;

    /// List checkpoints for a run.
    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, RuntimeError>;

    /// Set the checkpoint strategy for a run (periodic / on_tool_call / manual).
    ///
    /// Emits `CheckpointStrategySet` so the projection records the strategy.
    async fn set_strategy(
        &self,
        run_id: &RunId,
        strategy_id: String,
        description: String,
        interval_ms: u64,
        max_checkpoints: u32,
        trigger_on_task_complete: bool,
    ) -> Result<CheckpointStrategy, RuntimeError>;

    /// Get the current checkpoint strategy for a run.
    async fn get_strategy(
        &self,
        run_id: &RunId,
    ) -> Result<Option<CheckpointStrategy>, RuntimeError>;
}
