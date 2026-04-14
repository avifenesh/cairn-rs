//! End-to-end plugin execution pipeline.
//!
//! Bridges plugin registry, permission gate, concurrency limits, and
//! stdio transport into a single invocation flow for Plugin targets.

use std::sync::atomic::{AtomicU64, Ordering};

use cairn_domain::tenancy::ProjectKey;
use cairn_plugin_proto::wire::ToolsInvokeResult;

use crate::builtin::ToolOutcome;
use crate::permissions::{PermissionCheckResult, PermissionGate};
use crate::plugin_bridge::{build_tools_invoke_request, invoke_result_to_outcome};
use crate::plugins::PluginManifest;
use crate::registry::PluginRegistry;
use crate::transport::{PluginProcess, SpawnConfig};

/// Errors from plugin execution.
#[derive(Debug)]
pub enum PluginExecutionError {
    PluginNotFound(String),
    PermissionDenied(String),
    HeldForApproval(String),
    SpawnFailed(String),
    CommunicationFailed(String),
    ConcurrencyExceeded { plugin_id: String, max: u32 },
}

impl std::fmt::Display for PluginExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PluginExecutionError::PluginNotFound(id) => write!(f, "plugin not found: {id}"),
            PluginExecutionError::PermissionDenied(r) => write!(f, "permission denied: {r}"),
            PluginExecutionError::HeldForApproval(r) => write!(f, "held for approval: {r}"),
            PluginExecutionError::SpawnFailed(e) => write!(f, "plugin spawn failed: {e}"),
            PluginExecutionError::CommunicationFailed(e) => write!(f, "plugin comm failed: {e}"),
            PluginExecutionError::ConcurrencyExceeded { plugin_id, max } => {
                write!(f, "concurrency exceeded for {plugin_id}: max {max}")
            }
        }
    }
}

impl std::error::Error for PluginExecutionError {}

/// Tracks in-flight invocations per plugin for concurrency enforcement.
pub struct ConcurrencyTracker {
    counts: std::sync::Mutex<std::collections::HashMap<String, u32>>,
}

impl ConcurrencyTracker {
    pub fn new() -> Self {
        Self {
            counts: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    /// Try to acquire a concurrency slot. Returns false if at max.
    pub fn try_acquire(&self, plugin_id: &str, max: u32) -> bool {
        let mut counts = self.counts.lock().unwrap();
        let count = counts.entry(plugin_id.to_owned()).or_insert(0);
        if *count >= max {
            return false;
        }
        *count += 1;
        true
    }

    /// Release a concurrency slot.
    pub fn release(&self, plugin_id: &str) {
        let mut counts = self.counts.lock().unwrap();
        if let Some(count) = counts.get_mut(plugin_id) {
            *count = count.saturating_sub(1);
        }
    }

    /// Get current count for a plugin.
    pub fn current(&self, plugin_id: &str) -> u32 {
        let counts = self.counts.lock().unwrap();
        counts.get(plugin_id).copied().unwrap_or(0)
    }
}

impl Default for ConcurrencyTracker {
    fn default() -> Self {
        Self::new()
    }
}

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_request_id() -> String {
    let n = REQUEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    format!("rpc_{n}")
}

/// Execute a tool through a plugin via stdio transport.
///
/// Full pipeline:
/// 1. Look up plugin manifest in registry
/// 2. Check permissions via gate
/// 3. Acquire concurrency slot
/// 4. Spawn plugin process
/// 5. Send tools.invoke RPC
/// 6. Parse response
/// 7. Release concurrency slot
pub fn execute_plugin_tool<G: PermissionGate, R: PluginRegistry>(
    registry: &R,
    gate: &G,
    concurrency: &ConcurrencyTracker,
    plugin_id: &str,
    invocation_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    project: &ProjectKey,
) -> Result<ToolOutcome, PluginExecutionError> {
    // 1. Look up manifest.
    let manifest = registry
        .get(plugin_id)
        .ok_or_else(|| PluginExecutionError::PluginNotFound(plugin_id.to_owned()))?;

    // 2. Check permissions.
    let check = gate.check(project, &manifest.permissions, manifest.execution_class);
    match check {
        PermissionCheckResult::Granted(_) => {}
        PermissionCheckResult::Denied(verdict) => {
            return Err(PluginExecutionError::PermissionDenied(
                verdict.reason.unwrap_or_default(),
            ));
        }
        PermissionCheckResult::HeldForApproval(verdict) => {
            return Err(PluginExecutionError::HeldForApproval(
                verdict.reason.unwrap_or_default(),
            ));
        }
    }

    // 3. Acquire concurrency slot.
    let max_concurrency = manifest
        .limits
        .as_ref()
        .and_then(|l| l.max_concurrency)
        .unwrap_or(8);
    if !concurrency.try_acquire(plugin_id, max_concurrency) {
        return Err(PluginExecutionError::ConcurrencyExceeded {
            plugin_id: plugin_id.to_owned(),
            max: max_concurrency,
        });
    }

    // 4-6: Spawn, send, receive (wrapped in a guard to ensure release).
    let result = execute_with_transport(&manifest, invocation_id, tool_name, input, project);

    // 7. Release concurrency slot.
    concurrency.release(plugin_id);

    result
}

fn execute_with_transport(
    manifest: &PluginManifest,
    invocation_id: &str,
    tool_name: &str,
    input: serde_json::Value,
    project: &ProjectKey,
) -> Result<ToolOutcome, PluginExecutionError> {
    // Spawn plugin process.
    let config = SpawnConfig {
        command: manifest.command.clone(),
        allowed_env: vec!["PATH".to_owned(), "HOME".to_owned()],
        working_dir: None,
    };

    let mut process = PluginProcess::spawn(&config)
        .map_err(|e| PluginExecutionError::SpawnFailed(e.to_string()))?;

    // Send tools.invoke request.
    let request = build_tools_invoke_request(
        &next_request_id(),
        invocation_id,
        tool_name,
        input,
        project,
        &[],
    );

    process
        .send(&request)
        .map_err(|e| PluginExecutionError::CommunicationFailed(e.to_string()))?;

    // Read response.
    let response = process
        .recv()
        .map_err(|e| PluginExecutionError::CommunicationFailed(e.to_string()))?;

    // Kill the process (fire-and-forget; real impl would send shutdown first).
    let _ = process.kill();

    // Parse result.
    let invoke_result: ToolsInvokeResult = serde_json::from_value(response.result)
        .map_err(|e| PluginExecutionError::CommunicationFailed(e.to_string()))?;

    Ok(invoke_result_to_outcome(&invoke_result))
}

/// Notification types received from plugins.
#[derive(Clone, Debug)]
pub enum PluginNotification {
    Log {
        invocation_id: String,
        level: String,
        message: String,
    },
    Progress {
        invocation_id: String,
        message: String,
        percent: Option<u32>,
    },
    Event {
        invocation_id: String,
        event_type: String,
        payload: serde_json::Value,
    },
}

/// Per-invocation progress state, updated by `progress.update` notifications.
pub struct ProgressStore {
    /// Last reported progress percent per invocation ID (0–100).
    percentages: std::collections::HashMap<String, u32>,
}

impl ProgressStore {
    pub fn new() -> Self {
        Self {
            percentages: std::collections::HashMap::new(),
        }
    }

    /// Record a progress update for an invocation.
    pub fn set(&mut self, invocation_id: impl Into<String>, percent: u32) {
        self.percentages.insert(invocation_id.into(), percent);
    }

    /// Return the last reported percent for an invocation, if any.
    pub fn get(&self, invocation_id: &str) -> Option<u32> {
        self.percentages.get(invocation_id).copied()
    }
}

impl Default for ProgressStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Consume a parsed notification: print logs to stderr and store progress updates.
///
/// `plugin_name` is used as a label in log lines.  `progress` is updated
/// in-place for `Progress` notifications so callers can query current state.
/// `Event` notifications are currently a no-op — the caller can inspect
/// `PluginNotification::Event` directly if needed.
pub fn handle_notification(
    plugin_name: &str,
    notification: &PluginNotification,
    progress: &mut ProgressStore,
) {
    match notification {
        PluginNotification::Log { level, message, .. } => {
            eprintln!("[plugin:{plugin_name}] {level}: {message}");
        }
        PluginNotification::Progress {
            invocation_id,
            percent,
            message,
        } => {
            if let Some(pct) = percent {
                progress.set(invocation_id.clone(), *pct);
            }
            let pct_str = percent.map(|p| format!(" ({p}%)")).unwrap_or_default();
            eprintln!("[plugin:{plugin_name}] progress{pct_str}: {message}");
        }
        PluginNotification::Event { .. } => {
            // Callers that need event.emit payloads inspect the enum directly.
        }
    }
}

/// Parse a JSON-RPC notification into a typed PluginNotification.
pub fn parse_notification(method: &str, params: &serde_json::Value) -> Option<PluginNotification> {
    match method {
        "log.emit" => Some(PluginNotification::Log {
            invocation_id: params["invocationId"].as_str().unwrap_or("").to_owned(),
            level: params["level"].as_str().unwrap_or("info").to_owned(),
            message: params["message"].as_str().unwrap_or("").to_owned(),
        }),
        "progress.update" => Some(PluginNotification::Progress {
            invocation_id: params["invocationId"].as_str().unwrap_or("").to_owned(),
            message: params["message"].as_str().unwrap_or("").to_owned(),
            percent: params["percent"].as_u64().map(|v| v as u32),
        }),
        "event.emit" => Some(PluginNotification::Event {
            invocation_id: params["invocationId"].as_str().unwrap_or("").to_owned(),
            event_type: params["type"].as_str().unwrap_or("").to_owned(),
            payload: params["payload"].clone(),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{DeclaredPermissions, InvocationGrants, Permission};
    use crate::plugins::{PluginCapability, PluginLimits, PluginManifest};
    use crate::registry::InMemoryPluginRegistry;
    use cairn_domain::policy::ExecutionClass;

    fn test_manifest() -> PluginManifest {
        PluginManifest {
            id: "com.test.plugin".to_owned(),
            name: "Test Plugin".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["nonexistent-binary".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["test.tool".to_owned()],
            }],
            permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
            limits: Some(PluginLimits {
                max_concurrency: Some(2),
                default_timeout_ms: Some(5000),
            }),
            execution_class: ExecutionClass::SupervisedProcess,
            description: None,
            homepage: None,
        }
    }

    struct AllowGate;
    impl PermissionGate for AllowGate {
        fn check(
            &self,
            project: &ProjectKey,
            _: &DeclaredPermissions,
            ec: ExecutionClass,
        ) -> PermissionCheckResult {
            PermissionCheckResult::Granted(InvocationGrants {
                project: project.clone(),
                execution_class: ec,
                granted: vec![],
            })
        }
    }

    struct DenyGate;
    impl PermissionGate for DenyGate {
        fn check(
            &self,
            _: &ProjectKey,
            _: &DeclaredPermissions,
            _: ExecutionClass,
        ) -> PermissionCheckResult {
            PermissionCheckResult::Denied(cairn_domain::policy::PolicyVerdict::deny("blocked"))
        }
    }

    #[test]
    fn plugin_not_found_returns_error() {
        let registry = InMemoryPluginRegistry::new();
        let gate = AllowGate;
        let concurrency = ConcurrencyTracker::new();
        let project = ProjectKey::new("t", "w", "p");

        let result = execute_plugin_tool(
            &registry,
            &gate,
            &concurrency,
            "nonexistent",
            "inv_1",
            "tool",
            serde_json::json!({}),
            &project,
        );
        assert!(matches!(
            result,
            Err(PluginExecutionError::PluginNotFound(_))
        ));
    }

    #[test]
    fn permission_denied_returns_error() {
        let registry = InMemoryPluginRegistry::new();
        registry.register(test_manifest()).unwrap();
        let gate = DenyGate;
        let concurrency = ConcurrencyTracker::new();
        let project = ProjectKey::new("t", "w", "p");

        let result = execute_plugin_tool(
            &registry,
            &gate,
            &concurrency,
            "com.test.plugin",
            "inv_2",
            "test.tool",
            serde_json::json!({}),
            &project,
        );
        assert!(matches!(
            result,
            Err(PluginExecutionError::PermissionDenied(_))
        ));
    }

    #[test]
    fn concurrency_exceeded_returns_error() {
        let registry = InMemoryPluginRegistry::new();
        registry.register(test_manifest()).unwrap();
        let gate = AllowGate;
        let concurrency = ConcurrencyTracker::new();

        // Manually fill up the concurrency slots (max=2).
        assert!(concurrency.try_acquire("com.test.plugin", 2));
        assert!(concurrency.try_acquire("com.test.plugin", 2));
        assert!(!concurrency.try_acquire("com.test.plugin", 2));

        let project = ProjectKey::new("t", "w", "p");
        let result = execute_plugin_tool(
            &registry,
            &gate,
            &concurrency,
            "com.test.plugin",
            "inv_3",
            "test.tool",
            serde_json::json!({}),
            &project,
        );
        assert!(matches!(
            result,
            Err(PluginExecutionError::ConcurrencyExceeded { .. })
        ));
    }

    #[test]
    fn concurrency_release_frees_slot() {
        let tracker = ConcurrencyTracker::new();
        assert!(tracker.try_acquire("p1", 1));
        assert!(!tracker.try_acquire("p1", 1));
        tracker.release("p1");
        assert!(tracker.try_acquire("p1", 1));
    }

    #[test]
    fn spawn_failure_returns_error() {
        let registry = InMemoryPluginRegistry::new();
        registry.register(test_manifest()).unwrap();
        let gate = AllowGate;
        let concurrency = ConcurrencyTracker::new();
        let project = ProjectKey::new("t", "w", "p");

        // Binary doesn't exist — spawn should fail.
        let result = execute_plugin_tool(
            &registry,
            &gate,
            &concurrency,
            "com.test.plugin",
            "inv_4",
            "test.tool",
            serde_json::json!({}),
            &project,
        );
        assert!(matches!(result, Err(PluginExecutionError::SpawnFailed(_))));

        // Concurrency slot should be released even after failure.
        assert_eq!(concurrency.current("com.test.plugin"), 0);
    }

    #[test]
    fn parse_log_notification() {
        let params = serde_json::json!({
            "invocationId": "inv_1",
            "level": "info",
            "message": "cloned repo"
        });
        let notif = parse_notification("log.emit", &params).unwrap();
        match notif {
            PluginNotification::Log {
                invocation_id,
                level,
                message,
            } => {
                assert_eq!(invocation_id, "inv_1");
                assert_eq!(level, "info");
                assert_eq!(message, "cloned repo");
            }
            _ => panic!("expected Log"),
        }
    }

    #[test]
    fn parse_progress_notification() {
        let params = serde_json::json!({
            "invocationId": "inv_2",
            "message": "50% done",
            "percent": 50
        });
        let notif = parse_notification("progress.update", &params).unwrap();
        match notif {
            PluginNotification::Progress { percent, .. } => {
                assert_eq!(percent, Some(50));
            }
            _ => panic!("expected Progress"),
        }
    }

    #[test]
    fn parse_event_notification() {
        let params = serde_json::json!({
            "invocationId": "inv_3",
            "type": "signal.discovered",
            "payload": {"key": "value"}
        });
        let notif = parse_notification("event.emit", &params).unwrap();
        match notif {
            PluginNotification::Event {
                event_type,
                payload,
                ..
            } => {
                assert_eq!(event_type, "signal.discovered");
                assert_eq!(payload["key"], "value");
            }
            _ => panic!("expected Event"),
        }
    }

    #[test]
    fn parse_unknown_notification_returns_none() {
        let params = serde_json::json!({});
        assert!(parse_notification("unknown.method", &params).is_none());
    }

    #[test]
    fn handle_log_notification_prints_to_stderr() {
        // Just verify it doesn't panic and the notification is consumed.
        let notif = PluginNotification::Log {
            invocation_id: "inv_1".to_owned(),
            level: "warn".to_owned(),
            message: "disk almost full".to_owned(),
        };
        let mut progress = ProgressStore::new();
        handle_notification("my-plugin", &notif, &mut progress);
        // Progress store should be unaffected by log notifications.
        assert!(progress.get("inv_1").is_none());
    }

    #[test]
    fn handle_progress_notification_stores_percent() {
        let notif = PluginNotification::Progress {
            invocation_id: "inv_2".to_owned(),
            message: "halfway there".to_owned(),
            percent: Some(50),
        };
        let mut progress = ProgressStore::new();
        handle_notification("my-plugin", &notif, &mut progress);
        assert_eq!(progress.get("inv_2"), Some(50));
    }

    #[test]
    fn handle_progress_notification_without_percent_leaves_store_empty() {
        let notif = PluginNotification::Progress {
            invocation_id: "inv_3".to_owned(),
            message: "starting".to_owned(),
            percent: None,
        };
        let mut progress = ProgressStore::new();
        handle_notification("my-plugin", &notif, &mut progress);
        assert!(progress.get("inv_3").is_none());
    }

    #[test]
    fn progress_store_tracks_latest_percent() {
        let mut store = ProgressStore::new();
        store.set("inv_1", 25);
        store.set("inv_1", 75);
        assert_eq!(store.get("inv_1"), Some(75));
    }
}
