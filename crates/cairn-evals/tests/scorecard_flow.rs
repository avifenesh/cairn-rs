//! RFC 013 eval scorecard integration tests.
//!
//! Validates the full scoring pipeline:
//! - Three eval runs with varying metrics are created and completed.
//! - A baseline is set from the best run's metrics.
//! - Comparing the worst run to the baseline flags regressions on every dimension.
//! - Comparing the best run to the baseline reports no regressions.
//! - The scorecard tracks multiple metric dimensions correctly.

use std::sync::Arc;

use cairn_domain::{
    EvalRunId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId, TenantId,
};
use cairn_evals::{
    EvalBaselineServiceImpl, EvalMetrics, EvalRunService,
    scorecards::EvalSubjectKind,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn project_id() -> ProjectId {
    ProjectId::new("proj_scorecard")
}

fn tenant_id() -> TenantId {
    TenantId::new("tenant_scorecard")
}

fn asset_id() -> PromptAssetId {
    PromptAssetId::new("prompt_assistant")
}

/// Run lifecycle helper: create → start → complete in one call.
fn run_full(
    svc: &EvalRunService,
    id: &str,
    version: &str,
    release: &str,
    metrics: EvalMetrics,
) {
    svc.create_run(
        EvalRunId::new(id),
        project_id(),
        EvalSubjectKind::PromptRelease,
        "llm_judge".to_owned(),
        Some(asset_id()),
        Some(PromptVersionId::new(version)),
        Some(PromptReleaseId::new(release)),
        None, // created_by
    );
    svc.start_run(&EvalRunId::new(id)).unwrap();
    svc.complete_run(&EvalRunId::new(id), metrics, None).unwrap();
}

// ── baseline metrics ──────────────────────────────────────────────────────────
//
// Baseline (set from best run):
//   task_success_rate = 0.92  (higher is better)
//   latency_p50_ms    = 150   (lower is better)
//   cost_per_run      = 0.002 (lower is better)
//
// Best run: beats baseline on all dimensions by >5%.
//   task_success_rate = 0.97  (+5.4% → improvement)
//   latency_p50_ms    = 140   (-6.7% → improvement)
//   cost_per_run      = 0.0018 (-10% → improvement)
//
// Middle run: close to baseline, no regressions or improvements.
//   task_success_rate = 0.91  (-1.1% → within ±5% tolerance)
//   latency_p50_ms    = 155   (+3.3% → within tolerance)
//   cost_per_run      = 0.0022 (+10% → regression!)
//
// Worst run: regresses on all three dimensions.
//   task_success_rate = 0.75  (-18.5% → regression!)
//   latency_p50_ms    = 300   (+100%  → regression!)
//   cost_per_run      = 0.008 (+300%  → regression!)

fn best_metrics() -> EvalMetrics {
    EvalMetrics {
        task_success_rate: Some(0.97),  // +5.4% vs baseline 0.92 → improvement
        latency_p50_ms: Some(140),      // -6.7% vs baseline 150ms → improvement
        cost_per_run: Some(0.0018),     // -10%  vs baseline 0.002 → improvement
        ..Default::default()
    }
}

fn middle_metrics() -> EvalMetrics {
    EvalMetrics {
        task_success_rate: Some(0.91),
        latency_p50_ms: Some(155),
        cost_per_run: Some(0.0022),
        ..Default::default()
    }
}

fn worst_metrics() -> EvalMetrics {
    EvalMetrics {
        task_success_rate: Some(0.75),
        latency_p50_ms: Some(300),
        cost_per_run: Some(0.008),
        ..Default::default()
    }
}

fn baseline_metrics() -> EvalMetrics {
    EvalMetrics {
        task_success_rate: Some(0.92),
        latency_p50_ms: Some(150),
        cost_per_run: Some(0.002),
        ..Default::default()
    }
}

// ── tests ─────────────────────────────────────────────────────────────────────

/// (1) Create EvalRunService + EvalBaselineServiceImpl;
/// (2) Run 3 eval runs with varying metrics.
#[test]
fn three_runs_complete_successfully() {
    let eval_runs = Arc::new(EvalRunService::new());

    run_full(&eval_runs, "run_best",   "pv_1", "rel_best",   best_metrics());
    run_full(&eval_runs, "run_middle", "pv_2", "rel_middle",  middle_metrics());
    run_full(&eval_runs, "run_worst",  "pv_3", "rel_worst",   worst_metrics());

    let runs = eval_runs.list_by_project(&project_id());
    assert_eq!(runs.len(), 3, "all three runs must be listed");

    use cairn_evals::scorecards::EvalRunStatus;
    assert!(
        runs.iter().all(|r| r.status == EvalRunStatus::Completed),
        "all runs must be Completed"
    );
}

/// (3) Set baseline from best run's metrics; verify it is stored and retrievable.
#[test]
fn set_baseline_from_best_run() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());

    run_full(&eval_runs, "run_best", "pv_1", "rel_best", best_metrics());

    let baseline = baselines.set_baseline(
        tenant_id(),
        "Best run baseline".to_owned(),
        asset_id(),
        baseline_metrics(), // set explicit known-good values as the baseline
    );

    assert!(!baseline.baseline_id.is_empty(), "baseline ID must be assigned");
    assert_eq!(baseline.prompt_asset_id, asset_id());
    assert_eq!(baseline.metrics.task_success_rate, Some(0.92));
    assert_eq!(baseline.metrics.latency_p50_ms, Some(150));
    assert_eq!(baseline.metrics.cost_per_run, Some(0.002));

    // Must be retrievable.
    let stored = baselines.get(&baseline.baseline_id).expect("baseline must be stored");
    assert_eq!(stored.baseline_id, baseline.baseline_id);

    // Listed for the tenant.
    let all = baselines.list(&tenant_id());
    assert_eq!(all.len(), 1);
}

/// (4) Compare worst run to baseline — all three tracked dimensions must be flagged.
#[test]
fn worst_run_regressions_flagged() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());

    run_full(&eval_runs, "run_worst", "pv_3", "rel_worst", worst_metrics());

    baselines.set_baseline(
        tenant_id(),
        "v1 baseline".to_owned(),
        asset_id(),
        baseline_metrics(),
    );

    let comparison = baselines
        .compare_to_baseline(&EvalRunId::new("run_worst"))
        .unwrap();

    assert!(
        !comparison.passed,
        "comparison must FAIL: worst run regresses on all key metrics"
    );
    assert!(
        !comparison.regressions.is_empty(),
        "regressions must be non-empty for the worst run"
    );

    // All three manager-specified dimensions must appear in regressions.
    assert!(
        comparison.regressions.contains(&"task_success_rate".to_owned()),
        "task_success_rate regression must be flagged (0.75 vs baseline 0.92, -18.5%)"
    );
    assert!(
        comparison.regressions.contains(&"latency_p50_ms".to_owned()),
        "latency_p50_ms regression must be flagged (300ms vs baseline 150ms, +100%)"
    );
    assert!(
        comparison.regressions.contains(&"cost_per_run".to_owned()),
        "cost_per_run regression must be flagged (0.008 vs baseline 0.002, +300%)"
    );
}

/// (5) Compare best run to baseline — no regressions on the key dimensions.
#[test]
fn best_run_no_regressions() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());

    run_full(&eval_runs, "run_best", "pv_1", "rel_best", best_metrics());

    baselines.set_baseline(
        tenant_id(),
        "v1 baseline".to_owned(),
        asset_id(),
        baseline_metrics(),
    );

    let comparison = baselines
        .compare_to_baseline(&EvalRunId::new("run_best"))
        .unwrap();

    assert!(
        comparison.passed,
        "comparison must PASS: best run does not regress on any metric"
    );
    assert!(
        comparison.regressions.is_empty(),
        "regressions must be empty for the best run, got: {:?}",
        comparison.regressions
    );

    // Best run improves on all three dimensions relative to baseline.
    assert!(
        comparison.improvements.contains(&"task_success_rate".to_owned()),
        "task_success_rate improvement expected (0.97 vs baseline 0.92, +5.4%)"
    );
    assert!(
        comparison.improvements.contains(&"latency_p50_ms".to_owned()),
        "latency_p50_ms improvement expected (140ms vs baseline 150ms, -6.7%)"
    );
    assert!(
        comparison.improvements.contains(&"cost_per_run".to_owned()),
        "cost_per_run improvement expected (0.0018 vs baseline 0.002, -10%)"
    );
}

/// (6) Scorecard tracks all three metric dimensions across all completed runs.
#[test]
fn scorecard_tracks_multiple_metric_dimensions() {
    let eval_runs = Arc::new(EvalRunService::new());

    run_full(&eval_runs, "run_best",   "pv_1", "rel_best",   best_metrics());
    run_full(&eval_runs, "run_middle", "pv_2", "rel_middle",  middle_metrics());
    run_full(&eval_runs, "run_worst",  "pv_3", "rel_worst",   worst_metrics());

    let scorecard = eval_runs.build_scorecard(&project_id(), &asset_id());

    assert_eq!(scorecard.entries.len(), 3, "scorecard must contain all 3 completed runs");
    assert_eq!(scorecard.prompt_asset_id, asset_id());
    assert_eq!(scorecard.project_id, project_id());

    // Every entry must carry all three tracked dimensions.
    for entry in &scorecard.entries {
        assert!(
            entry.metrics.task_success_rate.is_some(),
            "scorecard entry for {:?} must have task_success_rate",
            entry.prompt_release_id
        );
        assert!(
            entry.metrics.latency_p50_ms.is_some(),
            "scorecard entry for {:?} must have latency_p50_ms",
            entry.prompt_release_id
        );
        assert!(
            entry.metrics.cost_per_run.is_some(),
            "scorecard entry for {:?} must have cost_per_run",
            entry.prompt_release_id
        );
    }

    // Verify the spread across entries on task_success_rate.
    let mut rates: Vec<f64> = scorecard
        .entries
        .iter()
        .map(|e| e.metrics.task_success_rate.unwrap())
        .collect();
    rates.sort_by(|a, b| a.partial_cmp(b).unwrap());

    assert!(
        (rates[0] - 0.75).abs() < 0.001,
        "lowest task_success_rate must be worst run (0.75)"
    );
    assert!(
        (rates[2] - 0.97).abs() < 0.001,
        "highest task_success_rate must be best run (0.97)"
    );

    // Best entry has lowest latency.
    let best_entry = scorecard
        .entries
        .iter()
        .min_by_key(|e| e.metrics.latency_p50_ms.unwrap_or(u64::MAX))
        .unwrap();
    assert_eq!(
        best_entry.metrics.latency_p50_ms,
        Some(140),
        "best latency must be 140ms"
    );

    // Worst entry has highest cost.
    let worst_entry = scorecard
        .entries
        .iter()
        .max_by(|a, b| {
            a.metrics
                .cost_per_run
                .unwrap_or(0.0)
                .partial_cmp(&b.metrics.cost_per_run.unwrap_or(0.0))
                .unwrap()
        })
        .unwrap();
    assert!(
        (worst_entry.metrics.cost_per_run.unwrap() - 0.008).abs() < 0.0001,
        "worst cost must be 0.008"
    );
}

/// Tolerance band: metrics within ±5% of baseline are neither regressions nor improvements.
#[test]
fn within_tolerance_band_no_regression_no_improvement() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());

    // Run with metrics exactly at baseline (0% delta).
    run_full(
        &eval_runs,
        "run_exact",
        "pv_exact",
        "rel_exact",
        baseline_metrics(),
    );

    baselines.set_baseline(
        tenant_id(),
        "exact baseline".to_owned(),
        asset_id(),
        baseline_metrics(),
    );

    let comparison = baselines
        .compare_to_baseline(&EvalRunId::new("run_exact"))
        .unwrap();

    assert!(
        comparison.passed,
        "identical metrics to baseline must pass with no regressions"
    );
    assert!(
        comparison.regressions.is_empty(),
        "zero-delta run must have no regressions"
    );
    assert!(
        comparison.improvements.is_empty(),
        "zero-delta run must have no improvements either"
    );
}
