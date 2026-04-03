//! Week 4: End-to-end prompt release + eval + scorecard integration test.
//!
//! Exercises the full prompt-as-product lifecycle:
//! 1. Create prompt asset and versions
//! 2. Create and activate releases with selectors
//! 3. Resolve prompt at runtime
//! 4. Run evals against releases
//! 5. Build scorecard and compare
//! 6. Rollback based on eval results

use cairn_domain::*;
use cairn_evals::{
    EvalMetrics, EvalRunService, EvalSubjectKind, PromptReleaseService, PromptReleaseState,
    ResolutionContext, RolloutTarget, SelectorResolver,
};

fn project() -> ProjectId {
    ProjectId::new("proj_alpha")
}

fn asset() -> PromptAssetId {
    PromptAssetId::new("prompt_planner_system")
}

#[test]
fn full_prompt_release_eval_scorecard_slice() {
    let release_svc = PromptReleaseService::new();
    let eval_svc = EvalRunService::new();

    let project_id = project();
    let prompt_asset_id = asset();

    // -- Step 1: Create two prompt releases (v1 and v2) --

    release_svc.create(
        PromptReleaseId::new("rel_v1"),
        project_id.clone(),
        prompt_asset_id.clone(),
        PromptVersionId::new("pv_v1"),
        RolloutTarget::project_default(),
    );
    release_svc
        .transition(
            &PromptReleaseId::new("rel_v1"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
    release_svc
        .transition(
            &PromptReleaseId::new("rel_v1"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

    release_svc.create(
        PromptReleaseId::new("rel_v2"),
        project_id.clone(),
        prompt_asset_id.clone(),
        PromptVersionId::new("pv_v2"),
        RolloutTarget::project_default(),
    );
    release_svc
        .transition(
            &PromptReleaseId::new("rel_v2"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();

    // v1 is active, v2 is approved but not yet activated

    // -- Step 2: Resolve prompt → should get v1 (active) --

    let v1_release = release_svc.get(&PromptReleaseId::new("rel_v1")).unwrap();
    let v2_release = release_svc.get(&PromptReleaseId::new("rel_v2")).unwrap();
    let releases = vec![v1_release, v2_release.clone()];

    let ctx = ResolutionContext::default();
    let resolved = SelectorResolver::resolve(&releases, &project_id, &prompt_asset_id, &ctx);
    assert_eq!(
        resolved.unwrap().prompt_release_id,
        PromptReleaseId::new("rel_v1")
    );

    // -- Step 3: Run eval on v1 --

    eval_svc.create_run(
        EvalRunId::new("eval_v1"),
        project_id.clone(),
        EvalSubjectKind::PromptRelease,
        "auto_scorer".to_owned(),
        Some(prompt_asset_id.clone()),
        Some(PromptVersionId::new("pv_v1")),
        Some(PromptReleaseId::new("rel_v1")),
        None,
    );
    eval_svc.start_run(&EvalRunId::new("eval_v1")).unwrap();
    eval_svc
        .complete_run(
            &EvalRunId::new("eval_v1"),
            EvalMetrics {
                task_success_rate: Some(0.78),
                latency_p50_ms: Some(220),
                cost_per_run: Some(0.005),
                ..Default::default()
            },
            Some(0.25),
        )
        .unwrap();

    // -- Step 4: Run eval on v2 --

    eval_svc.create_run(
        EvalRunId::new("eval_v2"),
        project_id.clone(),
        EvalSubjectKind::PromptRelease,
        "auto_scorer".to_owned(),
        Some(prompt_asset_id.clone()),
        Some(PromptVersionId::new("pv_v2")),
        Some(PromptReleaseId::new("rel_v2")),
        None,
    );
    eval_svc.start_run(&EvalRunId::new("eval_v2")).unwrap();
    eval_svc
        .complete_run(
            &EvalRunId::new("eval_v2"),
            EvalMetrics {
                task_success_rate: Some(0.93),
                latency_p50_ms: Some(180),
                cost_per_run: Some(0.004),
                ..Default::default()
            },
            Some(0.20),
        )
        .unwrap();

    // -- Step 5: Build scorecard and compare --

    let scorecard = eval_svc.build_scorecard(&project_id, &prompt_asset_id);
    assert_eq!(scorecard.entries.len(), 2);

    let best = scorecard
        .entries
        .iter()
        .max_by(|a, b| {
            a.metrics
                .task_success_rate
                .partial_cmp(&b.metrics.task_success_rate)
                .unwrap()
        })
        .unwrap();

    assert_eq!(best.prompt_release_id, PromptReleaseId::new("rel_v2"));
    assert_eq!(best.metrics.task_success_rate, Some(0.93));

    // -- Step 6: Promote v2 based on eval results --

    release_svc
        .transition(
            &PromptReleaseId::new("rel_v2"),
            PromptReleaseState::Active,
            None,
            Some("eval shows v2 outperforms v1 (93% vs 78%)".to_owned()),
        )
        .unwrap();

    // v1 should now be deactivated, v2 active
    let v1 = release_svc.get(&PromptReleaseId::new("rel_v1")).unwrap();
    let v2 = release_svc.get(&PromptReleaseId::new("rel_v2")).unwrap();
    assert_eq!(v1.state, PromptReleaseState::Approved);
    assert_eq!(v2.state, PromptReleaseState::Active);

    // Resolution now returns v2
    let releases = vec![v1.clone(), v2];
    let resolved = SelectorResolver::resolve(&releases, &project_id, &prompt_asset_id, &ctx);
    assert_eq!(
        resolved.unwrap().prompt_release_id,
        PromptReleaseId::new("rel_v2")
    );

    // -- Step 7: Rollback to v1 if regression discovered --

    let rolled_back = release_svc
        .rollback(
            &PromptReleaseId::new("rel_v2"),
            &PromptReleaseId::new("rel_v1"),
            None,
            Some("production regression in v2".to_owned()),
        )
        .unwrap();
    assert_eq!(rolled_back.state, PromptReleaseState::Active);
    assert_eq!(
        rolled_back.prompt_release_id,
        PromptReleaseId::new("rel_v1")
    );

    // Verify audit trail has the rollback
    let actions = release_svc.actions();
    let rollback_actions: Vec<_> = actions
        .iter()
        .filter(|a| a.action_type == cairn_evals::ReleaseActionType::Rollback)
        .collect();
    assert_eq!(rollback_actions.len(), 1);
    assert_eq!(
        rollback_actions[0]
            .from_release_id
            .as_ref()
            .unwrap()
            .as_str(),
        "rel_v2"
    );
}

/// Selector precedence: agent_type release overrides project_default.
#[test]
fn selector_precedence_in_multi_release_scenario() {
    let release_svc = PromptReleaseService::new();
    let project_id = project();
    let prompt_asset_id = asset();

    // Default release
    release_svc.create(
        PromptReleaseId::new("rel_default"),
        project_id.clone(),
        prompt_asset_id.clone(),
        PromptVersionId::new("pv_default"),
        RolloutTarget::project_default(),
    );
    release_svc
        .transition(
            &PromptReleaseId::new("rel_default"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
    release_svc
        .transition(
            &PromptReleaseId::new("rel_default"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

    // Agent-specific release
    release_svc.create(
        PromptReleaseId::new("rel_planner"),
        project_id.clone(),
        prompt_asset_id.clone(),
        PromptVersionId::new("pv_planner"),
        RolloutTarget::agent_type("planner"),
    );
    release_svc
        .transition(
            &PromptReleaseId::new("rel_planner"),
            PromptReleaseState::Approved,
            None,
            None,
        )
        .unwrap();
    release_svc
        .transition(
            &PromptReleaseId::new("rel_planner"),
            PromptReleaseState::Active,
            None,
            None,
        )
        .unwrap();

    let default_rel = release_svc
        .get(&PromptReleaseId::new("rel_default"))
        .unwrap();
    let planner_rel = release_svc
        .get(&PromptReleaseId::new("rel_planner"))
        .unwrap();
    let releases = vec![default_rel, planner_rel];

    // Planner context → should get agent-specific release
    let ctx = ResolutionContext {
        agent_type: Some("planner".to_owned()),
        ..Default::default()
    };
    let resolved = SelectorResolver::resolve(&releases, &project_id, &prompt_asset_id, &ctx);
    assert_eq!(
        resolved.unwrap().prompt_release_id,
        PromptReleaseId::new("rel_planner")
    );

    // Coder context → no agent match, falls back to default
    let ctx = ResolutionContext {
        agent_type: Some("coder".to_owned()),
        ..Default::default()
    };
    let resolved = SelectorResolver::resolve(&releases, &project_id, &prompt_asset_id, &ctx);
    assert_eq!(
        resolved.unwrap().prompt_release_id,
        PromptReleaseId::new("rel_default")
    );
}
