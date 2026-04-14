//! Linear integration plugin for Cairn.
//!
//! Built-in provider type `"linear"` — receives Linear webhooks, parses
//! issue/comment events, and triggers agent orchestration.
//!
//! Linear webhooks:
//! - Signature: HMAC-SHA256 in `Linear-Signature` header
//! - Payload: `{ "action": "create"|"update"|"remove", "type": "Issue"|"Comment"|..., "data": {...} }`
//! - Events: Issue created/updated, Comment created, Project updates, etc.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::{
    EventAction, EventActionMapping, Integration, IntegrationError, IntegrationEvent, QueueStats,
    WorkItem, WorkItemStatus,
};

/// Linear-specific configuration.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct LinearConfig {
    /// Webhook signing secret from Linear settings.
    pub webhook_secret: String,
    /// Linear API key for reading issue details (optional — enriches goal prompts).
    #[serde(default)]
    pub api_key: Option<String>,
    /// Team key filter — only process issues from this team (optional).
    #[serde(default)]
    pub team_filter: Option<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
}

fn default_max_concurrent() -> u32 {
    3
}

/// Linear integration plugin.
pub struct LinearPlugin {
    webhook_secret: String,
    api_key: Option<String>,
    team_filter: Option<String>,
    pub queue: RwLock<VecDeque<WorkItem>>,
    pub queue_paused: AtomicBool,
    pub queue_running: AtomicBool,
    pub max_concurrent: AtomicU32,
    pub run_semaphore: Arc<Semaphore>,
}

impl LinearPlugin {
    pub fn new(config: LinearConfig) -> Self {
        let max = config.max_concurrent;
        Self {
            webhook_secret: config.webhook_secret,
            api_key: config.api_key,
            team_filter: config.team_filter,
            queue: RwLock::new(VecDeque::new()),
            queue_paused: AtomicBool::new(false),
            queue_running: AtomicBool::new(false),
            max_concurrent: AtomicU32::new(max),
            run_semaphore: Arc::new(Semaphore::new(max as usize)),
        }
    }
}

impl std::fmt::Debug for LinearPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinearPlugin")
            .field("has_api_key", &self.api_key.is_some())
            .field("team_filter", &self.team_filter)
            .finish()
    }
}

const DEFAULT_LINEAR_AGENT_PROMPT: &str = "\
You are a senior software engineer working autonomously. You have been \
assigned a Linear ticket and must resolve it by writing code and opening \
a pull request.\n\
\n\
Follow the workflow in the goal description. Use tool_search to discover \
available tools. Explore the codebase before writing code. Write real, \
working code — not descriptions or TODO comments. Verify your changes \
compile and tests pass before opening the PR.\n\
\n\
Do not call complete_run until you have opened a pull request.";

#[async_trait]
impl Integration for LinearPlugin {
    fn id(&self) -> &str {
        "linear"
    }

    fn display_name(&self) -> &str {
        "Linear"
    }

    fn is_configured(&self) -> bool {
        true
    }

    fn default_agent_prompt(&self) -> &str {
        DEFAULT_LINEAR_AGENT_PROMPT
    }

    fn default_event_actions(&self) -> Vec<EventActionMapping> {
        vec![
            EventActionMapping {
                event_pattern: "Issue.create".into(),
                label_filter: None,
                repo_filter: None,
                action: EventAction::CreateAndOrchestrate,
            },
            EventActionMapping {
                event_pattern: "Issue.update".into(),
                label_filter: Some("cairn".into()),
                repo_filter: None,
                action: EventAction::CreateAndOrchestrate,
            },
        ]
    }

    async fn verify_webhook(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<(), IntegrationError> {
        let sig = headers
            .get("Linear-Signature")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                IntegrationError::VerificationFailed("missing Linear-Signature header".into())
            })?;

        // Linear uses raw HMAC-SHA256 hex digest (no "sha256=" prefix).
        // We use the same HMAC verification but handle both formats.
        let sig_with_prefix = if sig.starts_with("sha256=") {
            sig.to_owned()
        } else {
            format!("sha256={sig}")
        };

        cairn_github::verify_signature(&sig_with_prefix, self.webhook_secret.as_bytes(), body)
            .map_err(|e| {
                IntegrationError::VerificationFailed(format!(
                    "Linear signature verification failed: {e}"
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

        // Linear webhook format: { "action": "create", "type": "Issue", "data": {...} }
        let action = raw["action"].as_str().unwrap_or("unknown");
        let event_type = raw["type"].as_str().unwrap_or("Unknown");
        let event_key = format!("{event_type}.{action}");

        let data = &raw["data"];
        let title = data["title"].as_str().map(|s| s.to_owned());
        let body_text = data["description"].as_str().map(|s| s.to_owned());

        let labels: Vec<String> = data["labels"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l["name"].as_str().map(|s| s.to_owned()))
                    .collect()
            })
            .unwrap_or_default();

        let team = data["team"]
            .as_object()
            .and_then(|t| t["key"].as_str())
            .map(|s| s.to_owned());

        // Apply team filter if configured.
        if let (Some(filter), Some(t)) = (&self.team_filter, &team)
            && t != filter
        {
            return Err(IntegrationError::Other(format!(
                "event from team {t} filtered (expected {filter})"
            )));
        }

        Ok(IntegrationEvent {
            integration_id: "linear".into(),
            event_key,
            source_id: data["id"].as_str().unwrap_or("").to_owned(),
            repository: team,
            title,
            body: body_text,
            labels,
            raw,
        })
    }

    async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError> {
        Ok(format!(
            "## Task\n\
             Resolve Linear ticket: **{title}**\n\n\
             {body}\n\n\
             ## Workflow\n\
             1. **Explore** — Use tool_search to find available tools. Search the codebase \
             to understand the relevant code.\n\
             2. **Plan** — Identify which files need to change.\n\
             3. **Implement** — Write the code. Make minimal, focused changes.\n\
             4. **Verify** — Run tests. Fix any failures.\n\
             5. **Deliver** — Commit, push, and open a pull request.\n\
             6. **Complete** — After the PR is open, call complete_run with a summary.\n\n\
             ## Tips\n\
             - Explore before implementing. Read relevant files first.\n\
             - If a tool call fails, try a different approach.\n\
             - Write real code, not TODO comments.\n\
             - Do not call complete_run until you have opened a PR.",
            title = item.title,
            body = item.body,
        ))
    }

    async fn prepare_tool_registry(
        &self,
        base: &cairn_tools::BuiltinToolRegistry,
        _item: &WorkItem,
    ) -> Arc<cairn_tools::BuiltinToolRegistry> {
        // Linear plugin uses system tools only (file, shell, git).
        // No Linear-specific API tools yet — could add comment/update tools later.
        Arc::new(cairn_tools::BuiltinToolRegistry::from_existing(base))
    }

    fn auth_exempt_paths(&self) -> Vec<String> {
        vec!["/v1/webhooks/linear".into()]
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

    fn make_plugin() -> LinearPlugin {
        LinearPlugin::new(LinearConfig {
            webhook_secret: "test-secret".into(),
            api_key: None,
            team_filter: None,
            max_concurrent: 2,
        })
    }

    #[test]
    fn plugin_identity() {
        let p = make_plugin();
        assert_eq!(p.id(), "linear");
        assert_eq!(p.display_name(), "Linear");
        assert!(p.is_configured());
    }

    #[test]
    fn default_prompt_mentions_linear() {
        let p = make_plugin();
        assert!(p.default_agent_prompt().contains("Linear ticket"));
    }

    #[test]
    fn default_event_actions_target_issues() {
        let p = make_plugin();
        let actions = p.default_event_actions();
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].event_pattern, "Issue.create");
    }

    #[test]
    fn auth_exempt_paths() {
        let p = make_plugin();
        assert!(
            p.auth_exempt_paths()
                .contains(&"/v1/webhooks/linear".to_owned())
        );
    }

    #[tokio::test]
    async fn parse_event_extracts_linear_fields() {
        let p = make_plugin();
        let body = serde_json::to_vec(&serde_json::json!({
            "action": "create",
            "type": "Issue",
            "data": {
                "id": "abc-123",
                "title": "Fix login bug",
                "description": "Users can't login after session timeout",
                "team": { "key": "ENG" },
                "labels": [{"name": "bug"}, {"name": "cairn"}]
            }
        }))
        .unwrap();

        let event = p.parse_event(&http::HeaderMap::new(), &body).await.unwrap();
        assert_eq!(event.event_key, "Issue.create");
        assert_eq!(event.title.as_deref(), Some("Fix login bug"));
        assert_eq!(event.source_id, "abc-123");
        assert_eq!(event.repository.as_deref(), Some("ENG"));
        assert_eq!(event.labels, vec!["bug", "cairn"]);
    }

    #[tokio::test]
    async fn parse_event_team_filter_rejects() {
        let p = LinearPlugin::new(LinearConfig {
            webhook_secret: "secret".into(),
            api_key: None,
            team_filter: Some("BACKEND".into()),
            max_concurrent: 1,
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "action": "create",
            "type": "Issue",
            "data": { "id": "x", "team": { "key": "FRONTEND" } }
        }))
        .unwrap();

        let result = p.parse_event(&http::HeaderMap::new(), &body).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn build_goal_includes_title_and_body() {
        let p = make_plugin();
        let item = WorkItem {
            integration_id: "linear".into(),
            source_id: "abc".into(),
            external_id: "LIN-42".into(),
            repo: "ENG".into(),
            title: "Fix the login".into(),
            body: "Session timeout breaks auth".into(),
            run_id: "run_1".into(),
            session_id: "sess_1".into(),
            status: WorkItemStatus::Processing,
        };
        let goal = p.build_goal(&item).await.unwrap();
        assert!(goal.contains("Fix the login"));
        assert!(goal.contains("Session timeout"));
    }

    #[tokio::test]
    async fn queue_stats_empty() {
        let p = make_plugin();
        let stats = p.queue_stats().await;
        assert_eq!(stats.pending, 0);
        assert_eq!(stats.processing, 0);
    }
}
