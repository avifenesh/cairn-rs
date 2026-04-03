//! Implementation of Worker 8's FeedEndpoints backed by an in-memory feed store.
//!
//! Signal pollers push FeedItems into this store via `push_item()`.
//! The API layer reads through the `FeedEndpoints` trait.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Mutex;

use cairn_api::feed::{FeedEndpoints, FeedItem, FeedQuery};
use cairn_api::http::ListResponse;
use cairn_domain::ProjectKey;

/// In-memory feed store that signal pollers can push into.
///
/// Implements Worker 8's `FeedEndpoints` trait for API consumption.
pub struct FeedStore {
    items: Mutex<HashMap<String, FeedItem>>,
    /// Items ordered by created_at descending for listing.
    order: Mutex<Vec<String>>,
}

impl FeedStore {
    pub fn new() -> Self {
        Self {
            items: Mutex::new(HashMap::new()),
            order: Mutex::new(Vec::new()),
        }
    }

    /// Push a new feed item (called by signal pollers after polling).
    pub fn push_item(&self, item: FeedItem) {
        let id = item.id.clone();
        self.items.lock().unwrap().insert(id.clone(), item);
        self.order.lock().unwrap().insert(0, id);
    }

    /// Get the total count of items.
    pub fn count(&self) -> usize {
        self.items.lock().unwrap().len()
    }
}

impl Default for FeedStore {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl FeedEndpoints for FeedStore {
    type Error = String;

    async fn list(
        &self,
        _project: &ProjectKey,
        query: &FeedQuery,
    ) -> Result<ListResponse<FeedItem>, Self::Error> {
        let items = self.items.lock().unwrap();
        let order = self.order.lock().unwrap();
        let limit = query.effective_limit();

        let mut results: Vec<FeedItem> = Vec::new();
        let mut skipping = query.before.is_some();

        for id in order.iter() {
            if skipping {
                if Some(id.as_str()) == query.before.as_deref() {
                    skipping = false;
                }
                continue;
            }

            if let Some(item) = items.get(id) {
                // Apply filters.
                if let Some(ref source_filter) = query.source {
                    if item.source != *source_filter {
                        continue;
                    }
                }
                if query.unread == Some(true) && item.is_read {
                    continue;
                }

                results.push(item.clone());
                if results.len() >= limit {
                    break;
                }
            }
        }

        let has_more = results.len() >= limit;

        Ok(ListResponse {
            items: results,
            has_more,
        })
    }

    async fn mark_read(&self, item_id: &str) -> Result<(), Self::Error> {
        let mut items = self.items.lock().unwrap();
        if let Some(item) = items.get_mut(item_id) {
            item.is_read = true;
            Ok(())
        } else {
            Err(format!("feed item not found: {item_id}"))
        }
    }

    async fn read_all(&self, _project: &ProjectKey) -> Result<u32, Self::Error> {
        let mut items = self.items.lock().unwrap();
        let mut count = 0u32;
        for item in items.values_mut() {
            if !item.is_read {
                item.is_read = true;
                count += 1;
            }
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(id: &str, source: &str, read: bool) -> FeedItem {
        FeedItem {
            id: id.to_owned(),
            source: source.to_owned(),
            kind: None,
            title: Some(format!("Item {id}")),
            body: None,
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
    async fn feed_list_returns_items_in_push_order() {
        let store = FeedStore::new();
        store.push_item(make_item("f1", "rss", false));
        store.push_item(make_item("f2", "api", false));
        store.push_item(make_item("f3", "rss", false));

        let project = ProjectKey::new("t", "w", "p");
        let result = store.list(&project, &FeedQuery::default()).await.unwrap();

        assert_eq!(result.items.len(), 3);
        // Most recent first (push_item inserts at front).
        assert_eq!(result.items[0].id, "f3");
        assert_eq!(result.items[2].id, "f1");
    }

    #[tokio::test]
    async fn feed_filters_by_source() {
        let store = FeedStore::new();
        store.push_item(make_item("f1", "rss", false));
        store.push_item(make_item("f2", "api", false));

        let project = ProjectKey::new("t", "w", "p");
        let result = store
            .list(
                &project,
                &FeedQuery {
                    source: Some("rss".to_owned()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].source, "rss");
    }

    #[tokio::test]
    async fn feed_filters_unread() {
        let store = FeedStore::new();
        store.push_item(make_item("f1", "rss", true));
        store.push_item(make_item("f2", "rss", false));

        let project = ProjectKey::new("t", "w", "p");
        let result = store
            .list(
                &project,
                &FeedQuery {
                    unread: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].id, "f2");
    }

    #[tokio::test]
    async fn mark_read_and_read_all() {
        let store = FeedStore::new();
        store.push_item(make_item("f1", "rss", false));
        store.push_item(make_item("f2", "rss", false));
        store.push_item(make_item("f3", "rss", false));

        store.mark_read("f1").await.unwrap();

        let project = ProjectKey::new("t", "w", "p");
        let count = store.read_all(&project).await.unwrap();
        assert_eq!(count, 2); // f2 and f3 were unread.

        // All should be read now.
        let result = store
            .list(
                &project,
                &FeedQuery {
                    unread: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        assert!(result.items.is_empty());
    }
}
