use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

use async_trait::async_trait;
use cairn_domain::providers::{
    EmbeddingProvider as DomainEmbeddingProvider, GenerationProvider, GenerationResponse,
    ProviderAdapterError, ProviderBindingSettings, ProviderConnectionRecord,
    ProviderConnectionStatus,
};
use cairn_domain::{CredentialId, ProviderConnectionId, TenantId};
use cairn_providers::chat::{ChatMessage, ChatProvider, FunctionDef, Tool};
use cairn_providers::wire::openai_compat::OpenAiCompat;
use cairn_providers::{Backend, ProviderBuilder};
use cairn_store::projections::{
    CredentialReadModel, DefaultsReadModel, ProviderConnectionReadModel,
};

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
    backend: String,
    model: Option<String>,
}

impl StartupProviderEntry {
    pub fn generation(provider: Arc<dyn GenerationProvider>) -> Self {
        Self {
            generation: provider,
            chat: None,
            embedding: None,
            backend: "unknown".to_owned(),
            model: None,
        }
    }

    pub fn with_chat(generation: Arc<dyn GenerationProvider>, chat: Arc<dyn ChatProvider>) -> Self {
        Self {
            generation,
            chat: Some(chat),
            embedding: None,
            backend: "unknown".to_owned(),
            model: None,
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
            backend: "unknown".to_owned(),
            model: None,
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
            backend: "unknown".to_owned(),
            model: None,
        }
    }

    pub fn with_metadata(mut self, backend: impl Into<String>, model: Option<String>) -> Self {
        self.backend = backend.into();
        self.model = model.filter(|value| !value.is_empty());
        self
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
    connection_id: String,
    backend: String,
    model: String,
    chat: Arc<dyn ChatProvider>,
    generation: Arc<dyn GenerationProvider>,
    embedding: Option<Arc<dyn DomainEmbeddingProvider>>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProviderRegistryConnectionState {
    pub connection_id: String,
    pub backend: String,
    pub model: String,
    pub cached: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProviderRegistryFallbackState {
    pub source: String,
    pub backend: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct ProviderRegistrySnapshot {
    pub connections: Vec<ProviderRegistryConnectionState>,
    pub fallbacks: Vec<ProviderRegistryFallbackState>,
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

    pub fn snapshot(&self) -> ProviderRegistrySnapshot {
        let mut connections: Vec<_> = lock(&self.cache)
            .values()
            .map(|cached| ProviderRegistryConnectionState {
                connection_id: cached.connection_id.clone(),
                backend: cached.backend.clone(),
                model: cached.model.clone(),
                cached: true,
            })
            .collect();
        connections.sort_by_key(|r| r.connection_id.clone());

        let fallbacks = fallback_snapshot(&read_lock(&self.fallbacks));

        ProviderRegistrySnapshot {
            connections,
            fallbacks,
        }
    }
}

impl<S> ProviderRegistry<S>
where
    S: ProviderConnectionReadModel
        + DefaultsReadModel
        + CredentialReadModel
        + Send
        + Sync
        + 'static,
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
        let cached = self
            .cached_provider_for_connection(connection, model_id)
            .await?;
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
        let cached = self
            .cached_provider_for_connection(connection, model_id)
            .await?;
        Ok(Some(cached.chat.clone()))
    }

    pub async fn has_active_connections(&self, tenant_id: &TenantId) -> Result<bool, RuntimeError> {
        Ok(!self.active_connections(tenant_id).await?.is_empty())
    }

    /// Summarise the active connections for a tenant as a list of
    /// `(connection_id, supported_models)` pairs.
    ///
    /// Used by operator-facing error messages to tell the caller exactly
    /// which connections are registered and which models they serve when
    /// model resolution fails — turning the unhelpful "no provider
    /// configured" 503 into an actionable "this model isn't served by
    /// any active connection" hint. See issue #156.
    pub async fn active_connection_summaries(
        &self,
        tenant_id: &TenantId,
    ) -> Result<Vec<(String, Vec<String>)>, RuntimeError> {
        let connections = self.active_connections(tenant_id).await?;
        Ok(connections
            .into_iter()
            .map(|c| {
                (
                    c.provider_connection_id.as_str().to_owned(),
                    c.supported_models.clone(),
                )
            })
            .collect())
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
        let cached = self
            .cached_provider_for_connection(connection, model_id)
            .await?;
        Ok(cached.embedding.clone())
    }

    /// Resolve the `GenerationProvider` adapter for an **exact** provider
    /// connection by ID, rather than by "first connection that supports
    /// `model_id`". Used by the orchestrator when composing a
    /// `RoutedGenerationService` across multiple bindings — if two
    /// connections share a model slug (common for proxy providers that
    /// both expose `gpt-4o-mini`), `resolve_generation_for_model` would
    /// return the same adapter for both bindings and silently conflate
    /// them. This API is deterministic on connection ID.
    pub async fn resolve_generation_for_connection(
        &self,
        tenant_id: &TenantId,
        connection_id: &ProviderConnectionId,
        model_id: &str,
    ) -> Result<Option<Arc<dyn GenerationProvider>>, RuntimeError> {
        let active_connections = self.active_connections(tenant_id).await?;
        let Some(connection) = active_connections
            .iter()
            .find(|c| c.provider_connection_id == *connection_id)
        else {
            return Ok(None);
        };
        let cached = self
            .cached_provider_for_connection(connection, model_id)
            .await?;
        Ok(Some(cached.generation.clone()))
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
        let endpoint = self
            .endpoint_for_connection(&connection.provider_connection_id)
            .await?;
        let api_key = self
            .api_key_for_connection(&connection.provider_connection_id)
            .await?;
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

        let chat: Arc<dyn ChatProvider> =
            Arc::from(
                builder
                    .build_chat()
                    .map_err(|err| RuntimeError::Validation {
                        reason: format!(
                            "failed to build provider connection {}: {err}",
                            connection.provider_connection_id
                        ),
                    })?,
            );

        let default_model = configured_model.unwrap_or_default();
        let generation: Arc<dyn GenerationProvider> = Arc::new(ChatProviderGenerationAdapter::new(
            chat.clone(),
            default_model.clone(),
        ));
        let embedding =
            build_embedding_provider(&backend, endpoint, api_key, connection, requested_model)
                .map_err(|err| RuntimeError::Validation {
                    reason: format!(
                        "failed to build embedding provider for connection {}: {err}",
                        connection.provider_connection_id
                    ),
                })?;

        Ok(CachedProvider {
            connection_id: connection.provider_connection_id.as_str().to_owned(),
            backend: backend.to_string(),
            model: default_model,
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

        let Some(credential_id) =
            setting.and_then(|setting| setting.value.as_str().map(str::to_owned))
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
            return fallbacks
                .bedrock
                .as_ref()
                .map(|entry| entry.generation.clone());
        }

        match purpose {
            ProviderResolutionPurpose::Brain => fallbacks
                .brain
                .as_ref()
                .map(|entry| entry.generation.clone())
                .or_else(|| {
                    fallbacks
                        .worker
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| {
                    fallbacks
                        .openrouter
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| {
                    fallbacks
                        .bedrock
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| {
                    fallbacks
                        .ollama
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                }),
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
                            fallbacks
                                .worker
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .openrouter
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .bedrock
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                } else {
                    fallbacks
                        .worker
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                        .or_else(|| {
                            fallbacks
                                .brain
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .openrouter
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                        .or_else(|| {
                            fallbacks
                                .bedrock
                                .as_ref()
                                .map(|entry| entry.generation.clone())
                        })
                }
            }
            ProviderResolutionPurpose::Stream => fallbacks
                .brain
                .as_ref()
                .map(|entry| entry.generation.clone())
                .or_else(|| {
                    fallbacks
                        .worker
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| {
                    fallbacks
                        .openrouter
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| {
                    fallbacks
                        .bedrock
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                })
                .or_else(|| {
                    fallbacks
                        .ollama
                        .as_ref()
                        .map(|entry| entry.generation.clone())
                }),
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

        if let Some(ollama) = fallbacks
            .ollama
            .as_ref()
            .and_then(|entry| entry.embedding.clone())
        {
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
        Self {
            chat,
            default_model,
        }
    }
}

#[async_trait]
impl GenerationProvider for ChatProviderGenerationAdapter {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        _settings: &ProviderBindingSettings,
        tools: &[serde_json::Value],
    ) -> Result<GenerationResponse, ProviderAdapterError> {
        let chat_messages = json_messages_to_chat_messages(&messages);
        let native_tools: Option<Vec<Tool>> = if tools.is_empty() {
            None
        } else {
            let converted: Vec<Tool> = tools
                .iter()
                .filter_map(|t| {
                    let func = t.get("function")?;
                    Some(Tool {
                        tool_type: "function".to_owned(),
                        function: FunctionDef {
                            name: func.get("name")?.as_str()?.to_owned(),
                            description: func
                                .get("description")
                                .and_then(|d| d.as_str())
                                .unwrap_or("")
                                .to_owned(),
                            parameters: func
                                .get("parameters")
                                .cloned()
                                .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
                        },
                    })
                })
                .collect();
            if converted.is_empty() {
                None
            } else {
                Some(converted)
            }
        };

        // Route the call to the exact model the orchestrator asked for.
        // The DECIDE-phase fallback chain (see `cairn_orchestrator::ModelFallbackChain`)
        // relies on this: attempt N sends `model_id = <chain[N]>`, and each
        // attempt must actually hit the requested upstream model rather than
        // falling back to whichever model happened to be the connection's
        // default. Before F17 this adapter silently ignored `model_id` and
        // every attempt hit the same upstream — which meant a single 429 on
        // the preferred model produced N consecutive 429s across the chain
        // and the fallback did nothing.
        let effective_model = if model_id.is_empty() {
            None
        } else {
            Some(model_id)
        };
        let response = self
            .chat
            .chat_with_tools_for_model(
                effective_model,
                &chat_messages,
                native_tools.as_deref(),
                None,
            )
            .await
            .map_err(map_provider_error)?;

        let usage = response.usage();
        let finish_reason = response.finish_reason();
        let text = response.text().unwrap_or_default();
        let tool_calls_vec: Vec<serde_json::Value> = response
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
            .collect();

        // Detect empty completions (successful HTTP, zero usable output).
        // MiniMax-minimax-m2.5:free dogfood failure mode — surfaces as
        // a distinct error so the fallback chain can retry on another model.
        let resolved_model_id = if model_id.is_empty() {
            self.default_model.clone()
        } else {
            model_id.to_owned()
        };
        if text.trim().is_empty() && tool_calls_vec.is_empty() {
            return Err(ProviderAdapterError::EmptyResponse {
                model_id: resolved_model_id,
                prompt_tokens: usage.as_ref().map(|u| u.prompt_tokens),
                completion_tokens: usage.as_ref().map(|u| u.completion_tokens),
            });
        }

        Ok(GenerationResponse {
            text,
            input_tokens: usage.as_ref().map(|usage| usage.prompt_tokens),
            output_tokens: usage.as_ref().map(|usage| usage.completion_tokens),
            model_id: resolved_model_id,
            tool_calls: tool_calls_vec,
            finish_reason,
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

/// Map a cairn-providers `ProviderError` to a domain-level
/// `ProviderAdapterError`, preserving enough classification info for the
/// DECIDE-phase fallback chain (see
/// `cairn_orchestrator::ModelFallbackChain::run`) to decide between
/// advancing and escalating.
///
/// Pre-F17 this collapsed `Auth` into `TransportFailure`, which meant the
/// fallback loop would (incorrectly) try the next model on a bad-credential
/// error — every model on the same connection uses the same credential, so
/// the next attempt would fail the same way. Now Auth surfaces as
/// `ProviderAdapterError::Auth`, which
/// [`ProviderAdapterError::is_fallback_eligible`] returns `false` for, so
/// the chain escalates to the operator immediately.
fn map_provider_error(error: cairn_providers::error::ProviderError) -> ProviderAdapterError {
    use cairn_providers::error::ProviderError;
    match error {
        ProviderError::RateLimited => ProviderAdapterError::RateLimited,
        ProviderError::Auth(message) => ProviderAdapterError::Auth(message),
        ProviderError::InvalidRequest(message) => ProviderAdapterError::InvalidRequest(message),
        ProviderError::ServerError { status, message } => {
            ProviderAdapterError::ServerError { status, message }
        }
        ProviderError::EmptyResponse {
            model_id,
            prompt_tokens,
            completion_tokens,
        } => ProviderAdapterError::EmptyResponse {
            model_id,
            prompt_tokens,
            completion_tokens,
        },
        ProviderError::ResponseFormat {
            message,
            raw_response,
        } => ProviderAdapterError::StructuredOutputInvalid(format!(
            "{message} (raw: {raw_response})"
        )),
        ProviderError::Http(message) => ProviderAdapterError::TransportFailure(message),
        ProviderError::Provider(message) => ProviderAdapterError::ProviderError(message),
        ProviderError::Json(message) => ProviderAdapterError::StructuredOutputInvalid(message),
        ProviderError::ToolConfig(message) | ProviderError::Unsupported(message) => {
            ProviderAdapterError::InvalidRequest(message)
        }
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
        "openai_compat" | "openai-compat" | "openai_compatible" => "openai-compatible".to_owned(),
        other => other.to_owned(),
    }
}

fn build_embedding_provider(
    backend: &Backend,
    endpoint: Option<String>,
    api_key: Option<String>,
    connection: &ProviderConnectionRecord,
    requested_model: &str,
) -> Result<Option<Arc<dyn DomainEmbeddingProvider>>, cairn_providers::error::ProviderError> {
    // Bedrock has its own SDK path and no OpenAI-shape embeddings endpoint.
    //
    // For Z.ai: the *native* adapter (wire::zai::ZaiProvider) implements the
    // chat surface only — no embeddings. We return None for both Backend::Zai
    // and Backend::ZaiCoding on purpose:
    //   * The coding endpoint (ZaiCoding) does not expose /embeddings.
    //   * The general paas endpoint (Zai) DOES have an /embeddings route, but
    //     this adapter has not been wired to call it. Until we add a native
    //     embeddings implementation under `wire::zai`, operators that want
    //     Z.ai embeddings should register a second connection with
    //     adapter_type="openai_compat" pointed at
    //     `https://api.z.ai/api/paas/v4/`, which routes through
    //     OpenAiCompat's well-tested embeddings path.
    // Copilot review on #280.
    if matches!(
        backend,
        Backend::Bedrock | Backend::Zai | Backend::ZaiCoding
    ) {
        return Ok(None);
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
    )?);
    Ok(Some(embedding))
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

fn fallback_snapshot(fallbacks: &StartupFallbackProviders) -> Vec<ProviderRegistryFallbackState> {
    let mut entries = Vec::new();
    push_fallback(
        &mut entries,
        "env:OLLAMA_HOST",
        fallbacks.ollama.as_ref(),
        "ollama",
    );
    push_fallback(
        &mut entries,
        "env:CAIRN_BRAIN_URL",
        fallbacks.brain.as_ref(),
        "openai-compatible",
    );
    push_fallback(
        &mut entries,
        "env:CAIRN_WORKER_URL",
        fallbacks.worker.as_ref(),
        "openai-compatible",
    );
    push_fallback(
        &mut entries,
        "env:OPENROUTER_API_KEY",
        fallbacks.openrouter.as_ref(),
        "openrouter",
    );
    push_fallback(
        &mut entries,
        "env:BEDROCK_API_KEY",
        fallbacks.bedrock.as_ref(),
        "bedrock",
    );
    entries
}

fn push_fallback(
    entries: &mut Vec<ProviderRegistryFallbackState>,
    source: &str,
    entry: Option<&StartupProviderEntry>,
    default_backend: &str,
) {
    let Some(entry) = entry else {
        return;
    };
    entries.push(ProviderRegistryFallbackState {
        source: source.to_owned(),
        backend: if entry.backend == "unknown" {
            default_backend.to_owned()
        } else {
            entry.backend.clone()
        },
        model: entry.model.clone(),
        active: true,
    });
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
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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
        CredentialServiceImpl, DefaultsServiceImpl, ProviderConnectionServiceImpl,
        TenantServiceImpl,
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
            _tools: &[serde_json::Value],
        ) -> Result<GenerationResponse, ProviderAdapterError> {
            Ok(GenerationResponse {
                text: self.label.to_owned(),
                input_tokens: None,
                output_tokens: None,
                model_id: model_id.to_owned(),
                tool_calls: Vec::new(),
                finish_reason: None,
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
        let fallback: Arc<dyn GenerationProvider> =
            Arc::new(FakeGenerationProvider { label: "fallback" });
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

    #[tokio::test]
    async fn snapshot_reports_cached_connections_and_configured_fallbacks() {
        let store = seeded_store().await;
        seed_connection(&store, "conn_snapshot").await;
        let registry = ProviderRegistry::new(store);
        registry.set_startup_fallbacks(StartupFallbackProviders {
            brain: Some(
                StartupProviderEntry::generation(Arc::new(FakeGenerationProvider {
                    label: "brain",
                }))
                .with_metadata("openai-compatible", Some("gpt-4.1-mini".to_owned())),
            ),
            bedrock: Some(
                StartupProviderEntry::generation(Arc::new(FakeGenerationProvider {
                    label: "bedrock",
                }))
                .with_metadata("bedrock", Some("minimax.minimax-m2.5".to_owned())),
            ),
            ..Default::default()
        });

        registry
            .resolve_generation_for_model(
                &TenantId::new("tenant_registry"),
                "gpt-4o-mini",
                ProviderResolutionPurpose::Generate,
            )
            .await
            .unwrap()
            .unwrap();

        let snapshot = registry.snapshot();
        assert_eq!(snapshot.connections.len(), 1);
        assert_eq!(snapshot.connections[0].connection_id, "conn_snapshot");
        assert_eq!(snapshot.connections[0].backend, "openai-compatible");
        assert_eq!(snapshot.connections[0].model, "gpt-4o-mini");
        assert!(snapshot.connections[0].cached);

        assert_eq!(snapshot.fallbacks.len(), 2);
        assert!(snapshot.fallbacks.iter().any(|entry| {
            entry.source == "env:CAIRN_BRAIN_URL"
                && entry.backend == "openai-compatible"
                && entry.model.as_deref() == Some("gpt-4.1-mini")
                && entry.active
        }));
        assert!(snapshot.fallbacks.iter().any(|entry| {
            entry.source == "env:BEDROCK_API_KEY"
                && entry.backend == "bedrock"
                && entry.model.as_deref() == Some("minimax.minimax-m2.5")
                && entry.active
        }));
    }

    async fn seeded_store() -> Arc<InMemoryStore> {
        let store = Arc::new(InMemoryStore::new());
        let tenants = TenantServiceImpl::new(store.clone());
        tenants
            .create(
                TenantId::new("tenant_registry"),
                "Registry Tenant".to_owned(),
            )
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
