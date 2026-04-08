//! RFC 006 full prompt lifecycle end-to-end integration test.
//!
//! Exercises the complete happy path:
//!   create asset → create version → create release (draft) →
//!   transition to approved → activate → resolve via selector →
//!   create + activate second release → verify first is deactivated.

use std::sync::Arc;

use cairn_domain::{ProjectKey, PromptAssetId, PromptReleaseId, PromptVersionId};
use cairn_runtime::{
    PromptAssetService, PromptAssetServiceImpl, PromptReleaseService, PromptReleaseServiceImpl,
    PromptVersionService, PromptVersionServiceImpl,
};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("tenant_lc", "ws_lc", "proj_lc")
}

struct Services {
    assets: PromptAssetServiceImpl<InMemoryStore>,
    versions: PromptVersionServiceImpl<InMemoryStore>,
    releases: PromptReleaseServiceImpl<InMemoryStore>,
}

fn setup() -> Services {
    let store = Arc::new(InMemoryStore::new());
    Services {
        assets: PromptAssetServiceImpl::new(store.clone()),
        versions: PromptVersionServiceImpl::new(store.clone()),
        releases: PromptReleaseServiceImpl::new(store.clone()),
    }
}

// ── Step-by-step assertions ────────────────────────────────────────────────

/// (1) Create a prompt asset.
#[tokio::test]
async fn step1_create_prompt_asset() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_lc_1");

    let record = svc
        .assets
        .create(
            &project(),
            asset_id.clone(),
            "Greeting prompt".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(record.prompt_asset_id, asset_id);
    assert_eq!(record.name, "Greeting prompt");
    assert_eq!(record.project, project());
}

/// (2) Create a prompt version linked to the asset.
#[tokio::test]
async fn step2_create_prompt_version() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_lc_2");
    let version_id = PromptVersionId::new("ver_lc_2");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "Asset".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();

    let version = svc
        .versions
        .create(
            &project(),
            version_id.clone(),
            asset_id.clone(),
            "sha256:abc".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(version.prompt_version_id, version_id);
    assert_eq!(version.prompt_asset_id, asset_id);
}

/// (3) Create a prompt release — initial state must be "draft".
#[tokio::test]
async fn step3_create_release_in_draft_state() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_lc_3");
    let version_id = PromptVersionId::new("ver_lc_3");
    let release_id = PromptReleaseId::new("rel_lc_3");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "A".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();
    svc.versions
        .create(
            &project(),
            version_id.clone(),
            asset_id.clone(),
            "hash3".to_owned(),
        )
        .await
        .unwrap();

    let release = svc
        .releases
        .create(&project(), release_id.clone(), asset_id, version_id)
        .await
        .unwrap();

    assert_eq!(release.prompt_release_id, release_id);
    assert_eq!(
        release.state, "draft",
        "newly created release must be in draft state"
    );
}

/// (4) Transition a release from draft to approved.
#[tokio::test]
async fn step4_transition_draft_to_approved() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_lc_4");
    let release_id = PromptReleaseId::new("rel_lc_4");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "A".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(
            &project(),
            release_id.clone(),
            asset_id,
            PromptVersionId::new("v4"),
        )
        .await
        .unwrap();

    let approved = svc
        .releases
        .transition(&release_id, "approved")
        .await
        .unwrap();
    assert_eq!(approved.state, "approved");
}

/// (5) Activate an approved release — state becomes "active".
#[tokio::test]
async fn step5_activate_approved_release() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_lc_5");
    let release_id = PromptReleaseId::new("rel_lc_5");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "A".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(
            &project(),
            release_id.clone(),
            asset_id,
            PromptVersionId::new("v5"),
        )
        .await
        .unwrap();
    svc.releases
        .transition(&release_id, "approved")
        .await
        .unwrap();

    // Direct activation from draft must fail (must go through approved first).
    let draft_release_id = PromptReleaseId::new("rel_lc_5_draft");
    svc.releases
        .create(
            &project(),
            draft_release_id.clone(),
            PromptAssetId::new("asset_lc_5b"),
            PromptVersionId::new("v5b"),
        )
        .await
        .unwrap();
    let draft_activate = svc.releases.activate(&draft_release_id).await;
    assert!(
        draft_activate.is_err(),
        "activating a draft release must fail"
    );

    // Approved release activates successfully.
    let active = svc.releases.activate(&release_id).await.unwrap();
    assert_eq!(active.state, "active");
}

/// (6) Active release is resolvable via selector.
#[tokio::test]
async fn step6_active_release_resolves_via_selector() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_lc_6");
    let release_id = PromptReleaseId::new("rel_lc_6");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "A".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();
    svc.releases
        .create(
            &project(),
            release_id.clone(),
            asset_id.clone(),
            PromptVersionId::new("v6"),
        )
        .await
        .unwrap();
    svc.releases
        .transition(&release_id, "approved")
        .await
        .unwrap();
    svc.releases.activate(&release_id).await.unwrap();

    // resolve() returns the active release for this asset + selector.
    let resolved = svc
        .releases
        .resolve(&project(), &asset_id, "user_abc")
        .await
        .unwrap();

    assert!(
        resolved.is_some(),
        "active release must be resolvable via selector"
    );
    let resolved = resolved.unwrap();
    assert_eq!(resolved.prompt_release_id, release_id);
    assert_eq!(resolved.state, "active");

    // A non-existent asset returns None, not an error.
    let none = svc
        .releases
        .resolve(&project(), &PromptAssetId::new("no_such_asset"), "user_abc")
        .await
        .unwrap();
    assert!(none.is_none(), "unknown asset must resolve to None");
}

/// (7) Activating a second release deactivates the first.
#[tokio::test]
async fn step7_second_release_activation_deactivates_first() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_lc_7");
    let rel1 = PromptReleaseId::new("rel_lc_7a");
    let rel2 = PromptReleaseId::new("rel_lc_7b");

    svc.assets
        .create(
            &project(),
            asset_id.clone(),
            "A".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();

    // Create, approve, and activate the first release.
    svc.releases
        .create(
            &project(),
            rel1.clone(),
            asset_id.clone(),
            PromptVersionId::new("v7a"),
        )
        .await
        .unwrap();
    svc.releases.transition(&rel1, "approved").await.unwrap();
    let active1 = svc.releases.activate(&rel1).await.unwrap();
    assert_eq!(active1.state, "active", "first release must be active");

    // Create, approve, and activate the second release.
    svc.releases
        .create(
            &project(),
            rel2.clone(),
            asset_id.clone(),
            PromptVersionId::new("v7b"),
        )
        .await
        .unwrap();
    svc.releases.transition(&rel2, "approved").await.unwrap();
    let active2 = svc.releases.activate(&rel2).await.unwrap();
    assert_eq!(active2.state, "active", "second release must now be active");

    // First release must no longer be active.
    let rel1_record = svc.releases.get(&rel1).await.unwrap().unwrap();
    assert_ne!(
        rel1_record.state, "active",
        "first release must be deactivated after second is activated"
    );

    // Selector must now resolve to the second release.
    let resolved = svc
        .releases
        .resolve(&project(), &asset_id, "any_selector")
        .await
        .unwrap()
        .expect("active release must resolve");
    assert_eq!(
        resolved.prompt_release_id, rel2,
        "selector must resolve to the newly activated second release"
    );
}

/// Full happy path: all 7 steps in sequence using a single shared setup.
#[tokio::test]
async fn full_prompt_lifecycle_happy_path() {
    let svc = setup();
    let asset_id = PromptAssetId::new("asset_full");
    let version_id = PromptVersionId::new("ver_full");
    let rel1_id = PromptReleaseId::new("rel_full_1");
    let rel2_id = PromptReleaseId::new("rel_full_2");

    // (1) Create asset.
    let asset = svc
        .assets
        .create(
            &project(),
            asset_id.clone(),
            "Full lifecycle asset".to_owned(),
            "system".to_owned(),
        )
        .await
        .unwrap();
    assert_eq!(asset.prompt_asset_id, asset_id);

    // (2) Create version.
    let version = svc
        .versions
        .create(
            &project(),
            version_id.clone(),
            asset_id.clone(),
            "sha256:full".to_owned(),
        )
        .await
        .unwrap();
    assert_eq!(version.prompt_asset_id, asset_id);

    // (3) Create release — starts in draft.
    let release = svc
        .releases
        .create(
            &project(),
            rel1_id.clone(),
            asset_id.clone(),
            version_id.clone(),
        )
        .await
        .unwrap();
    assert_eq!(release.state, "draft");

    // (4) Transition to approved.
    let approved = svc.releases.transition(&rel1_id, "approved").await.unwrap();
    assert_eq!(approved.state, "approved");

    // (5) Activate.
    let active = svc.releases.activate(&rel1_id).await.unwrap();
    assert_eq!(active.state, "active");

    // (6) Resolve via selector — finds the active release.
    let resolved = svc
        .releases
        .resolve(&project(), &asset_id, "session_xyz")
        .await
        .unwrap()
        .expect("must find active release");
    assert_eq!(resolved.prompt_release_id, rel1_id);
    assert_eq!(resolved.state, "active");

    // (7) Second release: create, approve, activate — first is deactivated.
    svc.releases
        .create(
            &project(),
            rel2_id.clone(),
            asset_id.clone(),
            version_id.clone(),
        )
        .await
        .unwrap();
    svc.releases.transition(&rel2_id, "approved").await.unwrap();
    let active2 = svc.releases.activate(&rel2_id).await.unwrap();
    assert_eq!(active2.state, "active");

    let rel1_after = svc.releases.get(&rel1_id).await.unwrap().unwrap();
    assert_ne!(
        rel1_after.state, "active",
        "first release deactivated by second activation"
    );

    let resolved2 = svc
        .releases
        .resolve(&project(), &asset_id, "session_xyz")
        .await
        .unwrap()
        .expect("second release must be resolvable");
    assert_eq!(resolved2.prompt_release_id, rel2_id);
}
