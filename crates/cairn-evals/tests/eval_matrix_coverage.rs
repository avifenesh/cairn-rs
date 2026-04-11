//! Eval matrix coverage tests (RFC 004).
//!
//! Validates the RFC 004 matrix boundaries: PromptComparisonMatrix,
//! GuardrailMatrix, PermissionMatrix, ProviderRoutingMatrix, and
//! SkillHealthMatrix all carry correctly-typed rows with the right fields,
//! project scoping, and eval_run_id linkage.
//!
//! Matrix types are pure data containers (Vec<Row>); the tests prove:
//!   - Row construction round-trips all fields
//!   - eval_run_id links each matrix row back to its originating eval run
//!   - project_id on every row enables cross-project filtering
//!   - Metric values (policy_pass_rate, task_success_rate, etc.) are correct
//!   - Matrices compose correctly when populated from EvalRunService results

use cairn_domain::{
    EvalRunId, PolicyId, ProjectId, PromptAssetId, PromptReleaseId, PromptVersionId,
    ProviderBindingId, RouteDecisionId, SourceId,
};
use cairn_evals::{
    EvalMetrics, EvalRunService, EvalSubjectKind, GuardrailMatrix, GuardrailPolicyRow,
    MemorySourceQualityMatrix, MemorySourceQualityRow, PermissionMatrix, PermissionRow,
    PromptComparisonMatrix, PromptComparisonRow, ProviderRoutingMatrix, ProviderRoutingRow,
    SkillHealthMatrix, SkillHealthRow,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn metrics(task_success: f64, policy_pass: f64, latency_p50: u64) -> EvalMetrics {
    EvalMetrics {
        task_success_rate: Some(task_success),
        policy_pass_rate: Some(policy_pass),
        latency_p50_ms: Some(latency_p50),
        ..Default::default()
    }
}

/// Complete an eval run and return its metrics.
fn run_with_metrics(
    svc: &EvalRunService,
    run_id: &str,
    project_id: &str,
    asset_id: &str,
    version_id: &str,
    release_id: &str,
    m: EvalMetrics,
) -> EvalMetrics {
    svc.create_run(
        EvalRunId::new(run_id),
        ProjectId::new(project_id),
        EvalSubjectKind::PromptRelease,
        "auto_scorer".to_owned(),
        Some(PromptAssetId::new(asset_id)),
        Some(PromptVersionId::new(version_id)),
        Some(PromptReleaseId::new(release_id)),
        None,
    );
    svc.start_run(&EvalRunId::new(run_id)).unwrap();
    let run = svc
        .complete_run(&EvalRunId::new(run_id), m.clone(), None)
        .unwrap();
    run.metrics
}

// ── 1. PromptComparisonMatrix: 2 prompts × 3 metrics ─────────────────────────

#[test]
fn prompt_comparison_matrix_two_prompts_three_metrics() {
    let svc = EvalRunService::new();
    let project_id = ProjectId::new("proj_matrix");
    let asset_id = PromptAssetId::new("asset_cmp");

    // Run for release A.
    let m_a = run_with_metrics(
        &svc,
        "run_a",
        "proj_matrix",
        "asset_cmp",
        "ver_a",
        "rel_a",
        metrics(0.88, 0.95, 150),
    );
    // Run for release B.
    let m_b = run_with_metrics(
        &svc,
        "run_b",
        "proj_matrix",
        "asset_cmp",
        "ver_b",
        "rel_b",
        metrics(0.92, 0.98, 120),
    );

    // Build PromptComparisonMatrix from completed runs.
    let matrix = PromptComparisonMatrix {
        rows: vec![
            PromptComparisonRow {
                project_id: project_id.clone(),
                prompt_release_id: PromptReleaseId::new("rel_a"),
                prompt_asset_id: asset_id.clone(),
                prompt_version_id: PromptVersionId::new("ver_a"),
                provider_binding_id: Some(ProviderBindingId::new("binding_openai")),
                eval_run_id: EvalRunId::new("run_a"),
                metrics: m_a.clone(),
            },
            PromptComparisonRow {
                project_id: project_id.clone(),
                prompt_release_id: PromptReleaseId::new("rel_b"),
                prompt_asset_id: asset_id.clone(),
                prompt_version_id: PromptVersionId::new("ver_b"),
                provider_binding_id: Some(ProviderBindingId::new("binding_openai")),
                eval_run_id: EvalRunId::new("run_b"),
                metrics: m_b.clone(),
            },
        ],
    };

    assert_eq!(matrix.rows.len(), 2, "2 rows for 2 prompt releases");

    // Verify 3 key metrics are present on each row.
    for row in &matrix.rows {
        assert!(
            row.metrics.task_success_rate.is_some(),
            "task_success_rate must be set"
        );
        assert!(
            row.metrics.policy_pass_rate.is_some(),
            "policy_pass_rate must be set"
        );
        assert!(
            row.metrics.latency_p50_ms.is_some(),
            "latency_p50_ms must be set"
        );
    }

    // Release B outperforms A on all three metrics.
    let row_a = &matrix.rows[0];
    let row_b = &matrix.rows[1];
    assert!(
        row_b.metrics.task_success_rate > row_a.metrics.task_success_rate,
        "B has higher task success rate"
    );
    assert!(
        row_b.metrics.latency_p50_ms < row_a.metrics.latency_p50_ms,
        "B has lower latency (better)"
    );
}

// ── 2. Matrix entries carry correct eval_run_id links ─────────────────────────

#[test]
fn prompt_comparison_matrix_eval_run_id_links_are_correct() {
    let project_id = ProjectId::new("proj_link");
    let asset_id = PromptAssetId::new("asset_link");

    let matrix = PromptComparisonMatrix {
        rows: vec![
            PromptComparisonRow {
                project_id: project_id.clone(),
                prompt_release_id: PromptReleaseId::new("rel_v1"),
                prompt_asset_id: asset_id.clone(),
                prompt_version_id: PromptVersionId::new("ver_v1"),
                provider_binding_id: None,
                eval_run_id: EvalRunId::new("eval_v1_run"),
                metrics: metrics(0.80, 0.90, 200),
            },
            PromptComparisonRow {
                project_id: project_id.clone(),
                prompt_release_id: PromptReleaseId::new("rel_v2"),
                prompt_asset_id: asset_id.clone(),
                prompt_version_id: PromptVersionId::new("ver_v2"),
                provider_binding_id: None,
                eval_run_id: EvalRunId::new("eval_v2_run"),
                metrics: metrics(0.91, 0.97, 110),
            },
        ],
    };

    // Each row carries a distinct eval_run_id pointing to its source run.
    assert_eq!(matrix.rows[0].eval_run_id.as_str(), "eval_v1_run");
    assert_eq!(matrix.rows[1].eval_run_id.as_str(), "eval_v2_run");
    assert_ne!(
        matrix.rows[0].eval_run_id, matrix.rows[1].eval_run_id,
        "each row must link to a different eval run"
    );

    // eval_run_id ↔ release linkage is consistent.
    assert_eq!(matrix.rows[0].prompt_release_id.as_str(), "rel_v1");
    assert_eq!(matrix.rows[1].prompt_release_id.as_str(), "rel_v2");

    // All rows belong to the same asset.
    assert!(matrix.rows.iter().all(|r| r.prompt_asset_id == asset_id));
}

// ── 3. GuardrailMatrix with policy pass/fail rates ────────────────────────────

#[test]
fn guardrail_matrix_tracks_policy_pass_fail_rates() {
    let project_id = ProjectId::new("proj_guard");

    let matrix = GuardrailMatrix {
        rows: vec![
            GuardrailPolicyRow {
                project_id: project_id.clone(),
                policy_id: PolicyId::new("policy_content_safety"),
                rule_name: "no_pii".to_owned(),
                eval_run_id: EvalRunId::new("run_g1"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(0.99), // 99% of calls passed the PII rule
                    task_success_rate: Some(0.95),
                    ..Default::default()
                },
            },
            GuardrailPolicyRow {
                project_id: project_id.clone(),
                policy_id: PolicyId::new("policy_content_safety"),
                rule_name: "no_harmful_content".to_owned(),
                eval_run_id: EvalRunId::new("run_g2"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(0.87), // 87% pass — below threshold
                    task_success_rate: Some(0.93),
                    ..Default::default()
                },
            },
            GuardrailPolicyRow {
                project_id: project_id.clone(),
                policy_id: PolicyId::new("policy_budget"),
                rule_name: "cost_cap_per_run".to_owned(),
                eval_run_id: EvalRunId::new("run_g3"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(1.0), // 100% of runs stayed under budget
                    cost_per_run: Some(0.002),
                    ..Default::default()
                },
            },
        ],
    };

    assert_eq!(matrix.rows.len(), 3);

    // All rows carry policy_pass_rate.
    assert!(matrix
        .rows
        .iter()
        .all(|r| r.metrics.policy_pass_rate.is_some()));

    // Identify rules below 90% pass rate.
    let failing: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.metrics.policy_pass_rate.unwrap_or(1.0) < 0.90)
        .collect();
    assert_eq!(failing.len(), 1);
    assert_eq!(failing[0].rule_name, "no_harmful_content");

    // Highest pass rate rule.
    let best = matrix
        .rows
        .iter()
        .max_by(|a, b| {
            a.metrics
                .policy_pass_rate
                .partial_cmp(&b.metrics.policy_pass_rate)
                .unwrap()
        })
        .unwrap();
    assert_eq!(best.rule_name, "cost_cap_per_run");
    assert_eq!(best.metrics.policy_pass_rate, Some(1.0));

    // eval_run_id links to distinct runs for each rule.
    let run_ids: Vec<_> = matrix.rows.iter().map(|r| r.eval_run_id.as_str()).collect();
    assert_eq!(run_ids, vec!["run_g1", "run_g2", "run_g3"]);
}

// ── 4. PermissionMatrix tracks tool-level allow/deny via policy_pass_rate ─────

#[test]
fn permission_matrix_tracks_tool_allow_deny_counts() {
    let project_id = ProjectId::new("proj_perm");

    // policy_pass_rate models allow/(allow+deny): 1.0 = always allowed, 0.0 = always denied.
    let matrix = PermissionMatrix {
        rows: vec![
            PermissionRow {
                project_id: project_id.clone(),
                policy_id: PolicyId::new("policy_tool_gate"),
                mode: "supervised".to_owned(),
                capability: "shell:exec".to_owned(),
                eval_run_id: EvalRunId::new("run_p1"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(0.60), // 60% allowed, 40% denied
                    task_success_rate: Some(0.90),
                    ..Default::default()
                },
            },
            PermissionRow {
                project_id: project_id.clone(),
                policy_id: PolicyId::new("policy_tool_gate"),
                mode: "supervised".to_owned(),
                capability: "file:read".to_owned(),
                eval_run_id: EvalRunId::new("run_p2"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(1.0), // always allowed
                    task_success_rate: Some(0.95),
                    ..Default::default()
                },
            },
            PermissionRow {
                project_id: project_id.clone(),
                policy_id: PolicyId::new("policy_tool_gate"),
                mode: "sandboxed".to_owned(),
                capability: "network:egress".to_owned(),
                eval_run_id: EvalRunId::new("run_p3"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(0.0), // always denied in sandboxed mode
                    task_success_rate: Some(0.0),
                    ..Default::default()
                },
            },
        ],
    };

    assert_eq!(matrix.rows.len(), 3);

    // Tools always allowed.
    let always_allowed: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.metrics.policy_pass_rate == Some(1.0))
        .collect();
    assert_eq!(always_allowed.len(), 1);
    assert_eq!(always_allowed[0].capability, "file:read");

    // Tools always denied.
    let always_denied: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.metrics.policy_pass_rate == Some(0.0))
        .collect();
    assert_eq!(always_denied.len(), 1);
    assert_eq!(always_denied[0].capability, "network:egress");
    assert_eq!(always_denied[0].mode, "sandboxed");

    // Partial allow (0 < rate < 1) — needs attention.
    let partial: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| {
            let rate = r.metrics.policy_pass_rate.unwrap_or(0.0);
            rate > 0.0 && rate < 1.0
        })
        .collect();
    assert_eq!(partial.len(), 1);
    assert_eq!(partial[0].capability, "shell:exec");

    // Each row has a distinct eval_run_id.
    let run_ids: std::collections::HashSet<_> =
        matrix.rows.iter().map(|r| r.eval_run_id.as_str()).collect();
    assert_eq!(
        run_ids.len(),
        3,
        "each tool permission row links to a distinct run"
    );
}

// ── 5. Matrices are project-scoped ────────────────────────────────────────────

#[test]
fn prompt_comparison_matrix_is_project_scoped() {
    let proj_a = ProjectId::new("proj_a");
    let proj_b = ProjectId::new("proj_b");

    let matrix = PromptComparisonMatrix {
        rows: vec![
            PromptComparisonRow {
                project_id: proj_a.clone(),
                prompt_release_id: PromptReleaseId::new("rel_a1"),
                prompt_asset_id: PromptAssetId::new("asset_a"),
                prompt_version_id: PromptVersionId::new("ver_a1"),
                provider_binding_id: None,
                eval_run_id: EvalRunId::new("run_a1"),
                metrics: metrics(0.88, 0.95, 150),
            },
            PromptComparisonRow {
                project_id: proj_a.clone(),
                prompt_release_id: PromptReleaseId::new("rel_a2"),
                prompt_asset_id: PromptAssetId::new("asset_a"),
                prompt_version_id: PromptVersionId::new("ver_a2"),
                provider_binding_id: None,
                eval_run_id: EvalRunId::new("run_a2"),
                metrics: metrics(0.92, 0.97, 130),
            },
            PromptComparisonRow {
                project_id: proj_b.clone(),
                prompt_release_id: PromptReleaseId::new("rel_b1"),
                prompt_asset_id: PromptAssetId::new("asset_b"),
                prompt_version_id: PromptVersionId::new("ver_b1"),
                provider_binding_id: None,
                eval_run_id: EvalRunId::new("run_b1"),
                metrics: metrics(0.75, 0.80, 300),
            },
        ],
    };

    // Filter by project A.
    let proj_a_rows: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.project_id == proj_a)
        .collect();
    assert_eq!(proj_a_rows.len(), 2, "project A has 2 rows");
    assert!(proj_a_rows.iter().all(|r| r.project_id == proj_a));

    // Filter by project B.
    let proj_b_rows: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.project_id == proj_b)
        .collect();
    assert_eq!(proj_b_rows.len(), 1, "project B has 1 row");
    assert_eq!(proj_b_rows[0].prompt_asset_id.as_str(), "asset_b");

    // Project A rows do not contain project B's eval_run_id.
    let proj_a_run_ids: Vec<_> = proj_a_rows.iter().map(|r| r.eval_run_id.as_str()).collect();
    assert!(
        !proj_a_run_ids.contains(&"run_b1"),
        "project B run must not appear in project A's rows"
    );
}

#[test]
fn guardrail_matrix_filters_by_project_id() {
    let proj_x = ProjectId::new("proj_x");
    let proj_y = ProjectId::new("proj_y");

    let matrix = GuardrailMatrix {
        rows: vec![
            GuardrailPolicyRow {
                project_id: proj_x.clone(),
                policy_id: PolicyId::new("pol_1"),
                rule_name: "rule_a".to_owned(),
                eval_run_id: EvalRunId::new("rx1"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(0.90),
                    ..Default::default()
                },
            },
            GuardrailPolicyRow {
                project_id: proj_y.clone(),
                policy_id: PolicyId::new("pol_1"),
                rule_name: "rule_a".to_owned(),
                eval_run_id: EvalRunId::new("ry1"),
                metrics: EvalMetrics {
                    policy_pass_rate: Some(0.70),
                    ..Default::default()
                },
            },
        ],
    };

    let x_rows: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.project_id == proj_x)
        .collect();
    assert_eq!(x_rows.len(), 1);
    assert_eq!(x_rows[0].metrics.policy_pass_rate, Some(0.90));

    let y_rows: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.project_id == proj_y)
        .collect();
    assert_eq!(y_rows.len(), 1);
    assert_eq!(y_rows[0].metrics.policy_pass_rate, Some(0.70));
}

// ── 6. ProviderRoutingMatrix: route decisions with latency + cost metrics ──────

#[test]
fn provider_routing_matrix_carries_latency_and_cost() {
    let project_id = ProjectId::new("proj_route");

    let matrix = ProviderRoutingMatrix {
        rows: vec![
            ProviderRoutingRow {
                project_id: project_id.clone(),
                route_decision_id: RouteDecisionId::new("rd_1"),
                provider_binding_id: Some(ProviderBindingId::new("binding_openai")),
                eval_run_id: EvalRunId::new("run_r1"),
                metrics: EvalMetrics {
                    latency_p50_ms: Some(120),
                    cost_per_run: Some(0.004),
                    ..Default::default()
                },
                total_cost_micros: 0,
                success_rate: 0.0,
            },
            ProviderRoutingRow {
                project_id: project_id.clone(),
                route_decision_id: RouteDecisionId::new("rd_2"),
                provider_binding_id: Some(ProviderBindingId::new("binding_anthropic")),
                eval_run_id: EvalRunId::new("run_r2"),
                metrics: EvalMetrics {
                    latency_p50_ms: Some(95),
                    cost_per_run: Some(0.006),
                    ..Default::default()
                },
                total_cost_micros: 0,
                success_rate: 0.0,
            },
        ],
    };

    assert_eq!(matrix.rows.len(), 2);

    // Fastest provider.
    let fastest = matrix
        .rows
        .iter()
        .min_by_key(|r| r.metrics.latency_p50_ms.unwrap_or(u64::MAX))
        .unwrap();
    assert_eq!(
        fastest.provider_binding_id.as_ref().unwrap().as_str(),
        "binding_anthropic"
    );

    // Cheapest provider.
    let cheapest = matrix
        .rows
        .iter()
        .min_by(|a, b| {
            a.metrics
                .cost_per_run
                .partial_cmp(&b.metrics.cost_per_run)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap();
    assert_eq!(
        cheapest.provider_binding_id.as_ref().unwrap().as_str(),
        "binding_openai"
    );

    // All rows are project-scoped.
    assert!(matrix.rows.iter().all(|r| r.project_id == project_id));
}

// ── 7. SkillHealthMatrix tracks skill-level metrics ───────────────────────────

#[test]
fn skill_health_matrix_tracks_per_skill_metrics() {
    let project_id = ProjectId::new("proj_skill");

    let matrix = SkillHealthMatrix {
        rows: vec![
            SkillHealthRow {
                project_id: project_id.clone(),
                skill_id: "content-pipeline".to_owned(),
                eval_run_id: EvalRunId::new("run_s1"),
                metrics: EvalMetrics {
                    task_success_rate: Some(0.94),
                    latency_p50_ms: Some(800),
                    ..Default::default()
                },
            },
            SkillHealthRow {
                project_id: project_id.clone(),
                skill_id: "decision-support".to_owned(),
                eval_run_id: EvalRunId::new("run_s2"),
                metrics: EvalMetrics {
                    task_success_rate: Some(0.78),
                    latency_p50_ms: Some(1_200),
                    ..Default::default()
                },
            },
        ],
    };

    assert_eq!(matrix.rows.len(), 2);

    // Identify degraded skills (success < 80%).
    let degraded: Vec<_> = matrix
        .rows
        .iter()
        .filter(|r| r.metrics.task_success_rate.unwrap_or(0.0) < 0.80)
        .collect();
    assert_eq!(degraded.len(), 1);
    assert_eq!(degraded[0].skill_id, "decision-support");

    // All rows project-scoped.
    assert!(matrix.rows.iter().all(|r| r.project_id == project_id));
}

// ── 8. MemorySourceQualityMatrix retrieval metrics ────────────────────────────

#[test]
fn memory_source_quality_matrix_tracks_retrieval_metrics() {
    let project_id = ProjectId::new("proj_mem");

    let matrix = MemorySourceQualityMatrix {
        rows: vec![
            MemorySourceQualityRow {
                project_id: project_id.clone(),
                source_id: SourceId::new("src_docs"),
                eval_run_id: EvalRunId::new("run_m1"),
                metrics: EvalMetrics {
                    retrieval_hit_at_k: Some(0.88),
                    citation_coverage: Some(0.72),
                    source_diversity: Some(0.65),
                    retrieval_latency_ms: Some(45),
                    ..Default::default()
                },
            },
            MemorySourceQualityRow {
                project_id: project_id.clone(),
                source_id: SourceId::new("src_wiki"),
                eval_run_id: EvalRunId::new("run_m2"),
                metrics: EvalMetrics {
                    retrieval_hit_at_k: Some(0.95),
                    citation_coverage: Some(0.91),
                    source_diversity: Some(0.80),
                    retrieval_latency_ms: Some(30),
                    ..Default::default()
                },
            },
        ],
    };

    assert_eq!(matrix.rows.len(), 2);

    // Best source by retrieval quality.
    let best = matrix
        .rows
        .iter()
        .max_by(|a, b| {
            a.metrics
                .retrieval_hit_at_k
                .partial_cmp(&b.metrics.retrieval_hit_at_k)
                .unwrap()
        })
        .unwrap();
    assert_eq!(best.source_id.as_str(), "src_wiki");
    assert_eq!(best.metrics.retrieval_hit_at_k, Some(0.95));

    // Lower latency source.
    let fastest = matrix
        .rows
        .iter()
        .min_by_key(|r| r.metrics.retrieval_latency_ms.unwrap_or(u64::MAX))
        .unwrap();
    assert_eq!(fastest.source_id.as_str(), "src_wiki");
}

// ── 9. MatrixCategory variants are distinct ───────────────────────────────────

#[test]
fn matrix_category_variants_are_distinct() {
    use cairn_evals::MatrixCategory;

    assert_ne!(
        MatrixCategory::PromptComparison,
        MatrixCategory::ProviderRouting
    );
    assert_ne!(MatrixCategory::ProviderRouting, MatrixCategory::Permission);
    assert_ne!(
        MatrixCategory::Permission,
        MatrixCategory::MemorySourceQuality
    );
    assert_ne!(
        MatrixCategory::MemorySourceQuality,
        MatrixCategory::SkillHealth
    );
    assert_ne!(
        MatrixCategory::SkillHealth,
        MatrixCategory::GuardrailPolicyOutcome
    );
}

// ── 10. EvalRunService build_scorecard links to matrix rows ───────────────────

#[test]
fn scorecard_entries_align_with_matrix_rows() {
    let svc = EvalRunService::new();
    let project_id = ProjectId::new("proj_sc_matrix");
    let asset_id = PromptAssetId::new("asset_sc");

    // Create two completed runs for different releases of the same asset.
    for (run_id, ver_id, rel_id, success) in [
        ("run_sc1", "ver_1", "rel_1", 0.85f64),
        ("run_sc2", "ver_2", "rel_2", 0.93f64),
    ] {
        svc.create_run(
            EvalRunId::new(run_id),
            project_id.clone(),
            EvalSubjectKind::PromptRelease,
            "auto".to_owned(),
            Some(asset_id.clone()),
            Some(PromptVersionId::new(ver_id)),
            Some(PromptReleaseId::new(rel_id)),
            None,
        );
        svc.start_run(&EvalRunId::new(run_id)).unwrap();
        svc.complete_run(
            &EvalRunId::new(run_id),
            EvalMetrics {
                task_success_rate: Some(success),
                ..Default::default()
            },
            None,
        )
        .unwrap();
    }

    // Build scorecard — should have 2 entries matching our runs.
    let scorecard = svc.build_scorecard(&project_id, &asset_id);
    assert_eq!(scorecard.entries.len(), 2);

    // Build a PromptComparisonMatrix from those scorecard entries.
    let matrix = PromptComparisonMatrix {
        rows: scorecard
            .entries
            .iter()
            .map(|e| PromptComparisonRow {
                project_id: project_id.clone(),
                prompt_release_id: e.prompt_release_id.clone(),
                prompt_asset_id: asset_id.clone(),
                prompt_version_id: e.prompt_version_id.clone(),
                provider_binding_id: None,
                eval_run_id: e.eval_run_id.clone(),
                metrics: EvalMetrics {
                    task_success_rate: e.metrics.task_success_rate,
                    ..Default::default()
                },
            })
            .collect(),
    };

    assert_eq!(matrix.rows.len(), 2);

    // Each matrix row's eval_run_id matches the scorecard entry.
    let sc_run_ids: Vec<_> = scorecard
        .entries
        .iter()
        .map(|e| e.eval_run_id.as_str())
        .collect();
    let mx_run_ids: Vec<_> = matrix.rows.iter().map(|r| r.eval_run_id.as_str()).collect();
    // Both contain the same eval_run_ids (order may differ).
    for id in &sc_run_ids {
        assert!(mx_run_ids.contains(id), "{id} must appear in matrix rows");
    }

    // Best release by task_success_rate.
    let best = matrix
        .rows
        .iter()
        .max_by(|a, b| {
            a.metrics
                .task_success_rate
                .partial_cmp(&b.metrics.task_success_rate)
                .unwrap()
        })
        .unwrap();
    assert_eq!(
        best.prompt_release_id.as_str(),
        "rel_2",
        "rel_2 (0.93) must be ranked above rel_1 (0.85)"
    );
}
