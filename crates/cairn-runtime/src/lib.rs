//! Durable runtime services for sessions, runs, tasks, and approvals.
//!
//! Recovery is split per RFC 020: FlowFabric's 14 background scanners own
//! **operational** recovery (lease expiry, attempt/execution/suspension
//! timeouts, pending waitpoint expiry, budget/quota/dependency reconcilers,
//! flow projection, index reconciliation, retention trimming, unblock
//! propagation, delayed promotion). `cairn-runtime::RecoveryService` owns
//! **run-level** recovery and runs once on cairn-app boot, after
//! `SandboxService::recover_all` and before the readiness-gate flip.
//!
//! `cairn-runtime` owns the runtime service boundaries that accept
//! commands, validate state transitions, persist events, and update
//! synchronous projections through `cairn-store`.

pub mod agent_roles;
pub mod aggregate;
pub mod approval_policies;
pub mod approvals;
pub mod audits;
pub mod bandit;
pub mod budgets;
pub mod channels;
pub mod checkpoints;
pub mod config_store;
pub mod context_builder;
pub mod credentials;
pub mod decisions;
pub mod defaults;
pub mod enrichment;
pub mod error;
pub mod eval_runs;
pub mod fleet;
pub mod guardian;
pub mod guardrails;
pub mod ingest_jobs;
pub mod licenses;
pub mod mailbox;
pub mod mailbox_delivery;
pub mod model_registry;
pub mod notification_prefs;
pub mod observability;
pub mod operator_profiles;
pub mod projects;
pub mod prompt_assets;
pub mod prompt_releases;
pub mod prompt_versions;
pub mod provider_bindings;
pub mod provider_connections;
pub mod provider_health;
pub mod provider_pools;
pub mod provider_registry;
pub mod quotas;
pub mod research;
pub mod resource_sharing;
pub mod retention;
pub mod route_policies;
pub mod routing;
pub mod run_cost_alerts;
pub mod run_sla;
pub mod runs;
pub mod runtime_config;
pub mod services;
pub mod sessions;
pub mod signal_routing;
pub mod signals;
pub mod skill_catalog;
pub mod soul_guard;
pub mod startup;
pub mod tasks;
pub mod telemetry;
pub mod tenants;
pub mod tool_call_approvals;
pub mod voice;
pub mod workspace_memberships;
pub mod workspaces;
pub mod worktree;

pub use agent_roles::AgentRoleRegistry;
pub use approval_policies::ApprovalPolicyService;
pub use approvals::ApprovalService;
pub use bandit::{BanditError, BanditServiceImpl, CreateExperimentRequest, SelectedArm};
pub use checkpoints::CheckpointService;
pub use config_store::{ConfigStore, ConfigStoreError, FileConfigStore, InMemoryConfigStore};
pub use context_builder::{
    ContextBuildError, ContextBuilder, ContextBuilderInput, DefaultContextBuilder,
};
pub use decisions::{
    DecisionError, DecisionResult, DecisionService, DecisionServiceImpl, StubDecisionService,
};
pub use enrichment::{
    ApprovalEnrichment, CheckpointEnrichment, RunEnrichment, RuntimeEnrichment, SessionEnrichment,
    StoreBackedEnrichment, TaskEnrichment,
};
pub use error::RuntimeError;
pub use eval_runs::EvalRunService;
pub use fleet::{FleetReport, FleetService, FleetServiceImpl, WorkerState};
pub use guardian::GuardianResolver;
pub use ingest_jobs::IngestJobService;
pub use mailbox::MailboxService;
pub use mailbox_delivery::{MailboxDeliveryService, MailboxWatcher};
pub use model_registry::ModelRegistry;
pub use observability::{LatencyStats, LlmObservabilityService};
pub use projects::ProjectService;
pub use prompt_assets::PromptAssetService;
pub use prompt_releases::PromptReleaseService;
pub use prompt_versions::PromptVersionService;
pub use routing::RouteResolverService;
pub use runs::RunService;
pub use runtime_config::{
    RuntimeConfig, KEY_BRAIN_MODEL, KEY_BRAIN_URL, KEY_EMBED_MODEL, KEY_GENERATE_MODEL,
    KEY_MAX_TOKENS, KEY_OLLAMA_EMBED_MODEL, KEY_STREAM_MODEL, KEY_THINKING_MODEL_PREFIXES,
    KEY_WORKER_URL,
};
pub use services::{
    AllowlistRevokedRun, ApprovalPolicyServiceImpl, ApprovalServiceImpl, BaseRevisionDriftRun,
    CheckpointServiceImpl, EvalRunServiceImpl, ExternalWorkerService, ExternalWorkerServiceImpl,
    IngestJobServiceImpl, LlmObservabilityServiceImpl, MailboxServiceImpl, ProjectServiceImpl,
    PromptAssetServiceImpl, PromptReleaseServiceImpl, PromptVersionServiceImpl, RecoveryService,
    RecoveryServiceImpl, SandboxLostRun, SandboxReattachedRun, SignalServiceImpl,
    SimpleRouteResolver, TenantServiceImpl, ToolCallApprovalReaderAdapter,
    ToolCallApprovalServiceImpl, ToolInvocationService, ToolInvocationServiceImpl,
    WorkspaceServiceImpl,
};
pub use sessions::SessionService;
pub use signals::SignalService;
pub use soul_guard::SoulGuard;
pub use tasks::TaskService;
pub use tenants::TenantService;
pub use tool_call_approvals::{
    AllowRule, ApprovalDecision as ToolCallApprovalDecision, ApprovedProposal, OperatorDecision,
    ToolCallApprovalReader, ToolCallApprovalService, ToolCallProposal,
};
pub use workspaces::WorkspaceService;
// Service trait exports
pub use audits::AuditService;
pub use budgets::BudgetService;
pub use channels::ChannelService;
pub use credentials::CredentialService;
pub use defaults::DefaultsService;
pub use guardrails::GuardrailService;
pub use licenses::LicenseService;
pub use notification_prefs::NotificationService;
pub use operator_profiles::OperatorProfileService;
pub use provider_bindings::ProviderBindingService;
pub use provider_connections::{ProviderConnectionConfig, ProviderConnectionService};
pub use provider_health::ProviderHealthService;
pub use provider_pools::ProviderConnectionPoolService;
pub use provider_registry::{
    json_messages_to_chat_messages, ProviderRegistry, ProviderResolutionPurpose,
    StartupFallbackProviders, StartupProviderEntry,
};
pub use quotas::QuotaService;
pub use resource_sharing::ResourceSharingService;
pub use retention::RetentionService;
pub use route_policies::RoutePolicyService;
pub use run_cost_alerts::RunCostAlertService;
pub use run_sla::RunSlaService;
pub use services::{InMemoryVoiceService, ProviderModelServiceImpl};
pub use signal_routing::SignalRouterService;
pub use voice::{SpeechToTextService, TextToSpeechService};
pub use workspace_memberships::WorkspaceMembershipService;

pub use aggregate::InMemoryServices;
pub use services::confidence_calibrator::{CalibrationAdjustment, ConfidenceCalibrator};
pub use services::event_helpers::seed_event_counter;

// ── Ollama local LLM + embedding providers ────────────────────────────────────
pub use services::{OllamaEmbeddingProvider, OllamaModel, OllamaProvider, OllamaTagsResponse};

// ── Stable app/runtime boundary exports ──────────────────────────────────────
pub use services::make_envelope;
pub use services::parse_outcome;
pub use services::{
    CredentialScopeKey, CredentialValue, GraphNodeId, MarketplaceCommand, MarketplaceError,
    MarketplaceEvent, MarketplaceRecord, MarketplaceService, MarketplaceState, RateLimitConfig,
    RunTemplate, SignalPattern, SkipReason, SuspensionReason, TemplateBudget, Trigger,
    TriggerCondition, TriggerError, TriggerEvent, TriggerService, TriggerState,
};

// ── RFC 009: Provider routing + health tracking ──────────────────────────────
pub use services::{
    DispatchEntry, ProviderHealthTracker, ProviderRouter, RoutableProvider, RoutingConfig,
    RoutingOutcome,
};

// ── Model chain + composed routed generation ─────────────────────────────────
pub use services::{
    format_attempt_summary, single_model_service, CooldownMap, FallbackAttempt, FallbackOutcome,
    ModelChain, RoutedBinding, RoutedGenerationError, RoutedGenerationService,
    RoutedGenerationSuccess, DEFAULT_RATE_LIMIT_COOLDOWN,
};

// ── RFC 007: Plugin lifecycle management ─────────────────────────────────────
pub use services::{
    CapabilityRegistry, DeliveryFailure, PluginError, PluginEventRouter, PluginHealthMonitor,
    PluginHost, PluginState,
};

// ── RFC 006: Prompt release pipeline ─────────────────────────────────────────
pub use services::{
    DiffKind, DiffLine, PromptReleasePipeline, RolloutState, RolloutStatus, RoutingDecision,
    VersionDiff,
};

// ── RFC 008: Tenant isolation + workspace quotas ─────────────────────────────
pub use services::{
    IsolationViolation, QuotaViolation, TenantAccessPolicy, WorkspaceQuotaManager,
    WorkspaceQuotaPolicy, WorkspaceUsage, WorkspaceUsageReport,
};

/// Orchestrator LLM integration — PromptBuilder, ResponseParser, BrainLlmClient.
/// ActionProposal and ActionType come from cairn_domain::orchestrator directly.
pub use services::orchestrator::{
    BrainLlmClient, ContextBundle, OrchestratorError, PromptBuilder, ResponseParser, TaskSummary,
};

std::thread_local! {
    static CURRENT_TRACE_ID: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

/// Set the current trace ID for event correlation (RFC 011).
///
/// Called by the request middleware to propagate the X-Trace-Id header
/// into events emitted during request handling via `make_envelope`.
pub fn set_current_trace_id(trace_id: &str) {
    CURRENT_TRACE_ID.with(|cell| {
        *cell.borrow_mut() = trace_id.to_owned();
    });
}

/// Read the current thread-local trace ID (empty string if unset).
pub fn get_current_trace_id() -> String {
    CURRENT_TRACE_ID.with(|cell| cell.borrow().clone())
}

#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_and_store_deps() {
        let id = cairn_domain::SessionId::new("test");
        assert_eq!(id.as_str(), "test");
    }
}
