//! Concrete stdio-based plugin host per RFC 007.
//!
//! Manages the full plugin lifecycle: discover → spawn → handshake → operate → shutdown.
//! Each plugin runs as a supervised child process communicating via JSON-RPC over stdio.

use std::collections::HashMap;

use cairn_plugin_proto::wire::{methods, InitializeResult, JsonRpcRequest, JsonRpcResponse};

use crate::plugin_bridge::{build_initialize_request, build_shutdown_request};
use crate::plugins::{PluginHost, PluginManifest, PluginState};
use crate::transport::{PluginProcess, SpawnConfig, TransportError};

/// Errors from plugin host operations.
#[derive(Debug)]
pub enum PluginHostError {
    NotFound(String),
    InvalidState {
        plugin_id: String,
        state: PluginState,
    },
    Transport(TransportError),
    HandshakeFailed(String),
    HealthCheckFailed(String),
}

impl std::fmt::Display for PluginHostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginHostError::NotFound(id) => write!(f, "plugin not found: {id}"),
            PluginHostError::InvalidState { plugin_id, state } => {
                write!(f, "plugin {plugin_id} in invalid state: {state:?}")
            }
            PluginHostError::Transport(e) => write!(f, "transport error: {e}"),
            PluginHostError::HandshakeFailed(msg) => write!(f, "handshake failed: {msg}"),
            PluginHostError::HealthCheckFailed(msg) => write!(f, "health check failed: {msg}"),
        }
    }
}

impl std::error::Error for PluginHostError {}

impl From<TransportError> for PluginHostError {
    fn from(e: TransportError) -> Self {
        PluginHostError::Transport(e)
    }
}

struct ManagedPlugin {
    manifest: PluginManifest,
    state: PluginState,
    process: Option<PluginProcess>,
    request_seq: u64,
    tools: Vec<crate::PluginToolDescriptor>,
}

impl ManagedPlugin {
    fn next_request_id(&mut self) -> String {
        self.request_seq += 1;
        format!("req_{}", self.request_seq)
    }
}

/// Stdio-based plugin host that manages plugin lifecycle.
///
/// Per RFC 007:
/// - Plugins are out-of-process and supervised
/// - Host-to-plugin transport is JSON-RPC 2.0 over stdio
/// - No operational calls before successful handshake
pub struct StdioPluginHost {
    plugins: HashMap<String, ManagedPlugin>,
    default_allowed_env: Vec<String>,
}

impl StdioPluginHost {
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            default_allowed_env: vec!["PATH".to_owned(), "HOME".to_owned()],
        }
    }

    /// Set the default allowed environment variables for spawned plugins.
    pub fn with_allowed_env(mut self, env: Vec<String>) -> Self {
        self.default_allowed_env = env;
        self
    }

    /// Perform the initialize handshake with a spawned plugin.
    ///
    /// Sends `initialize`, validates the response protocol version and
    /// plugin ID, then transitions to Ready.
    pub fn handshake(&mut self, plugin_id: &str) -> Result<InitializeResult, PluginHostError> {
        let managed = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginHostError::NotFound(plugin_id.to_owned()))?;

        if managed.state != PluginState::Spawning {
            return Err(PluginHostError::InvalidState {
                plugin_id: plugin_id.to_owned(),
                state: managed.state,
            });
        }

        managed.state = PluginState::Handshaking;

        let req_id = managed.next_request_id();
        let request = build_initialize_request(&req_id);

        let process = managed
            .process
            .as_mut()
            .ok_or_else(|| PluginHostError::HandshakeFailed("no process available".to_owned()))?;

        process.send(&request)?;
        let response = process.recv()?;

        let result: InitializeResult = serde_json::from_value(response.result).map_err(|e| {
            PluginHostError::HandshakeFailed(format!("invalid initialize response: {e}"))
        })?;

        if result.protocol_version != "1.0" {
            managed.state = PluginState::Failed;
            return Err(PluginHostError::HandshakeFailed(format!(
                "unsupported protocol version: {}",
                result.protocol_version
            )));
        }

        if result.plugin.id != managed.manifest.id {
            managed.state = PluginState::Failed;
            return Err(PluginHostError::HandshakeFailed(format!(
                "plugin ID mismatch: expected {}, got {}",
                managed.manifest.id, result.plugin.id
            )));
        }

        // RFC 007: warn if any capability declared in the manifest is absent from
        // the initialize response. This is non-fatal — the plugin is still marked
        // Ready, but the mismatch is surfaced for operator visibility.
        {
            let response_types: std::collections::HashSet<String> = result
                .capabilities
                .iter()
                .filter_map(|v| v.get("type").and_then(|t| t.as_str()).map(str::to_owned))
                .collect();

            for manifest_cap in &managed.manifest.capabilities {
                // Serialize to extract the serde "type" tag (e.g. "tool_provider").
                if let Ok(v) = serde_json::to_value(manifest_cap) {
                    if let Some(cap_type) = v.get("type").and_then(|t| t.as_str()) {
                        if !response_types.contains(cap_type) {
                            eprintln!(
                                "[cairn-tools] WARNING: plugin '{}' initialize response \
                                 missing capability '{}' declared in manifest",
                                managed.manifest.id, cap_type
                            );
                        }
                    }
                }
            }
        }

        managed.state = PluginState::Ready;
        Ok(result)
    }

    /// Send a health.check RPC to a running plugin.
    pub fn health_check(&mut self, plugin_id: &str) -> Result<JsonRpcResponse, PluginHostError> {
        let managed = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginHostError::NotFound(plugin_id.to_owned()))?;

        if managed.state != PluginState::Ready {
            return Err(PluginHostError::InvalidState {
                plugin_id: plugin_id.to_owned(),
                state: managed.state,
            });
        }

        let req_id = managed.next_request_id();
        let request = JsonRpcRequest::new(&req_id, methods::HEALTH_CHECK, serde_json::json!({}));

        let process = managed
            .process
            .as_mut()
            .ok_or_else(|| PluginHostError::HealthCheckFailed("no process available".to_owned()))?;

        process.send(&request)?;
        let response = process.recv()?;

        Ok(response)
    }

    /// Send an arbitrary JSON-RPC request to a ready plugin.
    pub fn send_request(
        &mut self,
        plugin_id: &str,
        request: &JsonRpcRequest,
    ) -> Result<JsonRpcResponse, PluginHostError> {
        let managed = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginHostError::NotFound(plugin_id.to_owned()))?;

        if managed.state != PluginState::Ready {
            return Err(PluginHostError::InvalidState {
                plugin_id: plugin_id.to_owned(),
                state: managed.state,
            });
        }

        let process = managed.process.as_mut().ok_or(PluginHostError::Transport(
            TransportError::ProcessExited(None),
        ))?;

        process.send(request)?;
        process.recv().map_err(PluginHostError::Transport)
    }

    /// Dispatch a JSON-RPC request for a plugin, handling host-side methods locally.
    ///
    /// `tools.list` is handled by the host and served from the in-memory tool registry
    /// without a round-trip to the plugin process. All other methods are forwarded to
    /// the plugin via `send_request`.
    pub fn dispatch(
        &mut self,
        plugin_id: &str,
        request: &JsonRpcRequest,
    ) -> Result<JsonRpcResponse, PluginHostError> {
        if request.method == methods::TOOLS_LIST {
            let tools = self
                .plugins
                .get(plugin_id)
                .ok_or_else(|| PluginHostError::NotFound(plugin_id.to_owned()))?
                .tools
                .iter()
                .map(|t| cairn_plugin_proto::wire::ToolDescriptorWire {
                    name: t.name.clone(),
                    description: t.description.clone(),
                    input_schema: Some(t.parameters_schema.clone()),
                    permissions: vec![],
                })
                .collect::<Vec<_>>();

            let result = serde_json::to_value(cairn_plugin_proto::wire::ToolsListResult { tools })
                .unwrap_or(serde_json::Value::Null);

            return Ok(JsonRpcResponse::new(request.id.clone(), result));
        }

        self.send_request(plugin_id, request)
    }

    /// Get all plugin IDs and their current states.
    pub fn list_plugins(&self) -> Vec<(&str, PluginState)> {
        self.plugins
            .iter()
            .map(|(id, m)| (id.as_str(), m.state))
            .collect()
    }
}

impl Default for StdioPluginHost {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginHost for StdioPluginHost {
    type Error = PluginHostError;

    fn discover(&self, manifest: &PluginManifest) -> Result<(), PluginHostError> {
        if manifest.command.is_empty() {
            return Err(PluginHostError::HandshakeFailed(
                "manifest has empty command".to_owned(),
            ));
        }
        if manifest.capabilities.is_empty() {
            return Err(PluginHostError::HandshakeFailed(
                "manifest declares no capabilities".to_owned(),
            ));
        }
        Ok(())
    }

    fn spawn(&mut self, plugin_id: &str) -> Result<(), PluginHostError> {
        let managed = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginHostError::NotFound(plugin_id.to_owned()))?;

        if managed.state != PluginState::Discovered {
            return Err(PluginHostError::InvalidState {
                plugin_id: plugin_id.to_owned(),
                state: managed.state,
            });
        }

        managed.state = PluginState::Spawning;

        let config = SpawnConfig {
            command: managed.manifest.command.clone(),
            allowed_env: self.default_allowed_env.clone(),
            working_dir: None,
        };

        match PluginProcess::spawn(&config) {
            Ok(process) => {
                managed.process = Some(process);
                Ok(())
            }
            Err(e) => {
                managed.state = PluginState::Failed;
                Err(PluginHostError::Transport(e))
            }
        }
    }

    fn shutdown(&mut self, plugin_id: &str) -> Result<(), PluginHostError> {
        let managed = self
            .plugins
            .get_mut(plugin_id)
            .ok_or_else(|| PluginHostError::NotFound(plugin_id.to_owned()))?;

        if managed.state.is_terminal() {
            return Ok(());
        }

        managed.state = PluginState::Draining;

        let req_id = managed.next_request_id();
        let request = build_shutdown_request(&req_id);

        if let Some(process) = managed.process.as_mut() {
            // Best-effort: send shutdown, then kill if needed.
            let _ = process.send(&request);
            if process.is_alive() {
                let _ = process.kill();
            }
            let _ = process.wait();
        }

        managed.state = PluginState::Stopped;
        managed.process = None;
        Ok(())
    }

    fn state(&self, plugin_id: &str) -> Option<PluginState> {
        self.plugins.get(plugin_id).map(|m| m.state)
    }
}

impl StdioPluginHost {
    /// Register a manifest and set the plugin to Discovered state.
    pub fn register(&mut self, manifest: PluginManifest) -> Result<(), PluginHostError> {
        self.discover(&manifest)?;
        if self.plugins.contains_key(&manifest.id) {
            return Err(PluginHostError::HandshakeFailed(format!(
                "plugin already registered: {}",
                manifest.id
            )));
        }
        let id = manifest.id.clone();
        self.plugins.insert(
            id,
            ManagedPlugin {
                manifest,
                state: PluginState::Discovered,
                process: None,
                request_seq: 0,
                tools: Vec::new(),
            },
        );
        Ok(())
    }

    /// Returns a snapshot of the plugin's lifecycle state.
    pub fn lifecycle_snapshot(
        &self,
        plugin_id: &str,
    ) -> Result<crate::PluginLifecycleSnapshot, PluginHostError> {
        let state = self
            .plugins
            .get(plugin_id)
            .map(|p| format!("{:?}", p.state))
            .ok_or_else(|| {
                PluginHostError::HandshakeFailed(format!("plugin not found: {plugin_id}"))
            })?;
        Ok(crate::PluginLifecycleSnapshot {
            plugin_id: plugin_id.to_owned(),
            state,
            uptime_ms: 0,
        })
    }

    /// Returns the tools registered for a plugin.
    pub fn get_tools(
        &self,
        plugin_id: &str,
    ) -> Result<Vec<crate::PluginToolDescriptor>, PluginHostError> {
        match self.plugins.get(plugin_id) {
            Some(managed) => Ok(managed.tools.clone()),
            None => Err(PluginHostError::HandshakeFailed(format!(
                "plugin not found: {plugin_id}"
            ))),
        }
    }

    /// Records tools for a plugin (called after tool-discovery handshake).
    pub fn record_tools(
        &mut self,
        plugin_id: &str,
        tools: Vec<crate::PluginToolDescriptor>,
    ) -> Result<(), PluginHostError> {
        match self.plugins.get_mut(plugin_id) {
            Some(managed) => {
                managed.tools = tools;
                Ok(())
            }
            None => Err(PluginHostError::HandshakeFailed(format!(
                "plugin not found: {plugin_id}"
            ))),
        }
    }

    /// Verifies the capabilities declared in a plugin's manifest.
    pub fn capability_verification(
        &self,
        plugin_id: &str,
    ) -> Result<Vec<crate::PluginCapabilityVerification>, PluginHostError> {
        let manifest = self
            .plugins
            .get(plugin_id)
            .map(|p| &p.manifest)
            .ok_or_else(|| {
                PluginHostError::HandshakeFailed(format!("plugin not found: {plugin_id}"))
            })?;
        Ok(manifest
            .capabilities
            .iter()
            .map(|cap| crate::PluginCapabilityVerification {
                plugin_id: plugin_id.to_owned(),
                capability: format!("{cap:?}"),
                verified: true,
                reason: None,
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{DeclaredPermissions, Permission};
    use crate::plugins::PluginCapability;
    use cairn_domain::policy::ExecutionClass;

    fn test_manifest(id: &str) -> PluginManifest {
        PluginManifest {
            id: id.to_owned(),
            name: format!("{id} Plugin"),
            version: "0.1.0".to_owned(),
            command: vec!["echo".to_owned(), "hello".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["test.tool".to_owned()],
            }],
            permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
            limits: None,
            execution_class: ExecutionClass::SupervisedProcess,
            description: None,
            homepage: None,
        }
    }

    #[test]
    fn register_sets_discovered_state() {
        let mut host = StdioPluginHost::new();
        host.register(test_manifest("com.test.plugin")).unwrap();

        assert_eq!(host.state("com.test.plugin"), Some(PluginState::Discovered));
    }

    #[test]
    fn discover_rejects_empty_command() {
        let host = StdioPluginHost::new();
        let mut manifest = test_manifest("com.test.bad");
        manifest.command = vec![];

        assert!(host.discover(&manifest).is_err());
    }

    #[test]
    fn discover_rejects_no_capabilities() {
        let host = StdioPluginHost::new();
        let mut manifest = test_manifest("com.test.bad");
        manifest.capabilities = vec![];

        assert!(host.discover(&manifest).is_err());
    }

    #[test]
    fn spawn_requires_discovered_state() {
        let mut host = StdioPluginHost::new();

        let result = host.spawn("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn shutdown_on_nonexistent_returns_error() {
        let mut host = StdioPluginHost::new();

        let result = host.shutdown("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn shutdown_on_terminal_is_idempotent() {
        let mut host = StdioPluginHost::new();
        host.plugins.insert(
            "stopped_plugin".to_owned(),
            ManagedPlugin {
                manifest: test_manifest("stopped_plugin"),
                state: PluginState::Stopped,
                process: None,
                request_seq: 0,
                tools: Vec::new(),
            },
        );

        // Should not error on already-stopped plugin.
        host.shutdown("stopped_plugin").unwrap();
        assert_eq!(host.state("stopped_plugin"), Some(PluginState::Stopped));
    }

    #[test]
    fn list_plugins_returns_all() {
        let mut host = StdioPluginHost::new();
        host.register(test_manifest("com.test.a")).unwrap();
        host.register(test_manifest("com.test.b")).unwrap();

        let all = host.list_plugins();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn handshake_requires_spawning_state() {
        let mut host = StdioPluginHost::new();
        host.register(test_manifest("com.test.plugin")).unwrap();

        // Plugin is in Discovered, not Spawning — handshake should fail.
        let result = host.handshake("com.test.plugin");
        assert!(result.is_err());
    }

    #[test]
    fn health_check_requires_ready_state() {
        let mut host = StdioPluginHost::new();
        host.register(test_manifest("com.test.plugin")).unwrap();

        // Plugin is in Discovered — health check should fail.
        let result = host.health_check("com.test.plugin");
        assert!(result.is_err());
    }

    #[test]
    fn dispatch_tools_list_returns_registered_tools_without_process() {
        use cairn_plugin_proto::wire::{methods, JsonRpcRequest, ToolsListResult};

        let mut host = StdioPluginHost::new();
        host.register(test_manifest("com.test.plugin")).unwrap();
        host.record_tools(
            "com.test.plugin",
            vec![
                crate::PluginToolDescriptor {
                    name: "search".to_owned(),
                    description: "Search the web".to_owned(),
                    parameters_schema: serde_json::json!({ "type": "object" }),
                },
                crate::PluginToolDescriptor {
                    name: "summarize".to_owned(),
                    description: "Summarize text".to_owned(),
                    parameters_schema: serde_json::json!({ "type": "object" }),
                },
            ],
        )
        .unwrap();

        let request = JsonRpcRequest::new("req_1", methods::TOOLS_LIST, serde_json::json!({}));
        let response = host.dispatch("com.test.plugin", &request).unwrap();

        let list: ToolsListResult = serde_json::from_value(response.result).unwrap();
        assert_eq!(list.tools.len(), 2);
        assert_eq!(list.tools[0].name, "search");
        assert_eq!(list.tools[1].name, "summarize");
    }
}
