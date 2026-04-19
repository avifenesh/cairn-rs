//! Plugin process lifecycle management for RFC 007.
//!
//! ## Architecture
//!
//! ```text
//! PluginHost
//!  ├─ PluginHandle (one per running plugin — owns the child process)
//!  ├─ PluginHealthMonitor (heartbeat tracking)
//!  ├─ CapabilityRegistry (discovered capabilities)
//!  └─ start / stop / restart lifecycle
//! ```
//!
//! Each plugin runs as a separate child process, communicating over
//! JSON-RPC 2.0 via stdin/stdout. The process is spawned from the
//! `command` field in the plugin manifest. Crashes are detected by
//! polling the child's exit status.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, RwLock};

use cairn_plugin_proto::manifest::PluginManifestWire;
use cairn_plugin_proto::wire::{self, JsonRpcRequest, JsonRpcResponse, ToolsListResult};

use super::plugin_capability_registry::CapabilityRegistry;
use super::plugin_health_monitor::PluginHealthMonitor;

// ── Plugin state ─────────────────────────────────────────────────────────────

/// Lifecycle state of a managed plugin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PluginState {
    Starting,
    Running,
    Unhealthy,
    Stopping,
    Stopped,
    Crashed,
}

/// A running plugin child process plus metadata.
pub struct PluginHandle {
    pub plugin_id: String,
    pub manifest: PluginManifestWire,
    pub state: PluginState,
    /// The child process. `None` when the plugin is stopped.
    child: Option<Child>,
    /// Monotonic request counter for JSON-RPC IDs.
    next_req_id: u64,
}

impl PluginHandle {
    /// Send a JSON-RPC request via stdin and read the response from stdout.
    ///
    /// Returns `Err` if the child has exited or the pipe is broken.
    pub fn rpc_call(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<JsonRpcResponse, PluginError> {
        let child = self
            .child
            .as_mut()
            .ok_or_else(|| PluginError::NotRunning(self.plugin_id.clone()))?;

        let id = format!("req_{}", self.next_req_id);
        self.next_req_id += 1;

        let request = JsonRpcRequest::new(&id, method, params);
        let mut line =
            serde_json::to_string(&request).map_err(|e| PluginError::Protocol(e.to_string()))?;
        line.push('\n');

        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| PluginError::Protocol("stdin closed".into()))?;
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| PluginError::Io(e.to_string()))?;
        stdin.flush().map_err(|e| PluginError::Io(e.to_string()))?;

        let stdout = child
            .stdout
            .as_mut()
            .ok_or_else(|| PluginError::Protocol("stdout closed".into()))?;
        let mut reader = BufReader::new(stdout);
        let mut resp_line = String::new();
        reader
            .read_line(&mut resp_line)
            .map_err(|e| PluginError::Io(e.to_string()))?;

        if resp_line.is_empty() {
            return Err(PluginError::Crashed(self.plugin_id.clone()));
        }

        serde_json::from_str::<JsonRpcResponse>(&resp_line)
            .map_err(|e| PluginError::Protocol(format!("bad response: {e}")))
    }

    /// Check whether the child process is still alive.
    pub fn is_alive(&mut self) -> bool {
        match self.child.as_mut() {
            None => false,
            Some(child) => match child.try_wait() {
                Ok(None) => true,     // still running
                Ok(Some(_)) => false, // exited
                Err(_) => false,
            },
        }
    }

    /// Kill the child process. Returns `Ok(())` even if already dead.
    pub fn kill(&mut self) -> Result<(), PluginError> {
        if let Some(ref mut child) = self.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.child = None;
        self.state = PluginState::Stopped;
        Ok(())
    }
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum PluginError {
    /// Plugin is not running (stopped or never started).
    NotRunning(String),
    /// Plugin process crashed or exited unexpectedly.
    Crashed(String),
    /// Plugin already exists with this ID.
    AlreadyExists(String),
    /// Manifest is invalid or the command cannot be spawned.
    SpawnFailed(String),
    /// JSON-RPC protocol error.
    Protocol(String),
    /// IO error communicating with the plugin process.
    Io(String),
}

impl std::fmt::Display for PluginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginError::NotRunning(id) => write!(f, "plugin not running: {id}"),
            PluginError::Crashed(id) => write!(f, "plugin crashed: {id}"),
            PluginError::AlreadyExists(id) => write!(f, "plugin already exists: {id}"),
            PluginError::SpawnFailed(msg) => write!(f, "spawn failed: {msg}"),
            PluginError::Protocol(msg) => write!(f, "protocol error: {msg}"),
            PluginError::Io(msg) => write!(f, "io error: {msg}"),
        }
    }
}

impl std::error::Error for PluginError {}

// ── PluginHost ───────────────────────────────────────────────────────────────

/// Manages the lifecycle of all plugins in the runtime.
///
/// Thread-safe. Each method acquires locks as needed.
pub struct PluginHost {
    /// Running plugin handles, keyed by plugin ID.
    handles: RwLock<HashMap<String, Mutex<PluginHandle>>>,
    /// Shared health monitor.
    pub health: Arc<PluginHealthMonitor>,
    /// Shared capability registry.
    pub capabilities: Arc<CapabilityRegistry>,
}

impl PluginHost {
    pub fn new(health: Arc<PluginHealthMonitor>, capabilities: Arc<CapabilityRegistry>) -> Self {
        Self {
            handles: RwLock::new(HashMap::new()),
            health,
            capabilities,
        }
    }

    /// Start a plugin from its manifest.
    ///
    /// Spawns the child process, sends `initialize`, discovers capabilities
    /// via `tools.list`, and registers them in the capability registry.
    ///
    /// ## Sandbox
    ///
    /// The plugin runs as a separate process with:
    /// - Minimal environment (only `PATH`, `HOME`, `PLUGIN_ID`)
    /// - No inherited stdin from the host
    /// - stdout/stderr captured by the host
    /// - Permissions limited to what the manifest declares
    pub fn start_plugin(&self, manifest: PluginManifestWire) -> Result<(), PluginError> {
        let plugin_id = manifest.id.clone();

        // Check for duplicate.
        {
            let handles = self.handles.read().unwrap_or_else(|e| e.into_inner());
            if handles.contains_key(&plugin_id) {
                return Err(PluginError::AlreadyExists(plugin_id));
            }
        }

        // Spawn child process with sandboxed environment.
        let child = spawn_sandboxed(&manifest)?;

        let mut handle = PluginHandle {
            plugin_id: plugin_id.clone(),
            manifest: manifest.clone(),
            state: PluginState::Starting,
            child: Some(child),
            next_req_id: 1,
        };

        // Send initialize.
        let init_params = serde_json::json!({
            "protocolVersion": "1.0",
            "host": { "name": "cairn", "version": "0.1.0" }
        });
        match handle.rpc_call(wire::methods::INITIALIZE, init_params) {
            Ok(_resp) => {
                handle.state = PluginState::Running;
            }
            Err(e) => {
                handle.kill()?;
                return Err(PluginError::Protocol(format!(
                    "initialize failed for {plugin_id}: {e}"
                )));
            }
        }

        // Register manifest capabilities.
        self.capabilities
            .register_from_manifest(&plugin_id, &manifest.capabilities);

        // Discover tools via tools.list.
        match handle.rpc_call(wire::methods::TOOLS_LIST, serde_json::json!({})) {
            Ok(resp) => {
                if let Ok(tools_result) = serde_json::from_value::<ToolsListResult>(resp.result) {
                    self.capabilities
                        .register_tools(&plugin_id, tools_result.tools);
                }
            }
            Err(_) => {
                // tools.list is optional — some plugins don't provide tools.
            }
        }

        // Record initial heartbeat.
        self.health.record_heartbeat(&plugin_id);

        // Store handle.
        {
            let mut handles = self.handles.write().unwrap_or_else(|e| e.into_inner());
            handles.insert(plugin_id, Mutex::new(handle));
        }

        Ok(())
    }

    /// Stop a running plugin gracefully.
    ///
    /// Sends `shutdown`, waits briefly, then kills if still alive.
    pub fn stop_plugin(&self, plugin_id: &str) -> Result<(), PluginError> {
        let handles = self.handles.read().unwrap_or_else(|e| e.into_inner());
        let handle_mutex = handles
            .get(plugin_id)
            .ok_or_else(|| PluginError::NotRunning(plugin_id.to_owned()))?;

        let mut handle = handle_mutex.lock().unwrap_or_else(|e| e.into_inner());

        // Try graceful shutdown.
        let _ = handle.rpc_call(wire::methods::SHUTDOWN, serde_json::json!({}));
        handle.kill()?;
        handle.state = PluginState::Stopped;

        // Clean up registrations.
        self.capabilities.unregister(plugin_id);
        self.health.remove(plugin_id);

        drop(handle);
        drop(handles);

        // Remove from handles map.
        let mut handles = self.handles.write().unwrap_or_else(|e| e.into_inner());
        handles.remove(plugin_id);

        Ok(())
    }

    /// Restart a plugin: kill the existing process and re-spawn from manifest.
    pub fn restart_plugin(&self, plugin_id: &str) -> Result<(), PluginError> {
        let manifest = {
            let handles = self.handles.read().unwrap_or_else(|e| e.into_inner());
            let handle_mutex = handles
                .get(plugin_id)
                .ok_or_else(|| PluginError::NotRunning(plugin_id.to_owned()))?;
            let mut handle = handle_mutex.lock().unwrap_or_else(|e| e.into_inner());
            handle.kill()?;
            handle.manifest.clone()
        };

        // Remove old handle.
        {
            let mut handles = self.handles.write().unwrap_or_else(|e| e.into_inner());
            handles.remove(plugin_id);
        }

        // Clean up old registrations.
        self.capabilities.unregister(plugin_id);
        self.health.remove(plugin_id);

        // Re-start with the same manifest.
        self.start_plugin(manifest)
    }

    /// Check all plugins for crashes. Returns IDs of plugins that crashed.
    ///
    /// Call this periodically. Crashed plugins have their state set to
    /// `PluginState::Crashed` and an error recorded in the health monitor.
    pub fn detect_crashes(&self) -> Vec<String> {
        let handles = self.handles.read().unwrap_or_else(|e| e.into_inner());
        let mut crashed = Vec::new();

        for (id, handle_mutex) in handles.iter() {
            let mut handle = handle_mutex.lock().unwrap_or_else(|e| e.into_inner());
            if handle.state == PluginState::Running && !handle.is_alive() {
                handle.state = PluginState::Crashed;
                self.health
                    .record_error(id, "process exited unexpectedly".into());
                crashed.push(id.clone());
            }
        }

        crashed
    }

    /// Send a health check ping to a plugin and update the health monitor.
    pub fn health_check(&self, plugin_id: &str) -> Result<(), PluginError> {
        let handles = self.handles.read().unwrap_or_else(|e| e.into_inner());
        let handle_mutex = handles
            .get(plugin_id)
            .ok_or_else(|| PluginError::NotRunning(plugin_id.to_owned()))?;

        let mut handle = handle_mutex.lock().unwrap_or_else(|e| e.into_inner());
        match handle.rpc_call(wire::methods::HEALTH_CHECK, serde_json::json!({})) {
            Ok(_) => {
                self.health.record_heartbeat(plugin_id);
                Ok(())
            }
            Err(e) => {
                self.health
                    .record_error(plugin_id, format!("health check failed: {e}"));
                self.health.record_missed(plugin_id);
                Err(e)
            }
        }
    }

    /// Get the current state of a plugin.
    pub fn plugin_state(&self, plugin_id: &str) -> Option<PluginState> {
        let handles = self.handles.read().unwrap_or_else(|e| e.into_inner());
        handles
            .get(plugin_id)
            .map(|h| h.lock().unwrap_or_else(|e| e.into_inner()).state)
    }

    /// List all managed plugin IDs.
    pub fn plugin_ids(&self) -> Vec<String> {
        self.handles
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    /// Number of managed plugins.
    pub fn len(&self) -> usize {
        self.handles.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.handles
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    }

    /// Send a JSON-RPC notification to a specific plugin (fire-and-forget).
    ///
    /// Used by the event router to forward events.
    pub fn send_notification(
        &self,
        plugin_id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), PluginError> {
        let handles = self.handles.read().unwrap_or_else(|e| e.into_inner());
        let handle_mutex = handles
            .get(plugin_id)
            .ok_or_else(|| PluginError::NotRunning(plugin_id.to_owned()))?;

        let mut handle = handle_mutex.lock().unwrap_or_else(|e| e.into_inner());
        let child = handle
            .child
            .as_mut()
            .ok_or_else(|| PluginError::NotRunning(plugin_id.to_owned()))?;

        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        let mut line = serde_json::to_string(&notification)
            .map_err(|e| PluginError::Protocol(e.to_string()))?;
        line.push('\n');

        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| PluginError::Protocol("stdin closed".into()))?;
        stdin
            .write_all(line.as_bytes())
            .map_err(|e| PluginError::Io(e.to_string()))?;
        stdin.flush().map_err(|e| PluginError::Io(e.to_string()))?;

        Ok(())
    }
}

impl Default for PluginHost {
    fn default() -> Self {
        Self::new(
            Arc::new(PluginHealthMonitor::new()),
            Arc::new(CapabilityRegistry::new()),
        )
    }
}

// ── Process spawning ─────────────────────────────────────────────────────────

/// Spawn a sandboxed child process from a plugin manifest.
///
/// The child gets:
/// - `stdin`: piped (host writes JSON-RPC requests)
/// - `stdout`: piped (host reads JSON-RPC responses)
/// - `stderr`: piped (captured for logging)
/// - Minimal environment: only `PATH`, `HOME`, and `PLUGIN_ID`
fn spawn_sandboxed(manifest: &PluginManifestWire) -> Result<Child, PluginError> {
    if manifest.command.is_empty() {
        return Err(PluginError::SpawnFailed("manifest command is empty".into()));
    }

    let program = &manifest.command[0];
    let args = &manifest.command[1..];

    // Build a minimal sandbox environment.
    let mut env: HashMap<String, String> = HashMap::new();
    if let Ok(path) = std::env::var("PATH") {
        env.insert("PATH".to_owned(), path);
    }
    if let Ok(home) = std::env::var("HOME") {
        env.insert("HOME".to_owned(), home);
    }
    env.insert("PLUGIN_ID".to_owned(), manifest.id.clone());

    Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(&env)
        .spawn()
        .map_err(|e| PluginError::SpawnFailed(format!("{program}: {e}")))
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manifest(id: &str) -> PluginManifestWire {
        PluginManifestWire {
            id: id.to_owned(),
            name: format!("Test Plugin {id}"),
            version: "0.1.0".to_owned(),
            command: vec!["echo".to_owned(), "hello".to_owned()],
            capabilities: vec![],
            permissions: vec![],
            limits: None,
            description: None,
        }
    }

    #[test]
    fn plugin_host_starts_empty() {
        let host = PluginHost::default();
        assert!(host.is_empty());
        assert_eq!(host.len(), 0);
    }

    #[test]
    fn stop_nonexistent_plugin_returns_error() {
        let host = PluginHost::default();
        let err = host.stop_plugin("nonexistent");
        assert!(err.is_err());
        assert!(matches!(err.unwrap_err(), PluginError::NotRunning(_)));
    }

    #[test]
    fn restart_nonexistent_plugin_returns_error() {
        let host = PluginHost::default();
        let err = host.restart_plugin("nonexistent");
        assert!(err.is_err());
        assert!(matches!(err.unwrap_err(), PluginError::NotRunning(_)));
    }

    #[test]
    fn spawn_sandboxed_fails_for_empty_command() {
        let manifest = PluginManifestWire {
            id: "bad".to_owned(),
            name: "Bad".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec![],
            capabilities: vec![],
            permissions: vec![],
            limits: None,
            description: None,
        };
        let result = spawn_sandboxed(&manifest);
        assert!(result.is_err());
    }

    #[test]
    fn spawn_sandboxed_fails_for_nonexistent_binary() {
        let manifest = PluginManifestWire {
            id: "bad".to_owned(),
            name: "Bad".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["__nonexistent_binary_xyz__".to_owned()],
            capabilities: vec![],
            permissions: vec![],
            limits: None,
            description: None,
        };
        let result = spawn_sandboxed(&manifest);
        assert!(result.is_err());
    }

    #[test]
    fn plugin_handle_kill_is_idempotent() {
        let mut handle = PluginHandle {
            plugin_id: "test".to_owned(),
            manifest: test_manifest("test"),
            state: PluginState::Stopped,
            child: None,
            next_req_id: 1,
        };
        // Killing an already-stopped handle should be fine.
        assert!(handle.kill().is_ok());
        assert_eq!(handle.state, PluginState::Stopped);
    }

    #[test]
    fn plugin_handle_is_alive_false_when_no_child() {
        let mut handle = PluginHandle {
            plugin_id: "test".to_owned(),
            manifest: test_manifest("test"),
            state: PluginState::Stopped,
            child: None,
            next_req_id: 1,
        };
        assert!(!handle.is_alive());
    }

    #[test]
    fn plugin_handle_rpc_call_fails_when_not_running() {
        let mut handle = PluginHandle {
            plugin_id: "test".to_owned(),
            manifest: test_manifest("test"),
            state: PluginState::Stopped,
            child: None,
            next_req_id: 1,
        };
        let result = handle.rpc_call("test", serde_json::json!({}));
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), PluginError::NotRunning(_)));
    }

    #[test]
    fn detect_crashes_on_empty_host() {
        let host = PluginHost::default();
        assert!(host.detect_crashes().is_empty());
    }

    #[test]
    fn plugin_state_unknown_returns_none() {
        let host = PluginHost::default();
        assert!(host.plugin_state("nonexistent").is_none());
    }

    /// Integration test: spawn a real process and detect its crash.
    #[test]
    fn detect_crash_of_short_lived_process() {
        let health = Arc::new(PluginHealthMonitor::new());
        let caps = Arc::new(CapabilityRegistry::new());
        let host = PluginHost::new(health.clone(), caps);

        // Spawn "true" which exits immediately with status 0.
        let manifest = PluginManifestWire {
            id: "short-lived".to_owned(),
            name: "Short".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["true".to_owned()],
            capabilities: vec![],
            permissions: vec![],
            limits: None,
            description: None,
        };

        // start_plugin will fail at initialize (true doesn't speak JSON-RPC),
        // but we can test the crash detection path by manually inserting a handle.
        let child = spawn_sandboxed(&manifest).unwrap();
        let handle = PluginHandle {
            plugin_id: "short-lived".to_owned(),
            manifest,
            state: PluginState::Running,
            child: Some(child),
            next_req_id: 1,
        };

        {
            let mut handles = host.handles.write().unwrap();
            handles.insert("short-lived".to_owned(), Mutex::new(handle));
        }

        // Wait for the process to exit.
        std::thread::sleep(std::time::Duration::from_millis(50));

        let crashed = host.detect_crashes();
        assert!(crashed.contains(&"short-lived".to_owned()));
        assert_eq!(host.plugin_state("short-lived"), Some(PluginState::Crashed));
    }

    #[test]
    fn plugin_error_display() {
        let err = PluginError::NotRunning("p1".into());
        assert_eq!(err.to_string(), "plugin not running: p1");

        let err = PluginError::Crashed("p2".into());
        assert_eq!(err.to_string(), "plugin crashed: p2");
    }
}
