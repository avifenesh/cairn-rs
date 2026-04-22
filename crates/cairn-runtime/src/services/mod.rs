//! Concrete runtime service implementations.
//!
//! Each service accepts command parameters, validates state transitions,
//! emits events through the EventLog, and returns the updated projection.

pub mod approval_impl;
pub mod approval_policy_impl;
pub mod audit_impl;
pub mod budget_impl;
pub mod channel_impl;
pub mod checkpoint_impl;
pub mod confidence_calibrator;
pub mod credential_impl;
pub mod defaults_impl;
pub mod eval_run_impl;
pub mod event_helpers;
pub mod external_worker_impl;
pub mod guardrail_impl;
pub mod ingest_job_impl;
pub mod license_impl;
pub mod mailbox_impl;
pub mod notification_impl;
pub mod observability_impl;
pub mod operator_profile_impl;
pub mod orchestrator;
pub mod project_impl;
pub mod prompt_asset_impl;
pub mod prompt_release_impl;
pub mod prompt_version_impl;
pub mod provider_binding_impl;
pub mod provider_connection_impl;
pub mod provider_health_impl;
pub mod provider_model_impl;
pub mod provider_pool_impl;
pub mod quota_impl;
pub mod recovery_impl;
pub mod research_impl;
pub mod retention_impl;
pub mod route_policy_impl;
pub mod route_resolver_impl;
pub mod run_cost_alert_impl;
pub mod run_sla_impl;
pub mod signal_impl;
pub mod signal_router_impl;
pub mod skill_catalog_impl;
pub mod tenant_impl;
pub mod tool_invocation_impl;
pub mod voice_impl;
pub mod workspace_impl;
pub mod workspace_membership_impl;

pub use approval_impl::ApprovalServiceImpl;
pub use approval_policy_impl::ApprovalPolicyServiceImpl;
pub use checkpoint_impl::CheckpointServiceImpl;
pub use confidence_calibrator::{CalibrationAdjustment, ConfidenceCalibrator};
pub use eval_run_impl::EvalRunServiceImpl;
pub use event_helpers::make_envelope;
pub use external_worker_impl::{parse_outcome, ExternalWorkerService, ExternalWorkerServiceImpl};
pub use ingest_job_impl::IngestJobServiceImpl;
pub use mailbox_impl::MailboxServiceImpl;
pub use observability_impl::LlmObservabilityServiceImpl;
pub use project_impl::ProjectServiceImpl;
pub use prompt_asset_impl::PromptAssetServiceImpl;
pub use prompt_release_impl::PromptReleaseServiceImpl;
pub use prompt_version_impl::PromptVersionServiceImpl;
pub use route_resolver_impl::SimpleRouteResolver;
pub use signal_impl::SignalServiceImpl;
pub use skill_catalog_impl::SkillCatalogServiceImpl;
pub use tenant_impl::TenantServiceImpl;
pub use tool_invocation_impl::{ToolInvocationService, ToolInvocationServiceImpl};
pub use voice_impl::InMemoryVoiceService;
pub use workspace_impl::WorkspaceServiceImpl;

pub use audit_impl::AuditServiceImpl;
pub use budget_impl::BudgetServiceImpl;
pub use channel_impl::ChannelServiceImpl;
pub use credential_impl::CredentialServiceImpl;
pub use defaults_impl::DefaultsServiceImpl;
pub use guardrail_impl::GuardrailServiceImpl;
pub use license_impl::LicenseServiceImpl;
pub use notification_impl::NotificationServiceImpl;
pub use operator_profile_impl::OperatorProfileServiceImpl;
pub use provider_binding_impl::ProviderBindingServiceImpl;
pub use provider_connection_impl::ProviderConnectionServiceImpl;
pub use provider_health_impl::ProviderHealthServiceImpl;
pub use provider_model_impl::ProviderModelServiceImpl;
pub use provider_pool_impl::ProviderConnectionPoolServiceImpl;
pub use quota_impl::QuotaServiceImpl;
pub use recovery_impl::{
    AllowlistRevokedRun, BaseRevisionDriftRun, RecoveryService, RecoveryServiceImpl,
    SandboxLostRun, SandboxReattachedRun,
};
pub use research_impl::{InMemoryDigestService, InMemoryResearchService};
pub use retention_impl::RetentionServiceImpl;
pub use route_policy_impl::RoutePolicyServiceImpl;
pub use run_cost_alert_impl::RunCostAlertServiceImpl;
pub use run_sla_impl::RunSlaServiceImpl;
pub use signal_router_impl::SignalRouterServiceImpl;
pub use workspace_membership_impl::WorkspaceMembershipServiceImpl;

pub mod resource_sharing_impl;
pub use resource_sharing_impl::ResourceSharingServiceImpl;

pub mod ollama_provider;
pub use ollama_provider::{OllamaModel, OllamaProvider, OllamaTagsResponse};

pub mod ollama_embedding;
pub use ollama_embedding::OllamaEmbeddingProvider;

pub mod provider_health_tracker;
pub use provider_health_tracker::ProviderHealthTracker;

pub mod marketplace_service;
pub use marketplace_service::{
    catalog_entry_to_descriptor, is_plugin_tool_visible, is_signal_allowed, resolve_capture_policy,
    CredentialKind, CredentialScopeHint, CredentialScopeKey, CredentialSpec, CredentialValue,
    DescriptorSource, GraphNodeId, HealthCheckSpec, MarketplaceCommand, MarketplaceError,
    MarketplaceEvent, MarketplaceRecord, MarketplaceService, MarketplaceState, PluginDescriptor,
    PluginEnablement, ResolvedCapturePolicy,
};

pub mod trigger_service;
pub use trigger_service::{
    auto_approve_decision, evaluate_condition, evaluate_conditions, substitute_variables,
    RateLimitConfig, RunTemplate, SignalPattern, SkipReason, SuspensionReason, TemplateBudget,
    Trigger, TriggerCondition, TriggerDecisionOutcome, TriggerError, TriggerEvent, TriggerService,
    TriggerState,
};

pub mod plugin_health_monitor;
pub use plugin_health_monitor::PluginHealthMonitor;

pub mod plugin_capability_registry;
pub use plugin_capability_registry::CapabilityRegistry;

pub mod plugin_host;
pub use plugin_host::{PluginError, PluginHost, PluginState};

pub mod plugin_event_router;
pub use plugin_event_router::{DeliveryFailure, PluginEventRouter};

pub mod provider_router;
pub use provider_router::{
    DispatchEntry, ProviderRouter, RoutableProvider, RoutingConfig, RoutingOutcome,
};

pub mod prompt_release_pipeline;
pub use prompt_release_pipeline::{
    DiffKind, DiffLine, PromptReleasePipeline, RolloutState, RolloutStatus, RoutingDecision,
    VersionDiff,
};

pub mod workspace_quota;
pub use workspace_quota::{
    IsolationViolation, QuotaViolation, TenantAccessPolicy, WorkspaceQuotaManager,
    WorkspaceQuotaPolicy, WorkspaceUsage, WorkspaceUsageReport,
};
