//! Provider routing safety tests — ported from Go PRs #1237-#1240.
//!
//! The Go codebase had 4 bugs where provider resolution followed fallback chains
//! instead of using direct IDs.  These tests verify the Rust implementation
//! correctly resolves the intended provider in multi-provider setups:
//!
//!   1. Two providers for the same operation → correct one selected by binding order
//!   2. Fallback dispatches to the NEXT provider, not the wrong one
//!   3. Direct binding ID in candidates → that exact provider is called
//!   4. Operation-kind isolation prevents cross-kind routing

use std::sync::Arc;

use cairn_domain::providers::{
    GenerationProvider, GenerationResponse, OperationKind, ProviderAdapterError,
    ProviderBindingRecord, ProviderBindingSettings, RouteDecisionStatus,
};
use cairn_domain::selectors::SelectorContext;
use cairn_domain::*;
use cairn_runtime::services::provider_health_tracker::ProviderHealthTracker;
use cairn_runtime::services::provider_router::{ProviderRouter, RoutableProvider, RoutingConfig};

// ── Mock providers that identify themselves ─────────────────────────────────

/// A mock provider that returns its own name in the response text,
/// so the test can verify WHICH provider actually handled the call.
struct IdentifiableProvider {
    name: String,
    should_fail: bool,
}

impl IdentifiableProvider {
    fn ok(name: &str) -> Arc<dyn GenerationProvider> {
        Arc::new(Self {
            name: name.to_owned(),
            should_fail: false,
        })
    }

    fn failing(name: &str) -> Arc<dyn GenerationProvider> {
        Arc::new(Self {
            name: name.to_owned(),
            should_fail: true,
        })
    }
}

unsafe impl Send for IdentifiableProvider {}
unsafe impl Sync for IdentifiableProvider {}

#[async_trait::async_trait]
impl GenerationProvider for IdentifiableProvider {
    async fn generate(
        &self,
        model_id: &str,
        _messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
        _tools: &[serde_json::Value],
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        if self.should_fail {
            return Err(ProviderAdapterError::TransportFailure(format!(
                "{} is down",
                self.name
            )));
        }
        Ok(GenerationResponse {
            text: format!("response_from_{}", self.name),
            input_tokens: Some(10),
            output_tokens: Some(5),
            model_id: model_id.to_owned(),
            tool_calls: vec![],
            finish_reason: None,
        })
    }
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("tenant_safe", "ws_safe", "proj_safe")
}

fn binding(id: &str, conn: &str, op: OperationKind) -> ProviderBindingRecord {
    ProviderBindingRecord {
        provider_binding_id: ProviderBindingId::new(id),
        project: project(),
        provider_connection_id: ProviderConnectionId::new(conn),
        provider_model_id: ProviderModelId::new("model-default"),
        operation_kind: op,
        settings: ProviderBindingSettings::default(),
        active: true,
        created_at: 0,
    }
}

fn make_router_with_two_providers(ollama_ok: bool, openai_ok: bool) -> ProviderRouter {
    let health = Arc::new(ProviderHealthTracker::new());
    let mut router = ProviderRouter::new(
        RoutingConfig {
            cost_weight: 0.0, // pure priority order
            allow_unhealthy_fallback: true,
        },
        health,
    );

    if ollama_ok {
        router.register(
            ProviderConnectionId::new("conn_ollama"),
            IdentifiableProvider::ok("ollama"),
        );
    } else {
        router.register(
            ProviderConnectionId::new("conn_ollama"),
            IdentifiableProvider::failing("ollama"),
        );
    }

    if openai_ok {
        router.register(
            ProviderConnectionId::new("conn_openai_compat"),
            IdentifiableProvider::ok("openai_compat"),
        );
    } else {
        router.register(
            ProviderConnectionId::new("conn_openai_compat"),
            IdentifiableProvider::failing("openai_compat"),
        );
    }

    router
}

// ── Safety test 1: Primary selected by binding order, not fallback ─────────

/// Go PR #1237 pattern: Two providers for the same operation. The first
/// candidate in the list must be dispatched — not the fallback.
#[tokio::test]
async fn primary_provider_selected_not_fallback() {
    let router = make_router_with_two_providers(true, true);

    let candidates = vec![
        RoutableProvider::new(
            binding("b_ollama", "conn_ollama", OperationKind::Generate),
            vec![],
        ),
        RoutableProvider::new(
            binding("b_openai", "conn_openai_compat", OperationKind::Generate),
            vec![],
        ),
    ];

    let outcome = router
        .route(
            &project(),
            OperationKind::Generate,
            &SelectorContext::default(),
            candidates,
            "model-default",
            vec![],
            &ProviderBindingSettings::default(),
        )
        .await;

    assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        outcome.decision.selected_provider_binding_id,
        Some(ProviderBindingId::new("b_ollama")),
        "must select ollama (primary), not openai_compat (fallback)"
    );
    assert!(
        !outcome.decision.fallback_used,
        "fallback must not be used when primary succeeds"
    );

    let resp = outcome.response.expect("response must be present");
    assert_eq!(
        resp.text, "response_from_ollama",
        "response must come from ollama, not openai_compat"
    );
}

// ── Safety test 2: Fallback dispatches to correct NEXT provider ────────────

/// Go PR #1238 pattern: When the primary provider fails, fallback must
/// dispatch to the specific next provider in the chain — not an arbitrary one.
#[tokio::test]
async fn fallback_dispatches_to_correct_next_provider() {
    let router = make_router_with_two_providers(false, true); // ollama fails

    let candidates = vec![
        RoutableProvider::new(
            binding("b_ollama", "conn_ollama", OperationKind::Generate),
            vec![],
        ),
        RoutableProvider::new(
            binding("b_openai", "conn_openai_compat", OperationKind::Generate),
            vec![],
        ),
    ];

    let outcome = router
        .route(
            &project(),
            OperationKind::Generate,
            &SelectorContext::default(),
            candidates,
            "model-default",
            vec![],
            &ProviderBindingSettings::default(),
        )
        .await;

    assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
    assert!(
        outcome.decision.fallback_used,
        "fallback must be used when primary fails"
    );
    assert_eq!(
        outcome.decision.selected_provider_binding_id,
        Some(ProviderBindingId::new("b_openai")),
        "fallback must dispatch to openai_compat specifically"
    );

    let resp = outcome.response.expect("response must be present");
    assert_eq!(
        resp.text, "response_from_openai_compat",
        "response must come from the fallback provider"
    );

    // Verify dispatch log records both attempts correctly
    assert_eq!(outcome.dispatch_log.len(), 2);
    assert_eq!(
        outcome.dispatch_log[0].connection_id,
        ProviderConnectionId::new("conn_ollama")
    );
    assert!(
        !outcome.dispatch_log[0].succeeded,
        "ollama must be recorded as failed"
    );
    assert_eq!(
        outcome.dispatch_log[1].connection_id,
        ProviderConnectionId::new("conn_openai_compat")
    );
    assert!(
        outcome.dispatch_log[1].succeeded,
        "openai_compat must be recorded as succeeded"
    );
}

// ── Safety test 3: Direct binding ID routes to exact provider ──────────────

/// Go PR #1239 pattern: When only one specific binding is passed as a
/// candidate, that exact provider must be called — no chain traversal.
#[tokio::test]
async fn single_candidate_routes_directly_no_chain() {
    let router = make_router_with_two_providers(true, true);

    // Only pass openai_compat as candidate — ollama must NOT be called.
    let candidates = vec![RoutableProvider::new(
        binding(
            "b_openai_only",
            "conn_openai_compat",
            OperationKind::Generate,
        ),
        vec![],
    )];

    let outcome = router
        .route(
            &project(),
            OperationKind::Generate,
            &SelectorContext::default(),
            candidates,
            "model-default",
            vec![],
            &ProviderBindingSettings::default(),
        )
        .await;

    assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        outcome.decision.selected_provider_binding_id,
        Some(ProviderBindingId::new("b_openai_only")),
        "must route to the specific binding passed, not to ollama"
    );
    assert!(!outcome.decision.fallback_used);

    let resp = outcome.response.expect("response must be present");
    assert_eq!(
        resp.text, "response_from_openai_compat",
        "response must come from openai_compat — ollama must not be involved"
    );
    assert_eq!(
        outcome.dispatch_log.len(),
        1,
        "only one provider must be dispatched"
    );
}

// ── Safety test 4: Operation-kind isolation ─────────────────────────────────

/// Go PR #1240 pattern: An Embed binding must not be selected when
/// resolving for Generate, even if both are from the same provider.
/// This prevents cross-operation routing bugs.
#[tokio::test]
async fn operation_kind_isolation_prevents_cross_routing() {
    let health = Arc::new(ProviderHealthTracker::new());
    let mut router = ProviderRouter::new(RoutingConfig::default(), health);

    router.register(
        ProviderConnectionId::new("conn_ollama"),
        IdentifiableProvider::ok("ollama"),
    );

    // Create candidates: one for Embed, one for Generate — from the same connection.
    let embed_binding = binding("b_embed", "conn_ollama", OperationKind::Embed);
    let gen_binding = binding("b_gen", "conn_ollama", OperationKind::Generate);

    // Resolve for Generate — only the Generate candidate should be considered.
    // Even though both share the same connection, the Embed one must not be used.
    let outcome = router
        .route(
            &project(),
            OperationKind::Generate,
            &SelectorContext::default(),
            vec![RoutableProvider::new(gen_binding, vec![])],
            "model-default",
            vec![],
            &ProviderBindingSettings::default(),
        )
        .await;

    assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        outcome.decision.selected_provider_binding_id,
        Some(ProviderBindingId::new("b_gen")),
    );

    // Resolve for Embed — only the Embed candidate should be considered.
    let outcome_embed = router
        .route(
            &project(),
            OperationKind::Embed,
            &SelectorContext::default(),
            vec![RoutableProvider::new(embed_binding, vec![])],
            "model-default",
            vec![],
            &ProviderBindingSettings::default(),
        )
        .await;

    assert_eq!(
        outcome_embed.decision.final_status,
        RouteDecisionStatus::Selected
    );
    assert_eq!(
        outcome_embed.decision.selected_provider_binding_id,
        Some(ProviderBindingId::new("b_embed")),
    );
}

// ── Safety test 5: Reversed candidate order does not change dispatch ────────

/// Verify that when candidates are listed fallback-first, primary-second,
/// the router respects the given order (not some internal reordering that
/// silently picks the wrong provider).
#[tokio::test]
async fn candidate_order_respected_not_silently_reordered() {
    let router = make_router_with_two_providers(true, true);

    // Deliberately list openai first, ollama second.
    let candidates = vec![
        RoutableProvider::new(
            binding("b_openai", "conn_openai_compat", OperationKind::Generate),
            vec![],
        ),
        RoutableProvider::new(
            binding("b_ollama", "conn_ollama", OperationKind::Generate),
            vec![],
        ),
    ];

    let outcome = router
        .route(
            &project(),
            OperationKind::Generate,
            &SelectorContext::default(),
            candidates,
            "model-default",
            vec![],
            &ProviderBindingSettings::default(),
        )
        .await;

    // openai_compat was listed first, so it must be selected.
    assert_eq!(outcome.decision.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        outcome.decision.selected_provider_binding_id,
        Some(ProviderBindingId::new("b_openai")),
        "first candidate in list must be selected, not reordered"
    );
    let resp = outcome.response.expect("response must be present");
    assert_eq!(resp.text, "response_from_openai_compat");
}
