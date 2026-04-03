//! Integration test proving FeedStore from cairn-memory wires correctly
//! through the cairn-api FeedEndpoints trait boundary.

use cairn_api::feed::{FeedEndpoints, FeedItem, FeedQuery};
use cairn_domain::tenancy::ProjectKey;
use cairn_memory::feed_impl::FeedStore;

fn project() -> ProjectKey {
    ProjectKey::new("t", "w", "p")
}

fn make_item(id: &str, source: &str, read: bool) -> FeedItem {
    FeedItem {
        id: id.to_owned(),
        source: source.to_owned(),
        kind: Some("message".to_owned()),
        title: Some(format!("Item {id}")),
        body: Some("body".to_owned()),
        url: None,
        author: None,
        avatar_url: None,
        repo_full_name: None,
        is_read: read,
        is_archived: false,
        group_key: None,
        created_at: "2026-04-03T09:30:00Z".to_owned(),
    }
}

#[tokio::test]
async fn list_feed_items() {
    let store = FeedStore::new();
    store.push_item(make_item("1", "slack", false));
    store.push_item(make_item("2", "rss", false));
    store.push_item(make_item("3", "slack", true));

    let result = store.list(&project(), &FeedQuery::default()).await.unwrap();
    assert_eq!(result.items.len(), 3);
}

#[tokio::test]
async fn mark_read_and_read_all() {
    let store = FeedStore::new();
    store.push_item(make_item("1", "slack", false));
    store.push_item(make_item("2", "rss", false));

    store.mark_read("1").await.unwrap();

    let result = store.list(&project(), &FeedQuery::default()).await.unwrap();
    let item1 = result.items.iter().find(|i| i.id == "1").unwrap();
    assert!(item1.is_read);

    let changed = store.read_all(&project()).await.unwrap();
    assert!(changed >= 1);
}

#[tokio::test]
async fn feed_update_sse_from_feed_item() {
    let item = make_item("101", "slack", false);

    let frame = cairn_api::sse_payloads::build_feed_update_frame(item, None);
    assert_eq!(frame.event, cairn_api::sse::SseEventName::FeedUpdate);
    assert_eq!(frame.data["item"]["id"], "101");
    assert_eq!(frame.data["item"]["source"], "slack");
    assert_eq!(frame.data["item"]["isRead"], false);
    assert_eq!(frame.data["item"]["kind"], "message");
}
