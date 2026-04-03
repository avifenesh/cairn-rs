//! Eval run service boundary per RFC 004.

use async_trait::async_trait;
use cairn_domain::{EvalRunId, ProjectKey};
use cairn_store::projections::EvalRunRecord;

use crate::error::RuntimeError;

/// Eval run service boundary.
///
/// Per RFC 004, eval runs are durable product state that track
/// evaluation of subjects (prompt releases, routes, policies, etc.).
#[async_trait]
pub trait EvalRunService: Send + Sync {
    /// Start a new eval run.
    async fn start(
        &self,
        project: &ProjectKey,
        eval_run_id: EvalRunId,
        subject_kind: String,
        evaluator_type: String,
    ) -> Result<EvalRunRecord, RuntimeError>;

    /// Complete an eval run.
    async fn complete(
        &self,
        eval_run_id: &EvalRunId,
        success: bool,
        error_message: Option<String>,
    ) -> Result<EvalRunRecord, RuntimeError>;

    /// Get an eval run by ID.
    async fn get(&self, eval_run_id: &EvalRunId) -> Result<Option<EvalRunRecord>, RuntimeError>;

    /// List eval runs for a project.
    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<EvalRunRecord>, RuntimeError>;
}
