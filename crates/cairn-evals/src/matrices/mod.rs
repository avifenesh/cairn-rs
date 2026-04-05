//! Eval matrix boundaries per RFC 004.
//!
//! Matrices are product state backed by stable schemas. Each matrix has:
//! - A canonical subject type
//! - A canonical row grain
//! - A canonical metric set
//! - A canonical scope model

use cairn_domain::{
    EvalRunId, PolicyId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId,
    ProviderBindingId, RouteDecisionId, SourceId,
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

/// A single row in the provider routing matrix.
///
/// Canonical subject: `route_decision`
/// Canonical row grain: one row per provider binding.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderRoutingRow {
    pub project_id: ProjectId,
    pub route_decision_id: RouteDecisionId,
    pub provider_binding_id: Option<ProviderBindingId>,
    pub eval_run_id: EvalRunId,
    pub metrics: EvalMetrics,
    /// Accumulated cost across all provider calls for this binding (in micros).
    #[serde(default)]
    pub total_cost_micros: u64,
    /// Fraction of provider calls that succeeded (0.0–1.0).
    #[serde(default)]
    pub success_rate: f64,
}

/// RFC 004: aggregated provider routing matrix for a project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderRoutingMatrix {
    pub rows: Vec<ProviderRoutingRow>,
}

/// A single row in the permission matrix.
///
/// Canonical subject: permission decision family.
/// Canonical row grain: one row per effective permission policy outcome
/// for mode x capability x scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PermissionRow {
    pub project_id: ProjectId,
    pub policy_id: PolicyId,
    pub mode: String,
    pub capability: String,
    pub eval_run_id: EvalRunId,
    pub metrics: EvalMetrics,
}

/// A single row in the memory source quality matrix.
///
/// Canonical subject: memory source or source document family.
/// Canonical row grain: one row per source_id within scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemorySourceQualityRow {
    pub project_id: ProjectId,
    pub source_id: SourceId,
    pub eval_run_id: EvalRunId,
    pub metrics: EvalMetrics,
}

/// A single row in the skill health / intervention matrix.
///
/// Canonical subject: skill.
/// Canonical row grain: one row per skill_id within scope.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SkillHealthRow {
    pub project_id: ProjectId,
    pub skill_id: String,
    pub eval_run_id: EvalRunId,
    pub metrics: EvalMetrics,
}

/// A single row in the guardrail / policy outcome matrix.
///
/// Canonical subject: policy or guardrail rule.
/// Canonical row grain: one row per policy-rule outcome slice.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuardrailPolicyRow {
    pub project_id: ProjectId,
    pub policy_id: PolicyId,
    pub rule_name: String,
    pub eval_run_id: EvalRunId,
    pub metrics: EvalMetrics,
}

/// Built-in canonical metrics required for operator comparison.
///
/// Per RFC 004: plugin-defined supplemental metrics may extend but
/// not replace these. Re-exported from cairn_domain to keep a single canonical type.
pub use cairn_domain::EvalMetrics;

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

// ── Matrix container types ─────────────────────────────────────────────────
// These wrap the existing *Row types into named matrix responses used by
// the operator API.

/// Matrix of prompt comparison results across releases.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PromptComparisonMatrix {
    pub rows: Vec<PromptComparisonRow>,
}

/// Matrix of permission-policy eval results.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PermissionMatrix {
    pub rows: Vec<PermissionRow>,
}

/// Matrix of skill health eval results.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SkillHealthMatrix {
    pub rows: Vec<SkillHealthRow>,
}

/// Matrix of guardrail policy eval results.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GuardrailMatrix {
    pub rows: Vec<GuardrailPolicyRow>,
}

/// Matrix of memory source quality eval results.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MemorySourceQualityMatrix {
    pub rows: Vec<MemorySourceQualityRow>,
}
