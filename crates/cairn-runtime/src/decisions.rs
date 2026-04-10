//! Unified Decision Layer (RFC 019).
//!
//! Composes the existing guardrail, approval, budget, and visibility services
//! into a single atomic evaluation with one truth per decision.
//!
//! The `DecisionService` is the only call site for the RFC 018 resolver chain
//! and the only emitter of `DecisionRecorded` events.

use async_trait::async_trait;
use cairn_domain::decisions::{
    DecisionEvent, DecisionKey, DecisionOutcome, DecisionPolicy, DecisionRequest,
    DecisionScopeRef,
};
use cairn_domain::ids::DecisionId;
use cairn_domain::ProjectKey;

/// Error type for the decision service.
#[derive(Debug)]
pub enum DecisionError {
    /// An internal service call failed.
    Internal(String),
    /// The request was malformed.
    InvalidRequest(String),
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionError::Internal(msg) => write!(f, "decision error: {msg}"),
            DecisionError::InvalidRequest(msg) => write!(f, "invalid decision request: {msg}"),
        }
    }
}

impl std::error::Error for DecisionError {}

/// Result of evaluating a decision through the unified layer.
#[derive(Clone, Debug)]
pub struct DecisionResult {
    /// The unique ID assigned to this decision.
    pub decision_id: DecisionId,
    /// Whether the action is allowed or denied.
    pub outcome: DecisionOutcome,
    /// The cache key derived from the request (for cache operations).
    pub decision_key: DecisionKey,
    /// The event emitted by this evaluation.
    pub event: DecisionEvent,
}

/// Cache entry states for the singleflight pattern (RFC 019 §5).
#[derive(Clone, Debug)]
pub enum CacheEntryState {
    /// No entry exists for this key.
    Miss,
    /// Another request is being evaluated for this key.
    Pending {
        owner_decision_id: DecisionId,
        started_at: u64,
    },
    /// A resolved (possibly expired) entry exists.
    Resolved {
        decision_id: DecisionId,
        outcome: DecisionOutcome,
        expires_at: u64,
    },
}

/// Unified decision layer (RFC 019).
///
/// Evaluates "can I do this?" requests through the canonical 8-step order:
///
/// 1. Scope check (RFC 008)
/// 2. Visibility check (VisibilityContext)
/// 3. Guardrail check (GuardrailService)
/// 4. Budget check (BudgetService)
/// 5. Cache lookup (singleflight: Miss / Pending / Resolved)
/// 6. Approval resolution (RFC 018 resolver chain)
/// 7. Cache write (Pending → Resolved)
/// 8. Return outcome
///
/// No step is skipped. Every allow is the result of all steps passing.
/// Every deny comes from a specific step with an attributable reason.
#[async_trait]
pub trait DecisionService: Send + Sync {
    /// Evaluate a decision request through the full 8-step pipeline.
    ///
    /// Returns `DecisionResult` with the outcome, decision key, and the
    /// emitted event. The caller (orchestrator execute phase, marketplace,
    /// provider router) reads `outcome` to decide whether to proceed.
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResult, DecisionError>;

    /// Look up the default policy for a decision kind.
    ///
    /// Returns `None` if no policy is configured (falls back to deny).
    fn policy_for_kind(&self, kind_tag: &str) -> Option<DecisionPolicy>;

    /// Look up a cached decision by key.
    async fn cache_lookup(
        &self,
        key: &DecisionKey,
    ) -> Result<CacheEntryState, DecisionError>;

    /// Invalidate a specific cached decision by its decision ID.
    async fn invalidate(
        &self,
        decision_id: &DecisionId,
        reason: &str,
        invalidated_by: cairn_domain::decisions::ActorRef,
    ) -> Result<(), DecisionError>;

    /// Bulk invalidate cached decisions by scope and optional kind filter.
    async fn invalidate_by_scope(
        &self,
        scope: &DecisionScopeRef,
        kind_filter: Option<&str>,
        reason: &str,
        invalidated_by: cairn_domain::decisions::ActorRef,
    ) -> Result<u32, DecisionError>;

    /// Selective invalidation: invalidate cached decisions whose reasoning
    /// chain referenced a specific guardrail rule ID.
    async fn invalidate_by_rule(
        &self,
        rule_id: &cairn_domain::PolicyId,
        reason: &str,
        invalidated_by: cairn_domain::decisions::ActorRef,
    ) -> Result<u32, DecisionError>;

    /// List active cached decisions (learned rules) for a scope.
    async fn list_cached(
        &self,
        scope: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<CachedDecisionSummary>, DecisionError>;

    /// Get a specific decision by ID.
    async fn get_decision(
        &self,
        decision_id: &DecisionId,
    ) -> Result<Option<DecisionEvent>, DecisionError>;
}

/// Summary of a cached decision for the operator "learned rules" view.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CachedDecisionSummary {
    pub decision_id: DecisionId,
    pub decision_key: DecisionKey,
    pub outcome: DecisionOutcome,
    pub kind_tag: String,
    pub scope: DecisionScopeRef,
    pub ttl_remaining_secs: u64,
    pub source: cairn_domain::decisions::DecisionSource,
    pub hit_count: u64,
    pub created_at: u64,
    pub expires_at: u64,
}

// ── Stub implementation ─────────────────────────────────────────────────────

/// Stub `DecisionService` that allows everything.
///
/// Used during development before the full 8-step pipeline is wired.
/// Every request returns `Allowed` with a fresh evaluation source and
/// an empty reasoning chain.
pub struct StubDecisionService;

impl StubDecisionService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubDecisionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DecisionService for StubDecisionService {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResult, DecisionError> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        let decision_id = DecisionId::new(format!("dec_{now_ms}"));
        let decision_key = DecisionKey {
            kind_tag: kind_tag_for(&request.kind),
            scope_ref: DecisionScopeRef::Project(request.scope.clone()),
            semantic_hash: "stub".into(),
        };

        let event = DecisionEvent::DecisionRecorded {
            decision_id: decision_id.clone(),
            request: request.clone(),
            outcome: DecisionOutcome::Allowed,
            reasoning_chain: vec![cairn_domain::decisions::StepResult {
                step: 8,
                name: "stub_allow_all".into(),
                outcome: "allow".into(),
                detail: Some("StubDecisionService — all requests allowed".into()),
                rule_ids: vec![],
            }],
            source: cairn_domain::decisions::DecisionSource::FreshEvaluation,
            resolved_by: None,
            cached_for: None,
            decided_at: now_ms,
        };

        Ok(DecisionResult {
            decision_id,
            outcome: DecisionOutcome::Allowed,
            decision_key,
            event,
        })
    }

    fn policy_for_kind(&self, _kind_tag: &str) -> Option<DecisionPolicy> {
        None
    }

    async fn cache_lookup(
        &self,
        _key: &DecisionKey,
    ) -> Result<CacheEntryState, DecisionError> {
        Ok(CacheEntryState::Miss)
    }

    async fn invalidate(
        &self,
        _decision_id: &DecisionId,
        _reason: &str,
        _invalidated_by: cairn_domain::decisions::ActorRef,
    ) -> Result<(), DecisionError> {
        Ok(())
    }

    async fn invalidate_by_scope(
        &self,
        _scope: &DecisionScopeRef,
        _kind_filter: Option<&str>,
        _reason: &str,
        _invalidated_by: cairn_domain::decisions::ActorRef,
    ) -> Result<u32, DecisionError> {
        Ok(0)
    }

    async fn invalidate_by_rule(
        &self,
        _rule_id: &cairn_domain::PolicyId,
        _reason: &str,
        _invalidated_by: cairn_domain::decisions::ActorRef,
    ) -> Result<u32, DecisionError> {
        Ok(0)
    }

    async fn list_cached(
        &self,
        _scope: &ProjectKey,
        _limit: usize,
    ) -> Result<Vec<CachedDecisionSummary>, DecisionError> {
        Ok(vec![])
    }

    async fn get_decision(
        &self,
        _decision_id: &DecisionId,
    ) -> Result<Option<DecisionEvent>, DecisionError> {
        Ok(None)
    }
}

/// Derive the `kind_tag` string from a `DecisionKind`.
pub fn kind_tag_for(kind: &cairn_domain::decisions::DecisionKind) -> String {
    use cairn_domain::decisions::DecisionKind;
    match kind {
        DecisionKind::ToolInvocation { .. } => "tool_invocation".into(),
        DecisionKind::ProviderCall { .. } => "provider_call".into(),
        DecisionKind::PluginEnablement { .. } => "plugin_enablement".into(),
        DecisionKind::WorkspaceProvision { .. } => "workspace_provision".into(),
        DecisionKind::CredentialAccess { .. } => "credential_access".into(),
        DecisionKind::TriggerFire { .. } => "trigger_fire".into(),
        DecisionKind::DestructiveAction { .. } => "destructive_action".into(),
        DecisionKind::Other(s) => format!("other:{s}"),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::decisions::*;
    use cairn_domain::ids::*;

    fn sample_request() -> DecisionRequest {
        DecisionRequest {
            kind: DecisionKind::ToolInvocation {
                tool_name: "shell_exec".into(),
                effect: ToolEffect::External,
            },
            principal: Principal::Run {
                run_id: RunId::new("run_1"),
            },
            subject: DecisionSubject::ToolCall {
                tool_name: "shell_exec".into(),
                args: serde_json::json!({"command": "ls"}),
            },
            scope: ProjectKey::new("t", "w", "p"),
            cost_estimate: None,
            requested_at: 1700000000000,
            correlation_id: CorrelationId::new("cor_1"),
        }
    }

    #[tokio::test]
    async fn stub_service_allows_everything() {
        let svc = StubDecisionService::new();
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert_eq!(result.outcome, DecisionOutcome::Allowed);
        assert!(result.decision_id.as_str().starts_with("dec_"));
    }

    #[tokio::test]
    async fn stub_cache_lookup_returns_miss() {
        let svc = StubDecisionService::new();
        let key = DecisionKey {
            kind_tag: "tool_invocation".into(),
            scope_ref: DecisionScopeRef::Project(ProjectKey::new("t", "w", "p")),
            semantic_hash: "abc".into(),
        };
        let state = svc.cache_lookup(&key).await.unwrap();
        assert!(matches!(state, CacheEntryState::Miss));
    }

    #[tokio::test]
    async fn stub_list_cached_returns_empty() {
        let svc = StubDecisionService::new();
        let cached = svc.list_cached(&ProjectKey::new("t", "w", "p"), 10).await.unwrap();
        assert!(cached.is_empty());
    }

    #[test]
    fn kind_tag_derivation() {
        assert_eq!(
            kind_tag_for(&DecisionKind::ToolInvocation {
                tool_name: "x".into(),
                effect: ToolEffect::External,
            }),
            "tool_invocation"
        );
        assert_eq!(
            kind_tag_for(&DecisionKind::ProviderCall {
                model_id: "m".into(),
                estimated_tokens: 100,
            }),
            "provider_call"
        );
        assert_eq!(
            kind_tag_for(&DecisionKind::TriggerFire {
                trigger_id: "t".into(),
                signal_type: "webhook".into(),
            }),
            "trigger_fire"
        );
        assert_eq!(
            kind_tag_for(&DecisionKind::Other("custom".into())),
            "other:custom"
        );
    }

    #[tokio::test]
    async fn stub_invalidate_is_noop() {
        let svc = StubDecisionService::new();
        svc.invalidate(
            &DecisionId::new("dec_1"),
            "test",
            ActorRef::SystemPolicyChange,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn stub_event_has_reasoning_chain() {
        let svc = StubDecisionService::new();
        let result = svc.evaluate(sample_request()).await.unwrap();
        if let DecisionEvent::DecisionRecorded { reasoning_chain, .. } = &result.event {
            assert_eq!(reasoning_chain.len(), 1);
            assert_eq!(reasoning_chain[0].name, "stub_allow_all");
        } else {
            panic!("expected DecisionRecorded event");
        }
    }
}
