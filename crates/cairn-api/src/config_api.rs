//! Config API — runtime key-value configuration endpoints (GAP-008).
//!
//! Exposes `ConfigStore` over HTTP as a flat key-value namespace.
//! Keys use dot-separated segments (e.g. `agent.model`, `server.port`).
//!
//! # Routes
//! - `GET    /v1/config/:key`        — get a single value
//! - `PUT    /v1/config/:key`        — set a value (body: `{ "value": "..." }`)
//! - `DELETE /v1/config/:key`        — delete a key
//! - `GET    /v1/config?prefix=`     — list all keys (optionally filtered by prefix)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ── Request / Response types ───────────────────────────────────────────────

/// Response body for `GET /v1/config/:key`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigGetResponse {
    pub key: String,
    pub value: Option<String>,
}

/// Request body for `PUT /v1/config/:key`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigSetRequest {
    pub value: String,
}

/// Response body for `PUT /v1/config/:key`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigSetResponse {
    pub key: String,
    pub value: String,
}

/// Response body for `DELETE /v1/config/:key`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigDeleteResponse {
    pub key: String,
    pub deleted: bool,
}

/// One entry in the config listing.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
}

/// Request query params for `GET /v1/config`.
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct ConfigListQuery {
    /// Only return keys that start with this prefix. Empty or absent = all keys.
    #[serde(default)]
    pub prefix: String,
}

/// Response body for `GET /v1/config?prefix=`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConfigListResponse {
    pub prefix: String,
    pub entries: Vec<ConfigEntry>,
    pub count: usize,
}

// ── ConfigEndpoints trait ──────────────────────────────────────────────────

/// HTTP endpoint contract for the config API.
///
/// Implementors wire these to actual HTTP handlers against a `ConfigStore`.
#[async_trait]
pub trait ConfigEndpoints: Send + Sync {
    type Error;

    /// `GET /v1/config/:key`
    async fn get_config(&self, key: &str) -> Result<ConfigGetResponse, Self::Error>;

    /// `PUT /v1/config/:key`
    async fn set_config(
        &self,
        key: &str,
        req: ConfigSetRequest,
    ) -> Result<ConfigSetResponse, Self::Error>;

    /// `DELETE /v1/config/:key`
    async fn delete_config(&self, key: &str) -> Result<ConfigDeleteResponse, Self::Error>;

    /// `GET /v1/config?prefix=`
    async fn list_config(
        &self,
        query: ConfigListQuery,
    ) -> Result<ConfigListResponse, Self::Error>;
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_entry_equality() {
        let a = ConfigEntry { key: "agent.model".to_owned(), value: "sonnet".to_owned() };
        let b = ConfigEntry { key: "agent.model".to_owned(), value: "sonnet".to_owned() };
        assert_eq!(a, b);
    }

    #[test]
    fn config_list_query_default_empty_prefix() {
        let q = ConfigListQuery::default();
        assert_eq!(q.prefix, "");
    }

    #[test]
    fn config_set_request_round_trips() {
        let req = ConfigSetRequest { value: "haiku".to_owned() };
        let json = serde_json::to_string(&req).unwrap();
        let parsed: ConfigSetRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.value, "haiku");
    }

    #[test]
    fn config_get_response_none_value() {
        let resp = ConfigGetResponse { key: "missing".to_owned(), value: None };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["value"].is_null());
    }

    #[test]
    fn config_list_response_count_matches_entries() {
        let resp = ConfigListResponse {
            prefix: "agent.".to_owned(),
            entries: vec![
                ConfigEntry { key: "agent.model".to_owned(), value: "sonnet".to_owned() },
                ConfigEntry { key: "agent.provider".to_owned(), value: "anthropic".to_owned() },
            ],
            count: 2,
        };
        assert_eq!(resp.count, resp.entries.len());
    }
}
