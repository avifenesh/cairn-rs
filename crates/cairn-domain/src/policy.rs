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
    /// A tool that requires operator approval before the orchestrator dispatches
    /// it.  Any `ActionProposal` whose tool produces `ExecutionClass::Sensitive`
    /// has `requires_approval = true` injected automatically by the execute phase.
    Sensitive,
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

/// The kind of decision made by a guardrail rule evaluation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailDecisionKind {
    Allowed,
    Denied,
    Warned,
}

/// The effect that fires when a guardrail rule matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailRuleEffect {
    Allow,
    Deny,
    Block,
    Redact,
    Log,
    Alert,
}

#[cfg(test)]
mod tests {
    use super::{ApprovalRequirement, PolicyEffect, PolicyVerdict};

    #[test]
    fn hold_verdict_implies_approval_requirement() {
        let verdict = PolicyVerdict::hold("operator review required");

        assert_eq!(verdict.effect, PolicyEffect::Hold);
        assert_eq!(verdict.requirement, ApprovalRequirement::Required);
    }
}

/// Subject type for a guardrail policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardrailSubjectType {
    Run,
    Task,
    Session,
    Tool,
    Provider,
}

/// A single guardrail rule definition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailRule {
    pub subject_type: GuardrailSubjectType,
    /// None means "any subject ID".
    pub subject_id: Option<String>,
    pub action: String,
    pub effect: GuardrailRuleEffect,
    pub conditions: Vec<String>,
}

/// A guardrail policy consisting of one or more rules.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailPolicy {
    pub policy_id: String,
    pub name: String,
    pub rules: Vec<GuardrailRule>,
    pub enabled: bool,
}

/// The outcome of evaluating a guardrail — carries the verdict + which policy matched.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardrailDecision {
    pub decision: GuardrailDecisionKind,
    pub policy_id: Option<String>,
    pub reason: Option<String>,
}

/// Tenant-scoped configurable approval workflow record (RFC 006).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalPolicyRecord {
    pub policy_id: String,
    pub tenant_id: crate::ids::TenantId,
    pub name: String,
    pub required_approvers: u32,
    pub allowed_approver_roles: Vec<crate::tenancy::WorkspaceRole>,
    pub auto_approve_after_ms: Option<u64>,
    pub auto_reject_after_ms: Option<u64>,
    pub attached_release_ids: Vec<crate::ids::PromptReleaseId>,
}
