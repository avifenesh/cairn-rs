//! Notion integration plugin for Cairn.
//!
//! Built-in provider type `"notion"` — receives Notion webhooks and triggers
//! agent orchestration for database items (pages, tasks, issues).
//!
//! Notion webhooks (2025+):
//! - Signature: HMAC-SHA256 in `X-Notion-Signature` header
//! - Payload: `{ "type": "page.created" | "page.updated" | ..., "data": { "page": {...} } }`
//! - Events: page.created, page.content_updated, page.property_updated, page.deleted,
//!   database.created, database.updated, comment.created, comment.updated
//!
//! Notion API:
//! - REST at `https://api.notion.com/v1/`
//! - Auth: `Authorization: Bearer {api_key}` + `Notion-Version: 2022-06-28`
//! - Key endpoints: /pages/{id}, /databases/{id}/query, /blocks/{id}/children

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::{
    EventAction, EventActionMapping, Integration, IntegrationError, IntegrationEvent, QueueStats,
    WorkItem, WorkItemStatus,
};

/// Notion-specific configuration.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct NotionConfig {
    /// Notion Internal Integration Token (starts with `ntn_` or `secret_`).
    pub api_key: String,
    /// Webhook signing secret from Notion developer settings.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Database ID to watch (optional — filters events to this database).
    #[serde(default)]
    pub database_filter: Option<String>,
    /// Notion API version header (default: "2022-06-28").
    #[serde(default = "default_notion_version")]
    pub notion_version: String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
}

fn default_notion_version() -> String {
    "2022-06-28".into()
}

fn default_max_concurrent() -> u32 {
    3
}

/// Notion integration plugin.
pub struct NotionPlugin {
    api_key: String,
    webhook_secret: Option<String>,
    database_filter: Option<String>,
    notion_version: String,
    pub queue: RwLock<VecDeque<WorkItem>>,
    pub queue_paused: AtomicBool,
    pub queue_running: AtomicBool,
    pub max_concurrent: AtomicU32,
    pub run_semaphore: Arc<Semaphore>,
    http: reqwest::Client,
}

impl NotionPlugin {
    pub fn new(config: NotionConfig) -> Self {
        let max = config.max_concurrent;
        Self {
            api_key: config.api_key,
            webhook_secret: config.webhook_secret,
            database_filter: config.database_filter,
            notion_version: config.notion_version,
            queue: RwLock::new(VecDeque::new()),
            queue_paused: AtomicBool::new(false),
            queue_running: AtomicBool::new(false),
            max_concurrent: AtomicU32::new(max),
            run_semaphore: Arc::new(Semaphore::new(max as usize)),
            http: reqwest::Client::new(),
        }
    }

    /// Fetch a page from the Notion API.
    pub async fn get_page(&self, page_id: &str) -> Result<serde_json::Value, IntegrationError> {
        let url = format!("https://api.notion.com/v1/pages/{page_id}");
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Notion-Version", &self.notion_version)
            .send()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("Notion API error: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IntegrationError::ApiError(format!(
                "Notion API {status}: {body}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("Notion parse error: {e}")))
    }

    /// Query a database for pages.
    pub async fn query_database(
        &self,
        database_id: &str,
        filter: Option<serde_json::Value>,
    ) -> Result<Vec<serde_json::Value>, IntegrationError> {
        let url = format!("https://api.notion.com/v1/databases/{database_id}/query");
        let mut req = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Notion-Version", &self.notion_version);
        if let Some(f) = filter {
            req = req.json(&serde_json::json!({ "filter": f }));
        } else {
            req = req.json(&serde_json::json!({}));
        }
        let resp = req
            .send()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("Notion query error: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IntegrationError::ApiError(format!(
                "Notion query {status}: {body}"
            )));
        }
        let data: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("Notion parse error: {e}")))?;
        Ok(data["results"].as_array().cloned().unwrap_or_default())
    }

    /// Extract the title from a Notion page's properties.
    fn extract_title(page: &serde_json::Value) -> Option<String> {
        // Notion pages store title in properties.Name.title[0].plain_text
        // or properties.Title.title[0].plain_text (depends on the database schema).
        let props = page.get("properties")?;
        for (_key, value) in props.as_object()? {
            if let Some(title_arr) = value.get("title")
                && let Some(first) = title_arr.as_array()?.first()
            {
                return first.get("plain_text")?.as_str().map(|s| s.to_owned());
            }
        }
        None
    }
}

impl std::fmt::Debug for NotionPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NotionPlugin")
            .field("has_api_key", &!self.api_key.is_empty())
            .field("database_filter", &self.database_filter)
            .finish()
    }
}

const DEFAULT_NOTION_AGENT_PROMPT: &str = "\
You are an autonomous agent processing tasks from a Notion workspace. \
Read the task carefully, use available tools to research and implement \
a solution, and report your results. For code tasks, write real code \
and open a pull request. For research tasks, compile findings with sources.";

#[async_trait]
impl Integration for NotionPlugin {
    fn id(&self) -> &str {
        "notion"
    }

    fn display_name(&self) -> &str {
        "Notion"
    }

    fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    fn default_agent_prompt(&self) -> &str {
        DEFAULT_NOTION_AGENT_PROMPT
    }

    fn default_event_actions(&self) -> Vec<EventActionMapping> {
        vec![
            EventActionMapping {
                event_pattern: "page.created".into(),
                label_filter: None,
                repo_filter: None,
                action: EventAction::CreateAndOrchestrate,
            },
            EventActionMapping {
                event_pattern: "page.content_updated".into(),
                label_filter: None,
                repo_filter: None,
                action: EventAction::Acknowledge,
            },
        ]
    }

    async fn verify_webhook(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<(), IntegrationError> {
        let Some(ref secret) = self.webhook_secret else {
            return Ok(());
        };
        if secret.is_empty() {
            return Ok(());
        }

        let sig = headers
            .get("X-Notion-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                IntegrationError::VerificationFailed("missing X-Notion-Signature header".into())
            })?;

        let sig_with_prefix = if sig.starts_with("sha256=") {
            sig.to_owned()
        } else {
            format!("sha256={sig}")
        };

        cairn_github::verify_signature(&sig_with_prefix, secret.as_bytes(), body).map_err(|e| {
            IntegrationError::VerificationFailed(format!(
                "Notion signature verification failed: {e}"
            ))
        })
    }

    async fn parse_event(
        &self,
        _headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<IntegrationEvent, IntegrationError> {
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| IntegrationError::ParseError(format!("invalid JSON: {e}")))?;

        // Notion webhook format: { "type": "page.created", "data": { "page": {...} } }
        let event_type = raw["type"].as_str().unwrap_or("unknown").to_owned();

        let page = &raw["data"]["page"];
        let page_id = page["id"].as_str().unwrap_or("").to_owned();

        let title = Self::extract_title(page);

        // Check database filter if configured.
        if let Some(ref db_filter) = self.database_filter {
            let parent_db = page
                .get("parent")
                .and_then(|p| p.get("database_id"))
                .and_then(|d| d.as_str());
            if let Some(parent) = parent_db.filter(|p| *p != db_filter.as_str()) {
                return Err(IntegrationError::Other(format!(
                    "page from database {parent} filtered (expected {db_filter})"
                )));
            }
        }

        Ok(IntegrationEvent {
            integration_id: "notion".into(),
            event_key: event_type,
            source_id: page_id,
            repository: self.database_filter.clone(),
            title,
            body: None, // Notion page content requires a separate API call
            labels: vec![],
            raw,
        })
    }

    async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError> {
        Ok(format!(
            "## Task from Notion\n\
             **{title}**\n\n\
             {body}\n\n\
             ## Workflow\n\
             1. Read the task and understand what is being asked.\n\
             2. Use available tools to research, explore code, or gather information.\n\
             3. Implement the solution — write real code if needed.\n\
             4. Verify your work — run tests or check results.\n\
             5. Complete with a summary of what you accomplished.",
            title = item.title,
            body = if item.body.is_empty() {
                "(No additional details provided)"
            } else {
                &item.body
            },
        ))
    }

    async fn prepare_tool_registry(
        &self,
        base: &cairn_tools::BuiltinToolRegistry,
        _item: &WorkItem,
    ) -> Arc<cairn_tools::BuiltinToolRegistry> {
        // Notion plugin uses system tools. Could add Notion API tools later
        // (read page, update status, post comment).
        Arc::new(cairn_tools::BuiltinToolRegistry::from_existing(base))
    }

    fn auth_exempt_paths(&self) -> Vec<String> {
        vec!["/v1/webhooks/notion".into()]
    }

    async fn queue_stats(&self) -> QueueStats {
        let queue = self.queue.read().await;
        let mut stats = QueueStats::default();
        for item in queue.iter() {
            match &item.status {
                WorkItemStatus::Pending => stats.pending += 1,
                WorkItemStatus::Processing => stats.processing += 1,
                WorkItemStatus::WaitingApproval => stats.waiting_approval += 1,
                WorkItemStatus::Completed => stats.completed += 1,
                WorkItemStatus::Failed(_) => stats.failed += 1,
                WorkItemStatus::Skipped => {}
            }
        }
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_plugin() -> NotionPlugin {
        NotionPlugin::new(NotionConfig {
            api_key: "ntn_test_key".into(),
            webhook_secret: Some("notion-secret".into()),
            database_filter: Some("db-123".into()),
            notion_version: "2022-06-28".into(),
            max_concurrent: 2,
        })
    }

    #[test]
    fn plugin_identity() {
        let p = make_plugin();
        assert_eq!(p.id(), "notion");
        assert_eq!(p.display_name(), "Notion");
        assert!(p.is_configured());
    }

    #[test]
    fn unconfigured_when_empty_key() {
        let p = NotionPlugin::new(NotionConfig {
            api_key: String::new(),
            webhook_secret: None,
            database_filter: None,
            notion_version: "2022-06-28".into(),
            max_concurrent: 1,
        });
        assert!(!p.is_configured());
    }

    #[test]
    fn auth_exempt_includes_webhook() {
        let p = make_plugin();
        assert!(
            p.auth_exempt_paths()
                .contains(&"/v1/webhooks/notion".to_owned())
        );
    }

    #[test]
    fn extract_title_from_notion_page() {
        let page = serde_json::json!({
            "properties": {
                "Name": {
                    "title": [{"plain_text": "Fix login bug"}]
                }
            }
        });
        assert_eq!(
            NotionPlugin::extract_title(&page).as_deref(),
            Some("Fix login bug")
        );
    }

    #[test]
    fn extract_title_missing_returns_none() {
        let page = serde_json::json!({"properties": {}});
        assert!(NotionPlugin::extract_title(&page).is_none());
    }

    #[tokio::test]
    async fn parse_event_extracts_notion_fields() {
        let p = make_plugin();
        let body = serde_json::to_vec(&serde_json::json!({
            "type": "page.created",
            "data": {
                "page": {
                    "id": "page-abc-123",
                    "parent": {"database_id": "db-123"},
                    "properties": {
                        "Name": {"title": [{"plain_text": "Implement search"}]}
                    }
                }
            }
        }))
        .unwrap();

        let event = p.parse_event(&http::HeaderMap::new(), &body).await.unwrap();
        assert_eq!(event.event_key, "page.created");
        assert_eq!(event.source_id, "page-abc-123");
        assert_eq!(event.title.as_deref(), Some("Implement search"));
    }

    #[tokio::test]
    async fn parse_event_database_filter_rejects() {
        let p = make_plugin();
        let body = serde_json::to_vec(&serde_json::json!({
            "type": "page.created",
            "data": {
                "page": {
                    "id": "page-xyz",
                    "parent": {"database_id": "wrong-db"},
                    "properties": {}
                }
            }
        }))
        .unwrap();

        let result = p.parse_event(&http::HeaderMap::new(), &body).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn verify_webhook_skips_without_secret() {
        let p = NotionPlugin::new(NotionConfig {
            api_key: "key".into(),
            webhook_secret: None,
            database_filter: None,
            notion_version: "2022-06-28".into(),
            max_concurrent: 1,
        });
        assert!(
            p.verify_webhook(&http::HeaderMap::new(), b"{}")
                .await
                .is_ok()
        );
    }
}
