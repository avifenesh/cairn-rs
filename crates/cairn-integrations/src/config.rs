//! Config-driven integration registration.
//!
//! Operators register integrations at runtime via `POST /v1/integrations`
//! with a JSON payload. The registry creates the appropriate plugin type
//! based on the `type` field.

use serde::{Deserialize, Serialize};

use crate::{IntegrationError, IntegrationRegistry};

/// Per-integration tool set configuration.
///
/// Operators can customize which tools an agent receives for each integration:
/// - `include_core: true` — include Core tier tools (file_read, shell_exec, git, etc.)
/// - `add` — additional tool names to include beyond defaults
/// - `remove` — tool names to exclude from the default set
///
/// Example: a read-only reviewer agent might use `remove: ["file_write", "shell_exec"]`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolConfig {
    /// Include Core tier tools (default: true).
    #[serde(default = "default_true")]
    pub include_core: bool,
    /// Additional tool names to include beyond the integration's defaults.
    #[serde(default)]
    pub add: Vec<String>,
    /// Tool names to exclude from the default set.
    #[serde(default)]
    pub remove: Vec<String>,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            include_core: true,
            add: Vec::new(),
            remove: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

/// Serializable integration configuration — the payload operators POST.
///
/// Built-in provider types:
/// - `"github"` — GitHub App integration (HMAC webhooks, API tools, installation tokens)
/// - `"webhook"` — Generic webhook (config-driven, any service)
/// - `"plugin"`  — External JSON-RPC process (rich, custom logic)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IntegrationConfig {
    /// Unique identifier for this integration instance (e.g. "github", "my-linear").
    pub id: String,
    /// Provider type: "github", "webhook", or "plugin".
    #[serde(rename = "type")]
    pub provider_type: String,
    /// Provider-specific configuration (varies by type).
    #[serde(default)]
    pub config: serde_json::Value,
    /// Per-integration tool set customization.
    #[serde(default)]
    pub tools: Option<ToolConfig>,
}

/// GitHub-specific configuration fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GitHubConfig {
    pub app_id: u64,
    /// Path to PEM-encoded RSA private key file.
    pub private_key_file: String,
    pub webhook_secret: String,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
}

fn default_max_concurrent() -> u32 {
    3
}

/// Generic webhook configuration fields.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WebhookConfig {
    /// Display name for the UI (e.g. "Linear", "Jira").
    #[serde(default)]
    pub display_name: Option<String>,
    /// Shared secret for HMAC-SHA256 verification. If empty, no verification.
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Header name containing the signature (default: "X-Signature-256").
    #[serde(default = "default_signature_header")]
    pub signature_header: String,
    /// Agent prompt for runs triggered by this integration.
    #[serde(default)]
    pub agent_prompt: Option<String>,
    /// Event→action mappings.
    #[serde(default)]
    pub event_actions: Vec<crate::EventActionMapping>,
    /// JSON path to the event type in the webhook body (default: "action").
    #[serde(default = "default_event_type_path")]
    pub event_type_path: String,
    /// JSON path to the title field (default: "issue.title" or "title").
    #[serde(default)]
    pub title_path: Option<String>,
    /// JSON path to the body field (default: "issue.body" or "body").
    #[serde(default)]
    pub body_path: Option<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: u32,
}

fn default_signature_header() -> String {
    "X-Signature-256".into()
}

fn default_event_type_path() -> String {
    "action".into()
}

impl IntegrationRegistry {
    /// Register an integration from a config payload (runtime API).
    ///
    /// Creates the appropriate plugin type based on `config.provider_type`:
    /// - `"github"` → `GitHubPlugin`
    /// - `"webhook"` → `GenericWebhookPlugin`
    /// - `"plugin"` → returns error (JSON-RPC plugins not yet implemented)
    pub async fn register_from_config(
        &self,
        config: IntegrationConfig,
    ) -> Result<(), IntegrationError> {
        match config.provider_type.as_str() {
            "github" => {
                let gh_config: GitHubConfig = serde_json::from_value(config.config.clone())
                    .map_err(|e| {
                        IntegrationError::ConfigInvalid(format!("invalid github config: {e}"))
                    })?;
                let plugin = crate::github::GitHubPlugin::from_config(&config.id, gh_config)?;
                self.register(std::sync::Arc::new(plugin)).await;
                // Store the config for retrieval via GET /v1/integrations/{id}.
                let id = config.id.clone();
                self.store_config(&id, config).await;
                Ok(())
            }
            "notion" => {
                let notion_config: crate::notion::NotionConfig =
                    serde_json::from_value(config.config.clone()).map_err(|e| {
                        IntegrationError::ConfigInvalid(format!("invalid notion config: {e}"))
                    })?;
                let plugin = crate::notion::NotionPlugin::new(notion_config);
                self.register(std::sync::Arc::new(plugin)).await;
                let id = config.id.clone();
                self.store_config(&id, config).await;
                Ok(())
            }
            "obsidian" => {
                let obs_config: crate::obsidian::ObsidianConfig =
                    serde_json::from_value(config.config.clone()).map_err(|e| {
                        IntegrationError::ConfigInvalid(format!("invalid obsidian config: {e}"))
                    })?;
                let plugin = crate::obsidian::ObsidianPlugin::new(obs_config);
                self.register(std::sync::Arc::new(plugin)).await;
                let id = config.id.clone();
                self.store_config(&id, config).await;
                Ok(())
            }
            "linear" => {
                let lin_config: crate::linear::LinearConfig =
                    serde_json::from_value(config.config.clone()).map_err(|e| {
                        IntegrationError::ConfigInvalid(format!("invalid linear config: {e}"))
                    })?;
                let plugin = crate::linear::LinearPlugin::new(lin_config);
                self.register(std::sync::Arc::new(plugin)).await;
                let id = config.id.clone();
                self.store_config(&id, config).await;
                Ok(())
            }
            "webhook" => {
                let wh_config: WebhookConfig = serde_json::from_value(config.config.clone())
                    .map_err(|e| {
                        IntegrationError::ConfigInvalid(format!("invalid webhook config: {e}"))
                    })?;
                let plugin = crate::webhook::GenericWebhookPlugin::new(&config.id, wh_config);
                self.register(std::sync::Arc::new(plugin)).await;
                let id = config.id.clone();
                self.store_config(&id, config).await;
                Ok(())
            }
            "plugin" => Err(IntegrationError::ConfigInvalid(
                "JSON-RPC plugin integrations not yet implemented. \
                 Use type \"webhook\" for config-driven integrations."
                    .into(),
            )),
            other => Err(IntegrationError::ConfigInvalid(format!(
                "unknown integration type: \"{other}\". Valid types: github, webhook, plugin"
            ))),
        }
    }

    /// Remove a registered integration (runtime API).
    pub async fn unregister(&self, id: &str) -> Result<(), IntegrationError> {
        let mut integrations = self.integrations.write().await;
        if integrations.remove(id).is_none() {
            return Err(IntegrationError::NotConfigured(id.into()));
        }
        self.configs.write().await.remove(id);
        self.clear_overrides(id).await;
        Ok(())
    }

    /// Get the stored config for an integration.
    pub async fn get_config(&self, id: &str) -> Option<IntegrationConfig> {
        self.configs.read().await.get(id).cloned()
    }

    /// List all stored configs.
    pub async fn list_configs(&self) -> Vec<IntegrationConfig> {
        self.configs.read().await.values().cloned().collect()
    }

    /// Store config for later retrieval.
    async fn store_config(&self, id: &str, config: IntegrationConfig) {
        self.configs.write().await.insert(id.to_owned(), config);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn register_webhook_from_config() {
        let registry = IntegrationRegistry::new();
        let config = IntegrationConfig {
            id: "my-linear".into(),
            provider_type: "webhook".into(),
            config: serde_json::json!({
                "display_name": "Linear",
                "webhook_secret": "secret123",
                "agent_prompt": "You resolve Linear tickets.",
                "event_actions": [{
                    "event_pattern": "issue.created",
                    "action": "create_and_orchestrate"
                }]
            }),
            tools: None,
        };
        registry.register_from_config(config).await.unwrap();

        let integration = registry.get("my-linear").await.unwrap();
        assert_eq!(integration.display_name(), "Linear");
        assert!(integration.is_configured());
    }

    #[tokio::test]
    async fn register_unknown_type_fails() {
        let registry = IntegrationRegistry::new();
        let config = IntegrationConfig {
            id: "test".into(),
            provider_type: "unknown".into(),
            config: serde_json::json!({}),
            tools: None,
        };
        let result = registry.register_from_config(config).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unregister_removes_integration() {
        let registry = IntegrationRegistry::new();
        let config = IntegrationConfig {
            id: "test-wh".into(),
            provider_type: "webhook".into(),
            config: serde_json::json!({}),
            tools: None,
        };
        registry.register_from_config(config).await.unwrap();
        assert!(registry.get("test-wh").await.is_some());

        registry.unregister("test-wh").await.unwrap();
        assert!(registry.get("test-wh").await.is_none());
    }

    #[tokio::test]
    async fn unregister_nonexistent_fails() {
        let registry = IntegrationRegistry::new();
        let result = registry.unregister("nope").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_config_returns_stored() {
        let registry = IntegrationRegistry::new();
        let config = IntegrationConfig {
            id: "test-cfg".into(),
            provider_type: "webhook".into(),
            config: serde_json::json!({"display_name": "Test"}),
            tools: None,
        };
        registry.register_from_config(config).await.unwrap();

        let stored = registry.get_config("test-cfg").await.unwrap();
        assert_eq!(stored.provider_type, "webhook");
    }

    #[test]
    fn github_config_deserializes() {
        let json = serde_json::json!({
            "app_id": 12345,
            "private_key_file": "/path/to/key.pem",
            "webhook_secret": "secret"
        });
        let config: GitHubConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.app_id, 12345);
        assert_eq!(config.max_concurrent, 3); // default
    }

    #[test]
    fn webhook_config_defaults() {
        let json = serde_json::json!({});
        let config: WebhookConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.signature_header, "X-Signature-256");
        assert_eq!(config.event_type_path, "action");
        assert_eq!(config.max_concurrent, 3);
    }

    #[test]
    fn tool_config_defaults_to_include_core() {
        let tc = ToolConfig::default();
        assert!(tc.include_core);
        assert!(tc.add.is_empty());
        assert!(tc.remove.is_empty());
    }

    #[test]
    fn tool_config_deserializes_from_json() {
        let json = serde_json::json!({
            "include_core": true,
            "add": ["github_api.create_pr"],
            "remove": ["shell_exec"]
        });
        let tc: ToolConfig = serde_json::from_value(json).unwrap();
        assert!(tc.include_core);
        assert_eq!(tc.add, vec!["github_api.create_pr"]);
        assert_eq!(tc.remove, vec!["shell_exec"]);
    }

    #[test]
    fn tool_config_empty_json_uses_defaults() {
        let tc: ToolConfig = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(tc.include_core);
        assert!(tc.add.is_empty());
    }

    #[test]
    fn integration_config_with_tools() {
        let config: IntegrationConfig = serde_json::from_value(serde_json::json!({
            "id": "github",
            "type": "github",
            "config": {"app_id": 123, "private_key_file": "/key.pem", "webhook_secret": "s"},
            "tools": {
                "include_core": true,
                "remove": ["file_write"],
                "add": ["github_api.create_branch"]
            }
        }))
        .unwrap();
        let tc = config.tools.unwrap();
        assert!(tc.include_core);
        assert_eq!(tc.remove, vec!["file_write"]);
        assert_eq!(tc.add, vec!["github_api.create_branch"]);
    }

    #[test]
    fn integration_config_without_tools_is_none() {
        let config: IntegrationConfig = serde_json::from_value(serde_json::json!({
            "id": "test",
            "type": "webhook",
            "config": {}
        }))
        .unwrap();
        assert!(config.tools.is_none());
    }

    #[tokio::test]
    async fn effective_tool_config_returns_default_when_no_config() {
        let registry = IntegrationRegistry::new();
        let tc = registry.effective_tool_config("nonexistent").await;
        assert!(tc.include_core);
        assert!(tc.add.is_empty());
    }

    #[tokio::test]
    async fn effective_tool_config_uses_registration_config() {
        let registry = IntegrationRegistry::new();
        let config = IntegrationConfig {
            id: "test-tools".into(),
            provider_type: "webhook".into(),
            config: serde_json::json!({}),
            tools: Some(ToolConfig {
                include_core: true,
                add: vec!["custom_tool".into()],
                remove: vec![],
            }),
        };
        registry.register_from_config(config).await.unwrap();
        let tc = registry.effective_tool_config("test-tools").await;
        assert_eq!(tc.add, vec!["custom_tool"]);
    }

    #[tokio::test]
    async fn effective_tool_config_override_takes_precedence() {
        let registry = IntegrationRegistry::new();
        let config = IntegrationConfig {
            id: "test-override".into(),
            provider_type: "webhook".into(),
            config: serde_json::json!({}),
            tools: Some(ToolConfig {
                include_core: true,
                add: vec!["from_config".into()],
                remove: vec![],
            }),
        };
        registry.register_from_config(config).await.unwrap();
        registry
            .set_overrides(
                "test-override",
                crate::IntegrationOverrides {
                    tools: Some(ToolConfig {
                        include_core: false,
                        add: vec!["from_override".into()],
                        remove: vec!["shell_exec".into()],
                    }),
                    ..Default::default()
                },
            )
            .await;
        let tc = registry.effective_tool_config("test-override").await;
        assert!(!tc.include_core);
        assert_eq!(tc.add, vec!["from_override"]);
        assert_eq!(tc.remove, vec!["shell_exec"]);
    }
}
