use cairn_domain::RunId;

use cairn_fabric::services::budget_service::SpendResult;

use crate::TestHarness;

#[tokio::test]
#[ignore]
async fn test_budget_hard_limit() {
    let h = TestHarness::setup().await;
    let run_id = RunId::new(format!("budget_run_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 100, 1_000_000, 50)
        .await
        .expect("create budget failed");

    let spend_ok = h
        .fabric
        .budgets
        .record_spend(&budget_id, &[("tokens", 50)])
        .await
        .expect("spend failed");

    assert_eq!(spend_ok, SpendResult::Ok { was_retry: false });

    let spend_breach = h
        .fabric
        .budgets
        .record_spend(&budget_id, &[("tokens", 60)])
        .await
        .expect("spend failed");

    assert!(
        matches!(spend_breach, SpendResult::HardBreach { ref dimension, .. } if dimension == "tokens"),
        "expected HardBreach on tokens, got {spend_breach:?}"
    );

    h.teardown().await;
}

#[tokio::test]
#[ignore]
async fn test_budget_status_reflects_spend() {
    let h = TestHarness::setup().await;
    let run_id = RunId::new(format!("budget_status_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 1000, 1_000_000, 100)
        .await
        .expect("create budget failed");

    h.fabric
        .budgets
        .record_spend(&budget_id, &[("tokens", 42)])
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
#[ignore]
async fn test_budget_release_resets_usage() {
    let h = TestHarness::setup().await;
    let run_id = RunId::new(format!("budget_release_{}", uuid::Uuid::new_v4()));

    let budget_id = h
        .fabric
        .budgets
        .create_run_budget(&run_id, 100, 1_000_000, 50)
        .await
        .expect("create budget failed");

    h.fabric
        .budgets
        .record_spend(&budget_id, &[("tokens", 90)])
        .await
        .expect("spend failed");

    h.fabric
        .budgets
        .release_budget(&budget_id)
        .await
        .expect("release failed");

    let after = h
        .fabric
        .budgets
        .record_spend(&budget_id, &[("tokens", 50)])
        .await
        .expect("spend after release failed");

    assert_eq!(after, SpendResult::Ok { was_retry: false });

    h.teardown().await;
}
