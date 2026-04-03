//! Eval matrix boundaries per RFC 004.
//!
//! Matrices are product state backed by stable schemas. Each matrix has:
//! - A canonical subject type
//! - A canonical row grain
//! - A canonical metric set
//! - A canonical scope model

use cairn_domain::{
    EvalRunId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId, ProviderBindingId,
};
use serde::{Deserialize, Serialize};

/// Matrix category per RFC 004.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatrixCategory {
    PromptComparison,
    ProviderRouting,
    Permission,
    MemorySourceQuality,
    SkillHealth,
    GuardrailPolicyOutcome,
}

/// A single row in the prompt comparison matrix.
///
/// Canonical subject: `prompt_release`
/// Canonical row grain: one row per evaluated prompt_release_id x
/// provider_binding_id x effective selector context.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptComparisonRow {
    pub project_id: ProjectId,
    pub prompt_release_id: PromptReleaseId,
    pub prompt_asset_id: PromptAssetId,
    pub prompt_version_id: PromptVersionId,
    pub provider_binding_id: Option<ProviderBindingId>,
    pub eval_run_id: EvalRunId,
    pub metrics: EvalMetrics,
}

/// Built-in canonical metrics required for operator comparison.
///
/// Per RFC 004: plugin-defined supplemental metrics may extend but
/// not replace these.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
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

/// Supplemental plugin-defined metric.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginMetric {
    pub name: String,
    pub value_type: MetricValueType,
    pub value: MetricValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricValueType {
    Float,
    Integer,
    Boolean,
    String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetricValue {
    Float(f64),
    Integer(i64),
    Boolean(bool),
    String(String),
}

#[cfg(test)]
mod tests {
    use super::{EvalMetrics, MatrixCategory};

    #[test]
    fn matrix_categories_are_distinct() {
        assert_ne!(
            MatrixCategory::PromptComparison,
            MatrixCategory::ProviderRouting
        );
    }

    #[test]
    fn eval_metrics_default_is_all_none() {
        let m = EvalMetrics::default();
        assert!(m.task_success_rate.is_none());
        assert!(m.latency_p50_ms.is_none());
    }
}
