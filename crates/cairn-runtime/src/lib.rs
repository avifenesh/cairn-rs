//! Durable runtime services for sessions, runs, tasks, approvals, and recovery.
//!
//! `cairn-runtime` owns the runtime service boundaries that accept
//! commands, validate state transitions, persist events, and update
//! synchronous projections through `cairn-store`.

pub mod agent_roles;
pub mod aggregate;
pub mod approval_policies;
pub mod bandit;
pub mod config_store;
pub mod fleet;
pub mod soul_guard;
pub mod model_registry;
pub mod observability;
pub mod approvals;
pub mod checkpoints;
pub mod enrichment;
pub mod error;
pub mod eval_runs;
pub mod ingest_jobs;
pub mod mailbox;
pub mod mailbox_delivery;
pub mod skill_catalog;
pub mod spend_alert;
pub mod worktree;
pub mod research;
pub mod projects;
pub mod prompt_assets;
pub mod prompt_releases;
pub mod prompt_versions;
pub mod recovery;
pub mod routing;
pub mod runs;
pub mod services;
pub mod sessions;
pub mod signals;
pub mod tasks;
pub mod tenants;
pub mod voice;
pub mod audits;
pub mod budgets;
pub mod channels;
pub mod credentials;
pub mod defaults;
pub mod guardrails;
pub mod licenses;
pub mod notification_prefs;
pub mod operator_profiles;
pub mod provider_bindings;
pub mod provider_connections;
pub mod provider_health;
pub mod provider_pools;
pub mod quotas;
pub mod resource_sharing;
pub mod retention;
pub mod route_policies;
pub mod run_cost_alerts;
pub mod run_sla;
pub mod signal_routing;
pub mod workspace_memberships;
pub mod workspaces;

pub use approval_policies::ApprovalPolicyService;
pub use approvals::ApprovalService;
pub use checkpoints::CheckpointService;
pub use enrichment::{
    ApprovalEnrichment, CheckpointEnrichment, RunEnrichment, RuntimeEnrichment, SessionEnrichment,
    StoreBackedEnrichment, TaskEnrichment,
};
pub use error::RuntimeError;
pub use mailbox::MailboxService;
pub use mailbox_delivery::{MailboxDeliveryService, MailboxWatcher};
pub use recovery::{RecoveryAction, RecoveryService, RecoverySummary};
pub use runs::RunService;
pub use eval_runs::EvalRunService;
pub use ingest_jobs::IngestJobService;
pub use prompt_assets::PromptAssetService;
pub use prompt_releases::PromptReleaseService;
pub use prompt_versions::PromptVersionService;
pub use observability::{LatencyStats, LlmObservabilityService};
pub use services::{
    ApprovalPolicyServiceImpl, ApprovalServiceImpl, CheckpointServiceImpl, EvalRunServiceImpl, ExternalWorkerService,
    ExternalWorkerServiceImpl, IngestJobServiceImpl, LlmObservabilityServiceImpl, MailboxServiceImpl,
    ProjectServiceImpl, PromptAssetServiceImpl, PromptReleaseServiceImpl,
    PromptVersionServiceImpl, RecoveryServiceImpl,
    RunServiceImpl, SessionServiceImpl, SignalServiceImpl, TaskServiceImpl,
    TenantServiceImpl, ToolInvocationService, ToolInvocationServiceImpl,
    SimpleRouteResolver, WorkspaceServiceImpl,
};
pub use config_store::{ConfigStore, ConfigStoreError, FileConfigStore, InMemoryConfigStore};
pub use agent_roles::AgentRoleRegistry;
pub use bandit::{BanditError, BanditServiceImpl, CreateExperimentRequest, SelectedArm};
pub use fleet::{FleetReport, FleetService, FleetServiceImpl, WorkerState};
pub use soul_guard::SoulGuard;
pub use projects::ProjectService;
pub use model_registry::ModelRegistry;
pub use routing::RouteResolverService;
pub use sessions::SessionService;
pub use signals::SignalService;
pub use tasks::TaskService;
pub use tenants::TenantService;
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
pub use quotas::QuotaService;
pub use resource_sharing::ResourceSharingService;
pub use retention::RetentionService;
pub use route_policies::RoutePolicyService;
pub use run_cost_alerts::RunCostAlertService;
pub use run_sla::RunSlaService;
pub use signal_routing::SignalRouterService;
pub use voice::{SpeechToTextService, TextToSpeechService};
pub use workspace_memberships::WorkspaceMembershipService;
pub use services::{
    InMemoryVoiceService, ProviderModelServiceImpl,
};

pub use aggregate::InMemoryServices;

// ── Ollama local LLM provider ─────────────────────────────────────────────────
pub use services::{OllamaModel, OllamaProvider, OllamaTagsResponse};

/// Noop tracing hook — sets a trace ID in thread-local state (stub).
pub fn set_current_trace_id(_trace_id: &str) {}



#[cfg(test)]
mod tests {
    #[test]
    fn crate_compiles_with_domain_and_store_deps() {
        let id = cairn_domain::SessionId::new("test");
        assert_eq!(id.as_str(), "test");
    }
}
