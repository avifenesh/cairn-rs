//! Prompt registry, release controls, evaluations, and scorecard boundaries.
//!
//! `cairn-evals` owns the prompt-as-product lifecycle:
//!
//! - **Prompts**: assets, immutable versions, project-scoped releases (RFC 006)
//! - **Selectors**: rollout targeting and runtime resolution (RFC 006)
//! - **Matrices**: eval comparison grids with canonical and plugin metrics (RFC 004)
//! - **Scorecards**: aggregated eval results for operator comparison (RFC 004)

pub mod matrices;
pub mod prompts;
pub mod scorecards;
pub mod selectors;
pub mod services;

pub use matrices::{
    EvalMetrics, GuardrailPolicyRow, MatrixCategory, MemorySourceQualityRow, PermissionRow,
    PromptComparisonRow, ProviderRoutingRow, SkillHealthRow,
};
pub use prompts::{
    PromptAsset, PromptAssetStatus, PromptFormat, PromptKind, PromptRelease, PromptReleaseState,
    PromptVersion, PromptVersionMetadata, ReleaseAction, ReleaseActionType,
};
pub use scorecards::{
    DatasetSource, EvalRun, EvalRunStatus, EvalSubjectKind, Scorecard, ScorecardEntry,
};
pub use selectors::{ResolutionContext, RolloutTarget, SelectorKind, SelectorValue};
pub use services::{EvalRunService, GraphIntegration, PromptReleaseService, SelectorResolver};

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_dependency() {
        let id = cairn_domain::PromptAssetId::new("test");
        assert_eq!(id.as_str(), "test");
    }
}
