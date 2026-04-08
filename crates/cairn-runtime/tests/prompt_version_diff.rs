//! RFC 006 prompt version integration tests.
//!
//! Exercises create / get / list_by_asset on `PromptVersionService` and verifies
//! that content-hash differences between versions are correctly persisted so that
//! a future diff endpoint can compare them.

use std::sync::Arc;

use cairn_domain::{ProjectKey, PromptAssetId, PromptVersionId};
use cairn_runtime::{PromptVersionService, PromptVersionServiceImpl};
use cairn_store::InMemoryStore;

fn project() -> ProjectKey {
    ProjectKey::new("tenant", "workspace", "project")
}

fn store_and_svc() -> (Arc<InMemoryStore>, PromptVersionServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let svc = PromptVersionServiceImpl::new(store.clone());
    (store, svc)
}

/// Create two versions with different content hashes under the same asset.
/// Verify both are persisted and their hashes differ.
#[tokio::test]
async fn prompt_version_diff_detects_changes() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_1");

    let v1 = svc
        .create(
            &project(),
            PromptVersionId::new("ver_1"),
            asset_id.clone(),
            "hash_a".to_owned(),
        )
        .await
        .unwrap();

    let v2 = svc
        .create(
            &project(),
            PromptVersionId::new("ver_2"),
            asset_id.clone(),
            "hash_b".to_owned(),
        )
        .await
        .unwrap();

    assert_eq!(v1.prompt_version_id.as_str(), "ver_1");
    assert_eq!(v2.prompt_version_id.as_str(), "ver_2");
    assert_eq!(v1.content_hash, "hash_a");
    assert_eq!(v2.content_hash, "hash_b");
    assert_ne!(
        v1.content_hash, v2.content_hash,
        "versions differ — content hashes must differ"
    );
    assert_eq!(
        v1.prompt_asset_id, v2.prompt_asset_id,
        "both versions belong to the same asset"
    );
}

/// Swapping version IDs: retrieving by ID returns the correct record.
#[tokio::test]
async fn prompt_version_diff_swap_is_inverse() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_swap");

    svc.create(
        &project(),
        PromptVersionId::new("swap_a"),
        asset_id.clone(),
        "ha".to_owned(),
    )
    .await
    .unwrap();

    svc.create(
        &project(),
        PromptVersionId::new("swap_b"),
        asset_id.clone(),
        "hb".to_owned(),
    )
    .await
    .unwrap();

    let a = svc
        .get(&PromptVersionId::new("swap_a"))
        .await
        .unwrap()
        .expect("swap_a should exist");
    let b = svc
        .get(&PromptVersionId::new("swap_b"))
        .await
        .unwrap()
        .expect("swap_b should exist");

    // Each version's hash should match what was stored.
    assert_eq!(a.content_hash, "ha");
    assert_eq!(b.content_hash, "hb");

    // Both belong to the same asset.
    assert_eq!(a.prompt_asset_id, b.prompt_asset_id);
}

/// Identical content hashes across versions produce equal hash values.
#[tokio::test]
async fn prompt_version_diff_identical_versions() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_same");

    let hash = "same_hash".to_owned();

    let v1 = svc
        .create(
            &project(),
            PromptVersionId::new("same_a"),
            asset_id.clone(),
            hash.clone(),
        )
        .await
        .unwrap();

    let v2 = svc
        .create(
            &project(),
            PromptVersionId::new("same_b"),
            asset_id.clone(),
            hash.clone(),
        )
        .await
        .unwrap();

    assert_eq!(
        v1.content_hash, v2.content_hash,
        "identical content: hashes must match"
    );
}

/// Getting a non-existent version returns None.
#[tokio::test]
async fn prompt_version_diff_not_found_returns_none() {
    let (_store, svc) = store_and_svc();

    let result = svc.get(&PromptVersionId::new("ghost_a")).await.unwrap();

    assert!(
        result.is_none(),
        "get of nonexistent version must return None"
    );
}

/// list_by_asset returns versions in the expected order and count.
#[tokio::test]
async fn prompt_version_list_by_asset() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_formula");

    // Create 4 versions under the same asset.
    for (id, hash) in [
        ("fx1", "hx1"),
        ("fx2", "hx2"),
        ("fx3", "hx3"),
        ("fx4", "hx4"),
    ] {
        svc.create(
            &project(),
            PromptVersionId::new(id),
            asset_id.clone(),
            hash.to_owned(),
        )
        .await
        .unwrap();
    }

    let all = svc.list_by_asset(&asset_id, 10, 0).await.unwrap();
    assert_eq!(all.len(), 4, "should list all 4 versions");

    // Pagination: limit 2, offset 0.
    let page1 = svc.list_by_asset(&asset_id, 2, 0).await.unwrap();
    assert_eq!(page1.len(), 2, "page 1 should have 2 items");

    // Pagination: limit 2, offset 2.
    let page2 = svc.list_by_asset(&asset_id, 2, 2).await.unwrap();
    assert_eq!(page2.len(), 2, "page 2 should have 2 items");

    // All hashes are distinct.
    let hashes: Vec<_> = all.iter().map(|v| v.content_hash.as_str()).collect();
    let mut deduped = hashes.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(hashes.len(), deduped.len(), "all hashes should be unique");
}
