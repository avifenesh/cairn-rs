//! RFC 005 — checkpoint strategy system end-to-end integration tests.
//!
//! Tests the checkpoint strategy lifecycle:
//!   1. Set a checkpoint strategy for a run (interval + max_checkpoints)
//!   2. Verify the strategy is retrievable via CheckpointStrategyReadModel
//!   3. Save multiple checkpoints for the run
//!   4. Verify the oldest checkpoints are Superseded (max-retention enforcement)
//!   5. Verify the latest checkpoint (most recent save) is marked Latest

use std::sync::Arc;

use cairn_domain::lifecycle::CheckpointDisposition;
use cairn_domain::{
    CheckpointStrategySet, EventEnvelope, EventId, EventSource, ProjectKey, RunId,
    RuntimeEvent,
};
use cairn_domain::ids::CheckpointId;
use cairn_runtime::checkpoints::CheckpointService;
use cairn_runtime::services::CheckpointServiceImpl;
use cairn_store::projections::CheckpointStrategyReadModel;
use cairn_store::{EventLog, InMemoryStore};

fn project() -> ProjectKey {
    ProjectKey::new("t_strat", "w_strat", "p_strat")
}

fn run() -> RunId {
    RunId::new("run_strat_1")
}

fn cp(id: &str) -> CheckpointId {
    CheckpointId::new(id)
}

// ── Tests 1–2: set strategy, verify retrievable ──────────────────────────────

/// RFC 005 §3: a checkpoint strategy sets the interval and max_checkpoints
/// for a run.  The strategy must be retrievable by run_id after it is set.
///
/// Note: the CheckpointService trait only supports checkpoint CRUD; strategy
/// configuration is recorded via a CheckpointStrategySet domain event appended
/// directly to the store (the same pattern used by the runtime orchestrator).
#[tokio::test]
async fn set_strategy_and_verify_retrievable() {
    let store = Arc::new(InMemoryStore::new());

    // ── (1) Set a checkpoint strategy for the run ──────────────────────────
    // CheckpointStrategySet stores: strategy_id, description, set_at_ms, run_id.
    // The in-memory projection fills interval_ms=0, max_checkpoints=10 as defaults.
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_strat_set"),
            EventSource::Runtime,
            RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
                strategy_id: "strat_interval_30s".to_owned(),
                description: "Checkpoint every 30s, retain up to 5".to_owned(),
                set_at_ms: 1_700_000_000_000,
                run_id: Some(run()),
            }),
        )])
        .await
        .unwrap();

    // ── (2) Verify strategy is retrievable ────────────────────────────────
    let strategy = CheckpointStrategyReadModel::get_by_run(store.as_ref(), &run())
        .await
        .unwrap()
        .expect("strategy must be retrievable after CheckpointStrategySet event");

    assert_eq!(strategy.run_id, run(), "strategy must be scoped to the correct run");
    assert_eq!(
        strategy.strategy_id, "strat_interval_30s",
        "strategy_id must round-trip"
    );
    // max_checkpoints default from projection.
    assert!(
        strategy.max_checkpoints > 0,
        "max_checkpoints must be a positive default"
    );

    // No strategy for an unrelated run.
    let other = CheckpointStrategyReadModel::get_by_run(
        store.as_ref(),
        &RunId::new("run_no_strategy"),
    )
    .await
    .unwrap();
    assert!(
        other.is_none(),
        "no strategy must exist for a run that was never configured"
    );
}

// ── Tests 3–5: save multiple checkpoints, verify disposition chain ────────────

/// RFC 005 §3: saving a new checkpoint supersedes the previous Latest.
/// After N saves, exactly 1 checkpoint is Latest and N-1 are Superseded.
/// latest_for_run() must return the most recently saved checkpoint.
#[tokio::test]
async fn multiple_checkpoints_oldest_superseded_latest_is_most_recent() {
    let store = Arc::new(InMemoryStore::new());
    let svc = CheckpointServiceImpl::new(store.clone());

    // ── (3) Save five checkpoints in sequence ────────────────────────────
    let checkpoint_ids = [
        cp("cp_s1"), cp("cp_s2"), cp("cp_s3"), cp("cp_s4"), cp("cp_s5"),
    ];

    for id in &checkpoint_ids {
        svc.save(&project(), &run(), id.clone()).await.unwrap();
    }

    // ── (4) Verify oldest checkpoints are Superseded ──────────────────────
    // After 5 saves: cp_s1..cp_s4 must be Superseded; cp_s5 must be Latest.
    let all = svc.list_by_run(&run(), 20).await.unwrap();
    assert_eq!(all.len(), 5, "all 5 checkpoints must be stored");

    // Every checkpoint except the last must be Superseded.
    for id in &checkpoint_ids[..4] {
        let rec = svc.get(id).await.unwrap().expect("checkpoint must exist");
        assert_eq!(
            rec.disposition,
            CheckpointDisposition::Superseded,
            "RFC 005: checkpoint '{}' must be Superseded after a newer save; got: {:?}",
            id.as_str(),
            rec.disposition
        );
    }

    // ── (5) Verify the latest checkpoint is the most recent ───────────────
    let fifth = svc.get(&cp("cp_s5")).await.unwrap().unwrap();
    assert_eq!(
        fifth.disposition,
        CheckpointDisposition::Latest,
        "RFC 005: most recent checkpoint (cp_s5) must be Latest"
    );

    // latest_for_run() must return cp_s5.
    let latest = svc
        .latest_for_run(&run())
        .await
        .unwrap()
        .expect("latest_for_run must return a checkpoint after saves");
    assert_eq!(
        latest.checkpoint_id,
        cp("cp_s5"),
        "latest_for_run must return the most recently saved checkpoint"
    );

    // Exactly 1 Latest across all checkpoints.
    let latest_count = all
        .iter()
        .filter(|c| c.disposition == CheckpointDisposition::Latest)
        .count();
    assert_eq!(
        latest_count, 1,
        "RFC 005: exactly one checkpoint per run may be Latest; found {latest_count}"
    );

    // Exactly 4 Superseded.
    let superseded_count = all
        .iter()
        .filter(|c| c.disposition == CheckpointDisposition::Superseded)
        .count();
    assert_eq!(superseded_count, 4, "the 4 older checkpoints must be Superseded");
}

// ── Strategy update (second set overwrites) ───────────────────────────────────

/// RFC 005 §3: setting a strategy a second time for the same run must replace
/// the first (upsert by run_id).
#[tokio::test]
async fn strategy_update_replaces_previous() {
    let store = Arc::new(InMemoryStore::new());
    let run2 = RunId::new("run_strat_update");

    // First strategy.
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_strat_v1"),
            EventSource::Runtime,
            RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
                strategy_id: "strat_v1".to_owned(),
                description: "Initial strategy".to_owned(),
                set_at_ms: 1_700_000_000_000,
                run_id: Some(run2.clone()),
            }),
        )])
        .await
        .unwrap();

    let v1 = CheckpointStrategyReadModel::get_by_run(store.as_ref(), &run2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(v1.strategy_id, "strat_v1");

    // Second strategy — overwrites the first.
    store
        .append(&[EventEnvelope::for_runtime_event(
            EventId::new("evt_strat_v2"),
            EventSource::Runtime,
            RuntimeEvent::CheckpointStrategySet(CheckpointStrategySet {
                strategy_id: "strat_v2_updated".to_owned(),
                description: "Updated: 5 max checkpoints".to_owned(),
                set_at_ms: 1_700_000_001_000,
                run_id: Some(run2.clone()),
            }),
        )])
        .await
        .unwrap();

    let v2 = CheckpointStrategyReadModel::get_by_run(store.as_ref(), &run2)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        v2.strategy_id, "strat_v2_updated",
        "strategy must be replaced on second set"
    );
    assert_ne!(v2.strategy_id, v1.strategy_id, "updated strategy must differ from original");
}

// ── Checkpoints for different runs are independent ────────────────────────────

/// RFC 005 §3: checkpoint strategies and checkpoints are scoped per run.
/// Saving checkpoints for run A must not affect run B's state.
#[tokio::test]
async fn checkpoints_scoped_per_run() {
    let store = Arc::new(InMemoryStore::new());
    let svc = CheckpointServiceImpl::new(store.clone());

    let run_a = RunId::new("run_scope_a");
    let run_b = RunId::new("run_scope_b");
    let proj = project();

    // Save 3 checkpoints for run_a.
    for i in 0u32..3 {
        svc.save(&proj, &run_a, cp(&format!("cp_a_{i}"))).await.unwrap();
    }
    // Save 2 checkpoints for run_b.
    for i in 0u32..2 {
        svc.save(&proj, &run_b, cp(&format!("cp_b_{i}"))).await.unwrap();
    }

    let a_all = svc.list_by_run(&run_a, 20).await.unwrap();
    let b_all = svc.list_by_run(&run_b, 20).await.unwrap();

    assert_eq!(a_all.len(), 3, "run_a must have 3 checkpoints");
    assert_eq!(b_all.len(), 2, "run_b must have 2 checkpoints");

    // Each run has its own Latest.
    let a_latest = svc.latest_for_run(&run_a).await.unwrap().unwrap();
    let b_latest = svc.latest_for_run(&run_b).await.unwrap().unwrap();

    assert_eq!(a_latest.checkpoint_id, cp("cp_a_2"), "run_a latest must be cp_a_2");
    assert_eq!(b_latest.checkpoint_id, cp("cp_b_1"), "run_b latest must be cp_b_1");
    assert_ne!(
        a_latest.checkpoint_id, b_latest.checkpoint_id,
        "runs must have independent Latest checkpoints"
    );
}

// ── Single checkpoint remains Latest ─────────────────────────────────────────

#[tokio::test]
async fn single_checkpoint_is_always_latest() {
    let store = Arc::new(InMemoryStore::new());
    let svc = CheckpointServiceImpl::new(store);

    svc.save(&project(), &run(), cp("cp_only")).await.unwrap();

    let only = svc.get(&cp("cp_only")).await.unwrap().unwrap();
    assert_eq!(
        only.disposition,
        CheckpointDisposition::Latest,
        "a single checkpoint must always be Latest"
    );
    assert_eq!(only.run_id, run());
    assert_eq!(only.project, project());
}
