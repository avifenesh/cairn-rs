use cairn_domain::ids::ToolInvocationId;
use cairn_domain::policy::ExecutionClass;
use cairn_domain::tenancy::ProjectKey;

use crate::builtin::{ToolHost, ToolInput, ToolOutcome};
use crate::permissions::{InvocationGrants, PermissionCheckResult, PermissionGate};

/// Outcome of executing a tool through the full permissioned pipeline.
#[derive(Clone, Debug)]
pub enum ExecutionOutcome {
    /// Permission was granted and tool executed.
    Executed {
        outcome: ToolOutcome,
        grants: InvocationGrants,
    },
    /// Permission was denied before execution.
    PermissionDenied { reason: String },
    /// Permission decision is held pending operator approval.
    HeldForApproval { reason: String },
}

impl ExecutionOutcome {
    pub fn is_executed(&self) -> bool {
        matches!(self, ExecutionOutcome::Executed { .. })
    }

    pub fn is_denied(&self) -> bool {
        matches!(self, ExecutionOutcome::PermissionDenied { .. })
    }
}

/// Orchestrates builtin tool execution: permission check -> record -> execute -> finalize.
///
/// This is the primary execution path for builtin tools. It:
/// 1. Looks up the tool descriptor to find required permissions
/// 2. Checks permissions via the PermissionGate
/// 3. Creates a durable invocation record in Requested state
/// 4. Executes the tool via the ToolHost
/// 5. Finalizes the record with the outcome
///
/// Callers are responsible for persisting the invocation records and
/// permission-decision events through the store.
pub fn execute_builtin<G: PermissionGate, H: ToolHost>(
    gate: &G,
    host: &H,
    project: &ProjectKey,
    invocation_id: ToolInvocationId,
    tool_name: &str,
    params: serde_json::Value,
    execution_class: ExecutionClass,
    _now_ms: u64,
) -> ExecutionOutcome {
    // Find the tool descriptor to get required permissions
    let tools = host.list_tools();
    let descriptor = match tools.iter().find(|t| t.name == tool_name) {
        Some(d) => d,
        None => {
            return ExecutionOutcome::PermissionDenied {
                reason: format!("tool not found: {tool_name}"),
            };
        }
    };

    // Check permissions
    let check_result = gate.check(project, &descriptor.required_permissions, execution_class);

    match check_result {
        PermissionCheckResult::Granted(grants) => {
            // Build input and execute
            let input = ToolInput {
                invocation_id: invocation_id.clone(),
                tool_name: tool_name.to_owned(),
                project: project.clone(),
                grants: grants.clone(),
                params,
            };

            let outcome = host.invoke(input);

            ExecutionOutcome::Executed { outcome, grants }
        }
        PermissionCheckResult::Denied(verdict) => ExecutionOutcome::PermissionDenied {
            reason: verdict
                .reason
                .unwrap_or_else(|| "denied by policy".to_owned()),
        },
        PermissionCheckResult::HeldForApproval(verdict) => ExecutionOutcome::HeldForApproval {
            reason: verdict
                .reason
                .unwrap_or_else(|| "held for approval".to_owned()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::ToolInvocationId;
    use cairn_domain::policy::{ExecutionClass, PolicyVerdict};
    use cairn_domain::tenancy::ProjectKey;

    use crate::builtin::{ToolDescriptor, ToolHost, ToolInput, ToolOutcome};
    use crate::permissions::{
        DeclaredPermissions, InvocationGrants, Permission, PermissionCheckResult, PermissionGate,
    };

    // Test permission gate that always allows
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

    // Test permission gate that always denies
    struct DenyGate;
    impl PermissionGate for DenyGate {
        fn check(
            &self,
            _project: &ProjectKey,
            _required: &DeclaredPermissions,
            _execution_class: ExecutionClass,
        ) -> PermissionCheckResult {
            PermissionCheckResult::Denied(PolicyVerdict::deny("not allowed"))
        }
    }

    // Test tool host with one registered tool
    struct TestToolHost;
    impl ToolHost for TestToolHost {
        fn list_tools(&self) -> Vec<ToolDescriptor> {
            vec![ToolDescriptor {
                name: "test.echo".to_owned(),
                description: "Echo input".to_owned(),
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
    fn execute_with_permission_succeeds() {
        let result = execute_builtin(
            &AllowGate,
            &TestToolHost,
            &ProjectKey::new("t", "w", "p"),
            ToolInvocationId::new("inv_1"),
            "test.echo",
            serde_json::json!({"msg": "hello"}),
            ExecutionClass::SupervisedProcess,
            1000,
        );

        assert!(result.is_executed());
        if let ExecutionOutcome::Executed { outcome, grants } = &result {
            assert!(outcome.is_success());
            assert!(grants.is_granted(&Permission::FsRead));
        }
    }

    #[test]
    fn execute_with_denied_permission() {
        let result = execute_builtin(
            &DenyGate,
            &TestToolHost,
            &ProjectKey::new("t", "w", "p"),
            ToolInvocationId::new("inv_2"),
            "test.echo",
            serde_json::json!({}),
            ExecutionClass::SupervisedProcess,
            1000,
        );

        assert!(result.is_denied());
    }

    #[test]
    fn execute_unknown_tool_returns_denied() {
        let result = execute_builtin(
            &AllowGate,
            &TestToolHost,
            &ProjectKey::new("t", "w", "p"),
            ToolInvocationId::new("inv_3"),
            "nonexistent.tool",
            serde_json::json!({}),
            ExecutionClass::SupervisedProcess,
            1000,
        );

        assert!(result.is_denied());
        if let ExecutionOutcome::PermissionDenied { reason } = &result {
            assert!(reason.contains("not found"));
        }
    }
}
