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
    InvalidState { plugin_id: String, state: PluginState },
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

        let process = managed.process.as_mut().ok_or_else(|| {
            PluginHostError::HandshakeFailed("no process available".to_owned())
        })?;

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

        let process = managed.process.as_mut().ok_or_else(|| {
            PluginHostError::HealthCheckFailed("no process available".to_owned())
        })?;

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

        let process = managed.process.as_mut().ok_or_else(|| {
            PluginHostError::Transport(TransportError::ProcessExited(None))
        })?;

        process.send(request)?;
        process.recv().map_err(PluginHostError::Transport)
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
        let id = manifest.id.clone();
        self.plugins.insert(
            id,
            ManagedPlugin {
                manifest,
                state: PluginState::Discovered,
                process: None,
                request_seq: 0,
            },
        );
        Ok(())
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
        }
    }

    #[test]
    fn register_sets_discovered_state() {
        let mut host = StdioPluginHost::new();
        host.register(test_manifest("com.test.plugin")).unwrap();

        assert_eq!(
            host.state("com.test.plugin"),
            Some(PluginState::Discovered)
        );
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
}
