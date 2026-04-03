//! Feed endpoint boundaries per preserved route catalog.
//!
//! Covers: GET /v1/feed, POST /v1/feed/:id/read, POST /v1/feed/read-all

use async_trait::async_trait;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::http::ListResponse;

/// Feed item as consumed by the frontend.
///
/// Field names match the preserved Phase 0 fixture exactly:
/// `GET__v1_feed__limit20_unread_true.json`
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedItem {
    pub id: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_full_name: Option<String>,
    pub is_read: bool,
    #[serde(default)]
    pub is_archived: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_key: Option<String>,
    pub created_at: String,
}

/// Query parameters for GET /v1/feed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedQuery {
    pub limit: Option<usize>,
    pub before: Option<String>,
    pub source: Option<String>,
    pub unread: Option<bool>,
}

impl FeedQuery {
    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(20).min(100)
    }
}

/// Feed endpoint boundaries.
#[async_trait]
pub trait FeedEndpoints: Send + Sync {
    type Error;

    /// `GET /v1/feed` — list feed items with optional filters.
    async fn list(
        &self,
        project: &ProjectKey,
        query: &FeedQuery,
    ) -> Result<ListResponse<FeedItem>, Self::Error>;

    /// `POST /v1/feed/:id/read` — mark a single item as read.
    async fn mark_read(&self, item_id: &str) -> Result<(), Self::Error>;

    /// `POST /v1/feed/read-all` — mark all items as read.
    async fn read_all(&self, project: &ProjectKey) -> Result<u32, Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_item_matches_fixture_shape() {
        let item = FeedItem {
            id: "101".to_owned(),
            source: "slack".to_owned(),
            kind: Some("message".to_owned()),
            title: Some("Build pipeline needs approval".to_owned()),
            body: Some("Deploy is waiting on approval from ops.".to_owned()),
            url: Some("https://example.test/slack/101".to_owned()),
            author: Some("ops-bot".to_owned()),
            avatar_url: Some("https://example.test/avatar/ops-bot.png".to_owned()),
            repo_full_name: Some("avife/cairn".to_owned()),
            is_read: false,
            is_archived: false,
            group_key: Some("slack:deploy".to_owned()),
            created_at: "2026-04-03T09:30:00Z".to_owned(),
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "101");
        assert_eq!(json["source"], "slack");
        assert_eq!(json["kind"], "message");
        assert_eq!(json["isRead"], false);
        assert_eq!(json["isArchived"], false);
        assert_eq!(json["groupKey"], "slack:deploy");
        assert_eq!(json["avatarUrl"], "https://example.test/avatar/ops-bot.png");
        assert_eq!(json["repoFullName"], "avife/cairn");
        assert_eq!(json["createdAt"], "2026-04-03T09:30:00Z");
    }

    #[test]
    fn feed_query_defaults() {
        let query = FeedQuery::default();
        assert_eq!(query.effective_limit(), 20);
    }

    #[test]
    fn feed_query_clamps() {
        let query = FeedQuery {
            limit: Some(500),
            ..Default::default()
        };
        assert_eq!(query.effective_limit(), 100);
    }
}
