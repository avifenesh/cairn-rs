//! RFC 006 prompt version content diff integration tests.

use std::sync::Arc;

use cairn_domain::{PromptAssetId, PromptVersionId, WorkspaceKey};
use cairn_runtime::{PromptVersionService, PromptVersionServiceImpl};
use cairn_store::InMemoryStore;

fn workspace() -> WorkspaceKey {
    WorkspaceKey::new("tenant", "workspace")
}

fn store_and_svc() -> (Arc<InMemoryStore>, PromptVersionServiceImpl<InMemoryStore>) {
    let store = Arc::new(InMemoryStore::new());
    let svc = PromptVersionServiceImpl::new(store.clone());
    (store, svc)
}

/// Create two versions with different content.
/// diff(a, b): added_lines and removed_lines are non-empty, similarity < 1.0.
#[tokio::test]
async fn prompt_version_diff_detects_changes() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_1");

    svc.create(
        &workspace(),
        PromptVersionId::new("ver_1"),
        asset_id.clone(),
        "hash_a".to_owned(),
        Some("Hello world\nThis is line two\nShared line here".to_owned()),
        None,
    )
    .await
    .unwrap();

    svc.create(
        &workspace(),
        PromptVersionId::new("ver_2"),
        asset_id.clone(),
        "hash_b".to_owned(),
        Some("Hello world\nThis is a NEW line\nShared line here".to_owned()),
        None,
    )
    .await
    .unwrap();

    let diff = svc
        .diff(
            &PromptVersionId::new("ver_1"),
            &PromptVersionId::new("ver_2"),
        )
        .await
        .unwrap();

    assert_eq!(diff.version_a_id, "ver_1");
    assert_eq!(diff.version_b_id, "ver_2");

    assert!(!diff.added_lines.is_empty(), "should have added lines");
    assert!(!diff.removed_lines.is_empty(), "should have removed lines");
    assert!(!diff.unchanged_lines.is_empty(), "should have unchanged lines");

    assert!(
        diff.similarity_pct < 1.0,
        "versions differ — similarity must be < 1.0, got {}",
        diff.similarity_pct
    );
    assert!(
        diff.similarity_pct >= 0.0,
        "similarity must be non-negative, got {}",
        diff.similarity_pct
    );

    // Verify specific lines.
    assert!(diff.added_lines.contains(&"This is a NEW line".to_owned()));
    assert!(diff.removed_lines.contains(&"This is line two".to_owned()));
    assert!(diff.unchanged_lines.contains(&"Hello world".to_owned()));
    assert!(diff.unchanged_lines.contains(&"Shared line here".to_owned()));
}

/// Swapping a/b produces inverse: added ↔ removed, same unchanged.
#[tokio::test]
async fn prompt_version_diff_swap_is_inverse() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_swap");

    svc.create(
        &workspace(),
        PromptVersionId::new("swap_a"),
        asset_id.clone(),
        "ha".to_owned(),
        Some("Line alpha\nLine shared\nLine beta extra".to_owned()),
        None,
    )
    .await
    .unwrap();

    svc.create(
        &workspace(),
        PromptVersionId::new("swap_b"),
        asset_id.clone(),
        "hb".to_owned(),
        Some("Line gamma\nLine shared\nLine delta extra".to_owned()),
        None,
    )
    .await
    .unwrap();

    let ab = svc
        .diff(&PromptVersionId::new("swap_a"), &PromptVersionId::new("swap_b"))
        .await
        .unwrap();
    let ba = svc
        .diff(&PromptVersionId::new("swap_b"), &PromptVersionId::new("swap_a"))
        .await
        .unwrap();

    // added(a→b) == removed(b→a) and vice versa.
    let mut ab_added = ab.added_lines.clone();
    let mut ba_removed = ba.removed_lines.clone();
    ab_added.sort();
    ba_removed.sort();
    assert_eq!(ab_added, ba_removed, "added(a→b) must equal removed(b→a)");

    let mut ab_removed = ab.removed_lines.clone();
    let mut ba_added = ba.added_lines.clone();
    ab_removed.sort();
    ba_added.sort();
    assert_eq!(ab_removed, ba_added, "removed(a→b) must equal added(b→a)");

    // Unchanged lines are symmetric.
    let mut ab_unch = ab.unchanged_lines.clone();
    let mut ba_unch = ba.unchanged_lines.clone();
    ab_unch.sort();
    ba_unch.sort();
    assert_eq!(ab_unch, ba_unch, "unchanged lines must be symmetric");

    // Similarity is symmetric.
    assert!(
        (ab.similarity_pct - ba.similarity_pct).abs() < 1e-9,
        "similarity must be symmetric: a→b={} b→a={}",
        ab.similarity_pct,
        ba.similarity_pct
    );
}

/// Identical versions produce similarity = 1.0, no added or removed lines.
#[tokio::test]
async fn prompt_version_diff_identical_versions() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_same");

    let content = "Line one\nLine two\nLine three".to_owned();

    svc.create(
        &workspace(),
        PromptVersionId::new("same_a"),
        asset_id.clone(),
        "h1".to_owned(),
        Some(content.clone()),
        None,
    )
    .await
    .unwrap();

    svc.create(
        &workspace(),
        PromptVersionId::new("same_b"),
        asset_id.clone(),
        "h2".to_owned(),
        Some(content),
        None,
    )
    .await
    .unwrap();

    let diff = svc
        .diff(
            &PromptVersionId::new("same_a"),
            &PromptVersionId::new("same_b"),
        )
        .await
        .unwrap();

    assert!(diff.added_lines.is_empty(), "identical: no added lines");
    assert!(diff.removed_lines.is_empty(), "identical: no removed lines");
    assert_eq!(diff.unchanged_lines.len(), 3, "all 3 lines unchanged");
    assert!(
        (diff.similarity_pct - 1.0).abs() < 1e-9,
        "identical versions: similarity must be 1.0, got {}",
        diff.similarity_pct
    );
}

/// diff against a non-existent version returns NotFound error.
#[tokio::test]
async fn prompt_version_diff_not_found_returns_error() {
    let (_store, svc) = store_and_svc();

    let result = svc
        .diff(
            &PromptVersionId::new("ghost_a"),
            &PromptVersionId::new("ghost_b"),
        )
        .await;

    assert!(
        result.is_err(),
        "diff of nonexistent versions must fail"
    );
    match result.unwrap_err() {
        cairn_runtime::RuntimeError::NotFound { .. } => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}

/// Similarity formula: 1 - (added+removed)/(2*max_lines).
#[tokio::test]
async fn prompt_version_diff_similarity_formula() {
    let (_store, svc) = store_and_svc();
    let asset_id = PromptAssetId::new("asset_formula");

    // ver_x: 4 lines, ver_y: 4 lines, 2 shared, 2 added, 2 removed
    // similarity = 1 - (2+2)/(2*4) = 1 - 4/8 = 0.5
    svc.create(
        &workspace(),
        PromptVersionId::new("fx"),
        asset_id.clone(),
        "hx".to_owned(),
        Some("shared_1\nshared_2\nremoved_1\nremoved_2".to_owned()),
        None,
    )
    .await
    .unwrap();

    svc.create(
        &workspace(),
        PromptVersionId::new("fy"),
        asset_id.clone(),
        "hy".to_owned(),
        Some("shared_1\nshared_2\nadded_1\nadded_2".to_owned()),
        None,
    )
    .await
    .unwrap();

    let diff = svc
        .diff(&PromptVersionId::new("fx"), &PromptVersionId::new("fy"))
        .await
        .unwrap();

    assert_eq!(diff.added_lines.len(), 2);
    assert_eq!(diff.removed_lines.len(), 2);
    assert_eq!(diff.unchanged_lines.len(), 2);

    let expected_similarity = 1.0 - (2.0 + 2.0) / (2.0 * 4.0); // = 0.5
    assert!(
        (diff.similarity_pct - expected_similarity).abs() < 1e-9,
        "expected similarity {expected_similarity}, got {}",
        diff.similarity_pct
    );
}
