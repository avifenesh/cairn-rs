use cairn_domain::ids::ToolInvocationId;
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::permissions::{DeclaredPermissions, InvocationGrants};

/// Describes a builtin tool registered with the host.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub required_permissions: DeclaredPermissions,
}

/// Input envelope for a tool invocation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolInput {
    pub invocation_id: ToolInvocationId,
    pub tool_name: String,
    pub project: ProjectKey,
    pub grants: InvocationGrants,
    pub params: serde_json::Value,
}

/// Canonical tool invocation outcomes per RFC 007.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ToolOutcome {
    Success { output: serde_json::Value },
    RetryableFailure { reason: String },
    PermanentFailure { reason: String },
    Timeout,
    Canceled,
}

impl ToolOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, ToolOutcome::Success { .. })
    }

    pub fn is_terminal_failure(&self) -> bool {
        matches!(self, ToolOutcome::PermanentFailure { .. })
    }
}

/// Seam for builtin tool hosting. The runtime registers builtin tools
/// and invokes them through this trait.
pub trait ToolHost {
    fn list_tools(&self) -> Vec<ToolDescriptor>;
    fn invoke(&self, input: ToolInput) -> ToolOutcome;
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::ToolInvocationId;
    use cairn_domain::policy::ExecutionClass;
    use cairn_domain::tenancy::ProjectKey;

    use crate::permissions::{DeclaredPermissions, InvocationGrants, Permission};

    #[test]
    fn tool_descriptor_carries_permissions() {
        let descriptor = ToolDescriptor {
            name: "fs.read".to_owned(),
            description: "Read a file".to_owned(),
            required_permissions: DeclaredPermissions::new(vec![Permission::FsRead]),
        };

        assert!(descriptor
            .required_permissions
            .contains(&Permission::FsRead));
    }

    #[test]
    fn tool_outcome_classification() {
        let success = ToolOutcome::Success {
            output: serde_json::json!({"text": "ok"}),
        };
        assert!(success.is_success());
        assert!(!success.is_terminal_failure());

        let failure = ToolOutcome::PermanentFailure {
            reason: "bad input".to_owned(),
        };
        assert!(failure.is_terminal_failure());
        assert!(!failure.is_success());

        let retryable = ToolOutcome::RetryableFailure {
            reason: "transient".to_owned(),
        };
        assert!(!retryable.is_success());
        assert!(!retryable.is_terminal_failure());
    }

    #[test]
    fn tool_input_construction() {
        let input = ToolInput {
            invocation_id: ToolInvocationId::new("inv_1"),
            tool_name: "fs.read".to_owned(),
            project: ProjectKey::new("t", "w", "p"),
            grants: InvocationGrants {
                project: ProjectKey::new("t", "w", "p"),
                execution_class: ExecutionClass::SupervisedProcess,
                granted: vec![Permission::FsRead],
            },
            params: serde_json::json!({"path": "/tmp/file.txt"}),
        };

        assert_eq!(input.tool_name, "fs.read");
    }
}
