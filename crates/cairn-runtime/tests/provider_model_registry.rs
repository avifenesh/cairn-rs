//! RFC 009 provider model capability registry integration tests.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use async_trait::async_trait;
use cairn_domain::providers::{OperationKind, ProviderBindingSettings, ProviderModelCapability};
use cairn_domain::*;
use cairn_runtime::routing::RouteResolverService;
use cairn_runtime::services::provider_model_impl::ProviderModelServiceImpl;
use cairn_runtime::services::route_resolver_impl::{BindingQuery, SimpleRouteResolver};
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

// ---------------------------------------------------------------------------
// Test-local wrapper: combines InMemoryStore (EventLog) with an in-memory
// ProviderModelReadModel so that ProviderModelServiceImpl's trait bounds are
// satisfied without requiring a real impl on InMemoryStore.
// ---------------------------------------------------------------------------

struct TestStore {
    inner: InMemoryStore,
    models: RwLock<HashMap<String, ProviderModelCapability>>,
    /// Maps connection_id -> list of model_ids registered under it.
    conn_models: RwLock<HashMap<String, Vec<String>>>,
}

impl TestStore {
    fn new() -> Self {
        Self {
            inner: InMemoryStore::new(),
            models: RwLock::new(HashMap::new()),
            conn_models: RwLock::new(HashMap::new()),
        }
    }
}

#[async_trait]
impl EventLog for TestStore {
    async fn append(
        &self,
        events: &[cairn_domain::EventEnvelope<cairn_domain::RuntimeEvent>],
    ) -> Result<Vec<cairn_store::EventPosition>, cairn_store::StoreError> {
        // When we see a ProviderModelRegistered event, store the model in our map.
        for event in events {
            if let cairn_domain::RuntimeEvent::ProviderModelRegistered(ref e) = event.payload {
                if let Ok(cap) =
                    serde_json::from_str::<ProviderModelCapability>(&e.capabilities_json)
                {
                    let mut models = self.models.write().await;
                    models.insert(e.model_id.clone(), cap);
                    let mut conn = self.conn_models.write().await;
                    conn.entry(e.connection_id.as_str().to_owned())
                        .or_default()
                        .push(e.model_id.clone());
                }
            }
        }
        self.inner.append(events).await
    }

    async fn read_by_entity(
        &self,
        entity: &cairn_store::EntityRef,
        after: Option<cairn_store::EventPosition>,
        limit: usize,
    ) -> Result<Vec<cairn_store::StoredEvent>, cairn_store::StoreError> {
        self.inner.read_by_entity(entity, after, limit).await
    }

    async fn read_stream(
        &self,
        after: Option<cairn_store::EventPosition>,
        limit: usize,
    ) -> Result<Vec<cairn_store::StoredEvent>, cairn_store::StoreError> {
        self.inner.read_stream(after, limit).await
    }

    async fn head_position(
        &self,
    ) -> Result<Option<cairn_store::EventPosition>, cairn_store::StoreError> {
        self.inner.head_position().await
    }

    async fn find_by_causation_id(
        &self,
        causation_id: &str,
    ) -> Result<Option<cairn_store::EventPosition>, cairn_store::StoreError> {
        self.inner.find_by_causation_id(causation_id).await
    }
}

#[async_trait]
impl ProviderModelReadModel for TestStore {
    async fn get_model(
        &self,
        model_id: &str,
    ) -> Result<Option<ProviderModelCapability>, cairn_store::StoreError> {
        let models = self.models.read().await;
        Ok(models.get(model_id).cloned())
    }

    async fn list_by_connection(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Vec<ProviderModelCapability>, cairn_store::StoreError> {
        let conn = self.conn_models.read().await;
        let models = self.models.read().await;
        let ids = conn
            .get(connection_id.as_str())
            .cloned()
            .unwrap_or_default();
        Ok(ids
            .iter()
            .filter_map(|id| models.get(id).cloned())
            .collect())
    }
}

/// Register a model with `ProviderModelServiceImpl` and read it back.
#[tokio::test]
async fn provider_model_registry_register_and_get() {
    let store = Arc::new(TestStore::new());
    let svc = ProviderModelServiceImpl::new(store.clone());

    let caps = ProviderModelCapability {
        model_id: ProviderModelId::new("gpt-4o"),
        capabilities: vec![],
        provider_id: "conn_openai".to_owned(),
        operation_kinds: vec![OperationKind::Generate],
        context_window_tokens: Some(128_000),
        max_output_tokens: Some(4096),
        supports_streaming: true,
        cost_per_1k_input_tokens: Some(5_000.0),
        cost_per_1k_output_tokens: Some(15_000.0),
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

    assert_eq!(registered.model_id.as_str(), "gpt-4o");
    assert!(registered
        .operation_kinds
        .contains(&OperationKind::Generate));

    let fetched = ProviderModelReadModel::get_model(store.as_ref(), "gpt-4o")
        .await
        .unwrap()
        .expect("model should be stored");
    assert_eq!(fetched.model_id.as_str(), "gpt-4o");
    assert_eq!(fetched.context_window_tokens, Some(128_000));
}

/// A model with only Embed capability must NOT be selected for a Generate request.
#[tokio::test]
async fn provider_model_registry_embed_only_not_selected_for_generate() {
    let store = Arc::new(InMemoryStore::new());

    // Register provider connections.
    store
        .append(&[
            make_envelope(RuntimeEvent::ProviderConnectionRegistered(
                ProviderConnectionRegistered {
                    tenant: tenant_key(),
                    provider_connection_id: ProviderConnectionId::new("conn_gen"),
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses".to_owned(),
                    supported_models: vec![],
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
                    supported_models: vec![],
                    status: cairn_domain::providers::ProviderConnectionStatus::Active,
                    registered_at: 0,
                },
            )),
        ])
        .await
        .unwrap();

    // Create two bindings:
    // binding_gen: backed by gen_model (supports Generate via operation_kind on the binding).
    // binding_embed_as_gen: backed by embed_model (only supports Embed, but misconfigured as Generate).
    store
        .append(&[
            make_envelope(RuntimeEvent::ProviderBindingCreated(
                ProviderBindingCreated {
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
                },
            )),
            make_envelope(RuntimeEvent::ProviderBindingCreated(
                ProviderBindingCreated {
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
                },
            )),
        ])
        .await
        .unwrap();

    // Route resolver (uses first active binding in creation order).
    struct StoreBindings(Arc<InMemoryStore>);
    #[async_trait]
    impl BindingQuery for StoreBindings {
        async fn list_active_bindings(
            &self,
            project: &ProjectKey,
            operation: OperationKind,
        ) -> Result<Vec<cairn_domain::providers::ProviderBindingRecord>, cairn_runtime::RuntimeError>
        {
            Ok(ProviderBindingReadModel::list_active(self.0.as_ref(), project, operation).await?)
        }
    }

    let resolver = SimpleRouteResolver::new(StoreBindings(store.clone()));

    let decision = resolver
        .resolve(
            &project(),
            OperationKind::Generate,
            &cairn_domain::selectors::SelectorContext::default(),
        )
        .await
        .unwrap();

    // SimpleRouteResolver picks the first active binding; both are marked Generate.
    // The first created binding is "binding_gen", so it should be selected.
    let selected = decision
        .selected_provider_binding_id
        .expect("should have a selection");
    assert_eq!(
        selected.as_str(),
        "binding_gen",
        "the first Generate-capable binding must be selected, got: {}",
        selected.as_str()
    );
    assert_ne!(selected.as_str(), "binding_embed_as_gen");
}

/// list() returns all models registered for a connection.
#[tokio::test]
async fn provider_model_registry_list_by_connection() {
    let store = Arc::new(TestStore::new());
    let svc = ProviderModelServiceImpl::new(store.clone());
    let conn_id = ProviderConnectionId::new("conn_list");

    for (model_id, op) in [
        ("m1", OperationKind::Generate),
        ("m2", OperationKind::Embed),
    ] {
        svc.register(
            tenant(),
            conn_id.clone(),
            model_id.to_owned(),
            ProviderModelCapability {
                model_id: ProviderModelId::new(model_id),
                capabilities: vec![],
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
