use std::sync::Arc;

use cairn_domain::{EvalRunId, ProjectId, PromptAssetId, TenantId};
use cairn_evals::{EvalBaselineServiceImpl, EvalMetrics, EvalRunService, EvalSubjectKind};

#[test]
fn baseline_comparison_flags_task_success_regression() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());

    let prompt_asset_id = PromptAssetId::new("prompt_planner");
    let baseline = baselines.set_baseline(
        TenantId::new("tenant_baseline"),
        "Planner Baseline".to_owned(),
        prompt_asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.9),
            ..Default::default()
        },
    );
    assert!(!baseline.locked);

    let run = eval_runs.create_run(
        EvalRunId::new("eval_baseline_regression"),
        ProjectId::new("proj_eval"),
        EvalSubjectKind::PromptRelease,
        "llm_judge".to_owned(),
        Some(prompt_asset_id),
        None,
        None,
        None,
    );
    assert_eq!(run.dataset_id, None);

    eval_runs.start_run(&run.eval_run_id).unwrap();
    eval_runs
        .complete_run(
            &run.eval_run_id,
            EvalMetrics {
                task_success_rate: Some(0.8),
                ..Default::default()
            },
            None,
        )
        .unwrap();

    let comparison = baselines.compare_to_baseline(&run.eval_run_id).unwrap();
    assert_eq!(comparison.baseline_id, baseline.baseline_id);
    assert_eq!(comparison.run_metrics.task_success_rate, Some(0.8));
    assert_eq!(comparison.baseline_metrics.task_success_rate, Some(0.9));
    assert!(comparison
        .regressions
        .contains(&"task_success_rate".to_owned()));
    assert!(comparison.improvements.is_empty());
    assert!(!comparison.passed);
}
