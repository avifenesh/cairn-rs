//! Integration plugin framework for Cairn.
//!
//! Defines the `Integration` trait that any external service (GitHub, Linear,
//! Slack, Jira, etc.) implements to plug into Cairn's orchestration pipeline.
//!
//! Each integration provides:
//! - A default agent prompt and tool set
//! - Event→action mappings for webhook-driven automation
//! - HTTP routes for webhooks, scanning, and queue management
//! - Auth-exempt paths for incoming webhook receivers
//!
//! The operator can override any of these via the API. The core orchestrator
//! is integration-agnostic — it takes whatever the plugin/operator provides.

pub mod config;
pub mod github;
pub mod linear;
pub mod notion;
pub mod obsidian;
pub mod types;
pub mod webhook;

pub use config::{IntegrationConfig, ToolConfig};
pub use types::*;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::RwLock;

/// Errors returned by integration operations.
#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("webhook verification failed: {0}")]
    VerificationFailed(String),
    #[error("event parsing failed: {0}")]
    ParseError(String),
    #[error("integration not configured: {0}")]
    NotConfigured(String),
    #[error("external API error: {0}")]
    ApiError(String),
    #[error("{0}")]
    Other(String),
}

/// A configured, active integration that can receive events, queue work,
/// and trigger agent orchestration.
///
/// Implementations live in this crate (e.g. `github::GitHubPlugin`) and are
/// registered at startup via `IntegrationRegistry::register()`.
#[async_trait]
pub trait Integration: Send + Sync + 'static {
    /// Unique identifier (e.g. "github", "linear", "slack").
    fn id(&self) -> &str;

    /// Display name for the UI (e.g. "GitHub", "Linear").
    fn display_name(&self) -> &str;

    /// Whether this integration is currently configured and ready to process events.
    fn is_configured(&self) -> bool;

    /// Default system prompt for agents working on tasks from this integration.
    /// The operator can override this via `IntegrationOverrides`.
    fn default_agent_prompt(&self) -> &str;

    /// Default event→action mappings for this integration.
    fn default_event_actions(&self) -> Vec<EventActionMapping>;

    /// Verify an incoming webhook request (e.g. HMAC-SHA256 signature check).
    async fn verify_webhook(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<(), IntegrationError>;

    /// Parse a webhook payload into a normalised `IntegrationEvent`.
    async fn parse_event(
        &self,
        headers: &http::HeaderMap,
        body: &[u8],
    ) -> Result<IntegrationEvent, IntegrationError>;

    /// Build the goal prompt for a specific work item.
    /// This is the text the orchestrator sends to the LLM as the task.
    async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError>;

    /// Prepare the tool registry for a run triggered by this integration.
    ///
    /// Clones the base registry (which has all system tools) and adds
    /// integration-specific tools (e.g. `create_pr`, `merge_pr` for GitHub).
    async fn prepare_tool_registry(
        &self,
        base: &cairn_tools::BuiltinToolRegistry,
        item: &WorkItem,
    ) -> Arc<cairn_tools::BuiltinToolRegistry>;

    /// Paths that should be exempt from auth middleware
    /// (e.g. "/v1/webhooks/github" which uses its own HMAC verification).
    fn auth_exempt_paths(&self) -> Vec<String>;

    /// Current work queue statistics for this integration.
    async fn queue_stats(&self) -> QueueStats;
}

/// Registry of active integrations, keyed by their ID.
///
/// Also holds per-integration operator overrides that take precedence
/// over the integration's defaults.
pub struct IntegrationRegistry {
    pub(crate) integrations: RwLock<HashMap<String, Arc<dyn Integration>>>,
    pub(crate) overrides: RwLock<HashMap<String, IntegrationOverrides>>,
    /// Stored configs for retrieval via the API.
    pub(crate) configs: RwLock<HashMap<String, IntegrationConfig>>,
}

impl IntegrationRegistry {
    pub fn new() -> Self {
        Self {
            integrations: RwLock::new(HashMap::new()),
            overrides: RwLock::new(HashMap::new()),
            configs: RwLock::new(HashMap::new()),
        }
    }

    /// Register an integration. Replaces any existing integration with the same ID.
    pub async fn register(&self, integration: Arc<dyn Integration>) {
        let id = integration.id().to_owned();
        self.integrations.write().await.insert(id, integration);
    }

    /// Synchronous registration for startup (before the async runtime is entered).
    /// Only safe when you have exclusive `&mut` access to the registry.
    pub fn register_sync(&mut self, integration: Arc<dyn Integration>) {
        let id = integration.id().to_owned();
        self.integrations.get_mut().insert(id, integration);
    }

    /// Get an integration by ID.
    pub async fn get(&self, id: &str) -> Option<Arc<dyn Integration>> {
        self.integrations.read().await.get(id).cloned()
    }

    /// List all registered integrations.
    pub async fn list(&self) -> Vec<Arc<dyn Integration>> {
        self.integrations.read().await.values().cloned().collect()
    }

    /// Get the effective agent prompt for an integration (override or default).
    pub async fn effective_prompt(&self, id: &str) -> Option<String> {
        let overrides = self.overrides.read().await;
        if let Some(o) = overrides.get(id)
            && let Some(ref prompt) = o.agent_prompt
        {
            return Some(prompt.clone());
        }
        let integrations = self.integrations.read().await;
        integrations
            .get(id)
            .map(|i| i.default_agent_prompt().to_owned())
    }

    /// Get the effective event→action mappings for an integration.
    pub async fn effective_event_actions(&self, id: &str) -> Vec<EventActionMapping> {
        let overrides = self.overrides.read().await;
        if let Some(o) = overrides.get(id)
            && let Some(ref actions) = o.event_actions
        {
            return actions.clone();
        }
        let integrations = self.integrations.read().await;
        integrations
            .get(id)
            .map(|i| i.default_event_actions())
            .unwrap_or_default()
    }

    /// Get the operator overrides for an integration.
    pub async fn get_overrides(&self, id: &str) -> IntegrationOverrides {
        self.overrides
            .read()
            .await
            .get(id)
            .cloned()
            .unwrap_or_default()
    }

    /// Set operator overrides for an integration.
    pub async fn set_overrides(&self, id: &str, overrides: IntegrationOverrides) {
        self.overrides
            .write()
            .await
            .insert(id.to_owned(), overrides);
    }

    /// Reset operator overrides for an integration (revert to defaults).
    pub async fn clear_overrides(&self, id: &str) {
        self.overrides.write().await.remove(id);
    }

    /// Get the effective tool config for an integration.
    ///
    /// Priority: operator overrides → registration config → default (include all Core).
    pub async fn effective_tool_config(&self, id: &str) -> config::ToolConfig {
        let overrides = self.overrides.read().await;
        if let Some(o) = overrides.get(id)
            && let Some(ref tc) = o.tools
        {
            return tc.clone();
        }
        let configs = self.configs.read().await;
        if let Some(c) = configs.get(id)
            && let Some(ref tc) = c.tools
        {
            return tc.clone();
        }
        config::ToolConfig::default()
    }

    /// Collect all auth-exempt paths from all registered integrations.
    pub async fn all_auth_exempt_paths(&self) -> Vec<String> {
        let integrations = self.integrations.read().await;
        integrations
            .values()
            .flat_map(|i| i.auth_exempt_paths())
            .collect()
    }

    /// Get status summaries for all integrations (used by GET /v1/integrations).
    pub async fn all_statuses(&self) -> Vec<IntegrationStatus> {
        let integrations = self.integrations.read().await;
        let overrides = self.overrides.read().await;
        let mut statuses = Vec::new();
        for integration in integrations.values() {
            let id = integration.id().to_owned();
            let o = overrides.get(&id).cloned().unwrap_or_default();
            statuses.push(IntegrationStatus {
                id: id.clone(),
                display_name: integration.display_name().to_owned(),
                configured: integration.is_configured(),
                overrides: o,
                queue_stats: integration.queue_stats().await,
            });
        }
        statuses
    }
}

impl Default for IntegrationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test integration for unit tests.
    struct MockIntegration;

    #[async_trait]
    impl Integration for MockIntegration {
        fn id(&self) -> &str {
            "mock"
        }
        fn display_name(&self) -> &str {
            "Mock"
        }
        fn is_configured(&self) -> bool {
            true
        }
        fn default_agent_prompt(&self) -> &str {
            "You are a test agent."
        }
        fn default_event_actions(&self) -> Vec<EventActionMapping> {
            vec![EventActionMapping {
                event_pattern: "test.*".into(),
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
            Ok(())
        }
        async fn parse_event(
            &self,
            _headers: &http::HeaderMap,
            _body: &[u8],
        ) -> Result<IntegrationEvent, IntegrationError> {
            Ok(IntegrationEvent {
                integration_id: "mock".into(),
                event_key: "test.created".into(),
                source_id: "1".into(),
                repository: None,
                title: Some("Test event".into()),
                body: None,
                labels: vec![],
                raw: serde_json::json!({}),
            })
        }
        async fn build_goal(&self, item: &WorkItem) -> Result<String, IntegrationError> {
            Ok(format!("Process: {}", item.title))
        }
        async fn prepare_tool_registry(
            &self,
            base: &cairn_tools::BuiltinToolRegistry,
            _item: &WorkItem,
        ) -> Arc<cairn_tools::BuiltinToolRegistry> {
            Arc::new(cairn_tools::BuiltinToolRegistry::from_existing(base))
        }
        fn auth_exempt_paths(&self) -> Vec<String> {
            vec!["/v1/webhooks/mock".into()]
        }
        async fn queue_stats(&self) -> QueueStats {
            QueueStats::default()
        }
    }

    #[tokio::test]
    async fn register_and_retrieve_integration() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;

        let mock = registry.get("mock").await;
        assert!(mock.is_some());
        assert_eq!(mock.unwrap().display_name(), "Mock");
    }

    #[tokio::test]
    async fn list_returns_all_registered() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;

        let all = registry.list().await;
        assert_eq!(all.len(), 1);
    }

    #[tokio::test]
    async fn effective_prompt_returns_default_when_no_override() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;

        let prompt = registry.effective_prompt("mock").await.unwrap();
        assert_eq!(prompt, "You are a test agent.");
    }

    #[tokio::test]
    async fn effective_prompt_returns_override_when_set() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;
        registry
            .set_overrides(
                "mock",
                IntegrationOverrides {
                    agent_prompt: Some("Custom prompt".into()),
                    ..Default::default()
                },
            )
            .await;

        let prompt = registry.effective_prompt("mock").await.unwrap();
        assert_eq!(prompt, "Custom prompt");
    }

    #[tokio::test]
    async fn clear_overrides_reverts_to_default() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;
        registry
            .set_overrides(
                "mock",
                IntegrationOverrides {
                    agent_prompt: Some("Custom".into()),
                    ..Default::default()
                },
            )
            .await;
        registry.clear_overrides("mock").await;

        let prompt = registry.effective_prompt("mock").await.unwrap();
        assert_eq!(prompt, "You are a test agent.");
    }

    #[tokio::test]
    async fn auth_exempt_paths_collects_from_all_integrations() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;

        let paths = registry.all_auth_exempt_paths().await;
        assert!(paths.contains(&"/v1/webhooks/mock".to_owned()));
    }

    #[tokio::test]
    async fn all_statuses_returns_configured_integration() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;

        let statuses = registry.all_statuses().await;
        assert_eq!(statuses.len(), 1);
        assert!(statuses[0].configured);
        assert_eq!(statuses[0].id, "mock");
    }

    #[tokio::test]
    async fn effective_event_actions_returns_default() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;

        let actions = registry.effective_event_actions("mock").await;
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].event_pattern, "test.*");
    }

    #[tokio::test]
    async fn effective_event_actions_returns_override() {
        let registry = IntegrationRegistry::new();
        registry.register(Arc::new(MockIntegration)).await;
        registry
            .set_overrides(
                "mock",
                IntegrationOverrides {
                    event_actions: Some(vec![EventActionMapping {
                        event_pattern: "custom.*".into(),
                        label_filter: None,
                        repo_filter: None,
                        action: EventAction::Acknowledge,
                    }]),
                    ..Default::default()
                },
            )
            .await;

        let actions = registry.effective_event_actions("mock").await;
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].event_pattern, "custom.*");
    }
}
