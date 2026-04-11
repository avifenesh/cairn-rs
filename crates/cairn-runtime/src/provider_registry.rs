use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use cairn_domain::providers::{
    EmbeddingProvider as DomainEmbeddingProvider, GenerationProvider, GenerationResponse,
    ProviderAdapterError, ProviderBindingSettings, ProviderConnectionRecord,
    ProviderConnectionStatus,
};
use cairn_domain::{CredentialId, ProviderConnectionId, TenantId};
use cairn_providers::chat::{ChatMessage, ChatProvider};
use cairn_providers::wire::openai_compat::OpenAiCompat;
use cairn_providers::{Backend, ProviderBuilder};
use cairn_store::projections::{CredentialReadModel, DefaultsReadModel, ProviderConnectionReadModel};

use crate::error::RuntimeError;
use crate::services::credential_impl::decrypt_credential_record;

const SYSTEM_SCOPE_ID: &str = "system";
const MAX_CONNECTION_SCAN: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderResolutionPurpose {
    Brain,
    Generate,
    Stream,
}

#[derive(Clone)]
pub struct StartupProviderEntry {
    generation: Arc<dyn GenerationProvider>,
    chat: Option<Arc<dyn ChatProvider>>,
    embedding: Option<Arc<dyn DomainEmbeddingProvider>>,
}

impl StartupProviderEntry {
    pub fn generation(provider: Arc<dyn GenerationProvider>) -> Self {
        Self {
            generation: provider,
            chat: None,
            embedding: None,
        }
    }

    pub fn with_chat(
        generation: Arc<dyn GenerationProvider>,
        chat: Arc<dyn ChatProvider>,
    ) -> Self {
        Self {
            generation,
            chat: Some(chat),
            embedding: None,
        }
    }

    pub fn with_embedding(
        generation: Arc<dyn GenerationProvider>,
        embedding: Arc<dyn DomainEmbeddingProvider>,
    ) -> Self {
        Self {
            generation,
            chat: None,
            embedding: Some(embedding),
        }
    }

    pub fn with_chat_and_embedding(
        generation: Arc<dyn GenerationProvider>,
        chat: Arc<dyn ChatProvider>,
        embedding: Arc<dyn DomainEmbeddingProvider>,
    ) -> Self {
        Self {
            generation,
            chat: Some(chat),
            embedding: Some(embedding),
        }
    }
}

#[derive(Clone, Default)]
pub struct StartupFallbackProviders {
    pub ollama: Option<StartupProviderEntry>,
    pub brain: Option<StartupProviderEntry>,
    pub worker: Option<StartupProviderEntry>,
    pub openrouter: Option<StartupProviderEntry>,
    pub bedrock: Option<StartupProviderEntry>,
}

struct CachedProvider {
    chat: Arc<dyn ChatProvider>,
    generation: Arc<dyn GenerationProvider>,
    embedding: Option<Arc<dyn DomainEmbeddingProvider>>,
}

pub struct ProviderRegistry<S> {
    store: Arc<S>,
    cache: Mutex<HashMap<String, Arc<CachedProvider>>>,
    fallbacks: RwLock<StartupFallbackProviders>,
}

impl<S> ProviderRegistry<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self {
            store,
            cache: Mutex::new(HashMap::new()),
            fallbacks: RwLock::new(StartupFallbackProviders::default()),
        }
    }

    pub fn set_startup_fallbacks(&self, fallbacks: StartupFallbackProviders) {
        *write_lock(&self.fallbacks) = fallbacks;
        self.invalidate_all();
    }

    pub fn invalidate(&self, connection_id: &ProviderConnectionId) {
        lock(&self.cache).remove(connection_id.as_str());
    }

    pub fn invalidate_all(&self) {
        lock(&self.cache).clear();
    }
}

impl<S> ProviderRegistry<S>
where
    S: ProviderConnectionReadModel + DefaultsReadModel + CredentialReadModel + Send + Sync + 'static,
{
    pub async fn resolve_generation_for_model(
        &self,
        tenant_id: &TenantId,
        model_id: &str,
        purpose: ProviderResolutionPurpose,
    ) -> Result<Option<Arc<dyn GenerationProvider>>, RuntimeError> {
        let active_connections = self.active_connections(tenant_id).await?;
        if active_connections.is_empty() {
            return Ok(self.select_fallback_generation(model_id, purpose));
        }

        let Some(connection) = select_connection(&active_connections, model_id) else {
            return Ok(None);
        };
        let cached = self.cached_provider_for_connection(connection, model_id).await?;
        Ok(Some(cached.generation.clone()))
    }

    pub async fn resolve_chat_for_model(
        &self,
        tenant_id: &TenantId,
        model_id: &str,
        purpose: ProviderResolutionPurpose,
    ) -> Result<Option<Arc<dyn ChatProvider>>, RuntimeError> {
        let active_connections = self.active_connections(tenant_id).await?;
        if active_connections.is_empty() {
            return Ok(self.select_fallback_chat(model_id, purpose));
        }

        let Some(connection) = select_connection(&active_connections, model_id) else {
            return Ok(None);
        };
        let cached = self.cached_provider_for_connection(connection, model_id).await?;
        Ok(Some(cached.chat.clone()))
    }

    pub async fn resolve_embedding_for_model(
        &self,
        tenant_id: &TenantId,
        model_id: &str,
    ) -> Result<Option<Arc<dyn DomainEmbeddingProvider>>, RuntimeError> {
        let active_connections = self.active_connections(tenant_id).await?;
        if active_connections.is_empty() {
            return Ok(self.select_fallback_embedding(model_id));
        }

        let Some(connection) = select_connection(&active_connections, model_id) else {
            return Ok(None);
        };
        let cached = self.cached_provider_for_connection(connection, model_id).await?;
        Ok(cached.embedding.clone())
    }

    async fn active_connections(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<ProviderConnectionRecord>, RuntimeError> {
        let connections = ProviderConnectionReadModel::list_by_tenant(
            self.store.as_ref(),
            tenant_id,
            MAX_CONNECTION_SCAN,
            0,
        )
        .await?;
        Ok(connections
            .into_iter()
            .filter(|connection| connection.status == ProviderConnectionStatus::Active)
            .collect())
    }

    async fn cached_provider_for_connection(
        &self,
        connection: &ProviderConnectionRecord,
        requested_model: &str,
    ) -> Result<Arc<CachedProvider>, RuntimeError> {
        if let Some(cached) = lock(&self.cache)
            .get(connection.provider_connection_id.as_str())
            .cloned()
        {
            return Ok(cached);
        }

        let built = Arc::new(self.build_provider(connection, requested_model).await?);
        lock(&self.cache).insert(
            connection.provider_connection_id.as_str().to_owned(),
            built.clone(),
        );
        Ok(built)
    }

    async fn build_provider(
        &self,
        connection: &ProviderConnectionRecord,
        requested_model: &str,
    ) -> Result<CachedProvider, RuntimeError> {
        let backend = backend_for_connection(connection)?;
        let endpoint = self.endpoint_for_connection(&connection.provider_connection_id).await?;
        let api_key = self.api_key_for_connection(&connection.provider_connection_id).await?;
        let configured_model = if !requested_model.is_empty()
            && connection
                .supported_models
                .iter()
                .any(|model| model.eq_ignore_ascii_case(requested_model))
        {
            Some(requested_model.to_owned())
        } else {
            connection
                .supported_models
                .first()
                .cloned()
                .or_else(|| (!requested_model.is_empty()).then(|| requested_model.to_owned()))
        };

        let mut builder = ProviderBuilder::new(backend.clone());
        if let Some(endpoint) = endpoint.clone() {
            builder = builder.base_url(endpoint);
        }
        if let Some(api_key) = api_key.clone() {
            builder = builder.api_key(api_key);
        }
        if let Some(model) = configured_model.clone() {
            builder = builder.model(model);
        }

        let chat: Arc<dyn ChatProvider> = Arc::from(builder.build_chat().map_err(|err| {
            RuntimeError::Validation {
                reason: format!(
                    "failed to build provider connection {}: {err}",
                    connection.provider_connection_id
                ),
            }
        })?);

        let default_model = configured_model.unwrap_or_default();
        let generation: Arc<dyn GenerationProvider> =
            Arc::new(ChatProviderGenerationAdapter::new(chat.clone(), default_model));
        let embedding = build_embedding_provider(
            &backend,
            endpoint,
            api_key,
            connection,
            requested_model,
        );

        Ok(CachedProvider {
            chat,
            generation,
            embedding,
        })
    }

    async fn endpoint_for_connection(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Option<String>, RuntimeError> {
        let key = format!("provider_endpoint_{}", connection_id.as_str());
        let setting = DefaultsReadModel::get(
            self.store.as_ref(),
            cairn_domain::Scope::System,
            SYSTEM_SCOPE_ID,
            &key,
        )
        .await?;

        match setting {
            Some(setting) => Ok(setting.value.as_str().map(str::to_owned)),
            None => Ok(None),
        }
    }

    async fn api_key_for_connection(
        &self,
        connection_id: &ProviderConnectionId,
    ) -> Result<Option<String>, RuntimeError> {
        let key = format!("provider_credential_{}", connection_id.as_str());
        let setting = DefaultsReadModel::get(
            self.store.as_ref(),
            cairn_domain::Scope::System,
            SYSTEM_SCOPE_ID,
            &key,
        )
        .await?;

        let Some(credential_id) = setting.and_then(|setting| setting.value.as_str().map(str::to_owned))
        else {
            return Ok(None);
        };

        let credential = CredentialReadModel::get(
            self.store.as_ref(),
            &CredentialId::new(credential_id.as_str()),
        )
        .await?
        .ok_or_else(|| RuntimeError::NotFound {
            entity: "credential",
            id: credential_id.clone(),
        })?;

        if !credential.active {
            return Err(RuntimeError::Validation {
                reason: format!(
                    "credential {} linked to provider connection {} is revoked",
                    credential.id, connection_id
                ),
            });
        }

        Ok(Some(decrypt_credential_record(&credential)?))
    }

    fn select_fallback_generation(
        &self,
        model_id: &str,
        purpose: ProviderResolutionPurpose,
    ) -> Option<Arc<dyn GenerationProvider>> {
        let fallbacks = read_lock(&self.fallbacks);
        if is_bedrock_model(model_id) {
            return fallbacks.bedrock.as_ref().map(|entry| entry.generation.clone());
        }

        match purpose {
            ProviderResolutionPurpose::Brain => fallbacks
                .brain
                .as_ref()
                .map(|entry| entry.generation.clone())
                .or_else(|| fallbacks.worker.as_ref().map(|entry| entry.generation.clone()))
                .or_else(|| {
                    fallbacks
                        .openrouter
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| fallbacks.bedrock.as_ref().map(|entry| entry.generation.clone()))
                .or_else(|| fallbacks.ollama.as_ref().map(|entry| entry.generation.clone())),
            ProviderResolutionPurpose::Generate => {
                if let Some(ollama) = fallbacks.ollama.as_ref() {
                    return Some(ollama.generation.clone());
                }
                if is_brain_model(model_id) {
                    fallbacks
                        .brain
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                        .or_else(|| {
                            fallbacks.worker.as_ref().map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .openrouter
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks.bedrock.as_ref().map(|entry| entry.generation.clone())
                        })
                } else {
                    fallbacks
                        .worker
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                        .or_else(|| {
                            fallbacks.brain.as_ref().map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .openrouter
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks.bedrock.as_ref().map(|entry| entry.generation.clone())
                        })
                }
            }
            ProviderResolutionPurpose::Stream => fallbacks
                .brain
                .as_ref()
                .map(|entry| entry.generation.clone())
                .or_else(|| fallbacks.worker.as_ref().map(|entry| entry.generation.clone()))
                .or_else(|| {
                    fallbacks
                        .openrouter
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| fallbacks.bedrock.as_ref().map(|entry| entry.generation.clone()))
                .or_else(|| fallbacks.ollama.as_ref().map(|entry| entry.generation.clone())),
        }
    }

    fn select_fallback_chat(
        &self,
        model_id: &str,
        purpose: ProviderResolutionPurpose,
    ) -> Option<Arc<dyn ChatProvider>> {
        let fallbacks = read_lock(&self.fallbacks);
        if is_bedrock_model(model_id) {
            return fallbacks
                .bedrock
                .as_ref()
                .and_then(|entry| entry.chat.clone());
        }

        match purpose {
            ProviderResolutionPurpose::Brain => fallbacks
                .brain
                .as_ref()
                .and_then(|entry| entry.chat.clone())
                .or_else(|| {
                    fallbacks
                        .worker
                        .as_ref()
                        .and_then(|entry| entry.chat.clone())
                })
                .or_else(|| {
                    fallbacks
                        .openrouter
                        .as_ref()
                        .and_then(|entry| entry.chat.clone())
                }),
            ProviderResolutionPurpose::Generate | ProviderResolutionPurpose::Stream => {
                if is_brain_model(model_id) {
                    fallbacks
                        .brain
                        .as_ref()
                        .and_then(|entry| entry.chat.clone())
                        .or_else(|| {
                            fallbacks
                                .worker
                                .as_ref()
                                .and_then(|entry| entry.chat.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .openrouter
                                .as_ref()
                                .and_then(|entry| entry.chat.clone())
                        })
                } else {
                    fallbacks
                        .worker
                        .as_ref()
                        .and_then(|entry| entry.chat.clone())
                        .or_else(|| {
                            fallbacks
                                .brain
                                .as_ref()
                                .and_then(|entry| entry.chat.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .openrouter
                                .as_ref()
                                .and_then(|entry| entry.chat.clone())
                        })
                }
            }
        }
    }

    fn select_fallback_embedding(
        &self,
        model_id: &str,
    ) -> Option<Arc<dyn DomainEmbeddingProvider>> {
        let fallbacks = read_lock(&self.fallbacks);

        if let Some(ollama) = fallbacks.ollama.as_ref().and_then(|entry| entry.embedding.clone()) {
            return Some(ollama);
        }

        if is_brain_model(model_id) {
            fallbacks
                .brain
                .as_ref()
                .and_then(|entry| entry.embedding.clone())
                .or_else(|| {
                    fallbacks
                        .worker
                        .as_ref()
                        .and_then(|entry| entry.embedding.clone())
                })
                .or_else(|| {
                    fallbacks
                        .openrouter
                        .as_ref()
                        .and_then(|entry| entry.embedding.clone())
                })
        } else {
            fallbacks
                .worker
                .as_ref()
                .and_then(|entry| entry.embedding.clone())
                .or_else(|| {
                    fallbacks
                        .brain
                        .as_ref()
                        .and_then(|entry| entry.embedding.clone())
                })
                .or_else(|| {
                    fallbacks
                        .openrouter
                        .as_ref()
                        .and_then(|entry| entry.embedding.clone())
                })
        }
    }
}

struct ChatProviderGenerationAdapter {
    chat: Arc<dyn ChatProvider>,
    default_model: String,
}

impl ChatProviderGenerationAdapter {
    fn new(chat: Arc<dyn ChatProvider>, default_model: String) -> Self {
        Self { chat, default_model }
    }
}

#[async_trait]
impl GenerationProvider for ChatProviderGenerationAdapter {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let chat_messages = json_messages_to_chat_messages(&messages);
        let response = self
            .chat
            .chat_with_tools(&chat_messages, None, None)
            .await
            .map_err(map_provider_error)?;

        let usage = response.usage();
        Ok(GenerationResponse {
            text: response.text().unwrap_or_default(),
            input_tokens: usage.as_ref().map(|usage| usage.prompt_tokens),
            output_tokens: usage.as_ref().map(|usage| usage.completion_tokens),
            model_id: if model_id.is_empty() {
                self.default_model.clone()
            } else {
                model_id.to_owned()
            },
            tool_calls: response
                .tool_calls()
                .unwrap_or_default()
                .into_iter()
                .map(|tool_call| {
                    serde_json::json!({
                        "id": tool_call.id,
                        "type": tool_call.call_type,
                        "function": {
                            "name": tool_call.function.name,
                            "arguments": tool_call.function.arguments,
                        }
                    })
                })
                .collect(),
        })
    }
}

pub fn json_messages_to_chat_messages(messages: &[serde_json::Value]) -> Vec<ChatMessage> {
    messages
        .iter()
        .map(|message| {
            let role = message
                .get("role")
                .and_then(|value| value.as_str())
                .unwrap_or("user");
            let content = message
                .get("content")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .to_owned();
            match role {
                "system" => ChatMessage::system(content),
                "assistant" => ChatMessage::assistant(content),
                _ => ChatMessage::user(content),
            }
        })
        .collect()
}

fn map_provider_error(error: cairn_providers::error::ProviderError) -> ProviderAdapterError {
    match error {
        cairn_providers::error::ProviderError::RateLimited => ProviderAdapterError::RateLimited,
        cairn_providers::error::ProviderError::Http(message)
        | cairn_providers::error::ProviderError::Auth(message) => {
            ProviderAdapterError::TransportFailure(message)
        }
        other => ProviderAdapterError::ProviderError(other.to_string()),
    }
}

fn backend_for_connection(connection: &ProviderConnectionRecord) -> Result<Backend, RuntimeError> {
    for raw in [&connection.adapter_type, &connection.provider_family] {
        let normalized = normalize_backend(raw);
        if let Ok(backend) = normalized.parse::<Backend>() {
            return Ok(backend);
        }
    }
    Err(RuntimeError::Validation {
        reason: format!(
            "unsupported provider backend for connection {}",
            connection.provider_connection_id
        ),
    })
}

fn normalize_backend(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "openai_compat" | "openai-compat" | "openai_compatible" => {
            "openai-compatible".to_owned()
        }
        other => other.to_owned(),
    }
}

fn build_embedding_provider(
    backend: &Backend,
    endpoint: Option<String>,
    api_key: Option<String>,
    connection: &ProviderConnectionRecord,
    requested_model: &str,
) -> Option<Arc<dyn DomainEmbeddingProvider>> {
    if matches!(backend, Backend::Bedrock) {
        return None;
    }

    let model = if !requested_model.is_empty() {
        Some(requested_model.to_owned())
    } else {
        connection.supported_models.first().cloned()
    };

    let embedding: Arc<dyn DomainEmbeddingProvider> = Arc::new(OpenAiCompat::new(
        backend.config(),
        api_key.unwrap_or_default(),
        endpoint,
        model,
        None,
        None,
        None,
    ));
    Some(embedding)
}

fn select_connection<'a>(
    connections: &'a [ProviderConnectionRecord],
    model_id: &str,
) -> Option<&'a ProviderConnectionRecord> {
    if connections.is_empty() {
        return None;
    }
    (!model_id.is_empty()).then_some(())?;
    connections.iter().find(|connection| {
        connection
            .supported_models
            .iter()
            .any(|model| model.eq_ignore_ascii_case(model_id))
    })
}

fn is_bedrock_model(model_id: &str) -> bool {
    model_id.contains('.') && !model_id.contains('/')
}

fn is_brain_model(model_id: &str) -> bool {
    let normalized = model_id.to_ascii_lowercase();
    normalized == "openrouter/free"
        || normalized.contains("gemma-3-27b")
        || normalized.contains("qwen3-coder")
        || normalized.contains("gemma-4")
        || normalized.contains("gemma4")
        || normalized.contains("cyankiwi")
        || normalized.contains("brain")
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn read_lock<T>(lockable: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lockable
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_lock<T>(lockable: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lockable
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use cairn_domain::providers::{
        EmbeddingProvider as DomainEmbeddingProvider, EmbeddingResponse, GenerationProvider,
        GenerationResponse, ProviderAdapterError, ProviderBindingSettings,
    };
    use cairn_domain::{ProviderConnectionId, TenantId};
    use cairn_store::InMemoryStore;

    use crate::provider_connections::{ProviderConnectionConfig, ProviderConnectionService};
    use crate::services::{
        CredentialServiceImpl, DefaultsServiceImpl, ProviderConnectionServiceImpl, TenantServiceImpl,
    };
    use crate::tenants::TenantService;
    use crate::{CredentialService, DefaultsService};

    use super::{
        ProviderRegistry, ProviderResolutionPurpose, StartupFallbackProviders, StartupProviderEntry,
    };

    struct FakeGenerationProvider {
        label: &'static str,
    }

    struct FakeEmbeddingProvider {
        token_count: u32,
    }

    #[async_trait]
    impl GenerationProvider for FakeGenerationProvider {
        async fn generate(
            &self,
            model_id: &str,
            _messages: Vec<serde_json::Value>,
            _settings: &ProviderBindingSettings,
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Ok(GenerationResponse {
                text: self.label.to_owned(),
                input_tokens: None,
                output_tokens: None,
                model_id: model_id.to_owned(),
                tool_calls: Vec::new(),
            })
        }
    }

    #[async_trait]
    impl DomainEmbeddingProvider for FakeEmbeddingProvider {
        async fn embed(
            &self,
            model_id: &str,
            texts: Vec<String>,
        ) -> Result<EmbeddingResponse, ProviderAdapterError> {
            Ok(EmbeddingResponse {
                embeddings: texts
                    .into_iter()
                    .map(|text| vec![text.len() as f32])
                    .collect(),
                model_id: model_id.to_owned(),
                token_count: self.token_count,
            })
        }
    }

    #[tokio::test]
    async fn caches_connection_backed_generation_providers_by_connection_id() {
        let store = seeded_store().await;
        seed_connection(&store, "conn_cache").await;
        let registry = ProviderRegistry::new(store);

        let first = registry
            .resolve_generation_for_model(
                &TenantId::new("tenant_registry"),
                "gpt-4o-mini",
                ProviderResolutionPurpose::Generate,
            )
            .await
            .unwrap()
            .unwrap();
        let second = registry
            .resolve_generation_for_model(
                &TenantId::new("tenant_registry"),
                "gpt-4o-mini",
                ProviderResolutionPurpose::Generate,
            )
            .await
            .unwrap()
            .unwrap();

        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn invalidate_rebuilds_connection_backed_provider() {
        let store = seeded_store().await;
        seed_connection(&store, "conn_invalidate").await;
        let registry = ProviderRegistry::new(store);
        let connection_id = ProviderConnectionId::new("conn_invalidate");

        let first = registry
            .resolve_generation_for_model(
                &TenantId::new("tenant_registry"),
                "gpt-4o-mini",
                ProviderResolutionPurpose::Generate,
            )
            .await
            .unwrap()
            .unwrap();
        registry.invalidate(&connection_id);
        let second = registry
            .resolve_generation_for_model(
                &TenantId::new("tenant_registry"),
                "gpt-4o-mini",
                ProviderResolutionPurpose::Generate,
            )
            .await
            .unwrap()
            .unwrap();

        assert!(!Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn falls_back_to_startup_generation_when_no_connections_exist() {
        let store = seeded_store().await;
        let registry = ProviderRegistry::new(store);
        let fallback: Arc<dyn GenerationProvider> = Arc::new(FakeGenerationProvider {
            label: "fallback",
        });
        registry.set_startup_fallbacks(StartupFallbackProviders {
            worker: Some(StartupProviderEntry::generation(fallback.clone())),
            ..Default::default()
        });

        let resolved = registry
            .resolve_generation_for_model(
                &TenantId::new("tenant_registry"),
                "worker-lite",
                ProviderResolutionPurpose::Generate,
            )
            .await
            .unwrap()
            .unwrap();

        assert!(Arc::ptr_eq(&resolved, &fallback));
    }

    #[tokio::test]
    async fn falls_back_to_startup_embedding_when_no_connections_exist() {
        let store = seeded_store().await;
        let registry = ProviderRegistry::new(store);
        let fallback: Arc<dyn DomainEmbeddingProvider> =
            Arc::new(FakeEmbeddingProvider { token_count: 7 });
        registry.set_startup_fallbacks(StartupFallbackProviders {
            worker: Some(StartupProviderEntry::with_embedding(
                Arc::new(FakeGenerationProvider { label: "worker" }),
                fallback.clone(),
            )),
            ..Default::default()
        });

        let resolved = registry
            .resolve_embedding_for_model(&TenantId::new("tenant_registry"), "worker-embed")
            .await
            .unwrap()
            .unwrap();

        assert!(Arc::ptr_eq(&resolved, &fallback));
    }

    async fn seeded_store() -> Arc<InMemoryStore> {
        let store = Arc::new(InMemoryStore::new());
        let tenants = TenantServiceImpl::new(store.clone());
        tenants
            .create(TenantId::new("tenant_registry"), "Registry Tenant".to_owned())
            .await
            .unwrap();
        store
    }

    async fn seed_connection(store: &Arc<InMemoryStore>, connection_id: &str) {
        let connections = ProviderConnectionServiceImpl::new(store.clone());
        let credentials = CredentialServiceImpl::new(store.clone());
        let defaults = DefaultsServiceImpl::new(store.clone());

        connections
            .create(
                TenantId::new("tenant_registry"),
                ProviderConnectionId::new(connection_id),
                ProviderConnectionConfig {
                    provider_family: "openai".to_owned(),
                    adapter_type: "openai_compat".to_owned(),
                    supported_models: vec!["gpt-4o-mini".to_owned()],
                },
            )
            .await
            .unwrap();

        let credential = credentials
            .store(
                TenantId::new("tenant_registry"),
                "openai".to_owned(),
                "sk-test".to_owned(),
                Some("test-key".to_owned()),
            )
            .await
            .unwrap();

        defaults
            .set(
                cairn_domain::Scope::System,
                "system".to_owned(),
                format!("provider_credential_{connection_id}"),
                serde_json::json!(credential.id.as_str()),
            )
            .await
            .unwrap();
        defaults
            .set(
                cairn_domain::Scope::System,
                "system".to_owned(),
                format!("provider_endpoint_{connection_id}"),
                serde_json::json!("https://example.com/v1"),
            )
            .await
            .unwrap();
    }
}
