use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::policy::{
    GuardrailDecision, GuardrailDecisionKind, GuardrailPolicy, GuardrailRule, GuardrailRuleEffect,
    GuardrailSubjectType,
};
use cairn_domain::*;
use cairn_store::projections::{GuardrailReadModel, TenantReadModel};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::guardrails::GuardrailService;

pub struct GuardrailServiceImpl<S> {
    store: Arc<S>,
}

impl<S> GuardrailServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn rule_matches(
    rule: &GuardrailRule,
    subject_type: GuardrailSubjectType,
    subject_id: Option<&str>,
    action: &str,
) -> bool {
    rule.subject_type == subject_type
        && rule.action == action
        && match (&rule.subject_id, subject_id) {
            (None, _) => true,
            (Some(rule_subject_id), Some(subject_id)) => rule_subject_id == subject_id,
            (Some(_), None) => false,
        }
}

#[async_trait]
impl<S> GuardrailService for GuardrailServiceImpl<S>
where
    S: EventLog + GuardrailReadModel + TenantReadModel + Send + Sync + 'static,
{
    async fn create_policy(
        &self,
        tenant_id: TenantId,
        name: String,
        rules: Vec<GuardrailRule>,
    ) -> Result<GuardrailPolicy, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        let policy_id = format!("guardrail_policy_{}", now_ms());
        let event = make_envelope(RuntimeEvent::GuardrailPolicyCreated(
            GuardrailPolicyCreated {
                tenant_id: tenant_id.clone(),
                policy_id: policy_id.clone(),
                name,
                rules,
            },
        ));
        self.store.append(&[event]).await?;

        GuardrailReadModel::get_policy(self.store.as_ref(), &policy_id)
            .await?
            .ok_or_else(|| {
                RuntimeError::Internal("guardrail policy not found after create".to_owned())
            })
    }

    async fn evaluate(
        &self,
        tenant_id: TenantId,
        subject_type: GuardrailSubjectType,
        subject_id: Option<String>,
        action: String,
    ) -> Result<GuardrailDecision, RuntimeError> {
        if TenantReadModel::get(self.store.as_ref(), &tenant_id)
            .await?
            .is_none()
        {
            return Err(RuntimeError::NotFound {
                entity: "tenant",
                id: tenant_id.to_string(),
            });
        }

        let policies =
            GuardrailReadModel::list_policies(self.store.as_ref(), &tenant_id, usize::MAX, 0)
                .await?;

        let mut decision = GuardrailDecision {
            decision: GuardrailDecisionKind::Allowed,
            policy_id: None,
            reason: None,
        };

        for policy in policies {
            if let Some(rule) = policy.rules.iter().find(|rule| {
                rule_matches(rule, subject_type, subject_id.as_deref(), action.as_str())
            }) {
                decision = GuardrailDecision {
                    decision: match rule.effect {
                        GuardrailRuleEffect::Allow => GuardrailDecisionKind::Allowed,
                        GuardrailRuleEffect::Deny
                        | GuardrailRuleEffect::Block
                        | GuardrailRuleEffect::Redact
                        | GuardrailRuleEffect::Log
                        | GuardrailRuleEffect::Alert => GuardrailDecisionKind::Denied,
                    },
                    policy_id: Some(policy.policy_id.clone()),
                    reason: Some(format!(
                        "matched {} rule for {}",
                        match rule.effect {
                            GuardrailRuleEffect::Allow => "allow",
                            GuardrailRuleEffect::Deny
                            | GuardrailRuleEffect::Block
                            | GuardrailRuleEffect::Redact
                            | GuardrailRuleEffect::Log
                            | GuardrailRuleEffect::Alert => "deny",
                        },
                        action
                    )),
                };
                break;
            }
        }

        self.store
            .append(&[make_envelope(RuntimeEvent::GuardrailPolicyEvaluated(
                GuardrailPolicyEvaluated {
                    tenant_id,
                    policy_id: decision
                        .policy_id
                        .clone()
                        .unwrap_or_else(|| "implicit_allow".to_owned()),
                    subject_type,
                    subject_id,
                    action,
                    decision: decision.decision,
                    reason: decision.reason.clone(),
                    evaluated_at_ms: now_ms(),
                },
            ))])
            .await?;

        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use cairn_domain::policy::{
        GuardrailDecisionKind, GuardrailRule, GuardrailRuleEffect, GuardrailSubjectType,
    };
    use cairn_domain::TenantId;
    use cairn_store::InMemoryStore;

    use crate::guardrails::GuardrailService;
    use crate::services::{GuardrailServiceImpl, TenantServiceImpl};
    use crate::tenants::TenantService;

    #[tokio::test]
    async fn guardrail_denies_matching_tool_and_allows_non_matching_tool() {
        let store = Arc::new(InMemoryStore::new());
        let tenant_service = TenantServiceImpl::new(store.clone());
        tenant_service
            .create(TenantId::new("tenant_acme"), "Acme".to_owned())
            .await
            .unwrap();

        let service = GuardrailServiceImpl::new(store);
        service
            .create_policy(
                TenantId::new("tenant_acme"),
                "tool-delete-policy".to_owned(),
                vec![GuardrailRule {
                    subject_type: GuardrailSubjectType::Tool,
                    subject_id: Some("fs.delete".to_owned()),
                    action: "invoke".to_owned(),
                    effect: GuardrailRuleEffect::Deny,
                    conditions: vec![],
                }],
            )
            .await
            .unwrap();

        let denied = service
            .evaluate(
                TenantId::new("tenant_acme"),
                GuardrailSubjectType::Tool,
                Some("fs.delete".to_owned()),
                "invoke".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(denied.decision, GuardrailDecisionKind::Denied);

        let allowed = service
            .evaluate(
                TenantId::new("tenant_acme"),
                GuardrailSubjectType::Tool,
                Some("fs.read".to_owned()),
                "invoke".to_owned(),
            )
            .await
            .unwrap();
        assert_eq!(allowed.decision, GuardrailDecisionKind::Allowed);
    }
}
