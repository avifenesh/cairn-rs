use cairn_domain::RunId;

use ff_core::contracts::ReportUsageResult;
use ff_core::types::ExecutionId;

use crate::TestHarness;

#[tokio::test]
async fn test_budget_hard_limit() {
    let h = TestHarness::setup().await;
    let run_id = RunId::new(format!("budget_run_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 100, 1_000_000, 50)
        .await
        .expect("create budget failed");

    // Distinct ExecutionId per spend: each call is a logically different
    // operation, so dedup must NOT fire between them.
    let eid_a = ExecutionId::new();
    let spend_ok = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid_a, &[("tokens", 50)])
        .await
        .expect("spend failed");

    assert_eq!(spend_ok, ReportUsageResult::Ok);

    let eid_b = ExecutionId::new();
    let spend_breach = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid_b, &[("tokens", 60)])
        .await
        .expect("spend failed");

    assert!(
        matches!(spend_breach, ReportUsageResult::HardBreach { ref dimension, .. } if dimension == "tokens"),
        "expected HardBreach on tokens, got {spend_breach:?}"
    );

    h.teardown().await;
}

#[tokio::test]
async fn test_budget_status_reflects_spend() {
    let h = TestHarness::setup().await;
    let run_id = RunId::new(format!("budget_status_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 1000, 1_000_000, 100)
        .await
        .expect("create budget failed");

    let eid = ExecutionId::new();
    h.fabric
        .budgets
        .record_spend(&budget_id, &eid, &[("tokens", 42)])
        .await
        .expect("spend failed");

    let status = h
        .fabric
        .budgets
        .get_budget_status(&budget_id)
        .await
        .expect("status failed");

    assert_eq!(status.scope_type, "run");
    assert_eq!(*status.usage.get("tokens").unwrap_or(&0), 42);
    assert_eq!(*status.hard_limits.get("tokens").unwrap_or(&0), 1000);

    h.teardown().await;
}

#[tokio::test]
async fn test_budget_release_resets_usage() {
    let h = TestHarness::setup().await;
    let run_id = RunId::new(format!("budget_release_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 100, 1_000_000, 50)
        .await
        .expect("create budget failed");

    let eid_pre = ExecutionId::new();
    h.fabric
        .budgets
        .record_spend(&budget_id, &eid_pre, &[("tokens", 90)])
        .await
        .expect("spend failed");

    h.fabric
        .budgets
        .release_budget(&budget_id)
        .await
        .expect("release failed");

    let eid_post = ExecutionId::new();
    let after = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid_post, &[("tokens", 50)])
        .await
        .expect("spend after release failed");

    assert_eq!(after, ReportUsageResult::Ok);

    h.teardown().await;
}

/// End-to-end exercise of FF's dedup slot. Two identical spend calls (same
/// budget + same execution_id + same deltas) MUST produce:
///   1) `Ok` on the first call — budget usage increments,
///   2) `AlreadyApplied` on the second — budget usage does NOT increment again.
///
/// Regression guard for worker-2's BUG 1 (nil-UUID dedup collision): if the
/// ExecutionId argument is ever silently defaulted to a sentinel on behalf
/// of the caller, this test passes (the two calls share the sentinel so dedup
/// fires) — which is why the unit-level guard is to REQUIRE a non-optional
/// `&ExecutionId`. The integration test proves FF-side behavior matches.
#[tokio::test]
async fn test_budget_spend_dedup_returns_already_applied() {
    let h = TestHarness::setup().await;
    let run_id = RunId::new(format!("budget_dedup_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 1000, 1_000_000, 100)
        .await
        .expect("create budget failed");

    // Pin the ExecutionId so both calls produce the same idempotency key.
    let eid = ExecutionId::new();
    let deltas: &[(&str, u64)] = &[("tokens", 100)];

    let first = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid, deltas)
        .await
        .expect("first spend failed");
    assert_eq!(
        first,
        ReportUsageResult::Ok,
        "first spend must land fresh, got {first:?}"
    );

    let second = h
        .fabric
        .budgets
        .record_spend(&budget_id, &eid, deltas)
        .await
        .expect("second spend failed");
    assert_eq!(
        second,
        ReportUsageResult::AlreadyApplied,
        "second spend with same (budget, execution, deltas) must hit dedup, got {second:?}",
    );

    // Budget usage must reflect the single fresh spend, not two.
    let status = h
        .fabric
        .budgets
        .get_budget_status(&budget_id)
        .await
        .expect("status failed");
    assert_eq!(
        *status.usage.get("tokens").unwrap_or(&0),
        100,
        "dedup must prevent double-counting; expected 100, got {:?}",
        status.usage.get("tokens"),
    );

    h.teardown().await;
}
