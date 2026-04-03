//! Guard: Worker 8 can consume all prompt/eval/scorecard types from
//! cairn_evals crate root without reaching into submodules or re-deriving
//! any prompt/eval semantics locally.

use cairn_evals::{
    EvalMetrics, EvalRunService, EvalRunStatus, EvalSubjectKind, MatrixCategory, PromptAssetStatus,
    PromptFormat, PromptKind, PromptReleaseService, PromptReleaseState, ReleaseActionType,
    ResolutionContext, RolloutTarget, Scorecard, ScorecardEntry, SelectorKind,
};

/// Every type Worker 8 needs for API surfaces is importable from the crate root.
#[test]
fn all_api_facing_types_importable_from_crate_root() {
    // Release lifecycle types
    let _: PromptReleaseState = PromptReleaseState::Draft;
    let _: ReleaseActionType = ReleaseActionType::Activate;
    let _: PromptAssetStatus = PromptAssetStatus::Active;
    let _: PromptFormat = PromptFormat::PlainText;
    let _: PromptKind = PromptKind::System;

    // Selector types
    let _: SelectorKind = SelectorKind::ProjectDefault;
    let target = RolloutTarget::project_default();
    assert!(target.matches(&ResolutionContext::default()));

    // Eval types
    let _: EvalRunStatus = EvalRunStatus::Completed;
    let _: EvalSubjectKind = EvalSubjectKind::PromptRelease;
    let _: MatrixCategory = MatrixCategory::PromptComparison;

    // Metrics
    let metrics = EvalMetrics::default();
    assert!(metrics.task_success_rate.is_none());
}

/// Scorecard can be built and read without any submodule imports.
#[test]
fn scorecard_readable_from_crate_root_types() {
    let svc = EvalRunService::new();
    let release_svc = PromptReleaseService::new();

    // Create minimal data
    release_svc.create(
        cairn_domain::PromptReleaseId::new("r1"),
        cairn_domain::ProjectId::new("p1"),
        cairn_domain::PromptAssetId::new("a1"),
        cairn_domain::PromptVersionId::new("v1"),
        RolloutTarget::project_default(),
    );

    svc.create_run(
        cairn_domain::EvalRunId::new("e1"),
        cairn_domain::ProjectId::new("p1"),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        Some(cairn_domain::PromptAssetId::new("a1")),
        Some(cairn_domain::PromptVersionId::new("v1")),
        Some(cairn_domain::PromptReleaseId::new("r1")),
        None,
    );
    svc.start_run(&cairn_domain::EvalRunId::new("e1")).unwrap();
    svc.complete_run(
        &cairn_domain::EvalRunId::new("e1"),
        EvalMetrics {
            task_success_rate: Some(0.9),
            ..Default::default()
        },
        None,
    )
    .unwrap();

    let scorecard: Scorecard = svc.build_scorecard(
        &cairn_domain::ProjectId::new("p1"),
        &cairn_domain::PromptAssetId::new("a1"),
    );

    // Worker 8 reads this directly
    assert_eq!(scorecard.entries.len(), 1);
    let entry: &ScorecardEntry = &scorecard.entries[0];
    assert_eq!(entry.metrics.task_success_rate, Some(0.9));
}
