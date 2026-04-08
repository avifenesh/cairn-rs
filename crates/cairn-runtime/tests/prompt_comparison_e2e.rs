//! RFC 004 prompt comparison matrix end-to-end integration test.
//!
//! Validates the full prompt comparison pipeline:
//!   (1) create a prompt asset with two versions
//!   (2) create a release for each version
//!   (3) run evals on each release with different metrics
//!   (4) build a prompt comparison scorecard — both releases must appear
//!   (5) verify the winner has the higher task_success_rate
//!   (6) verify scorecard entries reference correct releases/versions
//!   (7) incomplete runs are excluded from scorecard

use std::sync::Arc;

use cairn_domain::{
    EvalRunId, ProjectId, ProjectKey, PromptAssetId, PromptReleaseId, PromptVersionId,
};
use cairn_evals::{EvalMetrics, EvalRunService, EvalSubjectKind};
use cairn_runtime::{
    PromptAssetService, PromptAssetServiceImpl, PromptReleaseService, PromptReleaseServiceImpl,
    PromptVersionService, PromptVersionServiceImpl,
};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("t_cmp", "ws_cmp", "proj_cmp")
}

fn project_id() -> ProjectId {
    ProjectId::new("proj_cmp")
}

struct Setup {
    assets: PromptAssetServiceImpl<InMemoryStore>,
    versions: PromptVersionServiceImpl<InMemoryStore>,
    releases: PromptReleaseServiceImpl<InMemoryStore>,
    evals: EvalRunService,
}

fn setup() -> Setup {
    let store = Arc::new(InMemoryStore::new());
    Setup {
        assets: PromptAssetServiceImpl::new(store.clone()),
        versions: PromptVersionServiceImpl::new(store.clone()),
        releases: PromptReleaseServiceImpl::new(store),
        evals: EvalRunService::new(),
    }
}

/// Seed an asset (idempotent), a version, and an active release.
async fn seed_release(
    svc: &Setup,
    asset_id: &PromptAssetId,
    version_id: &PromptVersionId,
    release_id: &PromptReleaseId,
    content_hash: &str,
) {
    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "Comparison Asset".to_owned(),
            "system".to_owned(),
        )
        .await
        .ok(); // idempotent — asset may already exist

    svc.versions
        .create(
            &project(),
            version_id.clone(),
            asset_id.clone(),
            content_hash.to_owned(),
        )
        .await
        .unwrap();

    svc.releases
        .create(
            &project(),
            release_id.clone(),
            asset_id.clone(),
            version_id.clone(),
        )
        .await
        .unwrap();

    svc.releases
        .transition(release_id, "approved")
        .await
        .unwrap();
    svc.releases.activate(release_id).await.unwrap();
}

/// Run a full eval lifecycle: create → start → complete with given metrics.
fn run_scored_eval(
    svc: &Setup,
    run_id: &EvalRunId,
    asset_id: &PromptAssetId,
    version_id: &PromptVersionId,
    release_id: &PromptReleaseId,
    task_success_rate: f64,
    latency_p50_ms: u64,
) {
    svc.evals.create_run(
        run_id.clone(),
        project_id(),
        EvalSubjectKind::PromptRelease,
        "regression".to_owned(),
        Some(asset_id.clone()),
        Some(version_id.clone()),
        Some(release_id.clone()),
        None,
    );
    svc.evals.start_run(run_id).unwrap();
    svc.evals
        .complete_run(
            run_id,
            EvalMetrics {
                task_success_rate: Some(task_success_rate),
                latency_p50_ms: Some(latency_p50_ms),
                cost_per_run: Some(0.01),
                ..EvalMetrics::default()
            },
            Some(0.01),
        )
        .unwrap();
}

// ── (1)+(2) Asset with two versions/releases ──────────────────────────────

#[tokio::test]
async fn two_releases_for_same_asset_second_supersedes_first() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_cmp_1");
    let v1 = PromptVersionId::new("ver_cmp_1a");
    let v2 = PromptVersionId::new("ver_cmp_1b");
    let rel1 = PromptReleaseId::new("rel_cmp_1a");
    let rel2 = PromptReleaseId::new("rel_cmp_1b");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "Dual Release Asset".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();

    svc.versions
        .create(
            &project(),
            v1.clone(),
            asset_id.clone(),
            "hash_v1".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(&project(), rel1.clone(), asset_id.clone(), v1)
        .await
        .unwrap();
    svc.releases.transition(&rel1, "approved").await.unwrap();
    let r1 = svc.releases.activate(&rel1).await.unwrap();
    assert_eq!(r1.state, "active");

    svc.versions
        .create(
            &project(),
            v2.clone(),
            asset_id.clone(),
            "hash_v2".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(&project(), rel2.clone(), asset_id.clone(), v2)
        .await
        .unwrap();
    svc.releases.transition(&rel2, "approved").await.unwrap();
    let r2 = svc.releases.activate(&rel2).await.unwrap();
    assert_eq!(r2.state, "active");

    let r1_after = svc.releases.get(&rel1).await.unwrap().unwrap();
    assert_ne!(r1_after.state, "active", "first release must be superseded");
}

// ── (3)+(4) Scorecard contains entry for each evaluated release ───────────

#[tokio::test]
async fn scorecard_contains_entry_for_each_evaluated_release() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_sc_2");
    let v1 = PromptVersionId::new("ver_sc_2a");
    let v2 = PromptVersionId::new("ver_sc_2b");
    let rel1 = PromptReleaseId::new("rel_sc_2a");
    let rel2 = PromptReleaseId::new("rel_sc_2b");

    seed_release(&svc, &asset_id, &v1, &rel1, "hash_sc_v1").await;

    svc.versions
        .create(
            &project(),
            v2.clone(),
            asset_id.clone(),
            "hash_sc_v2".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(&project(), rel2.clone(), asset_id.clone(), v2.clone())
        .await
        .unwrap();
    svc.releases.transition(&rel2, "approved").await.unwrap();
    svc.releases.activate(&rel2).await.unwrap();

    run_scored_eval(
        &svc,
        &EvalRunId::new("eval_sc_2a"),
        &asset_id,
        &v1,
        &rel1,
        0.78,
        250,
    );
    run_scored_eval(
        &svc,
        &EvalRunId::new("eval_sc_2b"),
        &asset_id,
        &v2,
        &rel2,
        0.91,
        180,
    );

    let scorecard = svc.evals.build_scorecard(&project_id(), &asset_id);

    assert_eq!(scorecard.prompt_asset_id, asset_id);
    assert_eq!(
        scorecard.entries.len(),
        2,
        "scorecard must have one entry per evaluated release"
    );

    let ids: Vec<_> = scorecard
        .entries
        .iter()
        .map(|e| &e.prompt_release_id)
        .collect();
    assert!(ids.contains(&&rel1), "rel1 must be in scorecard");
    assert!(ids.contains(&&rel2), "rel2 must be in scorecard");
}

// ── (5) Winner has higher task_success_rate ───────────────────────────────

#[tokio::test]
async fn winner_is_release_with_higher_task_success_rate() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_win_3");
    let v1 = PromptVersionId::new("ver_win_3a");
    let v2 = PromptVersionId::new("ver_win_3b");
    let rel1 = PromptReleaseId::new("rel_win_3a"); // 0.65 — loser
    let rel2 = PromptReleaseId::new("rel_win_3b"); // 0.88 — winner

    seed_release(&svc, &asset_id, &v1, &rel1, "hash_win_v1").await;

    svc.versions
        .create(
            &project(),
            v2.clone(),
            asset_id.clone(),
            "hash_win_v2".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(&project(), rel2.clone(), asset_id.clone(), v2.clone())
        .await
        .unwrap();
    svc.releases.transition(&rel2, "approved").await.unwrap();
    svc.releases.activate(&rel2).await.unwrap();

    run_scored_eval(
        &svc,
        &EvalRunId::new("eval_win_3a"),
        &asset_id,
        &v1,
        &rel1,
        0.65,
        300,
    );
    run_scored_eval(
        &svc,
        &EvalRunId::new("eval_win_3b"),
        &asset_id,
        &v2,
        &rel2,
        0.88,
        200,
    );

    let scorecard = svc.evals.build_scorecard(&project_id(), &asset_id);
    assert_eq!(scorecard.entries.len(), 2);

    let winner = scorecard
        .entries
        .iter()
        .max_by(|a, b| {
            a.metrics
                .task_success_rate
                .partial_cmp(&b.metrics.task_success_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .expect("scorecard must have at least one entry");

    assert_eq!(
        winner.prompt_release_id, rel2,
        "rel2 (0.88) must beat rel1 (0.65)"
    );
    assert_eq!(winner.metrics.task_success_rate, Some(0.88));
}

// ── (6) Scorecard entries reference correct release + version ─────────────

#[tokio::test]
async fn scorecard_entry_references_correct_release_and_version() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_ref_4");
    let v1 = PromptVersionId::new("ver_ref_4");
    let rel1 = PromptReleaseId::new("rel_ref_4");
    let run_id = EvalRunId::new("run_ref_4");

    seed_release(&svc, &asset_id, &v1, &rel1, "hash_ref_v1").await;
    run_scored_eval(&svc, &run_id, &asset_id, &v1, &rel1, 0.80, 220);

    let scorecard = svc.evals.build_scorecard(&project_id(), &asset_id);
    assert_eq!(scorecard.entries.len(), 1);

    let entry = &scorecard.entries[0];
    assert_eq!(entry.prompt_release_id, rel1);
    assert_eq!(entry.prompt_version_id, v1);
    assert_eq!(entry.eval_run_id, run_id);
    assert_eq!(entry.metrics.task_success_rate, Some(0.80));
    assert_eq!(entry.metrics.latency_p50_ms, Some(220));
}

// ── (7) Incomplete runs excluded from scorecard ───────────────────────────

#[tokio::test]
async fn incomplete_eval_runs_excluded_from_scorecard() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_inc_5");
    let v1 = PromptVersionId::new("ver_inc_5");
    let rel1 = PromptReleaseId::new("rel_inc_5");
    let done_id = EvalRunId::new("eval_inc_done");
    let pending_id = EvalRunId::new("eval_inc_pending");

    seed_release(&svc, &asset_id, &v1, &rel1, "hash_inc_v1").await;

    // Completed run — must appear.
    run_scored_eval(&svc, &done_id, &asset_id, &v1, &rel1, 0.75, 200);

    // Pending run — left in Pending state, must NOT appear.
    svc.evals.create_run(
        pending_id,
        project_id(),
        EvalSubjectKind::PromptRelease,
        "regression".to_owned(),
        Some(asset_id.clone()),
        Some(v1.clone()),
        Some(rel1.clone()),
        None,
    );

    let scorecard = svc.evals.build_scorecard(&project_id(), &asset_id);
    assert_eq!(
        scorecard.entries.len(),
        1,
        "only completed runs appear in scorecard"
    );
    assert_eq!(scorecard.entries[0].eval_run_id, done_id);
}

// ── Full comparison happy path ────────────────────────────────────────────

#[tokio::test]
async fn full_comparison_two_releases_winner_determined() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_full_cmp");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "Full Comparison Asset".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();

    // Version A: 0.70 success, 400ms latency.
    let va = PromptVersionId::new("ver_full_a");
    let rela = PromptReleaseId::new("rel_full_a");
    svc.versions
        .create(
            &project(),
            va.clone(),
            asset_id.clone(),
            "sha256:v1".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(&project(), rela.clone(), asset_id.clone(), va.clone())
        .await
        .unwrap();
    svc.releases.transition(&rela, "approved").await.unwrap();
    svc.releases.activate(&rela).await.unwrap();
    run_scored_eval(
        &svc,
        &EvalRunId::new("full_a"),
        &asset_id,
        &va,
        &rela,
        0.70,
        400,
    );

    // Version B: 0.95 success, 180ms latency — clear winner.
    let vb = PromptVersionId::new("ver_full_b");
    let relb = PromptReleaseId::new("rel_full_b");
    svc.versions
        .create(
            &project(),
            vb.clone(),
            asset_id.clone(),
            "sha256:v2".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(&project(), relb.clone(), asset_id.clone(), vb.clone())
        .await
        .unwrap();
    svc.releases.transition(&relb, "approved").await.unwrap();
    svc.releases.activate(&relb).await.unwrap();
    run_scored_eval(
        &svc,
        &EvalRunId::new("full_b"),
        &asset_id,
        &vb,
        &relb,
        0.95,
        180,
    );

    let scorecard = svc.evals.build_scorecard(&project_id(), &asset_id);
    assert_eq!(scorecard.entries.len(), 2, "both releases must appear");

    let winner = scorecard
        .entries
        .iter()
        .max_by(|a, b| {
            a.metrics
                .task_success_rate
                .partial_cmp(&b.metrics.task_success_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap();

    assert_eq!(
        winner.prompt_release_id, relb,
        "rel_full_b (0.95) is the winner"
    );
    assert_eq!(winner.metrics.task_success_rate, Some(0.95));
    assert!(
        winner.metrics.latency_p50_ms.unwrap() < 200,
        "winner has lower latency"
    );
}
