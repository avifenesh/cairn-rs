//! Obsidian vault integration plugin for Cairn.
//!
//! Connects to an Obsidian vault via the Local REST API plugin
//! (https://github.com/coddingtonbear/obsidian-local-rest-api).
//!
//! Unlike webhook-based integrations, Obsidian uses a poll/scan model:
//! - Operator triggers a scan of notes matching a pattern (e.g. tag, folder)
//! - Each matching note becomes a work item for agent orchestration
//! - The agent reads the note, works on the task, and appends results
//!
//! This tests a non-webhook integration pattern — API-driven discovery
//! instead of event-driven.
//!
//! Obsidian Local REST API endpoints:
//! - `GET /vault/{path}` — read a note
//! - `PUT /vault/{path}` — write/update a note
//! - `POST /vault/{path}` — append to a note
//! - `GET /search/simple/?query=...` — search notes
//! - `GET /vault/` — list all files
//!
//! Auth: `Authorization: Bearer {api_key}` header.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::{
    EventAction, EventActionMapping, Integration, IntegrationError, IntegrationEvent, QueueStats,
    WorkItem, WorkItemStatus,
};

/// Obsidian-specific configuration.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct ObsidianConfig {
    /// Base URL of the Obsidian Local REST API (e.g. "https://localhost:27124").
    pub base_url: String,
    /// API key for the Local REST API plugin.
    pub api_key: String,
    /// Folder to scan for task notes (e.g. "Tasks" or "Inbox").
    #[serde(default = "default_scan_folder")]
    pub scan_folder: String,
    /// Tag filter — only process notes with this tag (e.g. "#cairn" or "#todo").
    #[serde(default)]
    pub tag_filter: Option<String>,
    /// Folder where the agent writes results (e.g. "Agent Output").
    #[serde(default = "default_output_folder")]
    pub output_folder: String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
}

fn default_scan_folder() -> String {
    "Tasks".into()
}

fn default_output_folder() -> String {
    "Agent Output".into()
}

fn default_max_concurrent() -> u32 {
    2
}

/// Obsidian vault integration plugin.
pub struct ObsidianPlugin {
    base_url: String,
    api_key: String,
    scan_folder: String,
    #[allow(dead_code)]
    tag_filter: Option<String>,
    output_folder: String,
    pub queue: RwLock<VecDeque<WorkItem>>,
    pub queue_paused: AtomicBool,
    pub queue_running: AtomicBool,
    pub max_concurrent: AtomicU32,
    pub run_semaphore: Arc<Semaphore>,
    http: reqwest::Client,
}

impl ObsidianPlugin {
    pub fn new(config: ObsidianConfig) -> Self {
        let max = config.max_concurrent;
        // Accept self-signed certs for local Obsidian API.
        let http = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            api_key: config.api_key,
            scan_folder: config.scan_folder,
            tag_filter: config.tag_filter,
            output_folder: config.output_folder,
            queue: RwLock::new(VecDeque::new()),
            queue_paused: AtomicBool::new(false),
            queue_running: AtomicBool::new(false),
            max_concurrent: AtomicU32::new(max),
            run_semaphore: Arc::new(Semaphore::new(max as usize)),
            http,
        }
    }

    /// Read a note from the vault.
    pub async fn read_note(&self, path: &str) -> Result<String, IntegrationError> {
        let url = format!("{}/vault/{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "text/markdown")
            .send()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("Obsidian API error: {e}")))?;
        if !resp.status().is_success() {
            return Err(IntegrationError::ApiError(format!(
                "Obsidian API returned {}",
                resp.status()
            )));
        }
        resp.text()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("failed to read response: {e}")))
    }

    /// Search notes in the vault.
    pub async fn search_notes(&self, query: &str) -> Result<Vec<SearchResult>, IntegrationError> {
        let url = format!("{}/search/simple/?query={}", self.base_url, query);
        let resp = self
            .http
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("Obsidian search error: {e}")))?;
        if !resp.status().is_success() {
            return Err(IntegrationError::ApiError(format!(
                "Obsidian search returned {}",
                resp.status()
            )));
        }
        resp.json()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("failed to parse search: {e}")))
    }

    /// Append text to a note.
    pub async fn append_to_note(&self, path: &str, content: &str) -> Result<(), IntegrationError> {
        let url = format!("{}/vault/{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "text/markdown")
            .body(content.to_owned())
            .send()
            .await
            .map_err(|e| IntegrationError::ApiError(format!("Obsidian append error: {e}")))?;
        if !resp.status().is_success() {
            return Err(IntegrationError::ApiError(format!(
                "Obsidian append returned {}",
                resp.status()
            )));
        }
        Ok(())
    }
}

/// Search result from the Obsidian Local REST API.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SearchResult {
    pub filename: String,
    #[serde(default)]
    pub matches: Vec<SearchMatch>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SearchMatch {
    #[serde(rename = "match")]
    pub matched: SearchMatchContent,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SearchMatchContent {
    pub start: usize,
    pub end: usize,
}

impl std::fmt::Debug for ObsidianPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObsidianPlugin")
            .field("base_url", &self.base_url)
            .field("scan_folder", &self.scan_folder)
            .finish()
    }
}

const DEFAULT_OBSIDIAN_AGENT_PROMPT: &str = "\
You are a knowledge worker processing tasks from an Obsidian vault. \
Read the task note carefully, research the topic using available tools, \
and write your findings back to the vault.\n\
\n\
Your output should be a well-structured markdown note with sources cited. \
Append your results to the output folder specified in the task.";

#[async_trait]
impl Integration for ObsidianPlugin {
    fn id(&self) -> &str {
        "obsidian"
    }

    fn display_name(&self) -> &str {
        "Obsidian"
    }

    fn is_configured(&self) -> bool {
        !self.base_url.is_empty() && !self.api_key.is_empty()
    }

    fn default_agent_prompt(&self) -> &str {
        DEFAULT_OBSIDIAN_AGENT_PROMPT
    }

    fn default_event_actions(&self) -> Vec<EventActionMapping> {
        // Obsidian is scan-based, not event-based. Default: orchestrate all scanned notes.
        vec![EventActionMapping {
            event_pattern: "note.scanned".into(),
            label_filter: None,
            repo_filter: None,
            action: EventAction::CreateAndOrchestrate,
        }]
    }

    async fn verify_webhook(
        &self,
        _headers: &http::HeaderMap,
        _body: &[u8],
    ) -> Result<(), IntegrationError> {
        // Obsidian doesn't send webhooks — verification is n/a.
        // Scan requests come from authenticated Cairn API calls.
        Ok(())
    }

    async fn parse_event(
        &self,
        _headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<IntegrationEvent, IntegrationError> {
        // For scan-triggered events, the body contains the scan request.
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| IntegrationError::ParseError(format!("invalid JSON: {e}")))?;

        Ok(IntegrationEvent {
            integration_id: "obsidian".into(),
            event_key: "note.scanned".into(),
            source_id: self.base_url.clone(),
            repository: Some(self.scan_folder.clone()),
            title: raw["title"].as_str().map(|s| s.to_owned()),
            body: raw["content"].as_str().map(|s| s.to_owned()),
            labels: vec![],
            raw,
        })
    }

    async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError> {
        Ok(format!(
            "## Task from Obsidian vault\n\
             **Note:** {title}\n\n\
             {body}\n\n\
             ## Workflow\n\
             1. Read the task note carefully and understand what is being asked.\n\
             2. Use available tools to research, search, or gather information.\n\
             3. Write a well-structured markdown response with your findings.\n\
             4. Include sources and citations where applicable.\n\
             5. Complete the task and provide a summary.\n\n\
             ## Output\n\
             Write results as a new note in the `{output}` folder. \
             Use the same title with a \" - Results\" suffix.",
            title = item.title,
            body = item.body,
            output = self.output_folder,
        ))
    }

    async fn prepare_tool_registry(
        &self,
        base: &cairn_tools::BuiltinToolRegistry,
        _item: &WorkItem,
    ) -> Arc<cairn_tools::BuiltinToolRegistry> {
        // Obsidian plugin uses system tools + could add vault read/write tools later.
        Arc::new(cairn_tools::BuiltinToolRegistry::from_existing(base))
    }

    fn auth_exempt_paths(&self) -> Vec<String> {
        // Obsidian is scan-based, no webhook receiver.
        vec![]
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

    fn make_plugin() -> ObsidianPlugin {
        ObsidianPlugin::new(ObsidianConfig {
            base_url: "https://localhost:27124".into(),
            api_key: "test-key".into(),
            scan_folder: "Tasks".into(),
            tag_filter: Some("#cairn".into()),
            output_folder: "Agent Output".into(),
            max_concurrent: 2,
        })
    }

    #[test]
    fn plugin_identity() {
        let p = make_plugin();
        assert_eq!(p.id(), "obsidian");
        assert_eq!(p.display_name(), "Obsidian");
        assert!(p.is_configured());
    }

    #[test]
    fn unconfigured_when_empty_url() {
        let p = ObsidianPlugin::new(ObsidianConfig {
            base_url: String::new(),
            api_key: "key".into(),
            scan_folder: "Tasks".into(),
            tag_filter: None,
            output_folder: "Output".into(),
            max_concurrent: 1,
        });
        assert!(!p.is_configured());
    }

    #[test]
    fn default_prompt_mentions_obsidian() {
        let p = make_plugin();
        assert!(p.default_agent_prompt().contains("Obsidian vault"));
    }

    #[test]
    fn no_auth_exempt_paths() {
        let p = make_plugin();
        assert!(p.auth_exempt_paths().is_empty());
    }

    #[test]
    fn default_event_actions_for_scanned_notes() {
        let p = make_plugin();
        let actions = p.default_event_actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].event_pattern, "note.scanned");
    }

    #[tokio::test]
    async fn build_goal_includes_output_folder() {
        let p = make_plugin();
        let item = WorkItem {
            integration_id: "obsidian".into(),
            source_id: "vault".into(),
            external_id: "Tasks/Research AI agents.md".into(),
            repo: "Tasks".into(),
            title: "Research AI agents".into(),
            body: "Compile a list of the top 10 AI agent frameworks in 2026.".into(),
            run_id: "run_1".into(),
            session_id: "sess_1".into(),
            status: WorkItemStatus::Processing,
        };
        let goal = p.build_goal(&item).await.unwrap();
        assert!(goal.contains("Research AI agents"));
        assert!(goal.contains("Agent Output"));
    }

    #[tokio::test]
    async fn parse_event_returns_scanned_event() {
        let p = make_plugin();
        let body = serde_json::to_vec(&serde_json::json!({
            "title": "Research task",
            "content": "Find top frameworks"
        }))
        .unwrap();
        let event = p.parse_event(&http::HeaderMap::new(), &body).await.unwrap();
        assert_eq!(event.event_key, "note.scanned");
        assert_eq!(event.title.as_deref(), Some("Research task"));
    }

    #[tokio::test]
    async fn verify_webhook_always_ok() {
        let p = make_plugin();
        assert!(p.verify_webhook(&http::HeaderMap::new(), b"").await.is_ok());
    }
}
