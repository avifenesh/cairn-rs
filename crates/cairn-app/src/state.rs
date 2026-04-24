//! Application state, GitHub integration, and startup replay.

use async_trait::async_trait;
use std::{
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU32},
        Arc, Mutex,
    },
    time::Instant,
};
use tokio::sync::broadcast;

// ── cairn crates ─────────────────────────────────────────────────────────────

use cairn_api::auth::ServiceTokenRegistry;
use cairn_api::bootstrap::BootstrapConfig;
use cairn_api::onboarding::StarterTemplateRegistry;
use cairn_api::sse::SseFrame;

use cairn_domain::{
    KnowledgeDocumentId, ProjectId, ProjectKey, PromptTemplateVar, RuntimeEvent, SourceId, TaskId,
    TenantId, WorkspaceId,
};

use cairn_evals::services::eval_service::{MemoryDiagnosticsSource, SourceQualitySnapshot};
use cairn_evals::{
    EvalBaselineServiceImpl, EvalDatasetServiceImpl, EvalRubricServiceImpl,
    EvalRunService as ProductEvalRunService, EvalRunStatus, EvalSubjectKind,
    GraphIntegration as EvalGraphIntegration, ModelComparisonServiceImpl, PluginDimensionScore,
    PluginRubricScorer,
};

use cairn_graph::event_projector::EventProjector as RuntimeGraphProjector;
use cairn_graph::in_memory::InMemoryGraphStore;

use cairn_memory::api_impl::MemoryApiImpl;
use cairn_memory::deep_search_impl::{IterativeDeepSearch, KeywordDecomposer};
use cairn_memory::diagnostics::DiagnosticsService;
use cairn_memory::diagnostics_impl::InMemoryDiagnostics;
use cairn_memory::export_service_impl::InMemoryExportService;
use cairn_memory::feed_impl::FeedStore;
use cairn_memory::graph_expansion::GraphBackedExpansion;
use cairn_memory::import_service_impl::InMemoryImportService;
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::ingest::SourceType;
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};

use cairn_runtime::startup::ReadinessState;
use cairn_runtime::{
    InMemoryServices, LicenseService, MarketplaceService, ModelRegistry, ProjectService,
    TenantService, TriggerService, WorkspaceService,
};

use cairn_tools::{execute_eval_score, InMemoryPluginRegistry, StdioPluginHost};

// ── crate-internal ───────────────────────────────────────────────────────────

use crate::metrics::AppMetrics;
use crate::tokens::{OperatorTokenStore, RequestLogBuffer};
/// Default tenant ID used when no scope is supplied.
///
/// Exported as `pub` so the binary crate (`main.rs` + `bin_*` modules)
/// can reuse the same literal without redefining it. Pre-#185 these
/// lived as `pub(crate)` and the binary crate carried a duplicate
/// `const` to avoid reaching into the lib's private namespace.
pub const DEFAULT_TENANT_ID: &str = "default_tenant";
pub const DEFAULT_WORKSPACE_ID: &str = "default_workspace";
pub const DEFAULT_PROJECT_ID: &str = "default_project";

// ── Type aliases ─────────────────────────────────────────────────────────────

pub(crate) type AppDeepSearch = IterativeDeepSearch<
    InMemoryRetrieval,
    KeywordDecomposer,
    GraphBackedExpansion<Arc<InMemoryGraphStore>>,
>;

pub(crate) type AppIngestPipeline = IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>;

// Constants are defined in lib.rs and re-exported via crate::DEFAULT_*

// ── Adapter: MemoryDiagnosticsSource ─────────────────────────────────────────

/// Adapts `InMemoryDiagnostics` to `cairn_evals::MemoryDiagnosticsSource`, breaking the
/// circular dependency by not requiring `cairn-evals` to depend on `cairn-memory`.
pub(crate) struct DiagnosticsAdapter(pub(crate) Arc<InMemoryDiagnostics>);

#[async_trait]
impl MemoryDiagnosticsSource for DiagnosticsAdapter {
    async fn list_source_quality(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
    ) -> Result<Vec<SourceQualitySnapshot>, String> {
        let records = DiagnosticsService::list_source_quality(self.0.as_ref(), project, limit)
            .await
            .map_err(|e| e.to_string())?;
        Ok(records
            .into_iter()
            .map(|r| SourceQualitySnapshot {
                source_id: r.source_id.clone(),
                total_chunks: r.total_chunks,
                credibility_score: Some(r.credibility_score),
                retrieval_count: r.retrieval_count,
                query_hit_rate: r.query_hit_rate,
                error_rate: r.error_rate,
                last_ingested_at: Some(r.last_ingested_at),
            })
            .collect())
    }
}

// ── Adapter: PluginRubricScorer ──────────────────────────────────────────────

pub(crate) struct AppPluginRubricScorer {
    pub(crate) plugin_registry: Arc<InMemoryPluginRegistry>,
}

#[async_trait]
impl PluginRubricScorer for AppPluginRubricScorer {
    async fn score(
        &self,
        plugin_id: &str,
        input: &serde_json::Value,
        expected_output: Option<&serde_json::Value>,
        actual_output: &serde_json::Value,
    ) -> Result<PluginDimensionScore, cairn_evals::services::rubric_impl::EvalRubricError> {
        let result = execute_eval_score(
            self.plugin_registry.as_ref(),
            plugin_id,
            input.clone(),
            expected_output.cloned(),
            actual_output.clone(),
        )
        .await
        .map_err(|err| {
            cairn_evals::services::rubric_impl::EvalRubricError::PluginScoreFailed(err.to_string())
        })?;
        Ok(PluginDimensionScore {
            score: result.score,
            passed: result.passed,
            feedback: result.reasoning,
        })
    }
}

// ── Binding / view structs ───────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub(crate) struct SqEqSessionBinding {
    pub(crate) project: ProjectKey,
}

#[derive(Clone, Debug)]
pub(crate) struct A2aTaskBinding {
    pub(crate) task_id: TaskId,
    pub(crate) project: ProjectKey,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MailboxMessageView {
    pub(crate) message_id: String,
    pub(crate) run_id: Option<String>,
    pub(crate) task_id: Option<String>,
    pub(crate) sender_id: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) delivered: bool,
    pub(crate) created_at: u64,
}

#[derive(Clone, Debug)]
pub struct AppMailboxMessage {
    pub(crate) sender_id: Option<String>,
    pub(crate) body: Option<String>,
    pub(crate) delivered: bool,
}

#[derive(Clone, Debug, Default)]
pub struct AppSourceMetadata {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
}

/// Cached prompt version content and template vars (not in event payload).
#[derive(Clone, Debug, Default)]
pub struct AppVersionContent {
    pub(crate) content: String,
    pub(crate) template_vars: Vec<PromptTemplateVar>,
}

#[derive(Clone, Debug)]
pub struct PendingIngestJobPayload {
    pub(crate) project: ProjectKey,
    pub(crate) source_id: SourceId,
    pub(crate) document_id: KnowledgeDocumentId,
    pub(crate) content: String,
    pub(crate) source_type: SourceType,
}

#[derive(Clone, Copy, Debug)]
pub struct RateLimitBucket {
    pub(crate) count: u32,
    pub(crate) window_started_ms: u64,
}

// ── AppState ─────────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct AppState {
    pub config: BootstrapConfig,
    pub runtime: Arc<InMemoryServices>,
    pub evals: Arc<ProductEvalRunService>,
    pub eval_baselines: Arc<EvalBaselineServiceImpl>,
    pub eval_datasets: Arc<EvalDatasetServiceImpl>,
    #[allow(dead_code)]
    pub model_comparisons: Arc<ModelComparisonServiceImpl>,
    pub eval_rubrics: Arc<EvalRubricServiceImpl>,
    pub runtime_sse_tx: broadcast::Sender<SseFrame>,
    /// Ring buffer of the last 10,000 SSE frames with monotonic sequence IDs.
    /// Clients use Last-Event-ID to replay missed events after reconnect (RFC 002).
    pub sse_event_buffer: Arc<std::sync::RwLock<VecDeque<(u64, SseFrame)>>>,
    /// Monotonic counter for SSE frame sequence IDs.
    pub sse_seq: Arc<std::sync::atomic::AtomicU64>,
    pub graph: Arc<InMemoryGraphStore>,
    pub document_store: Arc<InMemoryDocumentStore>,
    pub retrieval: Arc<InMemoryRetrieval>,
    pub deep_search: Arc<AppDeepSearch>,
    pub ingest: Arc<AppIngestPipeline>,
    pub diagnostics: Arc<InMemoryDiagnostics>,
    pub feed: Arc<FeedStore>,
    pub bundle_import: Arc<InMemoryImportService>,
    pub bundle_export: Arc<InMemoryExportService>,
    pub source_metadata: Arc<Mutex<HashMap<String, AppSourceMetadata>>>,
    /// Cache of prompt version content + template vars, keyed by version_id.
    pub version_content: Arc<Mutex<HashMap<String, AppVersionContent>>>,
    pub pending_ingest_jobs: Arc<Mutex<HashMap<String, PendingIngestJobPayload>>>,
    pub mailbox_messages: Arc<Mutex<HashMap<String, AppMailboxMessage>>>,
    pub templates: Arc<StarterTemplateRegistry>,
    pub service_tokens: Arc<ServiceTokenRegistry>,
    /// Per-operator API token metadata store (token_id -> record + raw token).
    /// Separate from `service_tokens` which only holds the auth lookup map.
    pub operator_tokens: Arc<OperatorTokenStore>,
    pub plugin_registry: Arc<InMemoryPluginRegistry>,
    pub plugin_host: Arc<Mutex<StdioPluginHost>>,
    /// RFC 015: plugin marketplace service -- manages discover/install/enable lifecycle.
    pub marketplace: Arc<Mutex<MarketplaceService<cairn_store::InMemoryStore>>>,
    /// RFC 022: trigger service -- manages triggers and run templates.
    pub triggers: Arc<Mutex<TriggerService>>,
    pub repo_clone_cache: Arc<cairn_workspace::RepoCloneCache>,
    pub project_repo_access: Arc<cairn_workspace::ProjectRepoAccessService>,
    /// Per-project set of local-filesystem paths attached via
    /// `host=local_fs` on `POST /v1/projects/:project/repos`. Parallel to
    /// `project_repo_access` (which enforces the RFC 016 `owner/repo`
    /// shape); this map stores arbitrary directory paths so operators
    /// can point cairn at a local checkout as a pseudo-repo.
    pub project_local_paths: Arc<crate::repo_routes::ProjectLocalPaths>,
    pub sandbox_service: Arc<cairn_workspace::SandboxService>,
    pub(crate) sqeq_sessions: Arc<Mutex<HashMap<String, SqEqSessionBinding>>>,
    pub(crate) a2a_tasks: Arc<Mutex<HashMap<String, A2aTaskBinding>>>,
    pub rate_limits: Arc<Mutex<HashMap<String, RateLimitBucket>>>,
    /// T6b-C6: timestamp of the most recent eviction sweep over
    /// `rate_limits`. Used to amortize the O(N) retain pass so an
    /// attacker keeping the bucket map at the threshold can't force
    /// a sweep on every request.
    pub rate_limit_last_sweep_ms: Arc<std::sync::atomic::AtomicU64>,
    pub metrics: Arc<AppMetrics>,
    pub memory_api: Arc<MemoryApiImpl<InMemoryRetrieval>>,
    #[allow(dead_code)]
    pub memory_proposal_hook: Arc<crate::sse_hooks::SseMemoryProposalHook>,
    pub started_at: Instant,
    /// OTLP span exporter (RFC 021). Disabled by default.
    pub otlp_exporter: Arc<cairn_runtime::telemetry::OtlpExporter>,
    /// Brain LLM provider for orchestration -- set post-construction by main.rs
    /// once the concrete provider (Ollama or OpenAI-compat) is configured.
    /// `None` means orchestration is unavailable until a provider is configured.
    pub brain_provider: Option<Arc<dyn cairn_domain::providers::GenerationProvider>>,
    /// Bedrock provider -- used when the model_id is a Bedrock model (e.g. minimax.minimax-m2.5).
    pub bedrock_provider: Option<Arc<dyn cairn_domain::providers::GenerationProvider>>,
    /// Built-in tool registry wired by main.rs with real memory backends.
    /// `None` until set -- orchestrate handler falls back to stub dispatcher.
    pub tool_registry: Option<Arc<cairn_tools::BuiltinToolRegistry>>,
    /// Ring buffer of the last 2,000 structured request log entries, populated
    /// by the observability middleware.  Consumed by `GET /v1/admin/logs`.
    pub request_log: Arc<std::sync::RwLock<RequestLogBuffer>>,
    /// GitHub App integration -- set by main.rs when GITHUB_APP_ID + private key are configured.
    ///
    /// DEPRECATED: the canonical registration lives in `self.integrations` (the
    /// `IntegrationRegistry`).  This field is kept ONLY because the legacy webhook,
    /// queue, scan, and installation handlers below access `GitHubIntegration`
    /// fields directly (credentials, installations, issue_queue, etc.) and the
    /// `Integration` trait does not yet expose them.
    ///
    /// TODO(integration-migration): add `as_any()` to the `Integration` trait (or
    /// surface the needed fields through trait methods), migrate the handlers to
    /// look up GitHub via `state.integrations.get("github")`, then delete this
    /// field and the `GitHubIntegration` struct.
    pub github: Option<Arc<GitHubIntegration>>,
    /// Integration plugin registry -- holds all configured integrations (GitHub, Linear, etc.).
    pub integrations: Arc<cairn_integrations::IntegrationRegistry>,
    /// Model catalog — per-model metadata including cost rates and capabilities.
    /// Operators can override entries at runtime via the admin API.
    pub model_registry: ModelRegistry,
    /// FlowFabric services aggregate — `Some` in production (when
    /// `build_runtime_with_optional_fabric` successfully boots
    /// `FabricServices`). `None` only when a test fixture has injected a
    /// read-only runtime (see `crates/cairn-app/tests/support/fake_fabric.rs`).
    /// When `Some`, `runtime.runs / tasks / sessions` are the Fabric
    /// adapters (see `crate::fabric_adapter`); handlers call through the
    /// trait unchanged.
    ///
    /// Rare direct-access handlers (e.g. admin inspect endpoints) may reach
    /// through this field to `FabricServices::budgets`, `quotas`,
    /// `scheduler`, `signals` which aren't on the core trait surface.
    pub fabric: Option<Arc<cairn_fabric::FabricServices>>,
    /// RFC 020 §"Startup order": shared readiness state that the startup
    /// sequence mutates and the `/health/ready` handler + readiness
    /// middleware read. Starts with all branches `Pending`; flipped to
    /// `ready` once the startup graph completes (see `main.rs`).
    pub readiness: ReadinessState,
    /// RFC 020 Track 3: shared tool-call result cache, consulted by
    /// `RuntimeExecutePhase` before every tool dispatch. Populated at
    /// startup from the event log (every prior `ToolInvocationCompleted`)
    /// and incrementally on each completion thereafter. Wrapped in
    /// `Arc<Mutex<_>>` because both the orchestrator (hot path on each
    /// tool call) and the startup replay share ownership.
    pub tool_result_cache: Arc<std::sync::Mutex<cairn_runtime::startup::ToolCallResultCache>>,
    /// Skills catalog (GAP-012 / issue #147): in-process registry of
    /// capability bundles. Read by `GET /v1/skills` and `/v1/skills/:id`;
    /// populated by workers/operators via the skills registration API.
    /// Starts empty — UI shows an empty-state prompt until a worker
    /// registers a skill.
    pub skill_catalog: Arc<tokio::sync::RwLock<cairn_domain::skills::SkillCatalog>>,
    /// Cached providers-with-counts summary for `GET /v1/models/catalog/providers`.
    /// Populated on first request; immutable thereafter for the process
    /// lifetime. Runtime overrides via the admin CRUD API do NOT invalidate
    /// this cache — see module doc on `handlers::model_catalog`.
    pub model_catalog_providers_cache:
        Arc<std::sync::OnceLock<Vec<crate::handlers::model_catalog::ProviderCount>>>,
    /// Background task that derives lifecycle metrics from the event
    /// log broadcast. Kept on `AppState` so its lifetime tracks the
    /// process; drop/cancel is managed by shutdown paths.
    #[cfg(any(feature = "metrics-core", feature = "metrics-providers"))]
    pub metrics_tap: Option<crate::metrics_tap::MetricsTap>,
    /// Background task exporting RuntimeEvents as OTLP spans over
    /// HTTP/protobuf. Enabled when the `metrics-otel` feature is on
    /// AND `CAIRN_OTLP_ENABLED` is truthy at boot. `None` otherwise.
    #[cfg(feature = "metrics-otel")]
    pub otlp_tap: Option<crate::metrics_otel::OtlpTap>,
    /// Handle to the batching sink wrapping the OTLP transport.
    /// Kept separately so `shutdown_telemetry()` can flush any
    /// buffered spans before process exit. `None` when the OTLP
    /// export is not wired.
    #[cfg(feature = "metrics-otel")]
    pub otlp_batch: Option<Arc<crate::metrics_otel::BatchingSink>>,
    /// Provider-fallback cooldown storage partitioned by
    /// `(tenant_id, binding_id)` so a rate-limit on one tenant/connection
    /// does NOT cool down the same `model_id` for unrelated tenants or
    /// sibling connections with different credentials. Keyed internally
    /// by `model_id` inside each scoped `CooldownMap`. Populated when
    /// `provider.generate()` returns `ProviderAdapterError::RateLimited`
    /// so subsequent orchestrate calls skip the cooled-down model for
    /// `DEFAULT_RATE_LIMIT_COOLDOWN` (5 min). Cleared on process
    /// restart by design (in-memory; event-sourced cooldown is a
    /// follow-up).
    pub provider_fallback_cooldown: Arc<ScopedProviderFallbackCooldown>,
}

/// In-memory provider-fallback cooldown storage partitioned by
/// `(tenant_id, binding_id)`. Each partition holds its own
/// [`cairn_orchestrator::CooldownMap`] so rate-limit events are isolated.
///
/// See [`AppState::provider_fallback_cooldown`] for semantics.
#[derive(Debug, Default)]
pub struct ScopedProviderFallbackCooldown {
    inner: std::sync::Mutex<
        std::collections::HashMap<(String, String), cairn_orchestrator::CooldownMap>,
    >,
}

impl ScopedProviderFallbackCooldown {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fetch (or lazily create) the `CooldownMap` for the given
    /// `(tenant_id, binding_id)` scope.
    ///
    /// Prunes scopes whose inner `CooldownMap` is empty before returning —
    /// this keeps the outer map bounded under tenant/connection churn
    /// since `CooldownMap` entries already self-expire after their window.
    /// Without this sweep the outer map could grow unboundedly even
    /// though every inner entry had long since expired.
    pub fn get_or_create(
        &self,
        tenant_id: &str,
        binding_id: &str,
    ) -> cairn_orchestrator::CooldownMap {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());

        // Light amortised sweep: drop scopes whose CooldownMap is empty.
        // Cheap because `CooldownMap::is_empty` is a single mutex
        // acquisition + HashMap::retain. At steady state the outer map
        // only keeps scopes that are currently cooling down something,
        // so size is bounded by the number of simultaneously-throttled
        // (tenant, binding) pairs.
        guard.retain(|_, cooldown| !cooldown.is_empty());

        guard
            .entry((tenant_id.to_owned(), binding_id.to_owned()))
            .or_default()
            .clone()
    }

    /// Number of tracked scopes (after pruning). Primarily for
    /// observability / tests.
    pub fn scope_count(&self) -> usize {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|_, cooldown| !cooldown.is_empty());
        guard.len()
    }
}

// ── GitHubIntegration ────────────────────────────────────────────────────────

/// Parse a `tenant/workspace/project` env value into a `ProjectKey`.
/// Returns `None` when unset or malformed (missing parts, empty segments).
fn parse_triple_env(env_var: &str) -> Option<cairn_domain::ProjectKey> {
    let raw = std::env::var(env_var).ok()?;
    let parts: Vec<&str> = raw.split('/').collect();
    if parts.len() != 3 || parts.iter().any(|p| p.trim().is_empty()) {
        return None;
    }
    Some(cairn_domain::ProjectKey::new(
        parts[0].trim(),
        parts[1].trim(),
        parts[2].trim(),
    ))
}

/// T6a-C5: fallback project for unmapped GitHub installations, read from
/// `CAIRN_GITHUB_DEFAULT_PROJECT` in `tenant/workspace/project` form.
/// Returns `None` when unset — callers MUST reject the webhook in that
/// case rather than fall through to the old `default_tenant` triple.
pub(crate) fn default_github_project_from_env() -> Option<cairn_domain::ProjectKey> {
    parse_triple_env("CAIRN_GITHUB_DEFAULT_PROJECT")
}

/// GitHub App integration state.
pub struct GitHubIntegration {
    pub credentials: cairn_github::AppCredentials,
    pub webhook_secret: String,
    /// Map of installation_id -> InstallationToken (auto-refreshing).
    pub installations:
        tokio::sync::RwLock<std::collections::HashMap<u64, cairn_github::InstallationToken>>,
    /// Operator-configured event->action mappings.
    pub event_actions: tokio::sync::RwLock<Vec<GitHubEventAction>>,
    /// Issue processing queue.
    pub issue_queue: tokio::sync::RwLock<VecDeque<IssueQueueEntry>>,
    /// Whether the queue dispatcher is paused.
    pub queue_paused: AtomicBool,
    /// Whether the queue dispatcher loop is running.
    pub queue_running: AtomicBool,
    /// Max concurrent orchestration runs (operator-configurable).
    pub max_concurrent: AtomicU32,
    /// Semaphore controlling concurrent run slots.
    pub run_semaphore: Arc<tokio::sync::Semaphore>,
    pub http: reqwest::Client,
}

impl std::fmt::Debug for GitHubIntegration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitHubIntegration")
            .field("app_id", &self.credentials.app_id)
            .finish()
    }
}

impl GitHubIntegration {
    /// T6a-C5: resolve the ProjectKey for a GitHub App installation.
    ///
    /// Today this reads the per-installation env var
    /// `CAIRN_GITHUB_INSTALLATION_<id>_PROJECT` in the canonical
    /// `tenant/workspace/project` form. When no env exists, callers fall
    /// back to `default_github_project_from_env()` (or reject entirely).
    ///
    /// A future iteration will move this mapping into the event log via
    /// a dedicated `GitHubInstallationMapping` projection; this env
    /// shim is a placeholder so webhooks stop commingling tenants.
    pub async fn project_for_installation(
        &self,
        installation_id: u64,
    ) -> Option<cairn_domain::ProjectKey> {
        let key = format!("CAIRN_GITHUB_INSTALLATION_{installation_id}_PROJECT");
        parse_triple_env(&key)
    }

    /// Get or create an InstallationToken for the given installation ID.
    pub async fn token_for_installation(
        &self,
        installation_id: u64,
    ) -> cairn_github::InstallationToken {
        {
            let cache = self.installations.read().await;
            if let Some(token) = cache.get(&installation_id) {
                return token.clone();
            }
        }
        let token = cairn_github::InstallationToken::new(
            self.credentials.clone(),
            installation_id,
            self.http.clone(),
        );
        let mut cache = self.installations.write().await;
        cache.insert(installation_id, token.clone());
        token
    }

    /// Get a GitHubClient for the given installation.
    pub async fn client_for_installation(
        &self,
        installation_id: u64,
    ) -> cairn_github::GitHubClient {
        let token = self.token_for_installation(installation_id).await;
        cairn_github::GitHubClient::with_http(token, self.http.clone())
    }
}

// ── GitHubEventAction / WebhookAction ────────────────────────────────────────

/// Configurable event->action mapping for GitHub webhooks.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct GitHubEventAction {
    /// Event key pattern to match (e.g., "issues.opened", "issues.labeled", "push").
    /// Supports "*" as wildcard (e.g., "issues.*" matches all issue events).
    pub event_pattern: String,
    /// Optional label filter -- only trigger if the issue/PR has this label.
    #[serde(default)]
    pub label_filter: Option<String>,
    /// Optional repo filter -- only trigger for this repo (owner/repo).
    #[serde(default)]
    pub repo_filter: Option<String>,
    /// What to do when the event matches.
    pub action: WebhookAction,
}

/// What to do when a webhook event matches a configured pattern.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebhookAction {
    /// Create a session + run and trigger orchestration.
    /// The goal is derived from the issue/PR title + body.
    CreateAndOrchestrate,
    /// Post a comment acknowledging the event.
    Acknowledge,
    /// Ignore the event (useful for explicit deny rules).
    Ignore,
}

// ── IssueQueueEntry / IssueQueueStatus ───────────────────────────────────────

#[derive(Clone, Debug)]
pub struct IssueQueueEntry {
    pub repo: String,
    pub installation_id: u64,
    pub issue_number: u64,
    pub title: String,
    pub session_id: String,
    pub run_id: String,
    pub status: IssueQueueStatus,
}

#[derive(Clone, Debug, PartialEq)]
pub enum IssueQueueStatus {
    Pending,
    Processing,
    WaitingApproval,
    Completed,
    Failed(String),
}

// ── AppState impl ────────────────────────────────────────────────────────────

impl AppState {
    /// Replay all events from the store into the graph projector.
    ///
    /// Call this after any external seeding (e.g. demo data) that writes
    /// to the runtime store outside of the normal API write path.  The
    /// graph is otherwise populated lazily -- only when API handlers call
    /// `publish_runtime_frames_since` -- so startup seeding leaves it empty
    /// until this is called.
    pub async fn replay_graph(&self) {
        use cairn_store::event_log::EventLog;
        match self.runtime.store.read_stream(None, usize::MAX).await {
            Ok(events) => {
                let projector = RuntimeGraphProjector::new(self.graph.clone());
                if let Err(e) = projector.project_events(&events).await {
                    tracing::warn!("graph replay: projection error: {e:?}");
                }
            }
            Err(e) => tracing::warn!("graph replay: failed to read events: {e}"),
        }
    }

    /// Replay `EvalRunStarted` / `EvalRunCompleted` events from the event log
    /// into the in-memory eval service.
    ///
    /// `state.evals` is a standalone in-memory service -- it does NOT read from
    /// the event log on its own.  API handlers that create eval runs now write
    /// an `EvalRunStarted` event alongside their in-memory insert; this method
    /// reconstructs that state on boot so eval runs survive restarts.
    ///
    /// Note: metrics recorded via `/v1/evals/runs/:id/score` are NOT yet in the
    /// event log, so they will not be visible after a restart.
    pub async fn replay_evals(&self) {
        use cairn_store::event_log::EventLog;
        let events = match self.runtime.store.read_stream(None, usize::MAX).await {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("eval replay: failed to read events: {e}");
                return;
            }
        };

        let mut created: u32 = 0;
        let mut completed: u32 = 0;

        for stored in &events {
            match &stored.envelope.payload {
                cairn_domain::RuntimeEvent::EvalRunStarted(e) => {
                    // Skip if already present (could have been created before replay).
                    if self.evals.get(&e.eval_run_id).is_some() {
                        continue;
                    }
                    // Reconstruct the EvalRun with what the event carries.
                    // Metrics are not in the event -- they default to empty.
                    let subject_kind: EvalSubjectKind =
                        serde_json::from_str(&format!("\"{}\"", e.subject_kind))
                            .unwrap_or(EvalSubjectKind::PromptRelease);

                    self.evals.create_run(
                        e.eval_run_id.clone(),
                        ProjectId::new(e.project.project_id.as_str()),
                        subject_kind,
                        e.evaluator_type.clone(),
                        e.prompt_asset_id.clone(),
                        e.prompt_version_id.clone(),
                        e.prompt_release_id.clone(),
                        e.created_by.clone(),
                    );
                    // Issue #220 (dataset) + #223 (rubric + baseline): restore
                    // bindings that were previously only held in memory.
                    // Defaults to None for pre-#220/#223 events
                    // (serde default on `EvalRunStarted`).
                    if let Some(dataset_id) = e.dataset_id.as_ref() {
                        if let Err(err) = self
                            .evals
                            .set_dataset_id(&e.eval_run_id, dataset_id.clone())
                        {
                            tracing::warn!(
                                eval_run_id = %e.eval_run_id,
                                dataset_id = %dataset_id,
                                "eval replay: set_dataset_id failed: {err}",
                            );
                        }
                    }
                    if let Some(rubric_id) = e.rubric_id.as_ref() {
                        if let Err(err) =
                            self.evals.set_rubric_id(&e.eval_run_id, rubric_id.clone())
                        {
                            tracing::warn!(
                                eval_run_id = %e.eval_run_id,
                                rubric_id = %rubric_id,
                                "eval replay: set_rubric_id failed: {err}",
                            );
                        }
                    }
                    if let Some(baseline_id) = e.baseline_id.as_ref() {
                        if let Err(err) = self
                            .evals
                            .set_baseline_id(&e.eval_run_id, baseline_id.clone())
                        {
                            tracing::warn!(
                                eval_run_id = %e.eval_run_id,
                                baseline_id = %baseline_id,
                                "eval replay: set_baseline_id failed: {err}",
                            );
                        }
                    }
                    created += 1;
                }
                cairn_domain::RuntimeEvent::EvalRunCompleted(e) => {
                    // Transition to completed state if the run exists.
                    // Best-effort: ignore if run not found (could be from a different
                    // code path that didn't write EvalRunStarted).
                    if let Some(run) = self.evals.get(&e.eval_run_id) {
                        if run.status == EvalRunStatus::Running {
                            let _ = self.evals.complete_run(
                                &e.eval_run_id,
                                Default::default(), // metrics not in event
                                None,
                            );
                            completed += 1;
                        }
                    }
                }
                _ => {}
            }
        }

        if created > 0 || completed > 0 {
            tracing::info!("eval replay: restored {created} runs ({completed} completed)");
        }
    }

    /// Replay trigger/template lifecycle and fire outcomes into the in-memory trigger service.
    pub async fn replay_triggers(&self) {
        use cairn_store::event_log::EventLog;

        let events = match self.runtime.store.read_stream(None, usize::MAX).await {
            Ok(events) => events,
            Err(error) => {
                tracing::warn!("trigger replay: failed to read events: {error}");
                return;
            }
        };

        let mut triggers = self.triggers.lock().unwrap_or_else(|e| e.into_inner());
        *triggers = TriggerService::new();

        let mut restored_templates = 0u32;
        let mut restored_triggers = 0u32;
        let mut restored_fires = 0u32;

        for stored in &events {
            match &stored.envelope.payload {
                RuntimeEvent::RunTemplateCreated(event) => {
                    triggers.create_template(cairn_runtime::RunTemplate {
                        id: event.template_id.clone(),
                        project: event.project.clone(),
                        name: event.name.clone(),
                        description: event.description.clone(),
                        default_mode: event.default_mode.clone(),
                        system_prompt: event.system_prompt.clone(),
                        initial_user_message: event.initial_user_message.clone(),
                        plugin_allowlist: event.plugin_allowlist.clone(),
                        tool_allowlist: event.tool_allowlist.clone(),
                        budget: cairn_runtime::TemplateBudget {
                            max_tokens: event.budget_max_tokens,
                            max_wall_clock_ms: event.budget_max_wall_clock_ms,
                            max_iterations: event.budget_max_iterations,
                            exploration_budget_share: event.budget_exploration_budget_share,
                        },
                        sandbox_hint: event.sandbox_hint.clone(),
                        required_fields: event.required_fields.clone(),
                        created_by: event.created_by.clone(),
                        created_at: event.created_at,
                        updated_at: event.created_at,
                    });
                    restored_templates += 1;
                }
                RuntimeEvent::RunTemplateDeleted(event) => {
                    let _ = triggers.delete_template(&event.template_id, event.by.clone());
                }
                RuntimeEvent::TriggerCreated(event) => {
                    let conditions = match crate::trigger_conditions_from_values(&event.conditions)
                    {
                        Ok(conditions) => conditions,
                        Err(error) => {
                            tracing::warn!(
                                "trigger replay: failed to decode conditions for {}: {error}",
                                event.trigger_id
                            );
                            continue;
                        }
                    };
                    match triggers.create_trigger(cairn_runtime::Trigger {
                        id: event.trigger_id.clone(),
                        project: event.project.clone(),
                        name: event.name.clone(),
                        description: event.description.clone(),
                        signal_pattern: cairn_runtime::SignalPattern {
                            signal_type: event.signal_type.clone(),
                            plugin_id: event.plugin_id.clone(),
                        },
                        conditions,
                        run_template_id: event.run_template_id.clone(),
                        state: cairn_runtime::TriggerState::Enabled,
                        rate_limit: cairn_runtime::RateLimitConfig {
                            max_per_minute: event.max_per_minute,
                            max_burst: event.max_burst,
                        },
                        max_chain_depth: event.max_chain_depth,
                        created_by: event.created_by.clone(),
                        created_at: event.created_at,
                        updated_at: event.created_at,
                    }) {
                        Ok(_) => restored_triggers += 1,
                        Err(error) => tracing::warn!(
                            "trigger replay: failed to restore trigger {}: {error}",
                            event.trigger_id
                        ),
                    }
                }
                RuntimeEvent::TriggerEnabled(event) => {
                    let _ = triggers.restore_trigger_state(
                        &event.trigger_id,
                        cairn_runtime::TriggerState::Enabled,
                        event.at,
                    );
                }
                RuntimeEvent::TriggerDisabled(event) => {
                    let _ = triggers.restore_trigger_state(
                        &event.trigger_id,
                        cairn_runtime::TriggerState::Disabled {
                            reason: event.reason.clone(),
                            since: event.at,
                        },
                        event.at,
                    );
                }
                RuntimeEvent::TriggerSuspended(event) => {
                    let _ = triggers.restore_trigger_state(
                        &event.trigger_id,
                        cairn_runtime::TriggerState::Suspended {
                            reason: crate::runtime_trigger_suspension_reason(&event.reason),
                            since: event.at,
                        },
                        event.at,
                    );
                }
                RuntimeEvent::TriggerResumed(event) => {
                    let _ = triggers.restore_trigger_state(
                        &event.trigger_id,
                        cairn_runtime::TriggerState::Enabled,
                        event.at,
                    );
                }
                RuntimeEvent::TriggerDeleted(event) => {
                    let _ = triggers.delete_trigger(&event.trigger_id, event.by.clone());
                }
                RuntimeEvent::TriggerFired(event) => {
                    triggers.restore_fired_trigger(
                        &event.project,
                        &event.trigger_id,
                        &event.signal_id,
                        event.fired_at,
                    );
                    restored_fires += 1;
                }
                RuntimeEvent::TriggerSkipped(_)
                | RuntimeEvent::TriggerDenied(_)
                | RuntimeEvent::TriggerRateLimited(_)
                | RuntimeEvent::TriggerPendingApproval(_) => {}
                _ => {}
            }
        }

        if restored_templates > 0 || restored_triggers > 0 || restored_fires > 0 {
            tracing::info!(
                "trigger replay: restored {restored_templates} templates, {restored_triggers} triggers, {restored_fires} fires"
            );
        }
    }

    pub async fn new(config: BootstrapConfig) -> Result<Self, String> {
        // Construct FabricServices + install the FabricAdapter trio for
        // runs/tasks/sessions. Any boot failure on the Fabric path
        // (unreachable Valkey, HMAC validation, …) surfaces here before
        // cairn-app starts serving traffic — no silent fall-back.
        let (runtime, fabric) = build_runtime_with_optional_fabric().await?;
        Self::new_with_runtime(config, runtime, fabric).await
    }

    /// Build an `AppState` around a caller-provided runtime.
    ///
    /// Used by integration tests that need to inject a test fixture
    /// (e.g. `FakeFabric` under `tests/support/`) in place of live Fabric —
    /// the ancillary state (graph, eval services, metrics, tokens) wires up
    /// identically either way.
    ///
    /// Production callers use [`Self::new`], which builds the runtime via
    /// `build_runtime_with_optional_fabric` and then delegates here.
    pub async fn new_with_runtime(
        config: BootstrapConfig,
        runtime: Arc<InMemoryServices>,
        fabric: Option<Arc<cairn_fabric::FabricServices>>,
    ) -> Result<Self, String> {
        let graph = Arc::new(InMemoryGraphStore::new());
        let plugin_registry = Arc::new(InMemoryPluginRegistry::new());
        let document_store = Arc::new(InMemoryDocumentStore::new());
        let diagnostics = Arc::new(InMemoryDiagnostics::new());
        let evals = Arc::new(
            ProductEvalRunService::with_graph_and_event_log(
                Arc::new(EvalGraphIntegration::new(graph.clone())),
                runtime.store.clone(),
            )
            .with_memory_diagnostics(Arc::new(DiagnosticsAdapter(diagnostics.clone()))),
        );
        let eval_baselines = Arc::new(EvalBaselineServiceImpl::new(evals.clone()));
        let eval_datasets = Arc::new(EvalDatasetServiceImpl::new());
        let model_comparisons = Arc::new(ModelComparisonServiceImpl::new());
        let eval_rubrics = Arc::new(EvalRubricServiceImpl::with_plugin_scorer(
            evals.clone(),
            eval_datasets.clone(),
            Arc::new(AppPluginRubricScorer {
                plugin_registry: plugin_registry.clone(),
            }),
        ));
        let retrieval = Arc::new(
            InMemoryRetrieval::with_diagnostics(document_store.clone(), diagnostics.clone())
                .with_graph(graph.clone()),
        );
        let deep_search = Arc::new(
            IterativeDeepSearch::new(InMemoryRetrieval::new(document_store.clone()))
                .with_graph_hook(GraphBackedExpansion::new(graph.clone())),
        );
        let ingest = Arc::new(IngestPipeline::new(
            document_store.clone(),
            ParagraphChunker::default(),
        ));
        let feed = Arc::new(FeedStore::new());
        let bundle_import = Arc::new(InMemoryImportService::new(document_store.clone()));
        let bundle_export = Arc::new(InMemoryExportService::new(
            document_store.clone(),
            runtime.store.clone(),
            "cairn-app",
        ));
        let source_metadata = Arc::new(Mutex::new(HashMap::new()));
        let version_content: Arc<Mutex<HashMap<String, AppVersionContent>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let pending_ingest_jobs = Arc::new(Mutex::new(HashMap::new()));
        let mailbox_messages = Arc::new(Mutex::new(HashMap::new()));
        let service_tokens = Arc::new(ServiceTokenRegistry::new());
        let plugin_host = Arc::new(Mutex::new(StdioPluginHost::new()));
        let repo_clone_cache = Arc::new(cairn_workspace::RepoCloneCache::default());
        let project_repo_access = Arc::new(cairn_workspace::ProjectRepoAccessService::new());
        let project_local_paths = Arc::new(crate::repo_routes::ProjectLocalPaths::default());
        let sqeq_sessions = Arc::new(Mutex::new(HashMap::new()));
        let a2a_tasks = Arc::new(Mutex::new(HashMap::new()));
        let sandbox_repo_source = Arc::new(cairn_workspace::providers::RepoCloneCacheSource::new(
            repo_clone_cache.clone(),
        ));
        let sandbox_event_sink = Arc::new(crate::telemetry_routes::UsageSandboxEventSink::new(
            runtime.store.clone(),
            Arc::new(cairn_workspace::BufferedSandboxEventSink::default()),
        ));
        let sandbox_service = Arc::new(
            cairn_workspace::SandboxService::new(
                HashMap::from([
                    (
                        cairn_workspace::SandboxStrategy::Overlay,
                        Box::new(cairn_workspace::OverlayProvider::with_repo_source(
                            default_sandbox_base_dir(),
                            sandbox_repo_source.clone(),
                        )) as Box<dyn cairn_workspace::SandboxProvider>,
                    ),
                    (
                        cairn_workspace::SandboxStrategy::Reflink,
                        Box::new(cairn_workspace::ReflinkProvider::with_repo_source(
                            default_sandbox_base_dir(),
                            sandbox_repo_source,
                        )) as Box<dyn cairn_workspace::SandboxProvider>,
                    ),
                ]),
                sandbox_event_sink,
                default_sandbox_base_dir(),
                Arc::new(cairn_workspace::SystemClock),
            )
            // RFC 020 §"Run recovery matrix" — `AllowlistRevoked` row. Wire
            // the project-scoped repo allowlist so the recovery sweep can
            // detect repo bindings that are no longer authorised and emit
            // `SandboxAllowlistRevoked` for the run-level recovery service
            // to synthesize an operator approval against.
            .with_allowlist(project_repo_access.clone())
            // RFC 020 §"Run recovery matrix" — `BaseRevisionDrift` row.
            // Wire the repo clone cache so the recovery sweep can diff a
            // sandbox's stored `base_revision` against the live clone HEAD
            // and emit `SandboxBaseRevisionDrift` when the clone moved
            // between provisioning and recovery. Overlay-only; reflink
            // sandboxes are exempt per RFC 016 (physically independent).
            .with_clone_cache(repo_clone_cache.clone()),
        );
        // RFC 015: marketplace service wrapping the plugin host.
        let marketplace = {
            let mut svc = MarketplaceService::new(runtime.store.clone());
            svc.load_bundled_catalog();
            Arc::new(Mutex::new(svc))
        };
        let rate_limits = Arc::new(Mutex::new(HashMap::new()));
        let rate_limit_last_sweep_ms = Arc::new(std::sync::atomic::AtomicU64::new(0));
        let metrics = Arc::new(AppMetrics::default());
        let (runtime_sse_tx, _) = broadcast::channel(256);
        let sse_event_buffer = Arc::new(std::sync::RwLock::new(
            VecDeque::<(u64, SseFrame)>::with_capacity(10_000),
        ));
        let sse_seq = Arc::new(std::sync::atomic::AtomicU64::new(1));
        let memory_proposal_hook =
            Arc::new(crate::sse_hooks::SseMemoryProposalHook::with_sse_channel(
                runtime_sse_tx.clone(),
                sse_event_buffer.clone(),
                sse_seq.clone(),
            ));
        let memory_api = Arc::new(
            MemoryApiImpl::new(
                InMemoryRetrieval::with_diagnostics(document_store.clone(), diagnostics.clone())
                    .with_graph(graph.clone()),
                document_store.clone(),
            )
            .with_proposal_hook(Box::new(crate::sse_hooks::SharedMemoryProposalHook(
                memory_proposal_hook.clone(),
            ))),
        );

        runtime
            .tenants
            .create(
                TenantId::new(DEFAULT_TENANT_ID),
                "Default Tenant".to_owned(),
            )
            .await
            .map_err(|err| format!("failed to seed default tenant: {err}"))?;
        runtime
            .workspaces
            .create(
                TenantId::new(DEFAULT_TENANT_ID),
                WorkspaceId::new(DEFAULT_WORKSPACE_ID),
                "Default Workspace".to_owned(),
            )
            .await
            .map_err(|err| format!("failed to seed default workspace: {err}"))?;
        runtime
            .projects
            .create(
                ProjectKey::new(DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID, DEFAULT_PROJECT_ID),
                "Default Project".to_owned(),
            )
            .await
            .map_err(|err| format!("failed to seed default project: {err}"))?;
        runtime
            .licenses
            .activate(
                TenantId::new(DEFAULT_TENANT_ID),
                crate::deployment_mode_tier(config.mode),
                None,
            )
            .await
            .map_err(|err| format!("failed to seed default license: {err}"))?;

        #[allow(unused_mut)]
        let mut state = Self {
            config,
            document_store,
            retrieval,
            deep_search,
            ingest,
            diagnostics,
            feed,
            bundle_import,
            bundle_export,
            source_metadata,
            version_content,
            pending_ingest_jobs,
            mailbox_messages,
            templates: Arc::new(StarterTemplateRegistry::v1_defaults()),
            service_tokens,
            operator_tokens: Arc::new(OperatorTokenStore::new()),
            plugin_registry,
            plugin_host,
            marketplace,
            triggers: Arc::new(Mutex::new(TriggerService::new())),
            repo_clone_cache,
            project_repo_access,
            project_local_paths,
            sandbox_service,
            sqeq_sessions,
            a2a_tasks,
            rate_limits,
            rate_limit_last_sweep_ms,
            metrics,
            memory_api,
            memory_proposal_hook,
            started_at: Instant::now(),
            // Placeholder — swapped out below if metrics-otel is
            // enabled AND `CAIRN_OTLP_ENABLED` is set at boot.
            otlp_exporter: Arc::new(cairn_runtime::telemetry::OtlpExporter::disabled()),
            #[cfg(feature = "metrics-otel")]
            otlp_tap: None,
            #[cfg(feature = "metrics-otel")]
            otlp_batch: None,
            runtime_sse_tx,
            sse_event_buffer,
            sse_seq,
            runtime,
            evals,
            eval_baselines,
            eval_datasets,
            model_comparisons,
            eval_rubrics,
            graph,
            brain_provider: None,
            bedrock_provider: None,
            tool_registry: None,
            request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
            github: None,
            integrations: Arc::new(cairn_integrations::IntegrationRegistry::new()),
            model_registry: ModelRegistry::with_bundled()
                .unwrap_or_else(|_| ModelRegistry::empty()),
            fabric,
            readiness: ReadinessState::new(),
            tool_result_cache: Arc::new(std::sync::Mutex::new(
                cairn_runtime::startup::ToolCallResultCache::new(),
            )),
            skill_catalog: Arc::new(tokio::sync::RwLock::new(
                cairn_domain::skills::SkillCatalog::new(),
            )),
            model_catalog_providers_cache: Arc::new(std::sync::OnceLock::new()),
            #[cfg(any(feature = "metrics-core", feature = "metrics-providers"))]
            metrics_tap: None,
            provider_fallback_cooldown: Arc::new(ScopedProviderFallbackCooldown::new()),
        };
        state.runtime.store.reset_usage_counters();

        #[cfg(any(feature = "metrics-core", feature = "metrics-providers"))]
        {
            let tap = crate::metrics_tap::MetricsTap::spawn(
                state.runtime.store.clone(),
                state.metrics.clone(),
            );
            state.metrics_tap = Some(tap);
        }

        #[cfg(feature = "metrics-otel")]
        {
            use cairn_runtime::telemetry::OtlpExporter;
            let cfg = crate::metrics_otel::otlp_config_from_env();
            if cfg.enabled {
                // Only HTTP/protobuf is wired today. Warn loudly if
                // the operator asked for a transport we can't
                // deliver so the mismatch is visible in boot logs
                // instead of silently falling back.
                if !matches!(
                    cfg.protocol,
                    cairn_domain::protocols::OtlpProtocol::HttpBinary
                ) {
                    tracing::warn!(
                        requested = ?cfg.protocol,
                        fallback = "http/protobuf",
                        "CAIRN_OTLP_PROTOCOL requested an unsupported transport; \
                         falling back to http/protobuf. gRPC + http/json are \
                         tracked as follow-up work."
                    );
                }
                // Wrap HttpProtoSink in BatchingSink so bursty event
                // streams don't blast the collector with one POST per
                // event. 64 spans or 2 s, whichever fires first —
                // matches the defaults in RFC 021's exporter sketch.
                let transport = std::sync::Arc::new(crate::metrics_otel::HttpProtoSink::new(
                    &cfg.endpoint,
                    &cfg.service_name,
                ));
                let batched = std::sync::Arc::new(crate::metrics_otel::BatchingSink::new(
                    transport,
                    64,
                    std::time::Duration::from_millis(2_000),
                ));
                // The OtlpExporter holds the sink in a Box; wrap the
                // batcher in a shim that defers to the shared Arc so
                // AppState can retain the Arc for graceful
                // shutdown-flush while the exporter owns its Box.
                struct SinkArc(std::sync::Arc<dyn cairn_runtime::telemetry::SpanExportSink>);
                #[async_trait::async_trait]
                impl cairn_runtime::telemetry::SpanExportSink for SinkArc {
                    async fn export(
                        &self,
                        spans: &[cairn_runtime::telemetry::ExportableSpan],
                    ) -> Result<(), String> {
                        self.0.export(spans).await
                    }
                }
                let sink_for_exporter: std::sync::Arc<
                    dyn cairn_runtime::telemetry::SpanExportSink,
                > = batched.clone();
                let exporter = Arc::new(OtlpExporter::new(
                    cfg.clone(),
                    Box::new(SinkArc(sink_for_exporter)),
                ));
                state.otlp_exporter = exporter.clone();
                state.otlp_tap = Some(crate::metrics_otel::OtlpTap::spawn(
                    state.runtime.store.clone(),
                    exporter,
                ));
                // Retain the batcher so shutdown_telemetry() can
                // flush buffered spans before process exit.
                state.otlp_batch = Some(batched);
                tracing::info!(
                    endpoint = %cfg.endpoint,
                    redact_content = cfg.redact_content,
                    "OTLP export enabled"
                );
            }
        }

        Ok(state)
    }

    /// Graceful shutdown of background telemetry tasks. Call from
    /// the process-exit path so any buffered OTLP spans are flushed
    /// to the collector before the process ends; without this call
    /// the BatchingSink's timer task is cancelled asynchronously
    /// and the final batch is dropped.
    ///
    /// Idempotent — safe to call multiple times; second call is a
    /// no-op because both shutdown hooks `take()` their handles.
    pub async fn shutdown_telemetry(&self) {
        #[cfg(feature = "metrics-otel")]
        {
            // Stop the tap first so no new spans enter the
            // batcher's buffer after we've started draining.
            if let Some(tap) = &self.otlp_tap {
                tap.shutdown().await;
            }
            if let Some(batch) = &self.otlp_batch {
                batch.shutdown().await;
            }
        }
        #[cfg(any(feature = "metrics-core", feature = "metrics-providers"))]
        {
            if let Some(tap) = &self.metrics_tap {
                tap.shutdown().await;
            }
        }
    }
}

// ── Helpers (local copies of private lib.rs fns used by new()) ───────────────

/// Resolve the sandbox base directory.
///
/// Production default: `$TMPDIR/cairn-workspace-sandboxes`.
///
/// Honors the `CAIRN_SANDBOX_BASE_DIR` env var override when set. This
/// exists primarily for per-harness test isolation: the default
/// `/tmp/cairn-workspace-sandboxes` is a process-global directory, and
/// parallel integration tests that seed entries into its
/// `recovery_registry/` subdirectory can race one another during
/// `SandboxService::recover_all`'s drift sweep. Test harnesses should
/// set this env var to a unique per-harness path before spawning the
/// cairn-app subprocess. Production deployments leave it unset.
fn default_sandbox_base_dir() -> PathBuf {
    if let Ok(override_path) = std::env::var("CAIRN_SANDBOX_BASE_DIR") {
        if !override_path.is_empty() {
            return PathBuf::from(override_path);
        }
    }
    std::env::temp_dir().join("cairn-workspace-sandboxes")
}

/// Build the runtime aggregate.
///
/// Constructs `FabricServices` from env config, wires the
/// `FabricRunServiceAdapter` / `Task` / `Session` trio on top of a shared
/// `InMemoryStore`, and installs them via
/// `InMemoryServices::with_store_and_core`. Boot failure on the Fabric
/// path (unreachable Valkey, HMAC validation, …) surfaces here before
/// cairn-app serves traffic — no silent fall-back.
///
/// Integration tests that need to stand up an AppState without a live
/// Valkey build a `FakeFabric`-backed `InMemoryServices` (see
/// `crates/cairn-app/tests/support/fake_fabric.rs`) and call
/// [`AppBootstrap::router_with_injected_runtime`] directly, bypassing
/// this constructor.
async fn build_runtime_with_optional_fabric() -> Result<
    (
        Arc<InMemoryServices>,
        Option<Arc<cairn_fabric::FabricServices>>,
    ),
    String,
> {
    tracing::info!("constructing FabricServices (production runtime)");

    let fabric_config = cairn_fabric::FabricConfig::from_env()
        .map_err(|e| format!("FabricConfig::from_env failed: {e}"))?;

    // FabricServices::start needs a shared EventLog handle. We use the same
    // InMemoryStore that backs the runtime's projections so fabric's
    // EventBridge writes land on the same read model that cairn-app
    // handlers query.
    let store = Arc::new(cairn_store::InMemoryStore::new());
    let event_log: Arc<dyn cairn_store::event_log::EventLog + Send + Sync> = store.clone();

    // Wire the in-memory store under the cursor-store trait so the
    // lease-history subscriber can persist its XREAD cursors across
    // restarts against the same backing the rest of the projections use.
    let cursor_store: Arc<dyn cairn_store::projections::FfLeaseHistoryCursorStore> = store.clone();
    let fabric = cairn_fabric::FabricServices::start_with_lease_history(
        fabric_config,
        event_log,
        cursor_store,
    )
    .await
    .map_err(|e| format!("FabricServices::start failed: {e}"))?;
    let fabric = Arc::new(fabric);

    // One-shot cross-instance tag backfill. Gated on
    // `CAIRN_BACKFILL_INSTANCE_TAG=1` so a fresh deploy pays no cost.
    // Only applies when an operator does an in-place binary swap with
    // pre-existing `Running`/`WaitingApproval` executions that predate
    // the `cairn.instance_id` tag filter. Idempotent — running twice
    // is a no-op on the second pass because the HSET only fires on
    // hashes that lack the tag. Logs once at completion.
    if std::env::var("CAIRN_BACKFILL_INSTANCE_TAG").as_deref() == Ok("1") {
        match cairn_fabric::instance_tag_backfill::backfill_instance_tag(
            &fabric.runtime.client,
            fabric.runtime.config.worker_instance_id.as_str(),
        )
        .await
        {
            Ok(outcome) => {
                tracing::info!(
                    scanned = outcome.scanned,
                    tagged = outcome.tagged,
                    skipped_tagged = outcome.skipped_tagged,
                    skipped_foreign = outcome.skipped_foreign,
                    "backfilled cairn.instance_id on {} executions",
                    outcome.tagged,
                );
                eprintln!(
                    "backfilled cairn.instance_id on {} executions (scanned={} skipped_tagged={} skipped_foreign={})",
                    outcome.tagged,
                    outcome.scanned,
                    outcome.skipped_tagged,
                    outcome.skipped_foreign,
                );
            }
            Err(e) => {
                // Do not halt startup: the backfill is advisory. A
                // failed pass leaves existing foreign behavior intact
                // (lease expiries on untagged execs are dropped). An
                // operator can re-run by restarting with the env var
                // still set.
                tracing::error!(error = %e, "cairn.instance_id backfill failed");
                eprintln!("warning: cairn.instance_id backfill failed: {e}");
            }
        }
    }

    // Build the adapters that implement the cairn-runtime traits but
    // route mutations to Fabric. Each shares the same store for projection
    // reads (the resolvers look up project from bare ids).
    let runs: Arc<dyn cairn_runtime::runs::RunService> = Arc::new(
        crate::fabric_adapter::FabricRunServiceAdapter::new(fabric.clone(), store.clone()),
    );
    let tasks: Arc<dyn cairn_runtime::tasks::TaskService> = Arc::new(
        crate::fabric_adapter::FabricTaskServiceAdapter::new(fabric.clone(), store.clone()),
    );
    let sessions: Arc<dyn cairn_runtime::sessions::SessionService> = Arc::new(
        crate::fabric_adapter::FabricSessionServiceAdapter::new(fabric.clone(), store.clone()),
    );

    let mut services = InMemoryServices::with_store_and_core(store, runs, tasks, sessions);
    // Also expose the raw fabric via the type-erased slot on
    // InMemoryServices so non-trait surfaces (budgets, quotas, signals)
    // remain reachable from runtime-scoped code. Cast the Arc to Any here
    // because cairn-runtime does not name cairn-fabric types.
    services.fabric = Some(fabric.clone() as Arc<dyn std::any::Any + Send + Sync>);

    tracing::info!("fabric runtime installed; adapters active on runs/tasks/sessions");

    Ok((Arc::new(services), Some(fabric)))
}
