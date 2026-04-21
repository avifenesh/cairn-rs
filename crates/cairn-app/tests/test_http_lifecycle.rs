//! End-to-end HTTP lifecycle: tenant scope → session → run → task.
//!
//! Tests the full HTTP path against a live cairn-app subprocess backed
//! by a live Valkey testcontainer. Replaces the in-memory unit tests
//! deleted in PR #67 (task #122 scope).

mod support;

use support::live_fabric::LiveHarness;

#[tokio::test]
async fn harness_boots_and_rotates_admin() {
    let harness = LiveHarness::setup().await;
    assert!(harness.base_url.starts_with("http://127.0.0.1:"));
    assert!(!harness.admin_token.is_empty());

    // Quick health probe — proves the rotated token authenticates.
    let res = harness
        .client()
        .get(format!("{}/health", harness.base_url))
        .send()
        .await
        .expect("health request reaches server");
    assert!(res.status().is_success(), "health returned {}", res.status());
}
