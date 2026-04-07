//! Tool invocation, permissions, plugin host integration, and execution isolation.

pub mod builtin;
pub mod builtins;
pub mod execution_class;
pub mod executor;
pub mod graph_events;
pub mod invocation;
pub mod mcp_client;
pub mod mcp_server;
pub mod permission_events;
pub mod permissions;
pub mod pipeline;
pub mod plugin_bridge;
pub mod plugin_executor;
pub mod plugin_host;
pub mod plugins;
pub mod registry;
pub mod runtime_service;
pub mod runtime_service_impl;
pub mod sandboxed_process;
pub mod supervised_process;
pub mod transport;

pub use builtin::{ToolDescriptor, ToolHost, ToolInput, ToolOutcome};
pub use builtins::{
    BuiltinToolDescriptor, BuiltinToolRegistry,
    MemorySearchTool, MemoryStoreTool, ShellExecTool, ToolSearchTool, WebFetchTool,
    ToolError, ToolHandler, ToolResult, ToolTier,
};
pub use execution_class::{select_execution_config, SelectedConfig};
pub use executor::{execute_builtin, ExecutionOutcome};
pub use graph_events::{ToolInvocationNodeData, UsedToolEdgeData};
pub use invocation::{InvocationRequest, InvocationResult, InvocationService};
pub use permission_events::PermissionDecisionEvent;
pub use permissions::{
    DeclaredPermissions, InvocationGrants, Permission, PermissionCheckResult, PermissionGate,
};
pub use pipeline::{
    build_plugin_pipeline_request, run_builtin_pipeline, PipelineOutcome, PipelineResult,
};
pub use plugin_bridge::{
    build_cancel_request, build_channels_deliver_request, build_eval_score_request,
    build_hooks_post_turn_request, build_initialize_request, build_policy_evaluate_request,
    build_signals_poll_request, build_tools_invoke_request, invoke_result_to_outcome,
};
pub use mcp_client::{
    mcp_endpoint_for_manifest, McpClient, McpEndpoint, McpError, McpTool,
    MCP_PROTOCOL_VERSION,
};
pub use mcp_server::{
    McpCallRequest, McpCallResponse, McpServer, McpServerError, McpToolInfo, McpToolsResponse,
    MockMcpProcess,
};
pub use plugin_host::{PluginHostError, StdioPluginHost};
pub use plugins::{PluginCapability, PluginHost, PluginLimits, PluginManifest, PluginState};
pub use registry::{InMemoryPluginRegistry, PluginRegistry, RegistryError};
pub use runtime_service::{
    RuntimeToolOutcome, RuntimeToolRequest, RuntimeToolResponse, RuntimeToolService,
    ToolLifecycleOutput,
};
pub use runtime_service_impl::RuntimeToolServiceImpl;
pub use sandboxed_process::{SandboxedBoundary, SandboxedProcessConfig};
pub use supervised_process::{SupervisedBoundary, SupervisedProcessConfig};

#[cfg(test)]
mod tests {
    use cairn_domain::policy::ExecutionClass;

    use crate::permissions::{DeclaredPermissions, Permission};
    use crate::plugins::{PluginCapability, PluginManifest};

    #[test]
    fn execution_class_selects_config_module() {
        let exec_class = ExecutionClass::SandboxedProcess;
        let config = match exec_class {
            ExecutionClass::SupervisedProcess => {
                let c = crate::SupervisedProcessConfig::default();
                c.timeout_ms
            }
            ExecutionClass::SandboxedProcess => {
                let c = crate::SandboxedProcessConfig::default();
                c.timeout_ms
            }
            // Sensitive uses supervised config (approval is handled at a higher layer).
            ExecutionClass::Sensitive => {
                let c = crate::SupervisedProcessConfig::default();
                c.timeout_ms
            }
        };
        assert_eq!(config, 30_000);
    }

    #[test]
    fn manifest_permissions_gate_integration() {
        let manifest = PluginManifest {
            id: "test.plugin".to_owned(),
            name: "Test".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["test-bin".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["test.tool".to_owned()],
            }],
            permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
            limits: None,
            execution_class: ExecutionClass::SupervisedProcess,
            description: None,
            homepage: None,
        };

        assert!(manifest.permissions.contains(&Permission::FsRead));
        assert!(!manifest.permissions.contains(&Permission::NetworkEgress));
    }
}

// ── Stubs for cairn-app integration ──────────────────────────────────────

/// Snapshot of plugin lifecycle state for operator visibility.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PluginLifecycleSnapshot {
    pub plugin_id: String,
    pub state: String,
    pub uptime_ms: u64,
}

/// Metrics for a running plugin instance.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PluginMetrics {
    pub plugin_id: String,
    pub invocation_count: u64,
    pub error_count: u64,
    pub avg_latency_ms: f64,
}

/// A single log entry emitted by a plugin.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PluginLogEntry {
    pub plugin_id: String,
    pub level: String,
    pub message: String,
    pub timestamp_ms: u64,
}

/// Verification result for a plugin capability claim.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PluginCapabilityVerification {
    pub plugin_id: String,
    pub capability: String,
    pub verified: bool,
    pub reason: Option<String>,
}

/// A tool exposed by a plugin, as visible to the operator.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct PluginToolDescriptor {
    pub name: String,
    pub description: String,
    pub parameters_schema: serde_json::Value,
}

/// Result of an eval-score invocation through a plugin.
#[derive(Clone, Debug, Default)]
pub struct EvalScoreResult {
    pub score: f64,
    pub passed: bool,
    pub reasoning: Option<String>,
}

/// Cancel an in-flight plugin invocation. Best-effort; does not fail if not found.
pub fn cancel_plugin_invocation(
    _host: &mut crate::plugin_host::StdioPluginHost,
    _plugin_id: &str,
    _invocation_id: &str,
) {
    // Best-effort cancellation: send shutdown signal to plugin process.
    // No-op for now; real implementation would send a cancel RPC.
}

/// Execute an eval-score RPC through a plugin.
pub async fn execute_eval_score(
    _registry: &impl crate::registry::PluginRegistry,
    _plugin_id: &str,
    _input: serde_json::Value,
    _expected_output: Option<serde_json::Value>,
    _actual_output: serde_json::Value,
) -> Result<EvalScoreResult, String> {
    // Stub implementation — real version would invoke the plugin via JSON-RPC.
    Ok(EvalScoreResult { score: 0.0, passed: false, reasoning: None })
}
