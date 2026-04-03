//! Checkpoint service boundary per RFC 005.
//!
//! Checkpoints are first-class durable recovery points for runs.
//! They are immutable once created; one may be marked latest.

use async_trait::async_trait;
use cairn_domain::{CheckpointId, ProjectKey, RunId};
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
}
