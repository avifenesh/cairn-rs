//! Integration tests for the [`ControlPlaneBackend`] trait and the
//! [`Engine`]'s worker methods (Phase D PR 1).
//!
//! Exercises the FCALL-shaped control-plane trait (budget / quota /
//! rotation) plus the new worker-registry methods against a live
//! Valkey. These tests are the regression net for the Phase D PR 1
//! split — if `FabricBudgetService`'s delegation to the trait stops
//! preserving outcome variants, or the Engine's worker HSET layout
//! drifts, these fail.
//!
//! Note: rotation is exercised against the shared Valkey with the
//! SAME `(kid, secret)` every TestHarness instance seeds (see
//! `integration.rs`'s HMAC footgun banner). The test therefore
//! relies on the `noop` path for its assertion — we verify the
//! FCALL fan-out round-trips without changing state.
//!
//! [`ControlPlaneBackend`]: cairn_fabric::engine::ControlPlaneBackend
//! [`Engine`]: cairn_fabric::engine::Engine

use cairn_fabric::engine::{BudgetSpendOutcome, QuotaAdmission};
use ff_core::types::{BudgetId, ExecutionId, LaneId, WorkerId, WorkerInstanceId};

use crate::TestHarness;

fn test_eid(h: &TestHarness, seed: &str) -> ExecutionId {
    let uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, seed.as_bytes());
    ExecutionId::deterministic_solo(&LaneId::new("test"), h.partition_config(), uuid)
}

// ── Budget via ControlPlaneBackend ──────────────────────────────────────

#[tokio::test]
async fn control_plane_budget_create_spend_release_roundtrip() {
    let h = TestHarness::setup().await;
    let run_id = cairn_domain::RunId::new(format!("cp_budget_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 200, 1_000_000, 50)
        .await
        .expect("create_run_budget");

    let eid = test_eid(&h, "cp_budget_spend");
    let first = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid, &[("tokens", 50)])
        .await
        .expect("record_spend");
    assert_eq!(first, BudgetSpendOutcome::Ok, "first spend must land fresh");

    // Second identical spend (same execution, same deltas) — dedup must fire.
    let second = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid, &[("tokens", 50)])
        .await
        .expect("record_spend second");
    assert_eq!(
        second,
        BudgetSpendOutcome::AlreadyApplied,
        "dedup must fire on identical inputs"
    );

    // Status reflects the ONE applied spend.
    let status = h
        .fabric
        .budgets
        .get_budget_status(&budget_id)
        .await
        .expect("get_budget_status");
    assert_eq!(*status.usage.get("tokens").unwrap_or(&0), 50);

    // Release resets usage.
    h.fabric
        .budgets
        .release_budget(&budget_id)
        .await
        .expect("release_budget");
    let post_status = h
        .fabric
        .budgets
        .get_budget_status(&budget_id)
        .await
        .expect("get_budget_status post-release");
    // After reset, FF clears the usage hash — field absent is equivalent
    // to 0 from the admin-read perspective.
    assert_eq!(*post_status.usage.get("tokens").unwrap_or(&0), 0);

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_budget_get_status_missing_returns_not_found() {
    let h = TestHarness::setup().await;
    let missing = BudgetId::new();
    let err = h
        .fabric
        .budgets
        .get_budget_status(&missing)
        .await
        .expect_err("absent budget must error");
    match err {
        cairn_fabric::FabricError::NotFound { entity, .. } => assert_eq!(entity, "budget"),
        other => panic!("expected NotFound budget, got {other:?}"),
    }

    h.teardown().await;
}

#[tokio::test]
async fn control_plane_budget_hard_breach_preserves_dimension() {
    let h = TestHarness::setup().await;
    let run_id = cairn_domain::RunId::new(format!("cp_hard_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 100, 1_000_000, 10)
        .await
        .expect("create_run_budget");

    // First spend below limit.
    let eid_a = test_eid(&h, "cp_hard_a");
    let _ = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid_a, &[("tokens", 60)])
        .await
        .expect("first spend");

    // Second spend pushes over the hard limit of 100.
    let eid_b = test_eid(&h, "cp_hard_b");
    let breach = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid_b, &[("tokens", 50)])
        .await
        .expect("breach spend");
    assert!(
        matches!(
            breach,
            BudgetSpendOutcome::HardBreach { ref dimension, .. } if dimension == "tokens"
        ),
        "expected HardBreach on tokens, got {breach:?}"
    );

    h.teardown().await;
}

// ── Quota via ControlPlaneBackend ───────────────────────────────────────

#[tokio::test]
async fn control_plane_quota_admission_admits_under_limit() {
    let h = TestHarness::setup().await;

    let qid = h
        .fabric
        .quotas
        .create_quota_policy("run", "cp_quota", 60, 100, 10)
        .await
        .expect("create_quota_policy");

    let eid = test_eid(&h, "cp_quota_admit");
    let outcome = h
        .fabric
        .quotas
        .check_admission(&qid, &eid, 60, 100, 10)
        .await
        .expect("check_admission");
    assert_eq!(outcome, QuotaAdmission::Admitted);

    // Second check with the SAME execution_id must dedup to
    // AlreadyAdmitted.
    let repeat = h
        .fabric
        .quotas
        .check_admission(&qid, &eid, 60, 100, 10)
        .await
        .expect("check_admission repeat");
    assert_eq!(repeat, QuotaAdmission::AlreadyAdmitted);

    h.teardown().await;
}

// ── Rotation via ControlPlaneBackend ────────────────────────────────────

#[tokio::test]
async fn control_plane_rotation_noop_on_seeded_secret() {
    // The harness seeds a deterministic (kid, secret) on every
    // partition at startup. Re-submitting the SAME pair must return
    // `noop` for every partition — no real rotation happens. This is
    // the only rotation test that's safe against the shared Valkey;
    // see integration.rs for the HMAC-ROTATION footgun banner.
    let h = TestHarness::setup().await;
    let num_partitions = h.partition_config().num_flow_partitions;

    let outcome = h
        .fabric
        .rotation
        .rotate_waitpoint_hmac(
            "cairn-test-k1",
            "00000000000000000000000000000000000000000000000000000000000000aa",
            60_000,
        )
        .await;

    assert_eq!(outcome.new_kid, "cairn-test-k1");
    assert!(
        outcome.failed.is_empty(),
        "no partition must fail the replay, got {:?}",
        outcome.failed
    );
    // Rotated may be 0 or num_partitions depending on whether another
    // test in this suite ran a first-time seed — the fabric boot path
    // already did the initial rotate on harness setup, so the typical
    // case is all `noop`. Either way the count of (rotated + noop)
    // must equal the partition count.
    assert_eq!(
        outcome.rotated as u32 + outcome.noop as u32,
        num_partitions as u32,
        "every partition must be accounted for"
    );

    h.teardown().await;
}

// ── Worker registry via Engine ──────────────────────────────────────────

#[tokio::test]
async fn engine_register_heartbeat_mark_dead_roundtrip() {
    let h = TestHarness::setup().await;
    let wid = WorkerId::new(format!("cp_w_{}", uuid::Uuid::new_v4()));
    let iid = WorkerInstanceId::new(format!("cp_i_{}", uuid::Uuid::new_v4()));
    let caps = vec!["gpu=true".to_owned(), "linux=x86_64".to_owned()];

    let reg = h
        .fabric
        .worker
        .register_worker(&wid, &iid, &caps)
        .await
        .expect("register_worker");
    assert_eq!(reg.worker_id, wid);
    assert_eq!(reg.instance_id, iid);
    assert_eq!(reg.capabilities.len(), 2);
    assert!(
        reg.registered_at_ms > 1_700_000_000_000,
        "registered_at_ms must be a real epoch ms, got {}",
        reg.registered_at_ms
    );

    // Heartbeat must succeed.
    h.fabric
        .worker
        .heartbeat_worker(&iid)
        .await
        .expect("heartbeat_worker");

    // Mark dead must succeed (idempotent — no pre-state required).
    h.fabric
        .worker
        .mark_worker_dead(&iid)
        .await
        .expect("mark_worker_dead");

    h.teardown().await;
}
