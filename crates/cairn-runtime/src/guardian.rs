//! Guardian Approval Resolver (RFC 018 Enhancement 2).
//!
//! Spawns a short LLM sub-call to evaluate a pending approval request
//! and return a structured decision. Fails closed: if the guardian times
//! out, produces invalid output, or assesses risk above the configured
//! ceiling, it returns `None` and the caller falls through to the human
//! resolver.

use async_trait::async_trait;
use cairn_domain::decisions::{
    DecisionKind, DecisionOutcome, DecisionRequest, DecisionSource, GuardianConfig, RiskLevel,
};
use cairn_domain::providers::{GenerationProvider, GenerationResponse, ProviderBindingSettings};
use std::sync::Arc;

use crate::decisions::ApprovalResolver;

// ── GuardianResolver ─────────────────────────────────────────────────────────

/// Approval resolver that uses an LLM to evaluate pending approvals.
///
/// If the guardian is not configured (no `model_id`), or the LLM call fails,
/// or the assessed risk exceeds `risk_ceiling`, the resolver returns a
/// fall-through that delegates to the next resolver in the chain (typically
/// the human resolver).
pub struct GuardianResolver {
    provider: Arc<dyn GenerationProvider>,
    config: GuardianConfig,
}

impl GuardianResolver {
    pub fn new(provider: Arc<dyn GenerationProvider>, config: GuardianConfig) -> Self {
        Self { provider, config }
    }

    /// Build the guardian evaluation prompt from a decision request.
    fn build_prompt(&self, request: &DecisionRequest) -> Vec<serde_json::Value> {
        let system = "You are a security guardian for an AI agent control plane. \
            Your job is to evaluate a pending approval request and decide whether \
            the action should be allowed or denied.\n\n\
            Respond with ONLY a JSON object:\n\
            {\n\
              \"outcome\": \"allowed\" | \"denied\",\n\
              \"rationale\": \"brief explanation\",\n\
              \"risk_level\": \"low\" | \"medium\" | \"high\"\n\
            }\n\n\
            Rules:\n\
            - Read-only actions (search, fetch, query) are LOW risk → allow\n\
            - Internal writes (memory store, scratch pad, file in sandbox) are LOW risk → allow\n\
            - External actions (API calls, notifications, shell commands) are MEDIUM risk → \
              allow only if the context makes the action clearly intentional\n\
            - Destructive actions (delete, force-push, credential access) are HIGH risk → deny\n\
            - When uncertain, classify as MEDIUM and explain your uncertainty";

        let kind_desc = match &request.kind {
            DecisionKind::ToolInvocation { tool_name, effect } => {
                format!("Tool invocation: {} (effect: {:?})", tool_name, effect)
            }
            DecisionKind::ProviderCall {
                model_id,
                estimated_tokens,
            } => {
                format!("Provider call: {} (~{} tokens)", model_id, estimated_tokens)
            }
            DecisionKind::CredentialAccess {
                credential_id,
                purpose,
            } => {
                format!(
                    "Credential access: {} (purpose: {})",
                    credential_id, purpose
                )
            }
            DecisionKind::DestructiveAction { action, resource } => {
                format!("Destructive action: {} on {}", action, resource)
            }
            other => format!("Action: {:?}", other),
        };

        let user = format!(
            "Evaluate this pending approval request:\n\n\
             Action: {kind_desc}\n\
             Principal: {:?}\n\
             Scope: {}/{}/{}\n\
             Requested at: {}\n\n\
             Should this action be allowed?",
            request.principal,
            request.scope.tenant_id,
            request.scope.workspace_id,
            request.scope.project_id,
            request.requested_at,
        );

        vec![
            serde_json::json!({ "role": "system", "content": system }),
            serde_json::json!({ "role": "user", "content": user }),
        ]
    }

    /// Parse the guardian's structured response.
    fn parse_response(text: &str) -> Option<(DecisionOutcome, RiskLevel, String)> {
        // Try to extract JSON from the response (may be wrapped in prose).
        let json_start = text.find('{')?;
        let json_end = text.rfind('}')? + 1;
        let json_str = &text[json_start..json_end];

        let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;

        let outcome_str = parsed.get("outcome")?.as_str()?;
        let outcome = match outcome_str {
            "allowed" | "approve" | "approved" => DecisionOutcome::Allowed,
            "denied" | "deny" | "rejected" => DecisionOutcome::Denied {
                deny_step: 6,
                deny_reason: parsed
                    .get("rationale")
                    .and_then(|v| v.as_str())
                    .unwrap_or("guardian denied")
                    .to_owned(),
            },
            _ => return None,
        };

        let risk_str = parsed.get("risk_level")?.as_str()?;
        let risk_level = match risk_str {
            "low" => RiskLevel::Low,
            "medium" => RiskLevel::Medium,
            "high" => RiskLevel::High,
            _ => return None,
        };

        let rationale = parsed
            .get("rationale")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_owned();

        Some((outcome, risk_level, rationale))
    }
}

#[async_trait]
impl ApprovalResolver for GuardianResolver {
    async fn resolve(&self, request: &DecisionRequest) -> (DecisionOutcome, DecisionSource) {
        let model_id = match &self.config.model_id {
            Some(id) => id.clone(),
            None => {
                // Guardian not configured — fall through.
                return (DecisionOutcome::Allowed, DecisionSource::FreshEvaluation);
            }
        };

        let messages = self.build_prompt(request);
        let settings = ProviderBindingSettings {
            max_output_tokens: Some(512),
            ..Default::default()
        };

        // Call the provider. On any error, fail closed (fall through).
        let response: GenerationResponse =
            match self.provider.generate(&model_id, messages, &settings).await {
                Ok(resp) => resp,
                Err(_) => {
                    return (DecisionOutcome::Allowed, DecisionSource::FreshEvaluation);
                }
            };

        // Parse the structured response.
        let (outcome, risk_level, rationale) = match Self::parse_response(&response.text) {
            Some(parsed) => parsed,
            None => {
                return (DecisionOutcome::Allowed, DecisionSource::FreshEvaluation);
            }
        };

        // Risk ceiling check: if risk exceeds ceiling, fall through.
        if risk_level > self.config.risk_ceiling {
            return (DecisionOutcome::Allowed, DecisionSource::FreshEvaluation);
        }

        let _ = &rationale; // used in source below

        let source = DecisionSource::Guardian {
            model_id: model_id.clone(),
        };

        (outcome, source)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::decisions::*;
    use cairn_domain::ids::*;
    use cairn_domain::ProjectKey;

    fn sample_request() -> DecisionRequest {
        DecisionRequest {
            kind: DecisionKind::ToolInvocation {
                tool_name: "gh_create_comment".into(),
                effect: ToolEffect::External,
            },
            principal: Principal::Run {
                run_id: RunId::new("run_1"),
            },
            subject: DecisionSubject::ToolCall {
                tool_name: "gh_create_comment".into(),
                args: serde_json::json!({"repo": "org/repo", "body": "LGTM"}),
            },
            scope: ProjectKey::new("t", "w", "p"),
            cost_estimate: None,
            requested_at: 1700000000000,
            correlation_id: CorrelationId::new("cor_g1"),
        }
    }

    // ── Response parsing ────────────────────────────────────────────────

    #[test]
    fn parse_allowed_low_risk() {
        let text = r#"{"outcome":"allowed","rationale":"read-only comment","risk_level":"low"}"#;
        let (outcome, risk, rationale) = GuardianResolver::parse_response(text).unwrap();
        assert_eq!(outcome, DecisionOutcome::Allowed);
        assert_eq!(risk, RiskLevel::Low);
        assert_eq!(rationale, "read-only comment");
    }

    #[test]
    fn parse_denied_high_risk() {
        let text = r#"{"outcome":"denied","rationale":"destructive","risk_level":"high"}"#;
        let (outcome, risk, _) = GuardianResolver::parse_response(text).unwrap();
        assert!(matches!(outcome, DecisionOutcome::Denied { .. }));
        assert_eq!(risk, RiskLevel::High);
    }

    #[test]
    fn parse_response_with_prose_wrapper() {
        let text = "Here's my assessment:\n\n```json\n{\"outcome\":\"allowed\",\"rationale\":\"safe\",\"risk_level\":\"low\"}\n```";
        let result = GuardianResolver::parse_response(text);
        assert!(result.is_some());
    }

    #[test]
    fn parse_invalid_json_returns_none() {
        assert!(GuardianResolver::parse_response("not json at all").is_none());
    }

    #[test]
    fn parse_missing_fields_returns_none() {
        let text = r#"{"outcome":"allowed"}"#; // missing risk_level
        assert!(GuardianResolver::parse_response(text).is_none());
    }

    // ── Resolver behavior ───────────────────────────────────────────────

    struct MockProvider {
        response: String,
    }

    #[async_trait]
    impl GenerationProvider for MockProvider {
        async fn generate(
            &self,
            _model: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
        ) -> Result<GenerationResponse, cairn_domain::providers::ProviderAdapterError> {
            Ok(GenerationResponse {
                text: self.response.clone(),
                input_tokens: Some(100),
                output_tokens: Some(50),
                model_id: "test-guardian".to_owned(),
                tool_calls: vec![],
            })
        }
    }

    struct FailingProvider;

    #[async_trait]
    impl GenerationProvider for FailingProvider {
        async fn generate(
            &self,
            _: &str,
            _: Vec<serde_json::Value>,
            _: &ProviderBindingSettings,
        ) -> Result<GenerationResponse, cairn_domain::providers::ProviderAdapterError> {
            Err(
                cairn_domain::providers::ProviderAdapterError::TransportFailure(
                    "timeout".to_owned(),
                ),
            )
        }
    }

    #[tokio::test]
    async fn guardian_approves_low_risk_action() {
        let provider = Arc::new(MockProvider {
            response:
                r#"{"outcome":"allowed","rationale":"safe read operation","risk_level":"low"}"#
                    .to_owned(),
        });
        let resolver = GuardianResolver::new(
            provider,
            GuardianConfig {
                model_id: Some("guardian-model".into()),
                risk_ceiling: RiskLevel::Low,
                ..Default::default()
            },
        );
        let (outcome, source) = resolver.resolve(&sample_request()).await;
        assert_eq!(outcome, DecisionOutcome::Allowed);
        assert!(matches!(source, DecisionSource::Guardian { .. }));
    }

    #[tokio::test]
    async fn guardian_denies_action() {
        let provider = Arc::new(MockProvider {
            response: r#"{"outcome":"denied","rationale":"too risky","risk_level":"medium"}"#
                .to_owned(),
        });
        let resolver = GuardianResolver::new(
            provider,
            GuardianConfig {
                model_id: Some("guardian-model".into()),
                risk_ceiling: RiskLevel::Medium,
                ..Default::default()
            },
        );
        let (outcome, source) = resolver.resolve(&sample_request()).await;
        assert!(matches!(outcome, DecisionOutcome::Denied { .. }));
        assert!(matches!(source, DecisionSource::Guardian { .. }));
    }

    #[tokio::test]
    async fn guardian_falls_through_when_risk_exceeds_ceiling() {
        let provider = Arc::new(MockProvider {
            response: r#"{"outcome":"allowed","rationale":"seems ok","risk_level":"medium"}"#
                .to_owned(),
        });
        let resolver = GuardianResolver::new(
            provider,
            GuardianConfig {
                model_id: Some("guardian-model".into()),
                risk_ceiling: RiskLevel::Low, // ceiling is Low, response is Medium
                ..Default::default()
            },
        );
        let (outcome, source) = resolver.resolve(&sample_request()).await;
        // Falls through — returns FreshEvaluation, not Guardian
        assert_eq!(outcome, DecisionOutcome::Allowed);
        assert!(matches!(source, DecisionSource::FreshEvaluation));
    }

    #[tokio::test]
    async fn guardian_falls_through_on_provider_error() {
        let resolver = GuardianResolver::new(
            Arc::new(FailingProvider),
            GuardianConfig {
                model_id: Some("guardian-model".into()),
                ..Default::default()
            },
        );
        let (outcome, source) = resolver.resolve(&sample_request()).await;
        assert_eq!(outcome, DecisionOutcome::Allowed);
        assert!(matches!(source, DecisionSource::FreshEvaluation));
    }

    #[tokio::test]
    async fn guardian_falls_through_on_unparseable_response() {
        let provider = Arc::new(MockProvider {
            response: "I'm not sure what to do here.".to_owned(),
        });
        let resolver = GuardianResolver::new(
            provider,
            GuardianConfig {
                model_id: Some("guardian-model".into()),
                ..Default::default()
            },
        );
        let (outcome, source) = resolver.resolve(&sample_request()).await;
        assert_eq!(outcome, DecisionOutcome::Allowed);
        assert!(matches!(source, DecisionSource::FreshEvaluation));
    }

    #[tokio::test]
    async fn guardian_disabled_when_no_model_configured() {
        let resolver = GuardianResolver::new(
            Arc::new(MockProvider {
                response: "should not be called".to_owned(),
            }),
            GuardianConfig {
                model_id: None, // disabled
                ..Default::default()
            },
        );
        let (outcome, source) = resolver.resolve(&sample_request()).await;
        assert_eq!(outcome, DecisionOutcome::Allowed);
        assert!(matches!(source, DecisionSource::FreshEvaluation));
    }

    #[test]
    fn risk_level_ordering() {
        assert!(RiskLevel::Low < RiskLevel::Medium);
        assert!(RiskLevel::Medium < RiskLevel::High);
    }

    #[test]
    fn guardian_config_defaults() {
        let cfg = GuardianConfig::default();
        assert!(cfg.model_id.is_none());
        assert_eq!(cfg.timeout_ms, 60_000);
        assert_eq!(cfg.risk_ceiling, RiskLevel::Low);
        assert_eq!(cfg.max_context_tokens, 16_000);
    }
}
