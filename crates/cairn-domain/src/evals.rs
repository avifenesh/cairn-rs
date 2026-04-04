use crate::{PromptAssetId, TenantId};
use serde::{Deserialize, Serialize};

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

/// A single eval dataset entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalDatasetEntry {
    pub input: serde_json::Value,
    pub expected_output: Option<serde_json::Value>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A tenant-scoped dataset used to drive eval runs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvalDataset {
    pub dataset_id: String,
    pub tenant_id: TenantId,
    pub name: String,
    pub subject_kind: EvalSubjectKind,
    #[serde(default)]
    pub entries: Vec<EvalDatasetEntry>,
    pub created_at_ms: u64,
}

/// Built-in canonical metrics required for operator comparison.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct EvalMetrics {
    pub task_success_rate: Option<f64>,
    pub latency_p50_ms: Option<u64>,
    pub latency_p99_ms: Option<u64>,
    pub cost_per_run: Option<f64>,
    pub policy_pass_rate: Option<f64>,
    pub retrieval_hit_at_k: Option<f64>,
    pub citation_coverage: Option<f64>,
    pub source_diversity: Option<f64>,
    pub retrieval_latency_ms: Option<u64>,
    pub retrieval_cost: Option<f64>,
}

/// Supported rubric scoring functions for eval datasets.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RubricScoringFn {
    ExactMatch,
    Contains,
    Similarity,
    Plugin,
    Custom,
}

/// One scoring dimension within an eval rubric.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RubricDimension {
    pub name: String,
    pub weight: f64,
    pub scoring_fn: RubricScoringFn,
    pub threshold: Option<f64>,
    #[serde(default)]
    pub plugin_id: Option<String>,
}

/// Tenant-scoped rubric used to score eval runs against expected outputs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalRubric {
    pub rubric_id: String,
    pub tenant_id: TenantId,
    pub name: String,
    #[serde(default)]
    pub dimensions: Vec<RubricDimension>,
    pub created_at_ms: u64,
}

/// Result of applying a rubric to an eval run.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RubricScoreResult {
    pub run_id: String,
    pub rubric_id: String,
    pub dimension_scores: Vec<(String, f64)>,
    pub overall: f64,
}

/// Tenant-scoped eval baseline used to detect regressions over time.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EvalBaseline {
    pub baseline_id: String,
    pub tenant_id: TenantId,
    pub name: String,
    pub prompt_asset_id: PromptAssetId,
    pub metrics: EvalMetrics,
    pub created_at_ms: u64,
    pub locked: bool,
}

/// Result of comparing an eval run against its selected baseline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BaselineComparison {
    pub run_id: String,
    pub baseline_id: String,
    pub run_metrics: EvalMetrics,
    pub baseline_metrics: EvalMetrics,
    pub regressions: Vec<String>,
    pub improvements: Vec<String>,
    pub passed: bool,
}

/// Status of a model comparison run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelComparisonStatus {
    Pending,
    Running,
    Completed,
}

/// Side-by-side comparison of two AI model bindings on the same eval dataset.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelComparisonRun {
    pub comparison_id: String,
    pub tenant_id: TenantId,
    pub dataset_id: String,
    pub model_a_binding_id: String,
    pub model_b_binding_id: String,
    pub status: ModelComparisonStatus,
    pub results_a: Option<EvalMetrics>,
    pub results_b: Option<EvalMetrics>,
    /// binding_id of the winning model, or None if not yet determined.
    pub winner: Option<String>,
    pub created_at_ms: u64,
}
