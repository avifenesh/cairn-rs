//! Tool invocation, permissions, plugin host integration, and execution isolation.

pub mod builtin;
pub mod execution_class;
pub mod executor;
pub mod graph_events;
pub mod invocation;
pub mod permission_events;
pub mod permissions;
pub mod pipeline;
pub mod plugin_bridge;
pub mod plugins;
pub mod registry;
pub mod runtime_service;
pub mod runtime_service_impl;
pub mod sandboxed_process;
pub mod supervised_process;
pub mod transport;

pub use builtin::{ToolDescriptor, ToolHost, ToolInput, ToolOutcome};
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
        };

        assert!(manifest.permissions.contains(&Permission::FsRead));
        assert!(!manifest.permissions.contains(&Permission::NetworkEgress));
    }
}
