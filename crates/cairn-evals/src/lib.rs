//! Prompt registry, release controls, evaluations, and scorecard boundaries.
//!
//! `cairn-evals` owns the prompt-as-product lifecycle:
//!
//! - **Prompts**: assets, immutable versions, project-scoped releases (RFC 006)
//! - **Selectors**: rollout targeting and runtime resolution (RFC 006)
//! - **Matrices**: eval comparison grids with canonical and plugin metrics (RFC 004)
//! - **Scorecards**: aggregated eval results for operator comparison (RFC 004)

pub mod experiments;
pub mod matrices;
pub mod prompts;
pub mod scorecards;
pub mod selectors;
pub mod services;

pub use matrices::{
    DimensionScore, EvalMetrics, GuardrailMatrix, GuardrailPolicyRow, MatrixCategory,
    MemorySourceQualityMatrix, MemorySourceQualityRow, MetricDelta, ModelEvalCell, ModelEvalMatrix,
    PermissionMatrix, PermissionRow, PromptComparisonMatrix, PromptComparisonRow,
    ProviderRoutingMatrix, ProviderRoutingRow, RubricDimensionDef, RubricScoringResult,
    RunComparison, SkillHealthMatrix, SkillHealthRow,
};
pub use prompts::{
    PromptAsset, PromptAssetStatus, PromptFormat, PromptKind, PromptRelease, PromptReleaseState,
    PromptVersion, PromptVersionMetadata, ReleaseAction, ReleaseActionType,
};
pub use scorecards::{
    DatasetSource, EvalRun, EvalRunStatus, EvalSubjectKind, Scorecard, ScorecardEntry,
};
pub use selectors::{ResolutionContext, RolloutTarget, SelectorKind, SelectorValue};
pub use services::eval_service::{MemoryDiagnosticsSource, SourceQualitySnapshot};
pub use services::{
    EvalBaselineServiceImpl, EvalDatasetServiceImpl, EvalRubricServiceImpl, EvalRunService,
    GraphIntegration, ModelComparisonServiceImpl, PluginDimensionScore, PluginRubricScorer,
    PromptReleaseService, SelectorResolver,
};
// Re-export RubricDimension from domain for cairn-app convenience.
pub use cairn_domain::RubricDimension;

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_dependency() {
        let id = cairn_domain::PromptAssetId::new("test");
        assert_eq!(id.as_str(), "test");
    }
}
