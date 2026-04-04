use async_trait::async_trait;
use cairn_domain::{ApprovalPolicyRecord, PromptReleaseId, TenantId, WorkspaceRole};

use crate::error::RuntimeError;

#[async_trait]
pub trait ApprovalPolicyService: Send + Sync {
    async fn create(
        &self,
        tenant_id: TenantId,
        name: String,
        required_approvers: u32,
        allowed_approver_roles: Vec<WorkspaceRole>,
        auto_approve_after_ms: Option<u64>,
        auto_reject_after_ms: Option<u64>,
    ) -> Result<ApprovalPolicyRecord, RuntimeError>;

    async fn get(&self, policy_id: &str) -> Result<Option<ApprovalPolicyRecord>, RuntimeError>;

    async fn list(
        &self,
        tenant_id: &TenantId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalPolicyRecord>, RuntimeError>;

    async fn attach_to_release(
        &self,
        policy_id: &str,
        release_id: PromptReleaseId,
    ) -> Result<ApprovalPolicyRecord, RuntimeError>;
}
