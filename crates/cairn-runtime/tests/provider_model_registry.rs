//! RFC 009 provider model capability registry integration tests.

use std::sync::Arc;

use cairn_domain::providers::{OperationKind, ProviderBindingSettings, ProviderModelCapability};
use cairn_domain::*;
use cairn_runtime::services::provider_model_impl::ProviderModelServiceImpl;
use cairn_runtime::services::route_resolver_impl::{BindingQuery, SimpleRouteResolver};
use cairn_runtime::routing::RouteResolverService;
use cairn_store::projections::{ProviderBindingReadModel, ProviderModelReadModel};
use cairn_store::{EventLog, InMemoryStore};

fn tenant_key() -> TenantKey {
    TenantKey::new("tenant_cap")
}
fn tenant() -> TenantId {
    TenantId::new("tenant_cap")
}
fn project() -> ProjectKey {
    ProjectKey::new("tenant_cap", "ws_cap", "proj_cap")
}

use cairn_runtime::services::event_helpers::make_envelope;

/// Register a model with `ProviderModelServiceImpl` and read it back.
#[tokio::test]
async fn provider_model_registry_register_and_get() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ProviderModelServiceImpl::new(store.clone());

    let caps = ProviderModelCapability {
        model_id: "gpt-4o".to_owned(),
        provider_id: "conn_openai".to_owned(),
        operation_kinds: vec![OperationKind::Generate],
        context_window_tokens: Some(128_000),
        max_output_tokens: Some(4096),
        supports_streaming: true,
        cost_per_1k_input_tokens: Some(5_000),
        cost_per_1k_output_tokens: Some(15_000),
    };

    let registered = svc
        .register(
            tenant(),
            ProviderConnectionId::new("conn_openai"),
            "gpt-4o".to_owned(),
            caps.clone(),
        )
        .await
        .unwrap();

    assert_eq!(registered.model_id, "gpt-4o");
    assert!(registered.operation_kinds.contains(&OperationKind::Generate));

    let fetched = ProviderModelReadModel::get_model(store.as_ref(), "gpt-4o")
        .await
        .unwrap()
        .expect("model should be stored");
    assert_eq!(fetched.model_id, "gpt-4o");
    assert_eq!(fetched.context_window_tokens, Some(128_000));
}

/// A model with only Embed capability must NOT be selected for a Generate request.
#[tokio::test]
async fn provider_model_registry_embed_only_not_selected_for_generate() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ProviderModelServiceImpl::new(store.clone());

    // Register gen_model (supports Generate).
    svc.register(
        tenant(),
        ProviderConnectionId::new("conn_gen"),
        "gen_model".to_owned(),
        ProviderModelCapability {
            model_id: "gen_model".to_owned(),
            provider_id: "conn_gen".to_owned(),
            operation_kinds: vec![OperationKind::Generate],
            context_window_tokens: Some(8192),
            max_output_tokens: Some(2048),
            supports_streaming: true,
            cost_per_1k_input_tokens: None,
            cost_per_1k_output_tokens: None,
        },
    )
    .await
    .unwrap();

    // Register embed_model (supports ONLY Embed).
    svc.register(
        tenant(),
        ProviderConnectionId::new("conn_embed"),
        "embed_model".to_owned(),
        ProviderModelCapability {
            model_id: "embed_model".to_owned(),
            provider_id: "conn_embed".to_owned(),
            operation_kinds: vec![OperationKind::Embed],
            context_window_tokens: None,
            max_output_tokens: None,
            supports_streaming: false,
            cost_per_1k_input_tokens: None,
            cost_per_1k_output_tokens: None,
        },
    )
    .await
    .unwrap();

    // Register provider connections.
    store
        .append(&[
            make_envelope(RuntimeEvent::ProviderConnectionRegistered(
                ProviderConnectionRegistered {
                    tenant: tenant_key(),
                    provider_connection_id: ProviderConnectionId::new("conn_gen"),
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses".to_owned(),
                    status: cairn_domain::providers::ProviderConnectionStatus::Active,
                    registered_at: 0,
                },
            )),
            make_envelope(RuntimeEvent::ProviderConnectionRegistered(
                ProviderConnectionRegistered {
                    tenant: tenant_key(),
                    provider_connection_id: ProviderConnectionId::new("conn_embed"),
                    provider_family: "cohere".to_owned(),
                    adapter_type: "embed".to_owned(),
                    status: cairn_domain::providers::ProviderConnectionStatus::Active,
                    registered_at: 0,
                },
            )),
        ])
        .await
        .unwrap();

    // Create two bindings, both claiming Generate operation.
    // binding_gen: backed by gen_model (actually supports Generate).
    // binding_embed_as_gen: backed by embed_model (only supports Embed — misconfigured).
    store
        .append(&[
            make_envelope(RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                project: project(),
                provider_binding_id: ProviderBindingId::new("binding_gen"),
                policy_id: None,
                provider_connection_id: ProviderConnectionId::new("conn_gen"),
                provider_model_id: ProviderModelId::new("gen_model"),
                operation_kind: OperationKind::Generate,
                settings: ProviderBindingSettings::default(),
                active: true,
                created_at: 0,
                estimated_cost_micros: None,
            })),
            make_envelope(RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                project: project(),
                provider_binding_id: ProviderBindingId::new("binding_embed_as_gen"),
                policy_id: None,
                provider_connection_id: ProviderConnectionId::new("conn_embed"),
                provider_model_id: ProviderModelId::new("embed_model"),
                operation_kind: OperationKind::Generate,
                settings: ProviderBindingSettings::default(),
                active: true,
                created_at: 0,
                estimated_cost_micros: None,
            })),
        ])
        .await
        .unwrap();

    // Route resolver WITH capability filtering.
    struct StoreBindings(Arc<InMemoryStore>);
    #[async_trait::async_trait]
    impl BindingQuery for StoreBindings {
        async fn list_active_bindings(
            &self,
            project: &ProjectKey,
            operation: OperationKind,
        ) -> Result<Vec<cairn_domain::providers::ProviderBindingRecord>, cairn_runtime::RuntimeError> {
            Ok(ProviderBindingReadModel::list_active(self.0.as_ref(), project, operation).await?)
        }
    }

    let resolver = SimpleRouteResolver::new(StoreBindings(store.clone()), store.clone())
        .with_model_capabilities(store.clone());

    let decision = resolver
        .resolve(
            &project(),
            OperationKind::Generate,
            &cairn_domain::selectors::SelectorContext::default(),
        )
        .await
        .unwrap();

    let selected = decision.selected_provider_binding_id.expect("should have a selection");
    assert_eq!(selected.as_str(), "binding_gen",
        "only the Generate-capable model must be selected, got: {}", selected.as_str());
    assert_ne!(selected.as_str(), "binding_embed_as_gen");
}

/// list() returns all models registered for a connection.
#[tokio::test]
async fn provider_model_registry_list_by_connection() {
    let store = Arc::new(InMemoryStore::new());
    let svc = ProviderModelServiceImpl::new(store.clone());
    let conn_id = ProviderConnectionId::new("conn_list");

    for (model_id, op) in [("m1", OperationKind::Generate), ("m2", OperationKind::Embed)] {
        svc.register(
            tenant(),
            conn_id.clone(),
            model_id.to_owned(),
            ProviderModelCapability {
                model_id: model_id.to_owned(),
                provider_id: "conn_list".to_owned(),
                operation_kinds: vec![op],
                context_window_tokens: None,
                max_output_tokens: None,
                supports_streaming: false,
                cost_per_1k_input_tokens: None,
                cost_per_1k_output_tokens: None,
            },
        )
        .await
        .unwrap();
    }

    let models = svc.list(&conn_id).await.unwrap();
    assert_eq!(models.len(), 2);
    let ids: Vec<&str> = models.iter().map(|m| m.model_id.as_str()).collect();
    assert!(ids.contains(&"m1"));
    assert!(ids.contains(&"m2"));
}
