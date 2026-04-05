//! Eval pipeline integration tests (RFC 013).
//!
//! Validates the end-to-end eval scoring pipeline:
//!   EvalRunService::create_run → start_run → complete_run
//!     → EvalBaselineServiceImpl::set_baseline
//!       → compare_to_baseline (regression / improvement detection)
//!
//! Also validates:
//!   - Multiple runs for the same prompt asset are tracked independently
//!   - Scorecard aggregates only completed runs for the right asset
//!   - Locked baselines are preferred over unlocked ones
//!   - Invalid state transitions are rejected
//!   - Project isolation: list_by_project only returns owned runs

use std::sync::Arc;

use cairn_domain::{
    EvalRunId, PromptAssetId, PromptReleaseId, PromptVersionId, ProjectId, TenantId,
};
use cairn_evals::{
    EvalBaselineServiceImpl, EvalMetrics, EvalRunService, EvalRunStatus, EvalSubjectKind,
};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Complete the Pending→Running→Completed lifecycle and return the final EvalRun.
fn full_run(
    svc: &EvalRunService,
    id: &str,
    project_id: &str,
    asset_id: &PromptAssetId,
    version_id: &str,
    release_id: &str,
    metrics: EvalMetrics,
) -> cairn_evals::EvalRun {
    svc.create_run(
        EvalRunId::new(id),
        ProjectId::new(project_id),
        EvalSubjectKind::PromptRelease,
        "auto_scorer".to_owned(),
        Some(asset_id.clone()),
        Some(PromptVersionId::new(version_id)),
        Some(PromptReleaseId::new(release_id)),
        None,
    );
    svc.start_run(&EvalRunId::new(id)).unwrap();
    svc.complete_run(&EvalRunId::new(id), metrics, None).unwrap()
}

// ── 1. Create → start → complete lifecycle ────────────────────────────────────

#[test]
fn eval_run_full_lifecycle_pending_running_completed() {
    let svc = EvalRunService::new();
    let asset_id = PromptAssetId::new("asset_lifecycle");

    let pending = svc.create_run(
        EvalRunId::new("run_lc"),
        ProjectId::new("proj_lc"),
        EvalSubjectKind::PromptRelease,
        "auto_scorer".to_owned(),
        Some(asset_id.clone()),
        Some(PromptVersionId::new("ver_lc")),
        Some(PromptReleaseId::new("rel_lc")),
        None,
    );
    assert_eq!(pending.status, EvalRunStatus::Pending);
    assert!(pending.completed_at.is_none());

    let running = svc.start_run(&EvalRunId::new("run_lc")).unwrap();
    assert_eq!(running.status, EvalRunStatus::Running);

    let metrics = EvalMetrics {
        task_success_rate: Some(0.88),
        latency_p50_ms: Some(210),
        cost_per_run: Some(0.004),
        ..Default::default()
    };
    let completed = svc
        .complete_run(&EvalRunId::new("run_lc"), metrics.clone(), Some(0.12))
        .unwrap();

    assert_eq!(completed.status, EvalRunStatus::Completed);
    assert_eq!(completed.metrics.task_success_rate, Some(0.88));
    assert_eq!(completed.metrics.latency_p50_ms, Some(210));
    assert!(completed.completed_at.is_some(), "completed_at must be set");
    assert_eq!(completed.cost, Some(0.12));
}

// ── 2. Set a baseline and compare a run that matches ─────────────────────────

#[test]
fn baseline_comparison_passes_when_metrics_match() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());
    let asset_id = PromptAssetId::new("asset_match");

    baselines.set_baseline(
        TenantId::new("t1"),
        "Match Baseline".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.90),
            latency_p50_ms: Some(200),
            ..Default::default()
        },
    );

    full_run(
        &eval_runs,
        "run_match",
        "proj_match",
        &asset_id,
        "ver_1",
        "rel_1",
        EvalMetrics {
            task_success_rate: Some(0.90),  // identical → no delta
            latency_p50_ms: Some(200),
            ..Default::default()
        },
    );

    let cmp = baselines
        .compare_to_baseline(&EvalRunId::new("run_match"))
        .unwrap();

    assert!(cmp.passed, "identical metrics must pass baseline");
    assert!(cmp.regressions.is_empty(), "no regressions");
    assert!(cmp.improvements.is_empty(), "no improvements either");
}

// ── 3. Regression detected: task_success_rate drops > 5% ─────────────────────

#[test]
fn regression_detected_when_success_rate_drops() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());
    let asset_id = PromptAssetId::new("asset_regress");

    baselines.set_baseline(
        TenantId::new("t1"),
        "Regression Baseline".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.90),
            ..Default::default()
        },
    );

    // 0.80 is an 11% drop — beyond the 5% threshold.
    full_run(
        &eval_runs,
        "run_regress",
        "proj_regress",
        &asset_id,
        "ver_bad",
        "rel_bad",
        EvalMetrics {
            task_success_rate: Some(0.80),
            ..Default::default()
        },
    );

    let cmp = baselines
        .compare_to_baseline(&EvalRunId::new("run_regress"))
        .unwrap();

    assert!(!cmp.passed, "regression must fail the comparison");
    assert!(
        cmp.regressions.contains(&"task_success_rate".to_owned()),
        "task_success_rate must be flagged as regression, got: {:?}",
        cmp.regressions,
    );
    assert!(cmp.improvements.is_empty());
}

// ── 4. Improvement detected: task_success_rate rises > 5% ────────────────────

#[test]
fn improvement_detected_when_success_rate_rises() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());
    let asset_id = PromptAssetId::new("asset_improve");

    baselines.set_baseline(
        TenantId::new("t1"),
        "Improvement Baseline".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.80),
            ..Default::default()
        },
    );

    // 0.90 is a 12.5% gain — above the 5% threshold.
    full_run(
        &eval_runs,
        "run_improve",
        "proj_improve",
        &asset_id,
        "ver_good",
        "rel_good",
        EvalMetrics {
            task_success_rate: Some(0.90),
            ..Default::default()
        },
    );

    let cmp = baselines
        .compare_to_baseline(&EvalRunId::new("run_improve"))
        .unwrap();

    assert!(cmp.passed, "improvement still passes");
    assert!(
        cmp.improvements.contains(&"task_success_rate".to_owned()),
        "task_success_rate must be flagged as improvement, got: {:?}",
        cmp.improvements,
    );
    assert!(cmp.regressions.is_empty());
}

// ── 5. Latency regression: p50 increases > 5% ────────────────────────────────

#[test]
fn latency_regression_detected_when_p50_increases() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());
    let asset_id = PromptAssetId::new("asset_latency");

    baselines.set_baseline(
        TenantId::new("t1"),
        "Latency Baseline".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.90),
            latency_p50_ms: Some(100),  // baseline: 100 ms
            ..Default::default()
        },
    );

    // 120 ms is a 20% increase — beyond the 5% threshold (lower is better).
    full_run(
        &eval_runs,
        "run_slow",
        "proj_latency",
        &asset_id,
        "ver_slow",
        "rel_slow",
        EvalMetrics {
            task_success_rate: Some(0.90),
            latency_p50_ms: Some(120),
            ..Default::default()
        },
    );

    let cmp = baselines
        .compare_to_baseline(&EvalRunId::new("run_slow"))
        .unwrap();

    assert!(!cmp.passed, "latency regression must fail the comparison");
    assert!(
        cmp.regressions.contains(&"latency_p50_ms".to_owned()),
        "latency_p50_ms must be flagged as regression, got: {:?}",
        cmp.regressions,
    );
}

// ── 6. Latency improvement: p50 decreases > 5% ───────────────────────────────

#[test]
fn latency_improvement_detected_when_p50_decreases() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());
    let asset_id = PromptAssetId::new("asset_lat_fast");

    baselines.set_baseline(
        TenantId::new("t1"),
        "Fast Baseline".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            latency_p50_ms: Some(200),
            ..Default::default()
        },
    );

    // 150 ms is a 25% improvement.
    full_run(
        &eval_runs,
        "run_fast",
        "proj_lat_fast",
        &asset_id,
        "ver_fast",
        "rel_fast",
        EvalMetrics {
            latency_p50_ms: Some(150),
            ..Default::default()
        },
    );

    let cmp = baselines
        .compare_to_baseline(&EvalRunId::new("run_fast"))
        .unwrap();

    assert!(cmp.passed);
    assert!(
        cmp.improvements.contains(&"latency_p50_ms".to_owned()),
        "latency_p50_ms must be flagged as improvement"
    );
}

// ── 7. Multiple runs for the same asset are tracked independently ─────────────

#[test]
fn multiple_runs_for_same_asset_tracked_independently() {
    let svc = EvalRunService::new();
    let asset_id = PromptAssetId::new("asset_multi");
    let project_id = ProjectId::new("proj_multi");

    let run_a = full_run(
        &svc,
        "run_multi_a",
        "proj_multi",
        &asset_id,
        "ver_1",
        "rel_a",
        EvalMetrics {
            task_success_rate: Some(0.75),
            latency_p50_ms: Some(300),
            ..Default::default()
        },
    );

    let run_b = full_run(
        &svc,
        "run_multi_b",
        "proj_multi",
        &asset_id,
        "ver_2",
        "rel_b",
        EvalMetrics {
            task_success_rate: Some(0.92),
            latency_p50_ms: Some(180),
            ..Default::default()
        },
    );

    // Both runs are independently retrievable.
    let fetched_a = svc.get(&EvalRunId::new("run_multi_a")).unwrap();
    assert_eq!(fetched_a.metrics.task_success_rate, Some(0.75));

    let fetched_b = svc.get(&EvalRunId::new("run_multi_b")).unwrap();
    assert_eq!(fetched_b.metrics.task_success_rate, Some(0.92));

    // Runs don't share state.
    assert_ne!(run_a.eval_run_id, run_b.eval_run_id);
    assert_ne!(
        run_a.metrics.task_success_rate,
        run_b.metrics.task_success_rate
    );
}

// ── 8. Scorecard aggregates completed runs for the queried asset only ─────────

#[test]
fn scorecard_aggregates_only_completed_runs_for_asset() {
    let svc = EvalRunService::new();
    let project_id = ProjectId::new("proj_sc");
    let asset_a = PromptAssetId::new("asset_sc_a");
    let asset_b = PromptAssetId::new("asset_sc_b");

    // Two completed runs for asset A.
    full_run(&svc, "sc_a1", "proj_sc", &asset_a, "va1", "ra1", EvalMetrics {
        task_success_rate: Some(0.80),
        ..Default::default()
    });
    full_run(&svc, "sc_a2", "proj_sc", &asset_a, "va2", "ra2", EvalMetrics {
        task_success_rate: Some(0.88),
        ..Default::default()
    });

    // One completed run for asset B.
    full_run(&svc, "sc_b1", "proj_sc", &asset_b, "vb1", "rb1", EvalMetrics {
        task_success_rate: Some(0.60),
        ..Default::default()
    });

    // A pending run for asset A (not yet completed — must NOT appear).
    svc.create_run(
        EvalRunId::new("sc_a_pending"),
        project_id.clone(),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        Some(asset_a.clone()),
        Some(PromptVersionId::new("va_pending")),
        Some(PromptReleaseId::new("ra_pending")),
        None,
    );

    let scorecard_a = svc.build_scorecard(&project_id, &asset_a);
    assert_eq!(scorecard_a.entries.len(), 2, "only completed runs for asset A");
    let release_ids: Vec<_> = scorecard_a
        .entries
        .iter()
        .map(|e| e.prompt_release_id.as_str())
        .collect();
    assert!(release_ids.contains(&"ra1"));
    assert!(release_ids.contains(&"ra2"));

    let scorecard_b = svc.build_scorecard(&project_id, &asset_b);
    assert_eq!(scorecard_b.entries.len(), 1, "only asset B's run");
    assert_eq!(scorecard_b.entries[0].prompt_release_id.as_str(), "rb1");
}

// ── 9. Locked baseline takes priority over unlocked ───────────────────────────

#[test]
fn locked_baseline_preferred_over_unlocked() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());
    let asset_id = PromptAssetId::new("asset_lock");

    // Create an unlocked baseline with high success rate.
    baselines.set_baseline(
        TenantId::new("t_lock"),
        "Unlocked (should be ignored)".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.99),  // impossibly high — would cause regression
            ..Default::default()
        },
    );

    // Create a locked baseline with a realistic rate.
    let locked = baselines.set_baseline(
        TenantId::new("t_lock"),
        "Locked (should be used)".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.85),
            ..Default::default()
        },
    );
    baselines.lock(&locked.baseline_id).unwrap();

    // Run that matches the locked baseline (0.85).
    full_run(
        &eval_runs,
        "run_lock",
        "proj_lock",
        &asset_id,
        "ver_lock",
        "rel_lock",
        EvalMetrics {
            task_success_rate: Some(0.85),
            ..Default::default()
        },
    );

    let cmp = baselines
        .compare_to_baseline(&EvalRunId::new("run_lock"))
        .unwrap();

    // If it compared against the unlocked (0.99), this would be a regression.
    // Passing proves the locked baseline was selected.
    assert!(cmp.passed, "locked baseline (0.85) must be selected, not unlocked (0.99)");
    assert_eq!(cmp.baseline_metrics.task_success_rate, Some(0.85));
}

// ── 10. Invalid transitions are rejected ──────────────────────────────────────

#[test]
fn cannot_complete_a_pending_run() {
    let svc = EvalRunService::new();

    svc.create_run(
        EvalRunId::new("run_bad_transition"),
        ProjectId::new("proj_bt"),
        EvalSubjectKind::PromptRelease,
        "auto".to_owned(),
        None,
        None,
        None,
        None,
    );

    let err = svc.complete_run(
        &EvalRunId::new("run_bad_transition"),
        EvalMetrics::default(),
        None,
    );
    assert!(
        err.is_err(),
        "completing a Pending run (skipping Running) must fail"
    );
}

#[test]
fn cannot_start_a_completed_run() {
    let svc = EvalRunService::new();
    let asset_id = PromptAssetId::new("asset_no_restart");

    full_run(&svc, "run_done", "proj_no_restart", &asset_id, "v1", "r1", EvalMetrics::default());

    let err = svc.start_run(&EvalRunId::new("run_done"));
    assert!(
        err.is_err(),
        "starting a Completed run must fail"
    );
}

// ── 11. list_by_project is project-scoped ─────────────────────────────────────

#[test]
fn list_by_project_returns_only_matching_project_runs() {
    let svc = EvalRunService::new();
    let asset = PromptAssetId::new("asset_scope");

    full_run(&svc, "run_p1_a", "proj_p1", &asset, "v1", "r1", EvalMetrics::default());
    full_run(&svc, "run_p1_b", "proj_p1", &asset, "v2", "r2", EvalMetrics::default());
    full_run(&svc, "run_p2",   "proj_p2", &asset, "v3", "r3", EvalMetrics::default());

    let p1_runs = svc.list_by_project(&ProjectId::new("proj_p1"));
    assert_eq!(p1_runs.len(), 2, "proj_p1 owns 2 runs");
    assert!(p1_runs.iter().all(|r| r.project_id == ProjectId::new("proj_p1")));

    let p2_runs = svc.list_by_project(&ProjectId::new("proj_p2"));
    assert_eq!(p2_runs.len(), 1);
    assert_eq!(p2_runs[0].eval_run_id.as_str(), "run_p2");
}

// ── 12. compare_to_baseline links run_id and baseline_id correctly ─────────────

#[test]
fn comparison_result_carries_correct_run_and_baseline_ids() {
    let eval_runs = Arc::new(EvalRunService::new());
    let baselines = EvalBaselineServiceImpl::new(eval_runs.clone());
    let asset_id = PromptAssetId::new("asset_ids");

    let baseline = baselines.set_baseline(
        TenantId::new("t_ids"),
        "ID Check Baseline".to_owned(),
        asset_id.clone(),
        EvalMetrics {
            task_success_rate: Some(0.85),
            ..Default::default()
        },
    );

    full_run(
        &eval_runs,
        "run_ids",
        "proj_ids",
        &asset_id,
        "v_ids",
        "r_ids",
        EvalMetrics {
            task_success_rate: Some(0.85),
            ..Default::default()
        },
    );

    let cmp = baselines
        .compare_to_baseline(&EvalRunId::new("run_ids"))
        .unwrap();

    assert_eq!(cmp.run_id, "run_ids");
    assert_eq!(cmp.baseline_id, baseline.baseline_id);
    assert_eq!(cmp.run_metrics.task_success_rate, Some(0.85));
    assert_eq!(cmp.baseline_metrics.task_success_rate, Some(0.85));
}
