use cairn_domain::ids::{RunId, SessionId, TaskId, ToolInvocationId};
use cairn_domain::policy::ExecutionClass;
use cairn_domain::tenancy::ProjectKey;
use cairn_domain::tool_invocation::{ToolInvocationRecord, ToolInvocationTarget};

use crate::builtin::{ToolHost, ToolOutcome};
use crate::execution_class::{select_execution_config, DeploymentMode, SelectedConfig};
use crate::executor::{execute_builtin, ExecutionOutcome};
use crate::invocation::{create_requested_record, mark_finished, mark_started, InvocationRequest};
use crate::permissions::PermissionGate;
use crate::plugin_bridge::build_tools_invoke_request;
use crate::plugins::PluginManifest;

/// Full end-to-end tool invocation result from the pipeline.
#[derive(Clone, Debug)]
pub struct PipelineResult {
    pub records: Vec<ToolInvocationRecord>,
    pub outcome: PipelineOutcome,
}

/// Pipeline-level outcome classification.
#[derive(Clone, Debug)]
pub enum PipelineOutcome {
    /// Tool executed successfully through the full pipeline.
    Completed(ToolOutcome),
    /// Permission was denied before execution started.
    PermissionDenied { reason: String },
    /// Permission decision held for operator approval.
    HeldForApproval { reason: String },
}

impl PipelineOutcome {
    pub fn is_completed(&self) -> bool {
        matches!(self, PipelineOutcome::Completed(_))
    }
}

/// Run a builtin tool through the full durable pipeline.
///
/// Steps:
/// 1. Create invocation record in Requested state
/// 2. Check permissions via the gate
/// 3. If allowed: mark Started, execute, mark Finished
/// 4. If denied/held: record stays in Requested (runtime will handle)
///
/// Returns all record snapshots for the caller to persist.
pub fn run_builtin_pipeline<G: PermissionGate, H: ToolHost>(
    gate: &G,
    host: &H,
    project: &ProjectKey,
    invocation_id: ToolInvocationId,
    session_id: Option<SessionId>,
    run_id: Option<RunId>,
    task_id: Option<TaskId>,
    tool_name: &str,
    params: serde_json::Value,
    execution_class: ExecutionClass,
    now_ms: u64,
) -> PipelineResult {
    let request = InvocationRequest {
        invocation_id: invocation_id.clone(),
        project: project.clone(),
        session_id,
        run_id,
        task_id,
        target: ToolInvocationTarget::Builtin {
            tool_name: tool_name.to_owned(),
        },
        execution_class,
    };

    let requested = create_requested_record(&request, now_ms);
    let mut records = vec![requested.clone()];

    let exec_result = execute_builtin(
        gate,
        host,
        project,
        invocation_id,
        tool_name,
        params,
        execution_class,
        now_ms,
    );

    match exec_result {
        ExecutionOutcome::Executed { outcome, .. } => {
            let started = mark_started(&requested, now_ms + 1);
            let finished = mark_finished(&started, &outcome, now_ms + 2);
            records.push(started);
            records.push(finished);
            PipelineResult {
                records,
                outcome: PipelineOutcome::Completed(outcome),
            }
        }
        ExecutionOutcome::PermissionDenied { reason } => PipelineResult {
            records,
            outcome: PipelineOutcome::PermissionDenied { reason },
        },
        ExecutionOutcome::HeldForApproval { reason } => PipelineResult {
            records,
            outcome: PipelineOutcome::HeldForApproval { reason },
        },
    }
}

/// Run a plugin tool through the full durable pipeline.
///
/// This builds the JSON-RPC request for the plugin and maps
/// the result back through the durable record lifecycle.
/// The actual stdio transport is not implemented here — the caller
/// is responsible for sending the request and providing the result.
pub fn build_plugin_pipeline_request(
    manifest: &PluginManifest,
    invocation_id: &ToolInvocationId,
    session_id: Option<SessionId>,
    run_id: Option<RunId>,
    task_id: Option<TaskId>,
    tool_name: &str,
    input: serde_json::Value,
    project: &ProjectKey,
    mode: DeploymentMode,
    now_ms: u64,
) -> (ToolInvocationRecord, SelectedConfig, serde_json::Value) {
    let config = select_execution_config(manifest, mode);

    let request = InvocationRequest {
        invocation_id: invocation_id.clone(),
        project: project.clone(),
        session_id,
        run_id,
        task_id,
        target: ToolInvocationTarget::Plugin {
            plugin_id: manifest.id.clone(),
            tool_name: tool_name.to_owned(),
        },
        execution_class: config.execution_class(),
    };

    let record = create_requested_record(&request, now_ms);

    let grants: Vec<String> = manifest
        .permissions
        .permissions
        .iter()
        .map(|p| format!("{p:?}").to_lowercase())
        .collect();

    let rpc_request = build_tools_invoke_request(
        &format!("req_{}", invocation_id.as_str()),
        invocation_id.as_str(),
        tool_name,
        input,
        project,
        &grants,
    );

    let rpc_json = serde_json::to_value(&rpc_request).unwrap_or_default();

    (record, config, rpc_json)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::ToolInvocationId;
    use cairn_domain::policy::{ExecutionClass, PolicyVerdict};
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationState};

    use crate::builtin::{ToolDescriptor, ToolHost, ToolInput, ToolOutcome};
    use crate::permissions::{
        DeclaredPermissions, InvocationGrants, Permission, PermissionCheckResult, PermissionGate,
    };
    use crate::plugins::{PluginCapability, PluginLimits, PluginManifest};

    struct AllowGate;
    impl PermissionGate for AllowGate {
        fn check(
            &self,
            project: &ProjectKey,
            _required: &DeclaredPermissions,
            execution_class: ExecutionClass,
        ) -> PermissionCheckResult {
            PermissionCheckResult::Granted(InvocationGrants {
                project: project.clone(),
                execution_class,
                granted: vec![Permission::FsRead],
            })
        }
    }

    struct DenyGate;
    impl PermissionGate for DenyGate {
        fn check(
            &self,
            _project: &ProjectKey,
            _required: &DeclaredPermissions,
            _execution_class: ExecutionClass,
        ) -> PermissionCheckResult {
            PermissionCheckResult::Denied(PolicyVerdict::deny("blocked"))
        }
    }

    struct EchoHost;
    impl ToolHost for EchoHost {
        fn list_tools(&self) -> Vec<ToolDescriptor> {
            vec![ToolDescriptor {
                name: "echo".to_owned(),
                description: "Echo".to_owned(),
                required_permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
            }]
        }
        fn invoke(&self, input: ToolInput) -> ToolOutcome {
            ToolOutcome::Success {
                output: input.params,
            }
        }
    }

    #[test]
    fn builtin_pipeline_success_produces_three_records() {
        let result = run_builtin_pipeline(
            &AllowGate,
            &EchoHost,
            &ProjectKey::new("t", "w", "p"),
            ToolInvocationId::new("inv_1"),
            Some(SessionId::new("sess_1")),
            Some(RunId::new("run_1")),
            Some(TaskId::new("task_1")),
            "echo",
            serde_json::json!({"msg": "hi"}),
            ExecutionClass::SupervisedProcess,
            1000,
        );

        assert!(result.outcome.is_completed());
        assert_eq!(result.records.len(), 3);
        assert_eq!(result.records[0].state, ToolInvocationState::Requested);
        assert_eq!(result.records[1].state, ToolInvocationState::Started);
        assert_eq!(result.records[2].state, ToolInvocationState::Completed);
        assert_eq!(
            result.records[0].session_id.as_ref().map(|id| id.as_str()),
            Some("sess_1")
        );
        assert_eq!(
            result.records[0].run_id.as_ref().map(|id| id.as_str()),
            Some("run_1")
        );
        assert_eq!(
            result.records[0].task_id.as_ref().map(|id| id.as_str()),
            Some("task_1")
        );
        assert_eq!(
            result.records[2].outcome,
            Some(ToolInvocationOutcomeKind::Success)
        );
    }

    #[test]
    fn builtin_pipeline_denied_produces_one_record() {
        let result = run_builtin_pipeline(
            &DenyGate,
            &EchoHost,
            &ProjectKey::new("t", "w", "p"),
            ToolInvocationId::new("inv_2"),
            None,
            None,
            None,
            "echo",
            serde_json::json!({}),
            ExecutionClass::SupervisedProcess,
            1000,
        );

        assert!(!result.outcome.is_completed());
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].state, ToolInvocationState::Requested);
    }

    #[test]
    fn plugin_pipeline_request_builds_rpc_and_record() {
        let manifest = PluginManifest {
            id: "com.example.git".to_owned(),
            name: "Git".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["git-plugin".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["git.status".to_owned()],
            }],
            permissions: DeclaredPermissions::new(vec![
                Permission::FsRead,
                Permission::ProcessExec,
            ]),
            limits: Some(PluginLimits {
                max_concurrency: Some(2),
                default_timeout_ms: Some(10_000),
            }),
            execution_class: ExecutionClass::SupervisedProcess,
        };

        let (record, config, rpc_json) = build_plugin_pipeline_request(
            &manifest,
            &ToolInvocationId::new("inv_3"),
            Some(SessionId::new("sess_1")),
            Some(RunId::new("run_1")),
            Some(TaskId::new("task_1")),
            "git.status",
            serde_json::json!({"path": "/repo"}),
            &ProjectKey::new("t", "w", "p"),
            DeploymentMode::Local,
            2000,
        );

        assert_eq!(record.state, ToolInvocationState::Requested);
        assert!(matches!(record.target, ToolInvocationTarget::Plugin { .. }));
        assert_eq!(
            record.session_id.as_ref().map(|id| id.as_str()),
            Some("sess_1")
        );
        assert_eq!(record.run_id.as_ref().map(|id| id.as_str()), Some("run_1"));
        assert_eq!(
            record.task_id.as_ref().map(|id| id.as_str()),
            Some("task_1")
        );
        assert_eq!(config.execution_class(), ExecutionClass::SupervisedProcess);
        assert_eq!(config.timeout_ms(), 10_000);
        assert_eq!(rpc_json["method"], "tools.invoke");
        assert_eq!(rpc_json["params"]["toolName"], "git.status");
    }

    #[test]
    fn plugin_pipeline_team_mode_uses_sandboxed() {
        let manifest = PluginManifest {
            id: "test.plugin".to_owned(),
            name: "Test".to_owned(),
            version: "0.1.0".to_owned(),
            command: vec!["test".to_owned()],
            capabilities: vec![PluginCapability::ToolProvider {
                tools: vec!["t".to_owned()],
            }],
            permissions: DeclaredPermissions::default(),
            limits: None,
            execution_class: ExecutionClass::SupervisedProcess,
        };

        let (record, config, _) = build_plugin_pipeline_request(
            &manifest,
            &ToolInvocationId::new("inv_4"),
            None,
            None,
            None,
            "t",
            serde_json::json!({}),
            &ProjectKey::new("t", "w", "p"),
            DeploymentMode::SelfHostedTeam,
            3000,
        );

        assert_eq!(config.execution_class(), ExecutionClass::SandboxedProcess);
        assert_eq!(record.execution_class, ExecutionClass::SandboxedProcess);
    }
}
