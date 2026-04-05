use cairn_domain::policy::ExecutionClass;
use serde::{Deserialize, Serialize};

use crate::mcp_client::McpEndpoint;
use crate::permissions::DeclaredPermissions;

/// Plugin capability families per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PluginCapability {
    ToolProvider { tools: Vec<String> },
    SignalSource { signals: Vec<String> },
    ChannelProvider { channels: Vec<String> },
    PostTurnHook,
    PolicyHook,
    EvalScorer,
    /// This plugin connects to an external MCP server.
    ///
    /// When registered, cairn-tools will connect to the server via `McpClient`
    /// and expose its tools under the `mcp.<server_id>.<tool>` namespace.
    McpServer { endpoint: McpEndpoint },
}

/// Concurrency and timeout limits declared in the plugin manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginLimits {
    pub max_concurrency: Option<u32>,
    pub default_timeout_ms: Option<u64>,
}

/// Declarative plugin manifest loaded before process spawn.
/// Shape follows RFC 007 canonical manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub command: Vec<String>,
    pub capabilities: Vec<PluginCapability>,
    pub permissions: DeclaredPermissions,
    pub limits: Option<PluginLimits>,
    pub execution_class: ExecutionClass,
    /// RFC 007: human-readable description of what the plugin does.
    #[serde(default)]
    pub description: Option<String>,
    /// RFC 007: URL for plugin documentation or source repository.
    #[serde(default)]
    pub homepage: Option<String>,
}

/// Plugin lifecycle states managed by the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginState {
    Discovered,
    Spawning,
    Handshaking,
    Ready,
    Draining,
    Stopped,
    Failed,
}

impl PluginState {
    pub fn is_operational(self) -> bool {
        matches!(self, PluginState::Ready)
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, PluginState::Stopped | PluginState::Failed)
    }
}

/// Seam for plugin host lifecycle management.
/// The host discovers, spawns, handshakes, and shuts down plugins.
pub trait PluginHost {
    type Error;

    fn discover(&self, manifest: &PluginManifest) -> Result<(), Self::Error>;
    fn spawn(&mut self, plugin_id: &str) -> Result<(), Self::Error>;
    fn shutdown(&mut self, plugin_id: &str) -> Result<(), Self::Error>;
    fn state(&self, plugin_id: &str) -> Option<PluginState>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{DeclaredPermissions, Permission};
    use cairn_domain::policy::ExecutionClass;

    #[test]
    fn manifest_carries_capabilities_and_permissions() {
        let manifest = PluginManifest {
            id: "com.example.git-tools".to_owned(),
            name: "Git Tools".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["plugin-binary".to_owned(), "--serve".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["git.status".to_owned(), "git.diff".to_owned()],
            }],
            permissions: DeclaredPermissions::new(vec![
                Permission::FsRead,
                Permission::ProcessExec,
            ]),
            limits: Some(PluginLimits {
                max_concurrency: Some(4),
                default_timeout_ms: Some(30_000),
            }),
            execution_class: ExecutionClass::SupervisedProcess,
            description: None,
            homepage: None,
        };

        assert_eq!(manifest.capabilities.len(), 1);
        assert!(manifest.permissions.contains(&Permission::FsRead));
        assert_eq!(manifest.execution_class, ExecutionClass::SupervisedProcess);
    }

    #[test]
    fn plugin_state_lifecycle() {
        assert!(!PluginState::Discovered.is_operational());
        assert!(!PluginState::Spawning.is_operational());
        assert!(PluginState::Ready.is_operational());
        assert!(PluginState::Stopped.is_terminal());
        assert!(PluginState::Failed.is_terminal());
        assert!(!PluginState::Draining.is_terminal());
    }

    #[test]
    fn multi_capability_manifest() {
        let manifest = PluginManifest {
            id: "com.example.multi".to_owned(),
            name: "Multi Plugin".to_owned(),
            version: "0.2.0".to_owned(),
            command: vec!["multi-plugin".to_owned()],
            capabilities: vec![
                PluginCapability::ToolProvider {
                    tools: vec!["tool.a".to_owned()],
                },
                PluginCapability::PolicyHook,
                PluginCapability::EvalScorer,
            ],
            permissions: DeclaredPermissions::default(),
            limits: None,
            execution_class: ExecutionClass::SandboxedProcess,
            description: None,
            homepage: None,
        };

        assert_eq!(manifest.capabilities.len(), 3);
        assert_eq!(manifest.execution_class, ExecutionClass::SandboxedProcess);
    }
}
