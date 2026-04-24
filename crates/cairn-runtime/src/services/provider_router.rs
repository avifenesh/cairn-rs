//! RFC 009 provider routing with fallback chain, capability matching,
//! cost-aware selection, and health-aware dispatch.
//!
//! ## Architecture
//!
//! ```text
//! ProviderRouter
//!  ├─ capability filter  (drop bindings missing required caps)
//!  ├─ health filter      (deprioritize unhealthy providers)
//!  ├─ cost-aware sort    (cheaper providers first, configurable weight)
//!  └─ fallback dispatch  (try providers in order; on failure, advance)
//! ```
//!
//! The router wraps one or more [`GenerationProvider`] implementations and
//! dispatches through the ranked chain until a call succeeds or all
//! candidates are exhausted.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, OperationKind, ProviderAdapterError,
    ProviderBindingRecord, ProviderBindingSettings, ProviderCapability, ProviderModelCapability,
    RouteAttemptDecision, RouteAttemptRecord, RouteDecisionReason, RouteDecisionRecord,
    RouteDecisionStatus,
};
use cairn_domain::selectors::SelectorContext;
use cairn_domain::*;

use super::provider_health_tracker::ProviderHealthTracker;
use super::route_resolver_impl::check_required_capabilities;

// ── Configuration ────────────────────────────────────────────────────────────

/// How much weight cost receives when ranking providers.
///
/// `0.0` = ignore cost (pure priority order).
/// `1.0` = cost is the only ranking signal.
#[derive(Clone, Debug)]
pub struct RoutingConfig {
    /// Weight of cost in the composite score [0.0, 1.0].
    pub cost_weight: f64,
    /// When true, unhealthy providers are moved to the end instead of dropped.
    pub allow_unhealthy_fallback: bool,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            cost_weight: 0.3,
            allow_unhealthy_fallback: true,
        }
    }
}

// ── Candidate ────────────────────────────────────────────────────────────────

/// A provider binding enriched with capability + cost metadata for routing.
#[derive(Clone, Debug)]
pub struct RoutableProvider {
    pub binding: ProviderBindingRecord,
    pub capabilities: Vec<ProviderCapability>,
    /// Estimated cost per 1k input tokens (USD). `None` = unknown.
    pub cost_per_1k_input: Option<f64>,
    /// Estimated cost per 1k output tokens (USD). `None` = unknown.
    pub cost_per_1k_output: Option<f64>,
}

impl RoutableProvider {
    pub fn new(binding: ProviderBindingRecord, caps: Vec<ProviderCapability>) -> Self {
        Self {
            binding,
            capabilities: caps,
            cost_per_1k_input: None,
            cost_per_1k_output: None,
        }
    }

    pub fn with_cost(mut self, input: f64, output: f64) -> Self {
        self.cost_per_1k_input = Some(input);
        self.cost_per_1k_output = Some(output);
        self
    }

    /// Build from a binding + model capability record.
    pub fn from_capability(
        binding: ProviderBindingRecord,
        model_cap: &ProviderModelCapability,
    ) -> Self {
        Self {
            binding,
            capabilities: model_cap.capabilities.clone(),
            cost_per_1k_input: model_cap.cost_per_1k_input_tokens,
            cost_per_1k_output: model_cap.cost_per_1k_output_tokens,
        }
    }
}

// ── Routing result ───────────────────────────────────────────────────────────

/// Outcome of a routed provider dispatch.
#[derive(Debug)]
pub struct RoutingOutcome {
    pub decision: RouteDecisionRecord,
    pub attempts: Vec<RouteAttemptRecord>,
    pub response: Option<GenerationResponse>,
    /// Which providers were tried and whether they succeeded.
    pub dispatch_log: Vec<DispatchEntry>,
}

/// Log entry for one dispatch attempt.
#[derive(Debug, Clone)]
pub struct DispatchEntry {
    pub binding_id: ProviderBindingId,
    pub connection_id: ProviderConnectionId,
    pub succeeded: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

// ── Router ───────────────────────────────────────────────────────────────────

/// Provider router with fallback chain, capability matching, cost-aware
/// selection, and health tracking.
pub struct ProviderRouter {
    config: RoutingConfig,
    health: Arc<ProviderHealthTracker>,
    /// Registered generation providers keyed by connection ID.
    providers: HashMap<ProviderConnectionId, Arc<dyn GenerationProvider>>,
}

impl ProviderRouter {
    pub fn new(config: RoutingConfig, health: Arc<ProviderHealthTracker>) -> Self {
        Self {
            config,
            health,
            providers: HashMap::new(),
        }
    }

    /// Register a generation provider for a given connection ID.
    pub fn register(
        &mut self,
        connection_id: ProviderConnectionId,
        provider: Arc<dyn GenerationProvider>,
    ) {
        self.providers.insert(connection_id, provider);
    }

    /// Return a reference to the health tracker.
    pub fn health_tracker(&self) -> &Arc<ProviderHealthTracker> {
        &self.health
    }

    /// Rank, filter, and dispatch through the provider chain.
    ///
    /// Steps:
    /// 1. Filter by capability (veto bindings missing required caps).
    /// 2. Partition into healthy / unhealthy.
    /// 3. Sort each partition by cost (cheaper first, weighted by `cost_weight`).
    /// 4. Concatenate: healthy first, then unhealthy (if `allow_unhealthy_fallback`).
    /// 5. Dispatch in order; on failure, try next.
    ///
    /// `tools` is forwarded to every provider call so the model can see
    /// the native tool catalogue. Pass `&[]` only when no tools are
    /// required (text-only fallback).
    pub async fn route(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
        context: &SelectorContext,
        candidates: Vec<RoutableProvider>,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        settings: &ProviderBindingSettings,
        tools: &[serde_json::Value],
    ) -> RoutingOutcome {
        let ts = now_ms();
        let decision_id = RouteDecisionId::new(format!("rd_{ts}"));
        let mut attempts: Vec<RouteAttemptRecord> = Vec::new();
        let mut dispatch_log: Vec<DispatchEntry> = Vec::new();

        // ── Step 1: Capability filter ────────────────────────────────────
        let (capable, vetoed) = partition_by_capability(&candidates);

        for (index, v) in vetoed.iter().enumerate() {
            let missing = check_required_capabilities(&v.binding, &v.capabilities);
            attempts.push(RouteAttemptRecord {
                route_attempt_id: RouteAttemptId::new(format!("ra_{ts}_{index}")),
                route_decision_id: decision_id.clone(),
                project_id: project.project_id.clone(),
                operation_kind: operation,
                provider_binding_id: v.binding.provider_binding_id.clone(),
                selector_context: context.clone(),
                attempt_index: index as u16,
                decision: RouteAttemptDecision::Vetoed,
                decision_reason: RouteDecisionReason::MissingRequiredCapability,
                skip_reason: Some(format!("missing capability: {:?}", missing)),
                estimated_cost_micros: None,
            });
        }

        if capable.is_empty() {
            return RoutingOutcome {
                decision: no_viable_route(decision_id, project, operation, context, &attempts),
                attempts,
                response: None,
                dispatch_log,
            };
        }

        // ── Step 2+3: Health partition + cost sort ───────────────────────
        let ranked = self.rank_candidates(capable);

        // ── Step 4+5: Dispatch with fallback ─────────────────────────────
        let base_index = attempts.len();
        for (i, candidate) in ranked.iter().enumerate() {
            let attempt_index = (base_index + i) as u16;
            let attempt_id = RouteAttemptId::new(format!("ra_{ts}_{}", base_index + i));

            let provider = match self
                .providers
                .get(&candidate.binding.provider_connection_id)
            {
                Some(p) => p,
                None => {
                    attempts.push(RouteAttemptRecord {
                        route_attempt_id: attempt_id,
                        route_decision_id: decision_id.clone(),
                        project_id: project.project_id.clone(),
                        operation_kind: operation,
                        provider_binding_id: candidate.binding.provider_binding_id.clone(),
                        selector_context: context.clone(),
                        attempt_index,
                        decision: RouteAttemptDecision::Skipped,
                        decision_reason: RouteDecisionReason::TransportFailure,
                        skip_reason: Some("no registered provider adapter".into()),
                        estimated_cost_micros: None,
                    });
                    continue;
                }
            };

            let start = Instant::now();
            // Forward the caller-supplied tool catalogue — not `&[]` —
            // so the LLM can emit native tool_calls. Previous versions
            // dropped this parameter silently.
            let result = provider
                .generate(model_id, messages.clone(), settings, tools)
                .await;
            let latency_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(response) => {
                    self.health
                        .record_success(&candidate.binding.provider_connection_id, latency_ms);

                    dispatch_log.push(DispatchEntry {
                        binding_id: candidate.binding.provider_binding_id.clone(),
                        connection_id: candidate.binding.provider_connection_id.clone(),
                        succeeded: true,
                        latency_ms,
                        error: None,
                    });

                    attempts.push(RouteAttemptRecord {
                        route_attempt_id: attempt_id.clone(),
                        route_decision_id: decision_id.clone(),
                        project_id: project.project_id.clone(),
                        operation_kind: operation,
                        provider_binding_id: candidate.binding.provider_binding_id.clone(),
                        selector_context: context.clone(),
                        attempt_index,
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
                        selected_provider_binding_id: Some(
                            candidate.binding.provider_binding_id.clone(),
                        ),
                        selected_route_attempt_id: Some(attempt_id),
                        selector_context: context.clone(),
                        attempt_count: attempts.len() as u16,
                        fallback_used: i > 0 || !vetoed.is_empty(),
                        final_status: RouteDecisionStatus::Selected,
                    };

                    return RoutingOutcome {
                        decision,
                        attempts,
                        response: Some(response),
                        dispatch_log,
                    };
                }
                Err(err) => {
                    let error_str = err.to_string();
                    self.health.record_failure(
                        &candidate.binding.provider_connection_id,
                        latency_ms,
                        error_str.clone(),
                    );

                    dispatch_log.push(DispatchEntry {
                        binding_id: candidate.binding.provider_binding_id.clone(),
                        connection_id: candidate.binding.provider_connection_id.clone(),
                        succeeded: false,
                        latency_ms,
                        error: Some(error_str.clone()),
                    });

                    let reason = match err {
                        ProviderAdapterError::TimedOut => RouteDecisionReason::TimedOut,
                        ProviderAdapterError::RateLimited => RouteDecisionReason::RateLimited,
                        ProviderAdapterError::TransportFailure(_) => {
                            RouteDecisionReason::TransportFailure
                        }
                        _ => RouteDecisionReason::Other,
                    };

                    attempts.push(RouteAttemptRecord {
                        route_attempt_id: attempt_id,
                        route_decision_id: decision_id.clone(),
                        project_id: project.project_id.clone(),
                        operation_kind: operation,
                        provider_binding_id: candidate.binding.provider_binding_id.clone(),
                        selector_context: context.clone(),
                        attempt_index,
                        decision: RouteAttemptDecision::Failed,
                        decision_reason: reason,
                        skip_reason: Some(error_str),
                        estimated_cost_micros: None,
                    });

                    // Continue to next candidate (fallback).
                }
            }
        }

        // All candidates exhausted.
        let terminal = attempts.last().map(|a| a.route_attempt_id.clone());
        RoutingOutcome {
            decision: RouteDecisionRecord {
                route_decision_id: decision_id,
                project_id: project.project_id.clone(),
                operation_kind: operation,
                terminal_route_attempt_id: terminal,
                selected_provider_binding_id: None,
                selected_route_attempt_id: None,
                selector_context: context.clone(),
                attempt_count: attempts.len() as u16,
                fallback_used: true,
                final_status: RouteDecisionStatus::FailedAfterDispatch,
            },
            attempts,
            response: None,
            dispatch_log,
        }
    }

    /// Rank capable candidates: healthy first, sorted by cost, then unhealthy.
    fn rank_candidates<'a>(
        &self,
        candidates: Vec<&'a RoutableProvider>,
    ) -> Vec<&'a RoutableProvider> {
        let (mut healthy, mut unhealthy): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|c| self.health.is_healthy(&c.binding.provider_connection_id));

        let cost_weight = self.config.cost_weight;
        let sort_by_cost = |a: &&RoutableProvider, b: &&RoutableProvider| {
            let cost_a = composite_cost(a, cost_weight);
            let cost_b = composite_cost(b, cost_weight);
            cost_a
                .partial_cmp(&cost_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        };

        healthy.sort_by(sort_by_cost);
        unhealthy.sort_by(sort_by_cost);

        if self.config.allow_unhealthy_fallback {
            healthy.extend(unhealthy);
        }
        healthy
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Partition candidates into (capable, vetoed) based on required_capabilities.
fn partition_by_capability(
    candidates: &[RoutableProvider],
) -> (Vec<&RoutableProvider>, Vec<&RoutableProvider>) {
    let mut capable = Vec::new();
    let mut vetoed = Vec::new();

    for c in candidates {
        if check_required_capabilities(&c.binding, &c.capabilities).is_none() {
            capable.push(c);
        } else {
            vetoed.push(c);
        }
    }

    (capable, vetoed)
}

/// Composite cost score for ranking. Lower = preferred.
///
/// When cost info is missing, returns a high sentinel so known-cost providers
/// are preferred. The `weight` scales how much cost matters vs. natural order.
fn composite_cost(provider: &RoutableProvider, weight: f64) -> f64 {
    if weight <= 0.0 {
        return 0.0; // cost doesn't matter
    }
    match (provider.cost_per_1k_input, provider.cost_per_1k_output) {
        (Some(input), Some(output)) => (input + output) * weight,
        (Some(input), None) => input * weight,
        (None, Some(output)) => output * weight,
        (None, None) => 1_000.0 * weight, // unknown cost = deprioritize
    }
}

fn no_viable_route(
    decision_id: RouteDecisionId,
    project: &ProjectKey,
    operation: OperationKind,
    context: &SelectorContext,
    attempts: &[RouteAttemptRecord],
) -> RouteDecisionRecord {
    RouteDecisionRecord {
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
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::providers::{
        GenerationProvider, GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
        ProviderCapability,
    };

    /// Mock provider that either succeeds or fails.
    struct MockProvider {
        should_fail: bool,
        _fail_error: Option<ProviderAdapterError>,
    }

    impl MockProvider {
        fn ok() -> Arc<dyn GenerationProvider> {
            Arc::new(Self {
                should_fail: false,
                _fail_error: None,
            })
        }

        fn failing(err: ProviderAdapterError) -> Arc<dyn GenerationProvider> {
            Arc::new(Self {
                should_fail: true,
                _fail_error: Some(err),
            })
        }

        fn _timeout() -> Arc<dyn GenerationProvider> {
            Self::failing(ProviderAdapterError::TimedOut)
        }
    }

    // Safety: MockProvider is only used in tests, fail_error is consumed once.
    unsafe impl Send for MockProvider {}
    unsafe impl Sync for MockProvider {}

    #[async_trait::async_trait]
    impl GenerationProvider for MockProvider {
        async fn generate(
            &self,
            model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
            _tools: &[serde_json::Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            if self.should_fail {
                return Err(ProviderAdapterError::TransportFailure(
                    "mock failure".into(),
                ));
            }
            Ok(GenerationResponse {
                text: "hello from mock".into(),
                input_tokens: Some(10),
                output_tokens: Some(5),
                model_id: model_id.to_owned(),
                tool_calls: vec![],
                finish_reason: None,
            })
        }
    }

    fn test_binding(id: &str, conn: &str) -> ProviderBindingRecord {
        ProviderBindingRecord {
            provider_binding_id: ProviderBindingId::new(id),
            project: ProjectKey::new("t", "w", "p"),
            provider_connection_id: ProviderConnectionId::new(conn),
            provider_model_id: ProviderModelId::new("model-1"),
            operation_kind: OperationKind::Generate,
            settings: ProviderBindingSettings::default(),
            active: true,
            created_at: 1000,
        }
    }

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    #[tokio::test]
    async fn routes_to_first_healthy_provider() {
        let health = Arc::new(ProviderHealthTracker::new());
        let mut router = ProviderRouter::new(RoutingConfig::default(), health.clone());

        router.register(ProviderConnectionId::new("c1"), MockProvider::ok());
        router.register(ProviderConnectionId::new("c2"), MockProvider::ok());

        let candidates = vec![
            RoutableProvider::new(test_binding("b1", "c1"), vec![]),
            RoutableProvider::new(test_binding("b2", "c2"), vec![]),
        ];

        let outcome = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                candidates,
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
        assert!(outcome.response.is_some());
        assert!(!outcome.decision.fallback_used);
    }

    #[tokio::test]
    async fn falls_back_on_primary_failure() {
        let health = Arc::new(ProviderHealthTracker::new());
        let mut router = ProviderRouter::new(RoutingConfig::default(), health.clone());

        router.register(
            ProviderConnectionId::new("c1"),
            MockProvider::failing(ProviderAdapterError::TransportFailure("down".into())),
        );
        router.register(ProviderConnectionId::new("c2"), MockProvider::ok());

        let candidates = vec![
            RoutableProvider::new(test_binding("b1", "c1"), vec![]),
            RoutableProvider::new(test_binding("b2", "c2"), vec![]),
        ];

        let outcome = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                candidates,
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
        assert!(outcome.decision.fallback_used);
        assert_eq!(
            outcome.decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("b2"))
        );
        assert!(outcome.response.is_some());
        // Primary should have a Failed attempt recorded.
        assert_eq!(outcome.dispatch_log.len(), 2);
        assert!(!outcome.dispatch_log[0].succeeded);
        assert!(outcome.dispatch_log[1].succeeded);
    }

    #[tokio::test]
    async fn all_providers_fail_returns_failed_after_dispatch() {
        let health = Arc::new(ProviderHealthTracker::new());
        let mut router = ProviderRouter::new(RoutingConfig::default(), health.clone());

        router.register(
            ProviderConnectionId::new("c1"),
            MockProvider::failing(ProviderAdapterError::TimedOut),
        );
        router.register(
            ProviderConnectionId::new("c2"),
            MockProvider::failing(ProviderAdapterError::RateLimited),
        );

        let candidates = vec![
            RoutableProvider::new(test_binding("b1", "c1"), vec![]),
            RoutableProvider::new(test_binding("b2", "c2"), vec![]),
        ];

        let outcome = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                candidates,
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        assert_eq!(
            outcome.decision.final_status,
            RouteDecisionStatus::FailedAfterDispatch
        );
        assert!(outcome.response.is_none());
        assert!(outcome.decision.fallback_used);
    }

    #[tokio::test]
    async fn capability_mismatch_vetoes_binding() {
        let health = Arc::new(ProviderHealthTracker::new());
        let mut router = ProviderRouter::new(RoutingConfig::default(), health.clone());

        router.register(ProviderConnectionId::new("c1"), MockProvider::ok());
        router.register(ProviderConnectionId::new("c2"), MockProvider::ok());

        let mut binding_needs_tool = test_binding("b1", "c1");
        binding_needs_tool.settings.required_capabilities = vec![ProviderCapability::ToolUse];

        let candidates = vec![
            // c1 requires ToolUse but doesn't have it
            RoutableProvider::new(binding_needs_tool, vec![ProviderCapability::Streaming]),
            // c2 has no requirements
            RoutableProvider::new(test_binding("b2", "c2"), vec![]),
        ];

        let outcome = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                candidates,
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
        assert_eq!(
            outcome.decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("b2"))
        );
        // First attempt should be Vetoed
        assert!(outcome
            .attempts
            .iter()
            .any(|a| a.decision == RouteAttemptDecision::Vetoed));
    }

    #[tokio::test]
    async fn cost_aware_prefers_cheaper_provider() {
        let health = Arc::new(ProviderHealthTracker::new());
        let mut router = ProviderRouter::new(
            RoutingConfig {
                cost_weight: 1.0, // pure cost ranking
                allow_unhealthy_fallback: true,
            },
            health.clone(),
        );

        router.register(ProviderConnectionId::new("expensive"), MockProvider::ok());
        router.register(ProviderConnectionId::new("cheap"), MockProvider::ok());

        let candidates = vec![
            // Expensive listed first
            RoutableProvider::new(test_binding("b_exp", "expensive"), vec![]).with_cost(30.0, 60.0),
            // Cheap listed second
            RoutableProvider::new(test_binding("b_cheap", "cheap"), vec![]).with_cost(0.5, 1.5),
        ];

        let outcome = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                candidates,
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
        // Cheap should be selected even though expensive was listed first
        assert_eq!(
            outcome.decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("b_cheap"))
        );
    }

    #[tokio::test]
    async fn unhealthy_provider_deprioritized() {
        let health = Arc::new(ProviderHealthTracker::new());
        // Mark c1 as unhealthy (3 consecutive failures)
        let c1 = ProviderConnectionId::new("c1");
        for _ in 0..3 {
            health.record_failure(&c1, 10, "err".into());
        }

        let mut router = ProviderRouter::new(RoutingConfig::default(), health.clone());
        router.register(ProviderConnectionId::new("c1"), MockProvider::ok());
        router.register(ProviderConnectionId::new("c2"), MockProvider::ok());

        let candidates = vec![
            RoutableProvider::new(test_binding("b1", "c1"), vec![]),
            RoutableProvider::new(test_binding("b2", "c2"), vec![]),
        ];

        let outcome = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                candidates,
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
        // c2 should be preferred because c1 is unhealthy
        assert_eq!(
            outcome.decision.selected_provider_binding_id,
            Some(ProviderBindingId::new("b2"))
        );
    }

    #[tokio::test]
    async fn health_tracker_updated_on_dispatch() {
        let health = Arc::new(ProviderHealthTracker::new());
        let mut router = ProviderRouter::new(RoutingConfig::default(), health.clone());

        router.register(ProviderConnectionId::new("c1"), MockProvider::ok());

        let candidates = vec![RoutableProvider::new(test_binding("b1", "c1"), vec![])];

        let _ = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                candidates,
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        let stats = health.get(&ProviderConnectionId::new("c1")).unwrap();
        assert_eq!(stats.success_count, 1);
        assert_eq!(stats.failure_count, 0);
    }

    #[tokio::test]
    async fn no_candidates_returns_no_viable_route() {
        let health = Arc::new(ProviderHealthTracker::new());
        let router = ProviderRouter::new(RoutingConfig::default(), health);

        let outcome = router
            .route(
                &project(),
                OperationKind::Generate,
                &SelectorContext::default(),
                vec![],
                "model-1",
                vec![],
                &ProviderBindingSettings::default(),
                &[],
            )
            .await;

        assert_eq!(
            outcome.decision.final_status,
            RouteDecisionStatus::NoViableRoute
        );
        assert!(outcome.response.is_none());
    }

    #[test]
    fn composite_cost_with_zero_weight_returns_zero() {
        let p = RoutableProvider::new(test_binding("b", "c"), vec![]).with_cost(10.0, 20.0);
        assert_eq!(composite_cost(&p, 0.0), 0.0);
    }

    #[test]
    fn composite_cost_unknown_gets_sentinel() {
        let p = RoutableProvider::new(test_binding("b", "c"), vec![]);
        let score = composite_cost(&p, 1.0);
        assert!(score >= 1000.0);
    }

    #[test]
    fn capability_partition_works() {
        let mut binding_req = test_binding("b1", "c1");
        binding_req.settings.required_capabilities = vec![ProviderCapability::ToolUse];

        let candidates = vec![
            RoutableProvider::new(binding_req, vec![]), // lacks ToolUse
            RoutableProvider::new(test_binding("b2", "c2"), vec![]), // no requirements
        ];

        let (capable, vetoed) = partition_by_capability(&candidates);
        assert_eq!(capable.len(), 1);
        assert_eq!(vetoed.len(), 1);
        assert_eq!(
            vetoed[0].binding.provider_binding_id,
            ProviderBindingId::new("b1")
        );
    }
}
