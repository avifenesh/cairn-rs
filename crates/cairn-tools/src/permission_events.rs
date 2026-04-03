use cairn_domain::ids::{EventId, ToolInvocationId};
use cairn_domain::policy::{ExecutionClass, PolicyEffect};
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};

use crate::permissions::Permission;

/// Durable event recording the outcome of a permission check before tool invocation.
///
/// These events integrate with the runtime event model so that permission
/// decisions are inspectable, replayable, and visible in operator surfaces.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionDecisionEvent {
    pub event_id: EventId,
    pub invocation_id: ToolInvocationId,
    pub project: ProjectKey,
    pub execution_class: ExecutionClass,
    pub effect: PolicyEffect,
    pub required_permissions: Vec<Permission>,
    pub granted_permissions: Vec<Permission>,
    pub denied_permissions: Vec<Permission>,
    pub reason: Option<String>,
    pub decided_at_ms: u64,
}

impl PermissionDecisionEvent {
    pub fn was_allowed(&self) -> bool {
        self.effect == PolicyEffect::Allow
    }

    pub fn was_denied(&self) -> bool {
        self.effect == PolicyEffect::Deny
    }

    pub fn was_held(&self) -> bool {
        self.effect == PolicyEffect::Hold
    }
}

/// Builds a permission-decision event from a completed permission check.
pub fn build_decision_event(
    event_id: EventId,
    invocation_id: ToolInvocationId,
    project: ProjectKey,
    execution_class: ExecutionClass,
    effect: PolicyEffect,
    required: &[Permission],
    granted: &[Permission],
    reason: Option<String>,
    decided_at_ms: u64,
) -> PermissionDecisionEvent {
    let denied: Vec<Permission> = required
        .iter()
        .filter(|p| !granted.contains(p))
        .copied()
        .collect();

    PermissionDecisionEvent {
        event_id,
        invocation_id,
        project,
        execution_class,
        effect,
        required_permissions: required.to_vec(),
        granted_permissions: granted.to_vec(),
        denied_permissions: denied,
        reason,
        decided_at_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::ids::{EventId, ToolInvocationId};
    use cairn_domain::policy::{ExecutionClass, PolicyEffect};
    use cairn_domain::tenancy::ProjectKey;

    use crate::permissions::Permission;

    #[test]
    fn allowed_decision_event() {
        let event = build_decision_event(
            EventId::new("evt_1"),
            ToolInvocationId::new("inv_1"),
            ProjectKey::new("t", "w", "p"),
            ExecutionClass::SupervisedProcess,
            PolicyEffect::Allow,
            &[Permission::FsRead],
            &[Permission::FsRead],
            None,
            1000,
        );

        assert!(event.was_allowed());
        assert!(!event.was_denied());
        assert!(event.denied_permissions.is_empty());
    }

    #[test]
    fn denied_decision_event_captures_denied_permissions() {
        let event = build_decision_event(
            EventId::new("evt_2"),
            ToolInvocationId::new("inv_2"),
            ProjectKey::new("t", "w", "p"),
            ExecutionClass::SandboxedProcess,
            PolicyEffect::Deny,
            &[Permission::FsRead, Permission::NetworkEgress],
            &[Permission::FsRead],
            Some("network access denied by policy".to_owned()),
            1001,
        );

        assert!(event.was_denied());
        assert_eq!(event.denied_permissions, vec![Permission::NetworkEgress]);
        assert_eq!(
            event.reason,
            Some("network access denied by policy".to_owned())
        );
    }

    #[test]
    fn held_decision_event() {
        let event = build_decision_event(
            EventId::new("evt_3"),
            ToolInvocationId::new("inv_3"),
            ProjectKey::new("t", "w", "p"),
            ExecutionClass::SupervisedProcess,
            PolicyEffect::Hold,
            &[Permission::ProcessExec],
            &[],
            Some("operator review required".to_owned()),
            1002,
        );

        assert!(event.was_held());
        assert_eq!(event.denied_permissions, vec![Permission::ProcessExec]);
    }
}
