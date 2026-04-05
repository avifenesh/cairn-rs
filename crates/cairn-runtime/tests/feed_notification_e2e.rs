//! Feed/notification system integration test.
//!
//! The feed is a push-based in-memory notification store (FeedStore from
//! cairn-memory) wired through the FeedEndpoints trait from cairn-api.
//! Signal pollers push FeedItems; operators read, filter, and mark them.
//!
//! Tests:
//!   (1) create (push) a feed item
//!   (2) retrieve feed items — most recent first
//!   (3) mark a single item as read
//!   (4) verify read status persists
//!   (5) mark all items read (read_all)
//!   (6) verify all items are now read (unread filter returns empty)
//!   (7) source filter returns only matching items
//!   (8) unread filter excludes already-read items
//!   (9) pagination via limit
//!  (10) mark_read on unknown ID returns an error

use cairn_api::feed::{FeedEndpoints, FeedItem, FeedQuery};
use cairn_domain::ProjectKey;
use cairn_memory::feed_impl::FeedStore;

fn project() -> ProjectKey {
    ProjectKey::new("t_feed", "ws_feed", "proj_feed")
}

fn item(id: &str, source: &str) -> FeedItem {
    FeedItem {
        id: id.to_owned(),
        source: source.to_owned(),
        kind: Some("notification".to_owned()),
        title: Some(format!("Notification {id}")),
        body: Some(format!("Body for {id}")),
        url: None,
        author: Some("system".to_owned()),
        avatar_url: None,
        repo_full_name: None,
        is_read: false,
        is_archived: false,
        group_key: None,
        created_at: format!("2026-04-05T10:0{id}:00Z"),
    }
}

// ── (1) Create (push) a feed item ────────────────────────────────────────

#[tokio::test]
async fn push_item_adds_it_to_the_store() {
    let store = FeedStore::new();
    assert_eq!(store.count(), 0, "empty store must have count 0");

    store.push_item(item("1", "slack"));
    assert_eq!(store.count(), 1, "count must be 1 after push");

    let result = store.list(&project(), &FeedQuery::default()).await.unwrap();
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0].id, "1");
    assert_eq!(result.items[0].source, "slack");
    assert!(!result.items[0].is_read, "newly pushed item must be unread");
}

// ── (2) Retrieve — most recent first ─────────────────────────────────────

#[tokio::test]
async fn list_returns_items_most_recent_first() {
    let store = FeedStore::new();
    store.push_item(item("1", "rss"));
    store.push_item(item("2", "slack"));
    store.push_item(item("3", "github"));

    let result = store.list(&project(), &FeedQuery::default()).await.unwrap();
    assert_eq!(result.items.len(), 3);
    // push_item inserts at front of order — most recently pushed is first.
    assert_eq!(result.items[0].id, "3", "most recently pushed item must be first");
    assert_eq!(result.items[1].id, "2");
    assert_eq!(result.items[2].id, "1", "earliest item must be last");
}

// ── (3) Mark a single item as read ───────────────────────────────────────

#[tokio::test]
async fn mark_read_sets_is_read_true_for_target_item() {
    let store = FeedStore::new();
    store.push_item(item("a", "slack"));
    store.push_item(item("b", "rss"));

    store.mark_read("a").await.unwrap();

    let result = store.list(&project(), &FeedQuery::default()).await.unwrap();
    let a = result.items.iter().find(|i| i.id == "a").unwrap();
    let b = result.items.iter().find(|i| i.id == "b").unwrap();

    assert!(a.is_read, "item 'a' must be read after mark_read");
    assert!(!b.is_read, "item 'b' must remain unread");
}

// ── (4) Verify read status persists ──────────────────────────────────────

#[tokio::test]
async fn read_status_persists_across_multiple_list_calls() {
    let store = FeedStore::new();
    store.push_item(item("x", "api"));
    store.push_item(item("y", "api"));

    store.mark_read("x").await.unwrap();

    // First read.
    let r1 = store.list(&project(), &FeedQuery::default()).await.unwrap();
    let x1 = r1.items.iter().find(|i| i.id == "x").unwrap();
    assert!(x1.is_read, "x must be read on first list call");

    // Second read — state must not regress.
    let r2 = store.list(&project(), &FeedQuery::default()).await.unwrap();
    let x2 = r2.items.iter().find(|i| i.id == "x").unwrap();
    assert!(x2.is_read, "x must still be read on second list call");
    let y2 = r2.items.iter().find(|i| i.id == "y").unwrap();
    assert!(!y2.is_read, "y must remain unread");
}

// ── (5)+(6) mark_all_read; verify none remain unread ─────────────────────

#[tokio::test]
async fn read_all_marks_every_unread_item_and_returns_count() {
    let store = FeedStore::new();
    store.push_item(item("1", "slack"));
    store.push_item(item("2", "rss"));
    store.push_item(item("3", "github"));

    // Mark item 1 read manually — read_all should only count the remaining 2.
    store.mark_read("1").await.unwrap();

    let changed = store.read_all(&project()).await.unwrap();
    assert_eq!(changed, 2, "read_all must return count of newly-read items (2 were unread)");

    // Verify all are read.
    let unread = store
        .list(
            &project(),
            &FeedQuery { unread: Some(true), ..FeedQuery::default() },
        )
        .await
        .unwrap();
    assert!(unread.items.is_empty(), "no items must remain unread after read_all");

    // All items must now have is_read = true.
    let all = store.list(&project(), &FeedQuery::default()).await.unwrap();
    assert!(all.items.iter().all(|i| i.is_read), "all items must be is_read=true");
}

#[tokio::test]
async fn read_all_on_already_all_read_returns_zero() {
    let store = FeedStore::new();
    store.push_item(item("1", "slack"));
    store.read_all(&project()).await.unwrap();

    // Second call — nothing left to mark.
    let changed = store.read_all(&project()).await.unwrap();
    assert_eq!(changed, 0, "read_all on fully-read store must return 0");
}

// ── (7) Source filter ─────────────────────────────────────────────────────

#[tokio::test]
async fn source_filter_returns_only_matching_items() {
    let store = FeedStore::new();
    store.push_item(item("s1", "slack"));
    store.push_item(item("g1", "github"));
    store.push_item(item("s2", "slack"));
    store.push_item(item("r1", "rss"));

    let slack_only = store
        .list(
            &project(),
            &FeedQuery { source: Some("slack".to_owned()), ..FeedQuery::default() },
        )
        .await
        .unwrap();

    assert_eq!(slack_only.items.len(), 2, "source=slack must return exactly 2 items");
    assert!(slack_only.items.iter().all(|i| i.source == "slack"));
}

// ── (8) Unread filter excludes read items ─────────────────────────────────

#[tokio::test]
async fn unread_filter_excludes_read_items() {
    let store = FeedStore::new();
    store.push_item(item("u1", "api"));
    store.push_item(item("u2", "api"));
    store.push_item(item("u3", "api"));

    store.mark_read("u2").await.unwrap();

    let unread = store
        .list(
            &project(),
            &FeedQuery { unread: Some(true), ..FeedQuery::default() },
        )
        .await
        .unwrap();

    assert_eq!(unread.items.len(), 2, "only 2 unread items must appear");
    assert!(!unread.items.iter().any(|i| i.id == "u2"), "read item u2 must be excluded");
    assert!(unread.items.iter().all(|i| !i.is_read));
}

// ── (9) Pagination via limit ──────────────────────────────────────────────

#[tokio::test]
async fn limit_restricts_number_of_returned_items() {
    let store = FeedStore::new();
    for i in 0..10u32 {
        store.push_item(item(&i.to_string(), "api"));
    }

    let page = store
        .list(
            &project(),
            &FeedQuery { limit: Some(3), ..FeedQuery::default() },
        )
        .await
        .unwrap();

    assert_eq!(page.items.len(), 3, "limit=3 must return exactly 3 items");
    assert!(page.has_more, "has_more must be true when results were capped by limit");
}

// ── (10) mark_read on unknown ID returns error ────────────────────────────

#[tokio::test]
async fn mark_read_on_unknown_id_returns_error() {
    let store = FeedStore::new();
    store.push_item(item("known", "api"));

    let result = store.mark_read("does_not_exist").await;
    assert!(result.is_err(), "mark_read on unknown ID must return an error");
}
