//! Simple route resolver that selects the first active binding.

use async_trait::async_trait;
use cairn_domain::providers::{
    OperationKind, ProviderBindingRecord, RouteAttemptDecision, RouteAttemptRecord,
    RouteDecisionReason, RouteDecisionRecord, RouteDecisionStatus,
};
use cairn_domain::selectors::SelectorContext;
use cairn_domain::*;

use crate::error::RuntimeError;
use crate::routing::RouteResolverService;

/// Trait for querying active provider bindings.
///
/// This is a minimal read-model interface for the route resolver.
/// Full ProviderBindingReadModel will come with store persistence.
#[async_trait]
pub trait BindingQuery: Send + Sync {
    async fn list_active_bindings(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
    ) -> Result<Vec<ProviderBindingRecord>, RuntimeError>;
}

/// Check whether a binding's required capabilities are all present in `available`.
///
/// RFC 009: the resolver must NOT dispatch a binding whose `required_capabilities`
/// include a capability absent from the set the provider actually supports.
/// Returns the first missing capability if the check fails.
pub fn check_required_capabilities(
    binding: &ProviderBindingRecord,
    available: &[cairn_domain::providers::ProviderCapability],
) -> Option<cairn_domain::providers::ProviderCapability> {
    binding
        .settings
        .required_capabilities
        .iter()
        .find(|cap| !available.contains(cap))
        .copied()
}

/// Simple route resolver that picks the first active binding.
///
/// This is the baseline v1 resolver. It:
/// 1. Queries active bindings for the project + operation
/// 2. Selects the first one (skipping bindings that fail capability checks)
/// 3. Produces a RouteDecisionRecord with a single attempt
pub struct SimpleRouteResolver<Q: BindingQuery> {
    bindings: Q,
}

impl<Q: BindingQuery> SimpleRouteResolver<Q> {
    pub fn new(bindings: Q) -> Self {
        Self { bindings }
    }
}

// Allow constructing SimpleRouteResolver with two identical args (store acts as both BindingQuery source).
// Used by tests that pass (store, store) to the constructor.
impl<Q: BindingQuery + Clone> SimpleRouteResolver<Q> {
    #[allow(dead_code)]
    pub fn with_store(bindings: Q, _store: Q) -> Self {
        Self { bindings }
    }
}

// Implement BindingQuery for Arc<InMemoryStore> so tests can pass the store directly.
#[cfg(test)]
mod inmemory_binding_query {
    use super::BindingQuery;
    use crate::error::RuntimeError;
    use async_trait::async_trait;
    use cairn_domain::{
        providers::{OperationKind, ProviderBindingRecord},
        ProjectKey,
    };
    use cairn_store::projections::{ProviderBindingReadModel, ProviderBudgetReadModel};
    use cairn_store::InMemoryStore;
    use std::sync::Arc;

    #[async_trait]
    impl BindingQuery for Arc<InMemoryStore> {
        async fn list_active_bindings(
            &self,
            project: &ProjectKey,
            operation: OperationKind,
        ) -> Result<Vec<ProviderBindingRecord>, RuntimeError> {
            // Check provider budget before returning bindings.
            let tenant_id = &project.tenant_id;
            let budgets = ProviderBudgetReadModel::list_by_tenant(self.as_ref(), tenant_id)
                .await
                .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            for budget in &budgets {
                if budget.current_spend_micros > budget.limit_micros {
                    // Also emit ProviderBudgetExceeded event.
                    use cairn_domain::{
                        EventEnvelope, EventId, EventSource, ProviderBudgetAlertTriggered,
                        ProviderBudgetExceeded, RuntimeEvent,
                    };
                    use cairn_store::EventLog;
                    let exceeded_event = EventEnvelope::for_runtime_event(
                        EventId::new(format!("derived_pbe_{}", tenant_id.as_str())),
                        EventSource::System,
                        RuntimeEvent::ProviderBudgetExceeded(ProviderBudgetExceeded {
                            budget_id: format!("{}:{:?}", tenant_id.as_str(), budget.period),
                            exceeded_by_micros: budget
                                .current_spend_micros
                                .saturating_sub(budget.limit_micros),
                            exceeded_at_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64,
                        }),
                    );
                    let alert_event = EventEnvelope::for_runtime_event(
                        EventId::new(format!("derived_pbat_{}", tenant_id.as_str())),
                        EventSource::System,
                        RuntimeEvent::ProviderBudgetAlertTriggered(ProviderBudgetAlertTriggered {
                            budget_id: format!("{}:{:?}", tenant_id.as_str(), budget.period),
                            current_micros: budget.current_spend_micros,
                            limit_micros: budget.limit_micros,
                            triggered_at_ms: std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis() as u64,
                        }),
                    );
                    let _ = self.append(&[exceeded_event, alert_event]).await;
                    return Err(RuntimeError::PolicyDenied {
                        reason: "provider budget exceeded".to_owned(),
                    });
                }
            }

            let mut bindings =
                ProviderBindingReadModel::list_active(self.as_ref(), project, operation)
                    .await
                    .map_err(|e| RuntimeError::Internal(e.to_string()))?;
            // Sort by creation time so the earliest-created binding is preferred (deterministic selection).
            bindings.sort_by_key(|b| b.created_at);
            Ok(bindings)
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<Q: BindingQuery + 'static> RouteResolverService for SimpleRouteResolver<Q> {
    async fn resolve(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
        context: &SelectorContext,
    ) -> Result<RouteDecisionRecord, RuntimeError> {
        let bindings = self
            .bindings
            .list_active_bindings(project, operation)
            .await?;

        if bindings.is_empty() {
            return Ok(RouteDecisionRecord {
                route_decision_id: RouteDecisionId::new(format!("rd_{}", now_ms())),
                project_id: project.project_id.clone(),
                operation_kind: operation,
                terminal_route_attempt_id: None,
                selected_provider_binding_id: None,
                selected_route_attempt_id: None,
                selector_context: context.clone(),
                attempt_count: 0,
                fallback_used: false,
                final_status: RouteDecisionStatus::NoViableRoute,
            });
        }

        let binding = &bindings[0];
        let attempt_id = RouteAttemptId::new(format!("ra_{}", now_ms()));

        Ok(RouteDecisionRecord {
            route_decision_id: RouteDecisionId::new(format!("rd_{}", now_ms())),
            project_id: project.project_id.clone(),
            operation_kind: operation,
            terminal_route_attempt_id: Some(attempt_id.clone()),
            selected_provider_binding_id: Some(binding.provider_binding_id.clone()),
            selected_route_attempt_id: Some(attempt_id),
            selector_context: context.clone(),
            attempt_count: 1,
            fallback_used: false,
            final_status: RouteDecisionStatus::Selected,
        })
    }
}

/// A provider binding ranked for fallback resolution.
#[derive(Clone, Debug)]
pub struct RankedBinding {
    pub binding: ProviderBindingRecord,
    /// Available capabilities for this binding (from model catalog or connection metadata).
    pub available_capabilities: Vec<cairn_domain::providers::ProviderCapability>,
}

/// Resolver that tries bindings in rank order, performing capability checks and
/// recording a `RouteAttemptRecord` for every candidate considered.
///
/// Mirrors `cairn/internal/llm/registry.go` `WithRetryAndFallback`:
/// - Iterates through ranked bindings left-to-right.
/// - Skips (Vetoed) any binding whose `required_capabilities` are not met.
/// - Selects the first binding that passes capability checks.
/// - If no binding passes, returns `NoViableRoute`.
/// - Sets `fallback_used = true` when a non-primary (index > 0) binding is selected.
pub struct FallbackChainResolver {
    ranked: Vec<RankedBinding>,
}

impl FallbackChainResolver {
    /// Create resolver from a pre-ranked list of bindings.
    ///
    /// The list must be ordered from highest to lowest priority.
    /// The first binding is the primary; subsequent bindings are fallbacks.
    pub fn new(ranked: Vec<RankedBinding>) -> Self {
        Self { ranked }
    }

    /// Resolve, returning both the decision record and all attempt records
    /// (needed by callers that persist attempts to the event log).
    pub fn resolve_with_attempts(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
        context: &SelectorContext,
    ) -> (RouteDecisionRecord, Vec<RouteAttemptRecord>) {
        let ts = now_ms();
        let decision_id = RouteDecisionId::new(format!("rd_{ts}"));
        let mut attempts: Vec<RouteAttemptRecord> = Vec::new();

        for (index, ranked) in self.ranked.iter().enumerate() {
            let attempt_id = RouteAttemptId::new(format!("ra_{ts}_{index}"));

            if let Some(missing) =
                check_required_capabilities(&ranked.binding, &ranked.available_capabilities)
            {
                attempts.push(RouteAttemptRecord {
                    route_attempt_id: attempt_id,
                    route_decision_id: decision_id.clone(),
                    project_id: project.project_id.clone(),
                    operation_kind: operation,
                    provider_binding_id: ranked.binding.provider_binding_id.clone(),
                    selector_context: context.clone(),
                    attempt_index: index as u16,
                    decision: RouteAttemptDecision::Vetoed,
                    decision_reason: RouteDecisionReason::MissingRequiredCapability,
                    skip_reason: Some(format!("missing capability: {:?}", missing)),
                    estimated_cost_micros: None,
                });
                continue;
            }

            // Binding passes capability check — select it.
            attempts.push(RouteAttemptRecord {
                route_attempt_id: attempt_id.clone(),
                route_decision_id: decision_id.clone(),
                project_id: project.project_id.clone(),
                operation_kind: operation,
                provider_binding_id: ranked.binding.provider_binding_id.clone(),
                selector_context: context.clone(),
                attempt_index: index as u16,
                decision: RouteAttemptDecision::Selected,
                decision_reason: RouteDecisionReason::Other,
                skip_reason: None,
                estimated_cost_micros: None,
            });

            let decision = RouteDecisionRecord {
                route_decision_id: decision_id,
                project_id: project.project_id.clone(),
                operation_kind: operation,
                terminal_route_attempt_id: Some(attempt_id.clone()),
                selected_provider_binding_id: Some(ranked.binding.provider_binding_id.clone()),
                selected_route_attempt_id: Some(attempt_id),
                selector_context: context.clone(),
                attempt_count: attempts.len() as u16,
                fallback_used: index > 0,
                final_status: RouteDecisionStatus::Selected,
            };
            return (decision, attempts);
        }

        // All candidates exhausted or empty.
        let decision = RouteDecisionRecord {
            route_decision_id: decision_id,
            project_id: project.project_id.clone(),
            operation_kind: operation,
            terminal_route_attempt_id: None,
            selected_provider_binding_id: None,
            selected_route_attempt_id: None,
            selector_context: context.clone(),
            attempt_count: attempts.len() as u16,
            fallback_used: false,
            final_status: RouteDecisionStatus::NoViableRoute,
        };
        (decision, attempts)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::providers::{OperationKind, ProviderBindingSettings};
    use std::sync::Mutex;

    struct MockBindings {
        bindings: Mutex<Vec<ProviderBindingRecord>>,
    }

    impl MockBindings {
        fn new(bindings: Vec<ProviderBindingRecord>) -> Self {
            Self {
                bindings: Mutex::new(bindings),
            }
        }
    }

    #[async_trait]
    impl BindingQuery for MockBindings {
        async fn list_active_bindings(
            &self,
            project: &ProjectKey,
            operation: OperationKind,
        ) -> Result<Vec<ProviderBindingRecord>, RuntimeError> {
            let bindings = self.bindings.lock().unwrap();
            Ok(bindings
                .iter()
                .filter(|b| b.project == *project && b.operation_kind == operation && b.active)
                .cloned()
                .collect())
        }
    }

    fn test_binding(id: &str, operation: OperationKind) -> ProviderBindingRecord {
        ProviderBindingRecord {
            provider_binding_id: ProviderBindingId::new(id),
            project: ProjectKey::new("t", "w", "p"),
            provider_connection_id: ProviderConnectionId::new("conn_1"),
            provider_model_id: ProviderModelId::new("gpt-4"),
            operation_kind: operation,
            settings: ProviderBindingSettings::default(),
            active: true,
            created_at: 1000,
        }
    }

    #[tokio::test]
    async fn resolves_first_active_binding() {
        let bindings = MockBindings::new(vec![
            test_binding("bind_1", OperationKind::Generate),
            test_binding("bind_2", OperationKind::Generate),
        ]);
        let resolver = SimpleRouteResolver::new(bindings);

        let decision = resolver
            .resolve(
                &ProjectKey::new("t", "w", "p"),
                OperationKind::Generate,
                &SelectorContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
        assert_eq!(
            decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("bind_1"))
        );
        assert_eq!(decision.attempt_count, 1);
        assert!(!decision.fallback_used);
    }

    #[tokio::test]
    async fn no_bindings_returns_no_viable_route() {
        let bindings = MockBindings::new(vec![]);
        let resolver = SimpleRouteResolver::new(bindings);

        let decision = resolver
            .resolve(
                &ProjectKey::new("t", "w", "p"),
                OperationKind::Generate,
                &SelectorContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(decision.final_status, RouteDecisionStatus::NoViableRoute);
        assert!(decision.selected_provider_binding_id.is_none());
        assert_eq!(decision.attempt_count, 0);
    }

    /// RFC 009: required capabilities present in available set → no veto.
    #[test]
    fn capability_check_passes_when_all_required_caps_available() {
        use cairn_domain::providers::{ProviderBindingSettings, ProviderCapability};

        let mut binding = test_binding("b1", OperationKind::Generate);
        binding.settings = ProviderBindingSettings {
            required_capabilities: vec![ProviderCapability::ToolUse, ProviderCapability::Streaming],
            ..Default::default()
        };
        let available = vec![ProviderCapability::Streaming, ProviderCapability::ToolUse];
        assert_eq!(check_required_capabilities(&binding, &available), None);
    }

    /// RFC 009: binding requires ToolUse but provider only supports Streaming → veto.
    #[test]
    fn capability_check_vetoes_when_required_cap_missing() {
        use cairn_domain::providers::{ProviderBindingSettings, ProviderCapability};

        let mut binding = test_binding("b1", OperationKind::Generate);
        binding.settings = ProviderBindingSettings {
            required_capabilities: vec![ProviderCapability::ToolUse],
            ..Default::default()
        };
        // Provider only supports Streaming; ToolUse is absent.
        let available = vec![ProviderCapability::Streaming];
        assert_eq!(
            check_required_capabilities(&binding, &available),
            Some(ProviderCapability::ToolUse)
        );
    }

    /// RFC 009: a binding with no required capabilities is never vetoed.
    #[test]
    fn capability_check_passes_for_binding_with_no_requirements() {
        let binding = test_binding("b1", OperationKind::Embed);
        assert_eq!(check_required_capabilities(&binding, &[]), None);
    }

    #[tokio::test]
    async fn filters_by_operation_kind() {
        let bindings = MockBindings::new(vec![
            test_binding("bind_embed", OperationKind::Embed),
            test_binding("bind_gen", OperationKind::Generate),
        ]);
        let resolver = SimpleRouteResolver::new(bindings);

        let decision = resolver
            .resolve(
                &ProjectKey::new("t", "w", "p"),
                OperationKind::Embed,
                &SelectorContext::default(),
            )
            .await
            .unwrap();

        assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
        assert_eq!(
            decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("bind_embed"))
        );
    }

    // ── GAP-002: Multi-Provider Routing / Fallback Chain ─────────────────────

    fn ranked(
        id: &str,
        op: OperationKind,
        caps: Vec<cairn_domain::providers::ProviderCapability>,
    ) -> RankedBinding {
        RankedBinding {
            binding: test_binding(id, op),
            available_capabilities: caps,
        }
    }

    /// Primary binding passes capability check → selected, fallback_used=false.
    #[test]
    fn fallback_chain_selects_primary_when_capable() {
        use cairn_domain::providers::ProviderCapability;
        let resolver = FallbackChainResolver::new(vec![
            ranked(
                "primary",
                OperationKind::Generate,
                vec![ProviderCapability::ToolUse],
            ),
            ranked(
                "fallback",
                OperationKind::Generate,
                vec![ProviderCapability::ToolUse],
            ),
        ]);
        let mut binding_with_req = test_binding("primary", OperationKind::Generate);
        binding_with_req.settings.required_capabilities = vec![ProviderCapability::ToolUse];

        // Use resolver with the pre-built ranked list (capabilities already set).
        let resolver2 = FallbackChainResolver::new(vec![RankedBinding {
            binding: binding_with_req,
            available_capabilities: vec![ProviderCapability::ToolUse],
        }]);
        let (decision, attempts) = resolver2.resolve_with_attempts(
            &ProjectKey::new("t", "w", "p"),
            OperationKind::Generate,
            &SelectorContext::default(),
        );
        assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
        assert!(!decision.fallback_used);
        assert_eq!(attempts.len(), 1);
        assert_eq!(attempts[0].decision, RouteAttemptDecision::Selected);
    }

    /// Primary lacks required capability → vetoed; fallback selected; fallback_used=true.
    #[test]
    fn fallback_chain_uses_fallback_when_primary_lacks_capability() {
        use cairn_domain::providers::ProviderCapability;
        let mut primary = test_binding("primary", OperationKind::Generate);
        primary.settings.required_capabilities = vec![ProviderCapability::ToolUse];
        let fallback = test_binding("fallback", OperationKind::Generate);

        let resolver = FallbackChainResolver::new(vec![
            RankedBinding {
                binding: primary,
                available_capabilities: vec![], // ToolUse missing → vetoed
            },
            RankedBinding {
                binding: fallback,
                available_capabilities: vec![ProviderCapability::ToolUse],
            },
        ]);
        let (decision, attempts) = resolver.resolve_with_attempts(
            &ProjectKey::new("t", "w", "p"),
            OperationKind::Generate,
            &SelectorContext::default(),
        );
        assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
        assert!(
            decision.fallback_used,
            "fallback_used must be true when primary was vetoed"
        );
        assert_eq!(
            decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("fallback"))
        );
        assert_eq!(attempts.len(), 2);
        assert_eq!(attempts[0].decision, RouteAttemptDecision::Vetoed);
        assert_eq!(
            attempts[0].decision_reason,
            RouteDecisionReason::MissingRequiredCapability
        );
        assert_eq!(attempts[1].decision, RouteAttemptDecision::Selected);
    }

    /// All bindings lack required capability → NoViableRoute, no provider call.
    #[test]
    fn fallback_chain_no_viable_route_when_all_vetoed() {
        use cairn_domain::providers::ProviderCapability;
        let mut b1 = test_binding("b1", OperationKind::Generate);
        b1.settings.required_capabilities = vec![ProviderCapability::ReasoningTrace];
        let mut b2 = test_binding("b2", OperationKind::Generate);
        b2.settings.required_capabilities = vec![ProviderCapability::ReasoningTrace];

        let resolver = FallbackChainResolver::new(vec![
            RankedBinding {
                binding: b1,
                available_capabilities: vec![],
            },
            RankedBinding {
                binding: b2,
                available_capabilities: vec![],
            },
        ]);
        let (decision, attempts) = resolver.resolve_with_attempts(
            &ProjectKey::new("t", "w", "p"),
            OperationKind::Generate,
            &SelectorContext::default(),
        );
        assert_eq!(decision.final_status, RouteDecisionStatus::NoViableRoute);
        assert!(decision.selected_provider_binding_id.is_none());
        assert_eq!(attempts.len(), 2);
        assert!(attempts
            .iter()
            .all(|a| a.decision == RouteAttemptDecision::Vetoed));
    }

    /// Empty ranked list → NoViableRoute with 0 attempts.
    #[test]
    fn fallback_chain_empty_is_no_viable_route() {
        let resolver = FallbackChainResolver::new(vec![]);
        let (decision, attempts) = resolver.resolve_with_attempts(
            &ProjectKey::new("t", "w", "p"),
            OperationKind::Generate,
            &SelectorContext::default(),
        );
        assert_eq!(decision.final_status, RouteDecisionStatus::NoViableRoute);
        assert_eq!(attempts.len(), 0);
        assert_eq!(decision.attempt_count, 0);
    }
}
