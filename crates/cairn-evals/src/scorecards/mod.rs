//! Eval run and scorecard boundaries per RFC 004.
//!
//! An eval run evaluates a subject (prompt release, provider route, etc.)
//! and produces metrics. Scorecards aggregate eval results for comparison.

use cairn_domain::{
    EvalRunId, OperatorId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId,
};
use serde::{Deserialize, Serialize};

use crate::matrices::EvalMetrics;

/// Structured dataset source for eval runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DatasetSource {
    pub name: String,
    pub source_type: String,
    pub document_count: u32,
    pub description: Option<String>,
}

/// What is being evaluated.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalSubjectKind {
    PromptRelease,
    ProviderRoute,
    RetrievalPolicy,
    Skill,
    GuardrailPolicy,
}

/// Eval run status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvalRunStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Canceled,
}

/// A single evaluation run.
///
/// Per RFC 004: every eval run that evaluates prompt behavior must
/// reference prompt_asset_id, prompt_version_id, and prompt_release_id
/// when a release was under test.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvalRun {
    pub eval_run_id: EvalRunId,
    pub project_id: ProjectId,
    pub subject_kind: EvalSubjectKind,
    pub status: EvalRunStatus,
    /// Prompt linkage (required when subject is a prompt release).
    pub prompt_asset_id: Option<PromptAssetId>,
    pub prompt_version_id: Option<PromptVersionId>,
    pub prompt_release_id: Option<PromptReleaseId>,
    /// Evaluator configuration.
    pub evaluator_type: String,
    /// Optional dataset ID used for rubric scoring.
    pub dataset_id: Option<String>,
    pub dataset_source: Option<DatasetSource>,
    /// Aggregated metrics from this run.
    pub metrics: EvalMetrics,
    /// Supplemental plugin metrics.
    pub plugin_metrics: Vec<crate::matrices::PluginMetric>,
    pub cost: Option<f64>,
    pub created_by: Option<OperatorId>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
}

/// A scorecard aggregates eval results for a prompt asset across
/// releases, providing the operator with comparison data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Scorecard {
    pub project_id: ProjectId,
    pub prompt_asset_id: PromptAssetId,
    pub entries: Vec<ScorecardEntry>,
}

/// One entry in a scorecard, representing an evaluated release.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ScorecardEntry {
    pub prompt_release_id: PromptReleaseId,
    pub prompt_version_id: PromptVersionId,
    pub eval_run_id: EvalRunId,
    pub metrics: EvalMetrics,
}

impl EvalRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            EvalRunStatus::Completed | EvalRunStatus::Failed | EvalRunStatus::Canceled
        )
    }
}

#[cfg(test)]
mod tests {
    use super::EvalRunStatus;

    #[test]
    fn eval_run_terminal_states() {
        assert!(EvalRunStatus::Completed.is_terminal());
        assert!(EvalRunStatus::Failed.is_terminal());
        assert!(EvalRunStatus::Canceled.is_terminal());
        assert!(!EvalRunStatus::Pending.is_terminal());
        assert!(!EvalRunStatus::Running.is_terminal());
    }
}
