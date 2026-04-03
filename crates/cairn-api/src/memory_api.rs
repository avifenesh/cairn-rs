//! Memory endpoint boundaries per preserved route catalog.
//!
//! Covers: GET /v1/memories, GET /v1/memories/search,
//! POST /v1/memories, POST /v1/memories/:id/accept, POST /v1/memories/:id/reject

use async_trait::async_trait;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::endpoints::ListQuery;
use crate::http::ListResponse;

/// Memory item as consumed by the frontend.
///
/// Fields match the preserved Phase 0 fixture:
/// `GET__v1_memories_search__q_test_limit_10.json`
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryItem {
    pub id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub status: MemoryStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub created_at: String,
}

/// Memory lifecycle status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    Proposed,
    Accepted,
    Rejected,
}

/// Request body for POST /v1/memories.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateMemoryRequest {
    pub content: String,
    pub category: Option<String>,
}

/// Query parameters for GET /v1/memories/search.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemorySearchQuery {
    pub q: String,
    pub limit: Option<usize>,
}

impl MemorySearchQuery {
    pub fn effective_limit(&self) -> usize {
        self.limit.unwrap_or(10).min(50)
    }
}

/// Memory endpoint boundaries.
#[async_trait]
pub trait MemoryEndpoints: Send + Sync {
    type Error;

    /// `GET /v1/memories` — list memories with optional status/category filter.
    async fn list(
        &self,
        project: &ProjectKey,
        query: &ListQuery,
    ) -> Result<ListResponse<MemoryItem>, Self::Error>;

    /// `GET /v1/memories/search` — search memories by query string.
    async fn search(
        &self,
        project: &ProjectKey,
        query: &MemorySearchQuery,
    ) -> Result<Vec<MemoryItem>, Self::Error>;

    /// `POST /v1/memories` — create a new memory.
    async fn create(
        &self,
        project: &ProjectKey,
        request: &CreateMemoryRequest,
    ) -> Result<MemoryItem, Self::Error>;

    /// `POST /v1/memories/:id/accept` — accept a proposed memory.
    async fn accept(&self, memory_id: &str) -> Result<(), Self::Error>;

    /// `POST /v1/memories/:id/reject` — reject a proposed memory.
    async fn reject(&self, memory_id: &str) -> Result<(), Self::Error>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_item_matches_fixture_shape() {
        let item = MemoryItem {
            id: "memory_001".to_owned(),
            content: "The weekly digest should summarize blocked deploys first.".to_owned(),
            category: Some("project".to_owned()),
            status: MemoryStatus::Accepted,
            source: Some("ops-notes".to_owned()),
            confidence: Some(0.92),
            created_at: "2026-04-02T15:00:00Z".to_owned(),
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "memory_001");
        assert_eq!(json["category"], "project");
        assert_eq!(json["status"], "accepted");
        assert_eq!(json["source"], "ops-notes");
        assert_eq!(json["confidence"], 0.92);
        assert_eq!(json["createdAt"], "2026-04-02T15:00:00Z");
    }

    #[test]
    fn search_query_defaults() {
        let query = MemorySearchQuery {
            q: "test".to_owned(),
            limit: None,
        };
        assert_eq!(query.effective_limit(), 10);
    }

    #[test]
    fn create_request_serialization() {
        let req = CreateMemoryRequest {
            content: "important fact".to_owned(),
            category: Some("facts".to_owned()),
        };
        let json = serde_json::to_value(&req).unwrap();
        assert_eq!(json["content"], "important fact");
    }
}
