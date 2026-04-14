//! Generic webhook integration plugin.
//!
//! Config-driven — the operator provides webhook secret, agent prompt,
//! event→action mappings, and field paths via the API. No code needed.
//! Works with any service that sends JSON webhooks (Linear, Jira, etc.).

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32};

use async_trait::async_trait;
use tokio::sync::{RwLock, Semaphore};

use crate::config::WebhookConfig;
use crate::{
    EventAction, EventActionMapping, Integration, IntegrationError, IntegrationEvent, QueueStats,
    WorkItem, WorkItemStatus,
};

/// A config-driven webhook integration that works with any JSON webhook service.
pub struct GenericWebhookPlugin {
    id: String,
    display_name: String,
    webhook_secret: Option<String>,
    signature_header: String,
    agent_prompt: String,
    event_actions: Vec<EventActionMapping>,
    event_type_path: String,
    title_path: Option<String>,
    body_path: Option<String>,
    pub queue: RwLock<VecDeque<WorkItem>>,
    pub queue_paused: AtomicBool,
    pub queue_running: AtomicBool,
    pub max_concurrent: AtomicU32,
    pub run_semaphore: Arc<Semaphore>,
}

impl GenericWebhookPlugin {
    pub fn new(id: &str, config: WebhookConfig) -> Self {
        let display_name = config.display_name.unwrap_or_else(|| id.to_owned());
        let agent_prompt = config.agent_prompt.unwrap_or_else(|| {
            format!(
                "You are an autonomous agent working on a task from {display_name}. \
                 Use the available tools to understand the problem, implement a solution, \
                 and verify it works. Do not call complete_run until you have produced \
                 concrete artifacts."
            )
        });
        let max_concurrent = config.max_concurrent;
        Self {
            id: id.to_owned(),
            display_name,
            webhook_secret: config.webhook_secret,
            signature_header: config.signature_header,
            agent_prompt,
            event_actions: config.event_actions,
            event_type_path: config.event_type_path,
            title_path: config.title_path,
            body_path: config.body_path,
            queue: RwLock::new(VecDeque::new()),
            queue_paused: AtomicBool::new(false),
            queue_running: AtomicBool::new(false),
            max_concurrent: AtomicU32::new(max_concurrent),
            run_semaphore: Arc::new(Semaphore::new(max_concurrent as usize)),
        }
    }

    /// Extract a value from nested JSON using a dot-separated path (e.g. "issue.title").
    fn json_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
        let mut current = value;
        for key in path.split('.') {
            current = current.get(key)?;
        }
        Some(current)
    }
}

impl std::fmt::Debug for GenericWebhookPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GenericWebhookPlugin")
            .field("id", &self.id)
            .field("display_name", &self.display_name)
            .finish()
    }
}

#[async_trait]
impl Integration for GenericWebhookPlugin {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        &self.display_name
    }

    fn is_configured(&self) -> bool {
        true
    }

    fn default_agent_prompt(&self) -> &str {
        &self.agent_prompt
    }

    fn default_event_actions(&self) -> Vec<EventActionMapping> {
        if self.event_actions.is_empty() {
            // Default: orchestrate everything.
            vec![EventActionMapping {
                event_pattern: "*".into(),
                label_filter: None,
                repo_filter: None,
                action: EventAction::CreateAndOrchestrate,
            }]
        } else {
            self.event_actions.clone()
        }
    }

    async fn verify_webhook(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<(), IntegrationError> {
        let Some(ref secret) = self.webhook_secret else {
            // No secret configured — skip verification.
            return Ok(());
        };
        if secret.is_empty() {
            return Ok(());
        }

        let sig = headers
            .get(self.signature_header.as_str())
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                IntegrationError::VerificationFailed(format!(
                    "missing {} header",
                    self.signature_header
                ))
            })?;

        // Try HMAC-SHA256 verification (most common).
        cairn_github::verify_signature(sig, secret.as_bytes(), body).map_err(|e| {
            IntegrationError::VerificationFailed(format!("signature verification failed: {e}"))
        })
    }

    async fn parse_event(
        &self,
        _headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<IntegrationEvent, IntegrationError> {
        let raw: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| IntegrationError::ParseError(format!("invalid JSON: {e}")))?;

        let event_key = Self::json_path(&raw, &self.event_type_path)
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_owned();

        let title = self
            .title_path
            .as_deref()
            .and_then(|p| Self::json_path(&raw, p))
            .and_then(|v| v.as_str())
            .or_else(|| raw["title"].as_str())
            .map(|s| s.to_owned());

        let body_text = self
            .body_path
            .as_deref()
            .and_then(|p| Self::json_path(&raw, p))
            .and_then(|v| v.as_str())
            .or_else(|| raw["body"].as_str())
            .or_else(|| raw["description"].as_str())
            .map(|s| s.to_owned());

        Ok(IntegrationEvent {
            integration_id: self.id.clone(),
            event_key,
            source_id: String::new(),
            repository: raw["repository"]
                .as_str()
                .or_else(|| raw["repo"].as_str())
                .map(|s| s.to_owned()),
            title,
            body: body_text,
            labels: vec![],
            raw,
        })
    }

    async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError> {
        Ok(format!(
            "## Task\n\
             {title}\n\n\
             {body}\n\n\
             Use the available tools to implement a solution. \
             Explore the codebase first, then make changes and verify them.",
            title = item.title,
            body = item.body,
        ))
    }

    async fn prepare_tool_registry(
        &self,
        base: &cairn_tools::BuiltinToolRegistry,
        _item: &WorkItem,
    ) -> Arc<cairn_tools::BuiltinToolRegistry> {
        // Generic webhook plugin doesn't add custom tools — uses system tools only.
        Arc::new(cairn_tools::BuiltinToolRegistry::from_existing(base))
    }

    fn auth_exempt_paths(&self) -> Vec<String> {
        vec![format!("/v1/webhooks/{}", self.id)]
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

    fn make_plugin() -> GenericWebhookPlugin {
        GenericWebhookPlugin::new(
            "my-linear",
            WebhookConfig {
                display_name: Some("Linear".into()),
                webhook_secret: Some("secret".into()),
                signature_header: "X-Linear-Signature".into(),
                agent_prompt: Some("Resolve Linear tickets.".into()),
                event_actions: vec![EventActionMapping {
                    event_pattern: "issue.*".into(),
                    label_filter: None,
                    repo_filter: None,
                    action: EventAction::CreateAndOrchestrate,
                }],
                event_type_path: "type".into(),
                title_path: Some("data.title".into()),
                body_path: Some("data.description".into()),
                max_concurrent: 2,
            },
        )
    }

    #[test]
    fn plugin_identity() {
        let p = make_plugin();
        assert_eq!(p.id(), "my-linear");
        assert_eq!(p.display_name(), "Linear");
        assert!(p.is_configured());
    }

    #[test]
    fn default_prompt_from_config() {
        let p = make_plugin();
        assert_eq!(p.default_agent_prompt(), "Resolve Linear tickets.");
    }

    #[test]
    fn auth_exempt_includes_webhook_path() {
        let p = make_plugin();
        assert!(
            p.auth_exempt_paths()
                .contains(&"/v1/webhooks/my-linear".to_owned())
        );
    }

    #[test]
    fn json_path_extracts_nested() {
        let json = serde_json::json!({
            "data": {
                "title": "Bug fix",
                "description": "Fix the login issue"
            }
        });
        let title = GenericWebhookPlugin::json_path(&json, "data.title");
        assert_eq!(title.unwrap().as_str().unwrap(), "Bug fix");
    }

    #[test]
    fn json_path_returns_none_for_missing() {
        let json = serde_json::json!({"a": 1});
        assert!(GenericWebhookPlugin::json_path(&json, "b.c").is_none());
    }

    #[tokio::test]
    async fn parse_event_extracts_fields() {
        let p = make_plugin();
        let body = serde_json::to_vec(&serde_json::json!({
            "type": "issue.created",
            "data": {
                "title": "Login broken",
                "description": "Users can't login"
            }
        }))
        .unwrap();
        let headers = http::HeaderMap::new();
        let event = p.parse_event(&headers, &body).await.unwrap();
        assert_eq!(event.event_key, "issue.created");
        assert_eq!(event.title.as_deref(), Some("Login broken"));
        assert_eq!(event.body.as_deref(), Some("Users can't login"));
    }

    #[test]
    fn default_event_actions_when_empty() {
        let p = GenericWebhookPlugin::new(
            "empty",
            WebhookConfig {
                display_name: None,
                webhook_secret: None,
                signature_header: "X-Signature-256".into(),
                agent_prompt: None,
                event_actions: vec![],
                event_type_path: "action".into(),
                title_path: None,
                body_path: None,
                max_concurrent: 1,
            },
        );
        let actions = p.default_event_actions();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].event_pattern, "*");
    }

    #[tokio::test]
    async fn verify_webhook_skips_when_no_secret() {
        let p = GenericWebhookPlugin::new(
            "no-secret",
            WebhookConfig {
                display_name: None,
                webhook_secret: None,
                signature_header: "X-Sig".into(),
                agent_prompt: None,
                event_actions: vec![],
                event_type_path: "action".into(),
                title_path: None,
                body_path: None,
                max_concurrent: 1,
            },
        );
        let headers = http::HeaderMap::new();
        assert!(p.verify_webhook(&headers, b"{}").await.is_ok());
    }
}
