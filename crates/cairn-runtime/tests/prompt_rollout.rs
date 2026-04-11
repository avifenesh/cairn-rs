//! RFC 006 prompt rollout strategy integration tests.

use std::sync::Arc;

use cairn_domain::{ProjectKey, PromptAssetId, PromptReleaseId, PromptVersionId};
use cairn_runtime::{PromptReleaseService, PromptReleaseServiceImpl};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("tenant", "workspace", "project")
}
fn asset() -> PromptAssetId {
    PromptAssetId::new("asset_1")
}

async fn setup() -> (Arc<InMemoryStore>, PromptReleaseServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let svc = PromptReleaseServiceImpl::new(store.clone());
    (store, svc)
}

/// Helper: create, approve, and activate a release.
async fn make_active_release(svc: &PromptReleaseServiceImpl<InMemoryStore>, release_id: &str) {
    svc.create(
        &project(),
        PromptReleaseId::new(release_id),
        asset(),
        PromptVersionId::new("v1"),
    )
    .await
    .unwrap();
    svc.transition(&PromptReleaseId::new(release_id), "approved")
        .await
        .unwrap();
}

/// start_rollout activates the release and sets rollout_percent.
#[tokio::test]
async fn prompt_rollout_start_sets_percent_and_activates() {
    let (_store, svc) = setup().await;

    make_active_release(&svc, "rel_r1").await;

    let record = svc
        .start_rollout(&PromptReleaseId::new("rel_r1"), 30)
        .await
        .unwrap();

    assert_eq!(record.state, "active");
    assert_eq!(record.rollout_percent, Some(30));
}

/// Two releases with pct=80 and pct=20: across 100 selector strings,
/// the 80% release should be selected ~70–90 times.
#[tokio::test]
async fn prompt_rollout_distributes_traffic_by_percent() {
    let (_store, svc) = setup().await;

    // rel_big: 80%, rel_small: 20%
    make_active_release(&svc, "rel_big").await;
    make_active_release(&svc, "rel_small").await;

    svc.start_rollout(&PromptReleaseId::new("rel_big"), 80)
        .await
        .unwrap();
    svc.start_rollout(&PromptReleaseId::new("rel_small"), 20)
        .await
        .unwrap();

    let mut big_count = 0u32;
    let mut small_count = 0u32;

    for i in 0..100u32 {
        let selector = format!("selector_{i}");
        if let Some(record) = svc.resolve(&project(), &asset(), &selector).await.unwrap() {
            if record.prompt_release_id == PromptReleaseId::new("rel_big") {
                big_count += 1;
            } else {
                small_count += 1;
            }
        }
    }

    assert_eq!(
        big_count + small_count,
        100,
        "all 100 queries must resolve to one of the two releases"
    );

    assert!(
        (70..=90).contains(&big_count),
        "80% release should be selected 70–90 times out of 100, got {big_count}"
    );
    assert!(
        (10..=30).contains(&small_count),
        "20% release should be selected 10–30 times out of 100, got {small_count}"
    );
}

/// Determinism: the same selector always picks the same release.
#[tokio::test]
async fn prompt_rollout_selection_is_deterministic() {
    let (_store, svc) = setup().await;

    make_active_release(&svc, "det_a").await;
    make_active_release(&svc, "det_b").await;

    svc.start_rollout(&PromptReleaseId::new("det_a"), 50)
        .await
        .unwrap();
    svc.start_rollout(&PromptReleaseId::new("det_b"), 50)
        .await
        .unwrap();

    let selector = "my_constant_selector";
    let first = svc.resolve(&project(), &asset(), selector).await.unwrap();
    for _ in 0..10 {
        let again = svc.resolve(&project(), &asset(), selector).await.unwrap();
        assert_eq!(
            first.as_ref().map(|r| r.prompt_release_id.as_str()),
            again.as_ref().map(|r| r.prompt_release_id.as_str()),
            "same selector must always resolve to the same release"
        );
    }
}

/// start_rollout on an already-active release just sets the percent (no double-transition).
#[tokio::test]
async fn prompt_rollout_already_active_just_sets_percent() {
    let (_store, svc) = setup().await;

    make_active_release(&svc, "rel_pre").await;
    // Activate first via normal path
    svc.transition(&PromptReleaseId::new("rel_pre"), "active")
        .await
        .unwrap();

    // Now start_rollout — should just set percent, not error
    let record = svc
        .start_rollout(&PromptReleaseId::new("rel_pre"), 60)
        .await
        .unwrap();

    assert_eq!(record.state, "active");
    assert_eq!(record.rollout_percent, Some(60));
}

/// With a single active release (no rollout), resolve() returns it unchanged.
#[tokio::test]
async fn prompt_rollout_single_release_no_percent_resolves_normally() {
    let (_store, svc) = setup().await;

    make_active_release(&svc, "solo").await;
    svc.transition(&PromptReleaseId::new("solo"), "active")
        .await
        .unwrap();

    let record = svc
        .resolve(&project(), &asset(), "any_selector")
        .await
        .unwrap();

    assert!(record.is_some(), "active release should resolve");
    assert_eq!(
        record.unwrap().prompt_release_id,
        PromptReleaseId::new("solo")
    );
}
