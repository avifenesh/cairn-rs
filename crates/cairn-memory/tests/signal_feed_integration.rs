//! Signal-to-feed integration proof.
//!
//! Demonstrates the signal polling -> feed endpoint pipeline:
//! a SourcePoller produces poll results, which are converted into
//! FeedItems and pushed into FeedStore, which serves them through
//! the FeedEndpoints trait.

use cairn_api::feed::{FeedEndpoints, FeedItem, FeedQuery};
use cairn_domain::{ProjectKey, SourceId};
use cairn_memory::feed_impl::FeedStore;
use cairn_signal::pollers::{PollResult, SignalSource, SourceKind, SourcePoller};

/// Mock poller that returns a fixed number of new items.
struct MockPoller {
    items_per_poll: u32,
}

impl SourcePoller for MockPoller {
    type Error = String;

    fn poll(&self, source: &SignalSource, cursor: Option<&str>) -> Result<PollResult, Self::Error> {
        let next_cursor = cursor
            .map(|c| {
                let n: u32 = c.parse().unwrap_or(0);
                (n + self.items_per_poll).to_string()
            })
            .unwrap_or_else(|| self.items_per_poll.to_string());

        Ok(PollResult {
            source_id: source.source_id.clone(),
            new_items: self.items_per_poll,
            cursor: Some(next_cursor),
        })
    }
}

/// Convert poll results into feed items and push to store.
fn process_poll_result(
    feed: &FeedStore,
    source: &SignalSource,
    result: &PollResult,
    base_time: u64,
) {
    for i in 0..result.new_items {
        let item_id = format!("{}_{}", source.source_id.as_str(), i);
        feed.push_item(FeedItem {
            id: item_id,
            source: source.name.clone(),
            kind: None,
            title: Some(format!("Item {} from {}", i, source.name)),
            body: None,
            url: None,
            author: None,
            avatar_url: None,
            repo_full_name: None,
            is_read: false,
            is_archived: false,
            group_key: None,
            created_at: format!("2026-04-03T09:{}:00Z", base_time + i as u64),
        });
    }
}

#[tokio::test]
async fn signal_poll_flows_through_to_feed_endpoint() {
    let feed = FeedStore::new();
    let project = ProjectKey::new("acme", "eng", "support");

    // Set up signal sources.
    let rss_source = SignalSource {
        source_id: SourceId::new("tech_news"),
        project: project.clone(),
        kind: SourceKind::Rss,
        name: "Tech News RSS".to_owned(),
    };

    let api_source = SignalSource {
        source_id: SourceId::new("status_api"),
        project: project.clone(),
        kind: SourceKind::Api,
        name: "Status API".to_owned(),
    };

    // Poll sources.
    let poller = MockPoller { items_per_poll: 3 };

    let rss_result = poller.poll(&rss_source, None).unwrap();
    assert_eq!(rss_result.new_items, 3);
    process_poll_result(&feed, &rss_source, &rss_result, 1000);

    let api_result = poller.poll(&api_source, None).unwrap();
    process_poll_result(&feed, &api_source, &api_result, 2000);

    // Verify feed shows all items.
    let all = feed.list(&project, &FeedQuery::default()).await.unwrap();
    assert_eq!(all.items.len(), 6); // 3 from RSS + 3 from API

    // Filter by source.
    let rss_only = feed
        .list(
            &project,
            &FeedQuery {
                source: Some("Tech News RSS".to_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(rss_only.items.len(), 3);
    assert!(rss_only.items.iter().all(|i| i.source == "Tech News RSS"));

    // Mark one read, verify unread filter.
    feed.mark_read(&all.items[0].id).await.unwrap();
    let unread = feed
        .list(
            &project,
            &FeedQuery {
                unread: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(unread.items.len(), 5);

    // Second poll with cursor.
    let rss_result2 = poller
        .poll(&rss_source, rss_result.cursor.as_deref())
        .unwrap();
    assert_eq!(rss_result2.new_items, 3);
    assert_eq!(rss_result2.cursor.as_deref(), Some("6")); // 3 + 3

    process_poll_result(&feed, &rss_source, &rss_result2, 3000);

    let all_after = feed.list(&project, &FeedQuery::default()).await.unwrap();
    assert_eq!(all_after.items.len(), 9); // 6 + 3 more
}
