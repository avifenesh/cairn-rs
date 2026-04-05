//! Eval and prompt release API endpoints.
//!
//! Exposes Worker 7's scorecard and release types through the API
//! boundary without re-deriving prompt or eval semantics.

use async_trait::async_trait;
use cairn_domain::ids::{EvalRunId, ProjectId, PromptAssetId, PromptReleaseId};
use cairn_evals::scorecards::{EvalRun, Scorecard};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::endpoints::ListQuery;
use crate::http::ListResponse;

/// API endpoint boundary for eval and prompt surfaces.
#[async_trait]
pub trait EvalsEndpoints: Send + Sync {
    type Error;

    /// Get a scorecard for a prompt asset (comparison across releases).
    async fn get_scorecard(
        &self,
        project_id: &ProjectId,
        prompt_asset_id: &PromptAssetId,
    ) -> Result<Scorecard, Self::Error>;

    /// List eval runs for a prompt release.
    /// Returns the real `EvalRun` from cairn-evals, not a local summary.
    async fn list_eval_runs(
        &self,
        release_id: &PromptReleaseId,
        query: &ListQuery,
    ) -> Result<ListResponse<EvalRun>, Self::Error>;

    /// Get a single eval run by ID.
    async fn get_eval_run(&self, eval_run_id: &EvalRunId) -> Result<Option<EvalRun>, Self::Error>;

    /// Compare outcomes from two eval runs against a shared dataset.
    ///
    /// Returns an `EvalOutcomeComparison` with per-metric deltas and an
    /// optional winner.  The default stub returns an empty comparison (no
    /// winner, no deltas) so that callers compile without a concrete
    /// implementation; override to wire real scoring logic.
    async fn compare_outcomes(
        &self,
        dataset_id: &str,
        run_a_id: &EvalRunId,
        run_b_id: &EvalRunId,
    ) -> Result<EvalOutcomeComparison, Self::Error> {
        Ok(EvalOutcomeComparison {
            dataset_id: dataset_id.to_owned(),
            run_a_id: run_a_id.to_string(),
            run_b_id: run_b_id.to_string(),
            winner: None,
            metric_deltas: HashMap::new(),
        })
    }
}

/// Dataset-linked outcome comparison between two eval runs (RFC 010).
///
/// `metric_deltas` holds the signed difference `run_b - run_a` for each
/// metric that both runs reported.  A positive delta means run_b improved.
/// `winner` is `Some("a")`, `Some("b")`, or `None` when the runs are tied
/// or the comparison is inconclusive.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalOutcomeComparison {
    pub dataset_id: String,
    pub run_a_id: String,
    pub run_b_id: String,
    /// Identifier of the winning run (`"a"` or `"b"`), if determinable.
    pub winner: Option<String>,
    /// Per-metric signed deltas: value = metric(run_b) - metric(run_a).
    pub metric_deltas: HashMap<String, f64>,
}

/// Lightweight dataset summary for list and dashboard surfaces (RFC 010).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalDatasetSummary {
    pub dataset_id: String,
    pub name: String,
    pub record_count: u32,
    pub created_at: u64,
}

/// API-facing eval run summary for SSE/list contexts where the full
/// EvalRun is too heavy. Built from EvalRun, not invented locally.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvalRunSummary {
    pub eval_run_id: String,
    pub prompt_release_id: String,
    pub status: String,
    pub created_at: u64,
}

impl EvalRunSummary {
    pub fn from_eval_run(run: &EvalRun) -> Self {
        Self {
            eval_run_id: run.eval_run_id.to_string(),
            prompt_release_id: run
                .prompt_release_id
                .as_ref()
                .map(|id| id.to_string())
                .unwrap_or_default(),
            status: format!("{:?}", run.status).to_lowercase(),
            created_at: run.created_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_run_summary_serialization() {
        let summary = EvalRunSummary {
            eval_run_id: "eval_1".to_owned(),
            prompt_release_id: "release_1".to_owned(),
            status: "completed".to_owned(),
            created_at: 5000,
        };
        let json = serde_json::to_value(&summary).unwrap();
        assert_eq!(json["evalRunId"], "eval_1");
        assert_eq!(json["status"], "completed");
    }
}
