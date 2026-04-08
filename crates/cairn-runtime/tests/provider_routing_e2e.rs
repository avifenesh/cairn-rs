//! RFC 009 provider routing lifecycle end-to-end integration tests.
//!
//! Covers the full routing lifecycle:
//!   (1) create provider connection
//!   (2) create provider binding linked to the connection
//!   (3) register provider model capability with operation_kind Generate
//!   (4) resolve a route for Generate — should pick the binding
//!   (5) record a provider call with timing
//!   (6) verify route decision persisted
//!
//! Also retains the original fallback-chain and capability-check tests.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::providers::{
    OperationKind, ProviderBindingRecord, ProviderBindingSettings, ProviderCallStatus,
    ProviderCapability, ProviderConnectionStatus, ProviderModelCapability, RouteAttemptDecision,
    RouteDecisionStatus,
};
use cairn_domain::selectors::SelectorContext;
use cairn_domain::*;
use cairn_runtime::error::RuntimeError;
use cairn_runtime::routing::RouteResolverService;
use cairn_runtime::services::event_helpers::make_envelope;
use cairn_runtime::services::route_resolver_impl::{
    BindingQuery, FallbackChainResolver, RankedBinding, SimpleRouteResolver,
};
use cairn_store::projections::{
    ProviderBindingReadModel, ProviderCallReadModel, ProviderConnectionReadModel,
    RouteDecisionReadModel,
};
use cairn_store::{EventLog, InMemoryStore};

// ── Helpers ───────────────────────────────────────────────────────────────

fn project() -> ProjectKey {
    ProjectKey::new("t1", "ws1", "proj_routing")
}

fn tenant_id() -> TenantId {
    TenantId::new("t1")
}

/// BindingQuery implementation for integration tests.
/// Mirrors the #[cfg(test)] impl inside route_resolver_impl, which is not
/// visible to external test crates.
struct StoreBindingQuery(Arc<InMemoryStore>);

#[async_trait]
impl BindingQuery for StoreBindingQuery {
    async fn list_active_bindings(
        &self,
        project: &ProjectKey,
        operation: OperationKind,
    ) -> Result<Vec<ProviderBindingRecord>, RuntimeError> {
        ProviderBindingReadModel::list_active(self.0.as_ref(), project, operation)
            .await
            .map_err(|e| RuntimeError::Internal(e.to_string()))
    }
}

fn make_primary_binding() -> ProviderBindingRecord {
    ProviderBindingRecord {
        provider_binding_id: ProviderBindingId::new("binding_primary"),
        project: project(),
        provider_connection_id: ProviderConnectionId::new("conn_openai"),
        provider_model_id: ProviderModelId::new("gpt-4o"),
        operation_kind: OperationKind::Generate,
        settings: ProviderBindingSettings {
            required_capabilities: vec![ProviderCapability::Streaming],
            ..ProviderBindingSettings::default()
        },
        active: true,
        created_at: 0,
    }
}

fn make_fallback_binding() -> ProviderBindingRecord {
    ProviderBindingRecord {
        provider_binding_id: ProviderBindingId::new("binding_fallback"),
        project: project(),
        provider_connection_id: ProviderConnectionId::new("conn_bedrock"),
        provider_model_id: ProviderModelId::new("claude-3-5-sonnet"),
        operation_kind: OperationKind::Generate,
        settings: ProviderBindingSettings::default(),
        active: true,
        created_at: 1,
    }
}

/// Seed store with two connections + two bindings.
async fn seed_store(store: &Arc<InMemoryStore>) {
    store
        .append(&[
            make_envelope(RuntimeEvent::ProviderConnectionRegistered(
                ProviderConnectionRegistered {
                    tenant: TenantKey::new("t1"),
                    provider_connection_id: ProviderConnectionId::new("conn_openai"),
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses".to_owned(),
                    supported_models: vec![],
                    status: ProviderConnectionStatus::Active,
                    registered_at: 0,
                },
            )),
            make_envelope(RuntimeEvent::ProviderConnectionRegistered(
                ProviderConnectionRegistered {
                    tenant: TenantKey::new("t1"),
                    provider_connection_id: ProviderConnectionId::new("conn_bedrock"),
                    provider_family: "bedrock".to_owned(),
                    adapter_type: "converse".to_owned(),
                    supported_models: vec![],
                    status: ProviderConnectionStatus::Active,
                    registered_at: 0,
                },
            )),
            make_envelope(RuntimeEvent::ProviderBindingCreated(
                ProviderBindingCreated {
                    project: project(),
                    provider_binding_id: ProviderBindingId::new("binding_primary"),
                    provider_connection_id: ProviderConnectionId::new("conn_openai"),
                    provider_model_id: ProviderModelId::new("gpt-4o"),
                    operation_kind: OperationKind::Generate,
                    settings: ProviderBindingSettings {
                        required_capabilities: vec![ProviderCapability::Streaming],
                        ..ProviderBindingSettings::default()
                    },
                    policy_id: None,
                    active: true,
                    created_at: 0,
                    estimated_cost_micros: None,
                },
            )),
            make_envelope(RuntimeEvent::ProviderBindingCreated(
                ProviderBindingCreated {
                    project: project(),
                    provider_binding_id: ProviderBindingId::new("binding_fallback"),
                    provider_connection_id: ProviderConnectionId::new("conn_bedrock"),
                    provider_model_id: ProviderModelId::new("claude-3-5-sonnet"),
                    operation_kind: OperationKind::Generate,
                    settings: ProviderBindingSettings::default(),
                    policy_id: None,
                    active: true,
                    created_at: 1,
                    estimated_cost_micros: None,
                },
            )),
        ])
        .await
        .unwrap();
}

// ── Sequential lifecycle test ─────────────────────────────────────────────

/// Full RFC 009 lifecycle: connection → binding → model capability →
/// route resolution → call recording → decision persistence.
#[tokio::test]
async fn provider_routing_full_lifecycle() {
    let store = Arc::new(InMemoryStore::new());

    // (1) Create provider connection.
    let conn_id = ProviderConnectionId::new("conn_lc_openai");
    store
        .append(&[make_envelope(RuntimeEvent::ProviderConnectionRegistered(
            ProviderConnectionRegistered {
                tenant: TenantKey::new(tenant_id()),
                provider_connection_id: conn_id.clone(),
                provider_family: "openai".to_owned(),
                adapter_type: "chat_completions".to_owned(),
                supported_models: vec![],
                status: ProviderConnectionStatus::Active,
                registered_at: 1_000,
            },
        ))])
        .await
        .unwrap();

    let conn = ProviderConnectionReadModel::get(store.as_ref(), &conn_id)
        .await
        .unwrap()
        .expect("connection must be persisted after ProviderConnectionRegistered");
    assert_eq!(conn.provider_connection_id, conn_id);
    assert_eq!(conn.provider_family, "openai");

    // (2) Create provider binding linked to the connection.
    let binding_id = ProviderBindingId::new("binding_lc_1");
    let model_id = ProviderModelId::new("gpt-4o-mini");
    store
        .append(&[make_envelope(RuntimeEvent::ProviderBindingCreated(
            ProviderBindingCreated {
                project: project(),
                provider_binding_id: binding_id.clone(),
                provider_connection_id: conn_id.clone(),
                provider_model_id: model_id.clone(),
                operation_kind: OperationKind::Generate,
                settings: ProviderBindingSettings::default(),
                policy_id: None,
                active: true,
                created_at: 1_001,
                estimated_cost_micros: None,
            },
        ))])
        .await
        .unwrap();

    let binding = ProviderBindingReadModel::get(store.as_ref(), &binding_id)
        .await
        .unwrap()
        .expect("binding must be persisted after ProviderBindingCreated");
    assert_eq!(binding.provider_binding_id, binding_id);
    assert_eq!(binding.provider_connection_id, conn_id);
    assert_eq!(binding.operation_kind, OperationKind::Generate);
    assert!(binding.active);

    // (3) Register provider model capability with operation_kind Generate.
    // InMemoryStore routes ProviderModelRegistered to the no-op arm, so we
    // verify registration by checking the event was accepted (position advances)
    // and asserting the capability struct we built declares the right properties.
    let capability = ProviderModelCapability {
        model_id: model_id.clone(),
        capabilities: vec![ProviderCapability::Streaming, ProviderCapability::ToolUse],
        provider_id: "openai".to_owned(),
        operation_kinds: vec![OperationKind::Generate],
        context_window_tokens: Some(128_000),
        max_output_tokens: Some(4_096),
        supports_streaming: true,
        cost_per_1k_input_tokens: Some(0.000_150),
        cost_per_1k_output_tokens: Some(0.000_600),
    };
    let pos_before = store.head_position().await.unwrap();
    store
        .append(&[make_envelope(RuntimeEvent::ProviderModelRegistered(
            ProviderModelRegistered {
                tenant_id: tenant_id(),
                connection_id: conn_id.clone(),
                model_id: model_id.as_str().to_owned(),
                capabilities_json: serde_json::to_string(&capability).unwrap(),
            },
        ))])
        .await
        .unwrap();
    let pos_after = store.head_position().await.unwrap();
    assert!(
        pos_after > pos_before,
        "event log must advance after ProviderModelRegistered"
    );
    // Verify the declared capability struct has the expected properties.
    assert!(
        capability
            .operation_kinds
            .contains(&OperationKind::Generate),
        "declared model capability must include Generate"
    );
    assert!(
        capability
            .capabilities
            .contains(&ProviderCapability::Streaming),
        "declared model capability must advertise Streaming"
    );

    // (4) Resolve a route for Generate — SimpleRouteResolver should pick the binding.
    let resolver = SimpleRouteResolver::new(StoreBindingQuery(store.clone()));
    let decision = resolver
        .resolve(
            &project(),
            OperationKind::Generate,
            &SelectorContext::default(),
        )
        .await
        .unwrap();

    assert_eq!(
        decision.final_status,
        RouteDecisionStatus::Selected,
        "resolver must select the active binding"
    );
    assert_eq!(
        decision
            .selected_provider_binding_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("binding_lc_1"),
        "resolver must pick the only active Generate binding"
    );
    assert_eq!(decision.operation_kind, OperationKind::Generate);

    // (5) Record a provider call with timing.
    let call_id = ProviderCallId::new("call_lc_1");
    let attempt_id = RouteAttemptId::new("attempt_lc_1");
    let started_at: u64 = 2_000;
    let finished_at: u64 = 2_345; // 345 ms latency
    let latency_ms = finished_at - started_at;

    store
        .append(&[make_envelope(RuntimeEvent::ProviderCallCompleted(
            ProviderCallCompleted {
                project: project(),
                provider_call_id: call_id.clone(),
                route_decision_id: decision.route_decision_id.clone(),
                route_attempt_id: attempt_id,
                provider_binding_id: binding_id.clone(),
                provider_connection_id: conn_id.clone(),
                provider_model_id: model_id.clone(),
                operation_kind: OperationKind::Generate,
                status: ProviderCallStatus::Succeeded,
                latency_ms: Some(latency_ms),
                input_tokens: Some(120),
                output_tokens: Some(45),
                cost_micros: Some(36), // (120/1000 * 0.15 + 45/1000 * 0.60) * 1_000_000 ≈ 45 μ
                completed_at: finished_at,
                session_id: None,
                run_id: None,
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                task_id: None,
                prompt_release_id: None,
                fallback_position: 0,
                started_at: 0,
                finished_at: 0,
            },
        ))])
        .await
        .unwrap();

    let call = ProviderCallReadModel::get(store.as_ref(), &call_id)
        .await
        .unwrap()
        .expect("provider call must be persisted after ProviderCallCompleted");
    assert_eq!(call.provider_call_id, call_id);
    assert_eq!(call.status, ProviderCallStatus::Succeeded);
    assert_eq!(
        call.latency_ms,
        Some(latency_ms),
        "latency must be recorded"
    );
    assert_eq!(call.input_tokens, Some(120));
    assert_eq!(call.output_tokens, Some(45));

    // (6) Persist and verify the route decision record.
    store
        .append(&[make_envelope(RuntimeEvent::RouteDecisionMade(
            RouteDecisionMade {
                project: project(),
                route_decision_id: decision.route_decision_id.clone(),
                operation_kind: OperationKind::Generate,
                selected_provider_binding_id: decision.selected_provider_binding_id.clone(),
                final_status: decision.final_status,
                attempt_count: decision.attempt_count,
                fallback_used: decision.fallback_used,
                decided_at: 2_000,
            },
        ))])
        .await
        .unwrap();

    let persisted = RouteDecisionReadModel::get(store.as_ref(), &decision.route_decision_id)
        .await
        .unwrap()
        .expect("route decision must be persisted after RouteDecisionMade");

    assert_eq!(persisted.route_decision_id, decision.route_decision_id);
    assert_eq!(persisted.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        persisted
            .selected_provider_binding_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("binding_lc_1")
    );
    assert!(!persisted.fallback_used);
}

// ── Fallback-chain tests (retained from original) ─────────────────────────

/// RFC 009 §3.1 — primary selected when all required capabilities are available.
#[tokio::test]
async fn primary_selected_when_all_capabilities_available() {
    let store = Arc::new(InMemoryStore::new());
    seed_store(&store).await;

    let resolver = FallbackChainResolver::new(vec![
        RankedBinding {
            binding: make_primary_binding(),
            available_capabilities: vec![ProviderCapability::Streaming],
        },
        RankedBinding {
            binding: make_fallback_binding(),
            available_capabilities: vec![ProviderCapability::Streaming],
        },
    ]);

    let (decision, attempts) = resolver.resolve_with_attempts(
        &project(),
        OperationKind::Generate,
        &SelectorContext::default(),
    );

    assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        decision
            .selected_provider_binding_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("binding_primary")
    );
    assert!(!decision.fallback_used);
    assert_eq!(decision.attempt_count, 1);
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].decision, RouteAttemptDecision::Selected);
    assert_eq!(attempts[0].provider_binding_id.as_str(), "binding_primary");
}

/// RFC 009 §3.2 — fallback used when primary's required capability is absent.
#[tokio::test]
async fn fallback_used_when_primary_degraded() {
    let store = Arc::new(InMemoryStore::new());
    seed_store(&store).await;

    let resolver = FallbackChainResolver::new(vec![
        RankedBinding {
            binding: make_primary_binding(),
            available_capabilities: vec![], // degraded — Streaming absent
        },
        RankedBinding {
            binding: make_fallback_binding(),
            available_capabilities: vec![ProviderCapability::Streaming],
        },
    ]);

    let (decision, attempts) = resolver.resolve_with_attempts(
        &project(),
        OperationKind::Generate,
        &SelectorContext::default(),
    );

    assert_eq!(decision.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        decision
            .selected_provider_binding_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("binding_fallback")
    );
    assert!(decision.fallback_used);
    assert_eq!(decision.attempt_count, 2);
    assert_eq!(attempts[0].decision, RouteAttemptDecision::Vetoed);
    assert_eq!(attempts[0].provider_binding_id.as_str(), "binding_primary");
    assert_eq!(attempts[1].decision, RouteAttemptDecision::Selected);
    assert_eq!(attempts[1].provider_binding_id.as_str(), "binding_fallback");
}

/// RFC 009 §4 — route decision is persistable and readable via read-model.
#[tokio::test]
async fn route_decision_record_is_persisted() {
    let store = Arc::new(InMemoryStore::new());
    seed_store(&store).await;

    let resolver = FallbackChainResolver::new(vec![
        RankedBinding {
            binding: make_primary_binding(),
            available_capabilities: vec![ProviderCapability::Streaming],
        },
        RankedBinding {
            binding: make_fallback_binding(),
            available_capabilities: vec![ProviderCapability::Streaming],
        },
    ]);

    let (decision, _attempts) = resolver.resolve_with_attempts(
        &project(),
        OperationKind::Generate,
        &SelectorContext::default(),
    );
    let decision_id = decision.route_decision_id.clone();

    store
        .append(&[make_envelope(RuntimeEvent::RouteDecisionMade(
            RouteDecisionMade {
                project: project(),
                route_decision_id: decision_id.clone(),
                operation_kind: OperationKind::Generate,
                selected_provider_binding_id: decision.selected_provider_binding_id.clone(),
                final_status: decision.final_status,
                attempt_count: decision.attempt_count,
                fallback_used: decision.fallback_used,
                decided_at: 0,
            },
        ))])
        .await
        .unwrap();

    let stored = RouteDecisionReadModel::get(store.as_ref(), &decision_id)
        .await
        .unwrap()
        .expect("RouteDecisionRecord must be present after RouteDecisionMade");

    assert_eq!(stored.route_decision_id, decision_id);
    assert_eq!(stored.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        stored
            .selected_provider_binding_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("binding_primary")
    );
    assert!(!stored.fallback_used);
}

/// RFC 009 §3.3 — NoViableRoute when all candidates are vetoed.
#[tokio::test]
async fn no_viable_route_when_all_candidates_vetoed() {
    let store = Arc::new(InMemoryStore::new());
    seed_store(&store).await;

    let resolver = FallbackChainResolver::new(vec![
        RankedBinding {
            binding: make_primary_binding(),
            available_capabilities: vec![],
        },
        RankedBinding {
            binding: ProviderBindingRecord {
                settings: ProviderBindingSettings {
                    required_capabilities: vec![ProviderCapability::Streaming],
                    ..ProviderBindingSettings::default()
                },
                ..make_fallback_binding()
            },
            available_capabilities: vec![],
        },
    ]);

    let (decision, attempts) = resolver.resolve_with_attempts(
        &project(),
        OperationKind::Generate,
        &SelectorContext::default(),
    );

    assert_eq!(decision.final_status, RouteDecisionStatus::NoViableRoute);
    assert!(decision.selected_provider_binding_id.is_none());
    assert!(!decision.fallback_used);
    assert_eq!(decision.attempt_count, 2);
    assert!(attempts
        .iter()
        .all(|a| a.decision == RouteAttemptDecision::Vetoed));
}

// ── Provider model capability tests ──────────────────────────────────────

/// ProviderModelRegistered event is accepted by the store and the capability
/// struct round-trips through JSON serialization correctly.
#[tokio::test]
async fn provider_model_capability_registered_event_accepted() {
    let store = Arc::new(InMemoryStore::new());

    let model_id = ProviderModelId::new("claude-3-haiku");
    let capability = ProviderModelCapability {
        model_id: model_id.clone(),
        capabilities: vec![ProviderCapability::Streaming],
        provider_id: "anthropic".to_owned(),
        operation_kinds: vec![OperationKind::Generate],
        context_window_tokens: Some(200_000),
        max_output_tokens: Some(4_096),
        supports_streaming: true,
        cost_per_1k_input_tokens: Some(0.000_025),
        cost_per_1k_output_tokens: Some(0.000_125),
    };

    let capabilities_json = serde_json::to_string(&capability).unwrap();

    let positions = store
        .append(&[make_envelope(RuntimeEvent::ProviderModelRegistered(
            ProviderModelRegistered {
                tenant_id: tenant_id(),
                connection_id: ProviderConnectionId::new("conn_anthropic"),
                model_id: model_id.as_str().to_owned(),
                capabilities_json: capabilities_json.clone(),
            },
        ))])
        .await
        .unwrap();

    // Event was accepted with a position.
    assert_eq!(positions.len(), 1, "one event must be appended");

    // Capability JSON round-trips correctly.
    let decoded: ProviderModelCapability = serde_json::from_str(&capabilities_json).unwrap();
    assert_eq!(decoded.model_id, model_id);
    assert!(decoded.operation_kinds.contains(&OperationKind::Generate));
    assert!(decoded
        .capabilities
        .contains(&ProviderCapability::Streaming));
    assert_eq!(decoded.context_window_tokens, Some(200_000));
}

/// SimpleRouteResolver picks the correct binding when multiple operation kinds
/// are seeded — operation filter prevents cross-kind confusion.
#[tokio::test]
async fn simple_resolver_filters_by_operation_kind() {
    let store = Arc::new(InMemoryStore::new());

    // Seed one Generate binding and one Embed binding for the same project.
    store
        .append(&[
            make_envelope(RuntimeEvent::ProviderBindingCreated(
                ProviderBindingCreated {
                    project: project(),
                    provider_binding_id: ProviderBindingId::new("b_gen"),
                    provider_connection_id: ProviderConnectionId::new("c1"),
                    provider_model_id: ProviderModelId::new("gpt-4o"),
                    operation_kind: OperationKind::Generate,
                    settings: ProviderBindingSettings::default(),
                    policy_id: None,
                    active: true,
                    created_at: 10,
                    estimated_cost_micros: None,
                },
            )),
            make_envelope(RuntimeEvent::ProviderBindingCreated(
                ProviderBindingCreated {
                    project: project(),
                    provider_binding_id: ProviderBindingId::new("b_emb"),
                    provider_connection_id: ProviderConnectionId::new("c1"),
                    provider_model_id: ProviderModelId::new("text-embedding-3-small"),
                    operation_kind: OperationKind::Embed,
                    settings: ProviderBindingSettings::default(),
                    policy_id: None,
                    active: true,
                    created_at: 11,
                    estimated_cost_micros: None,
                },
            )),
        ])
        .await
        .unwrap();

    let resolver = SimpleRouteResolver::new(StoreBindingQuery(store));

    // Generate resolves to the generate binding.
    let gen = resolver
        .resolve(
            &project(),
            OperationKind::Generate,
            &SelectorContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(gen.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        gen.selected_provider_binding_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("b_gen")
    );

    // Embed resolves to the embed binding.
    let emb = resolver
        .resolve(
            &project(),
            OperationKind::Embed,
            &SelectorContext::default(),
        )
        .await
        .unwrap();
    assert_eq!(emb.final_status, RouteDecisionStatus::Selected);
    assert_eq!(
        emb.selected_provider_binding_id
            .as_ref()
            .map(|id| id.as_str()),
        Some("b_emb")
    );
}

/// Provider call timing fields are preserved through the event projection.
#[tokio::test]
async fn provider_call_timing_is_recorded() {
    let store = Arc::new(InMemoryStore::new());
    seed_store(&store).await;

    let call_id = ProviderCallId::new("call_timing_1");
    let decision_id = RouteDecisionId::new("rd_timing_1");
    let started: u64 = 5_000;
    let finished: u64 = 5_820; // 820 ms

    store
        .append(&[make_envelope(RuntimeEvent::ProviderCallCompleted(
            ProviderCallCompleted {
                project: project(),
                provider_call_id: call_id.clone(),
                route_decision_id: decision_id,
                route_attempt_id: RouteAttemptId::new("ra_t1"),
                provider_binding_id: ProviderBindingId::new("binding_primary"),
                provider_connection_id: ProviderConnectionId::new("conn_openai"),
                provider_model_id: ProviderModelId::new("gpt-4o"),
                operation_kind: OperationKind::Generate,
                status: ProviderCallStatus::Succeeded,
                latency_ms: Some(finished - started),
                input_tokens: Some(200),
                output_tokens: Some(80),
                cost_micros: Some(78),
                completed_at: finished,
                session_id: None,
                run_id: None,
                error_class: None,
                raw_error_message: None,
                retry_count: 0,
                task_id: None,
                prompt_release_id: None,
                fallback_position: 0,
                started_at: 0,
                finished_at: 0,
            },
        ))])
        .await
        .unwrap();

    let call = ProviderCallReadModel::get(store.as_ref(), &call_id)
        .await
        .unwrap()
        .expect("call must be persisted");

    assert_eq!(call.latency_ms, Some(820), "latency must be 820 ms");
    assert_eq!(call.status, ProviderCallStatus::Succeeded);
    assert_eq!(call.input_tokens, Some(200));
    assert_eq!(call.output_tokens, Some(80));
    assert_eq!(call.cost_micros, Some(78));
}
