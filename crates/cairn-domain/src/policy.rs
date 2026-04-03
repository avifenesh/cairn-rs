use crate::ids::PolicyId;
use crate::tenancy::{OwnershipKey, Scope};
use serde::{Deserialize, Serialize};

/// Policy outcomes must stay explicit across runtime and operator surfaces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyEffect {
    Allow,
    Deny,
    Hold,
}

/// Approval requirements are shared by runtime, prompt rollout, and governance logic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRequirement {
    NotRequired,
    Required,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalDecision {
    Approved,
    Rejected,
}

/// Shared execution classes used by policy and tool gating.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionClass {
    SupervisedProcess,
    SandboxedProcess,
}

/// Stable pointer to the policy that produced a decision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyReference {
    pub policy_id: PolicyId,
    pub scope: Scope,
    pub owner: OwnershipKey,
}

/// Canonical policy verdict envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyVerdict {
    pub effect: PolicyEffect,
    pub requirement: ApprovalRequirement,
    pub reason: Option<String>,
    pub source: Option<PolicyReference>,
}

impl PolicyVerdict {
    pub fn allow() -> Self {
        Self {
            effect: PolicyEffect::Allow,
            requirement: ApprovalRequirement::NotRequired,
            reason: None,
            source: None,
        }
    }

    pub fn hold(reason: impl Into<String>) -> Self {
        Self {
            effect: PolicyEffect::Hold,
            requirement: ApprovalRequirement::Required,
            reason: Some(reason.into()),
            source: None,
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            effect: PolicyEffect::Deny,
            requirement: ApprovalRequirement::NotRequired,
            reason: Some(reason.into()),
            source: None,
        }
    }
}

/// Approval mode for prompt release governance (RFC 006).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Releases require explicit review before activation.
    RequiresReview,
    /// Releases can go directly from draft to active.
    DraftToActive,
}

/// Project-scoped approval policy for prompt release governance.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPolicy {
    pub project_id: crate::ids::ProjectId,
    pub mode: ApprovalMode,
}

#[cfg(test)]
mod tests {
    use super::{ApprovalMode, ApprovalRequirement, PolicyEffect, PolicyVerdict};

    #[test]
    fn hold_verdict_implies_approval_requirement() {
        let verdict = PolicyVerdict::hold("operator review required");

        assert_eq!(verdict.effect, PolicyEffect::Hold);
        assert_eq!(verdict.requirement, ApprovalRequirement::Required);
    }
}
