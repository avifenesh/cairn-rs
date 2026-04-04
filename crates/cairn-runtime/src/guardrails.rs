//! Tenant-scoped guardrail policy service boundary for RFC 010.

use async_trait::async_trait;
use cairn_domain::policy::{
    GuardrailDecision, GuardrailPolicy, GuardrailRule, GuardrailSubjectType,
};
use cairn_domain::TenantId;

use crate::error::RuntimeError;

#[async_trait]
pub trait GuardrailService: Send + Sync {
    async fn create_policy(
        &self,
        tenant_id: TenantId,
        name: String,
        rules: Vec<GuardrailRule>,
    ) -> Result<GuardrailPolicy, RuntimeError>;

    async fn evaluate(
        &self,
        tenant_id: TenantId,
        subject_type: GuardrailSubjectType,
        subject_id: Option<String>,
        action: String,
    ) -> Result<GuardrailDecision, RuntimeError>;
}
