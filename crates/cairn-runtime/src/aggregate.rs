//! `InMemoryServices` — the bundled runtime service aggregate for cairn-app.
//!
//! Provides a single injectable struct that wires all runtime services against
//! a shared `InMemoryStore`. cairn-app constructs one instance at startup and
//! passes it to all HTTP handlers via `Arc<AppState>`.

use std::any::Any;
use std::sync::Arc;

use cairn_store::InMemoryStore;

use crate::runs::RunService;
use crate::services::resource_sharing_impl::ResourceSharingServiceImpl;
use crate::services::ToolInvocationServiceImpl;
use crate::services::{
    ApprovalPolicyServiceImpl, ApprovalServiceImpl, AuditServiceImpl, BudgetServiceImpl,
    ChannelServiceImpl, CheckpointServiceImpl, CredentialServiceImpl, DefaultsServiceImpl,
    EvalRunServiceImpl, ExternalWorkerServiceImpl, GuardrailServiceImpl, IngestJobServiceImpl,
    LicenseServiceImpl, LlmObservabilityServiceImpl, MailboxServiceImpl, NotificationServiceImpl,
    OperatorProfileServiceImpl, ProjectServiceImpl, PromptAssetServiceImpl,
    PromptReleaseServiceImpl, PromptVersionServiceImpl, ProviderBindingServiceImpl,
    ProviderConnectionPoolServiceImpl, ProviderConnectionServiceImpl, ProviderHealthServiceImpl,
    QuotaServiceImpl, RetentionServiceImpl, RoutePolicyServiceImpl, RunCostAlertServiceImpl,
    RunSlaServiceImpl, SignalRouterServiceImpl, SignalServiceImpl, TenantServiceImpl,
    WorkspaceMembershipServiceImpl, WorkspaceServiceImpl,
};
use crate::sessions::SessionService;
use crate::tasks::TaskService;
use crate::ProviderRegistry;

/// Bundled runtime services backed by `InMemoryStore`.
///
/// Core execution fields (`runs`, `tasks`, `sessions`) are `Arc<dyn Trait>`
/// so cairn-app installs `Fabric{Run,Task,Session}ServiceAdapter` at boot
/// — that is the only production path. All other fields remain concrete
/// `*ServiceImpl<InMemoryStore>` and back non-execution surfaces
/// (approvals, evals, provider bindings, etc.) that FF does not manage.
pub struct InMemoryServices {
    /// The shared append-only event log + synchronous projections.
    pub store: Arc<InMemoryStore>,

    // ── Core runtime ───────────────────────────────────────────────────────
    //
    // Trait-object fields so the Fabric adapter is installed at boot.
    // Handlers call trait methods through these fields; the single factory
    // `with_store_and_core(store, runs, tasks, sessions)` is how cairn-app
    // wires them in.
    pub runs: Arc<dyn RunService>,
    pub tasks: Arc<dyn TaskService>,
    pub sessions: Arc<dyn SessionService>,
    pub tenants: TenantServiceImpl<InMemoryStore>,
    pub workspaces: WorkspaceServiceImpl<InMemoryStore>,
    pub projects: ProjectServiceImpl<InMemoryStore>,

    // ── Approvals & checkpoints ────────────────────────────────────────────
    pub approvals: ApprovalServiceImpl<InMemoryStore>,
    pub approval_policies: ApprovalPolicyServiceImpl<InMemoryStore>,
    pub checkpoints: CheckpointServiceImpl<InMemoryStore>,

    // ── Prompts ────────────────────────────────────────────────────────────
    pub prompt_assets: PromptAssetServiceImpl<InMemoryStore>,
    pub prompt_releases: PromptReleaseServiceImpl<InMemoryStore>,
    pub prompt_versions: PromptVersionServiceImpl<InMemoryStore>,

    // ── Ingest & eval ──────────────────────────────────────────────────────
    pub ingest_jobs: IngestJobServiceImpl<InMemoryStore>,
    pub eval_runs: EvalRunServiceImpl<InMemoryStore>,

    // ── Communication & mailbox ────────────────────────────────────────────
    pub mailbox: MailboxServiceImpl<InMemoryStore>,
    pub signals: SignalServiceImpl<InMemoryStore>,
    pub signal_router: SignalRouterServiceImpl<InMemoryStore>,
    pub channels: ChannelServiceImpl<InMemoryStore>,

    // ── External workers ──────────────────────────────────────────────────
    pub external_workers: ExternalWorkerServiceImpl<InMemoryStore>,

    // ── Observability ──────────────────────────────────────────────────────
    //
    // RFC 020: Recovery ownership split.
    //
    // Operational recovery (FF state): LeaseExpiryScanner,
    // AttemptTimeoutScanner, ExecutionDeadlineScanner,
    // SuspensionTimeoutScanner, PendingWaitpointExpiryScanner,
    // BudgetResetScanner, BudgetReconciler, QuotaReconciler,
    // DependencyReconciler, FlowProjector, IndexReconciler,
    // RetentionTrimmer, UnblockScanner — owned by FlowFabric's 14
    // background scanners. They run continuously, not at cairn-app boot.
    //
    // Run-level recovery (cairn state): `RecoveryServiceImpl::recover_all`
    // runs once at startup (after `SandboxService::recover_all`, before the
    // readiness gate flips). It enumerates non-terminal runs, applies the
    // RFC 020 recovery matrix, emits `RecoveryAttempted`/`RecoveryCompleted`
    // events carrying the boot id, and lets the orchestrator resume the run
    // on its next tick. See RFC 020 §"Recovery ownership split" and the
    // design delta in `project_rfc020_delta_and_gaps.md` (Part A).
    pub observability: LlmObservabilityServiceImpl<InMemoryStore>,

    // ── Provider & routing ─────────────────────────────────────────────────
    pub provider_bindings: ProviderBindingServiceImpl<InMemoryStore>,
    pub provider_connections: ProviderConnectionServiceImpl<InMemoryStore>,
    pub provider_health: ProviderHealthServiceImpl<InMemoryStore>,
    pub provider_pools: ProviderConnectionPoolServiceImpl<InMemoryStore>,
    pub provider_registry: std::sync::Arc<ProviderRegistry<InMemoryStore>>,

    // ── Governance ────────────────────────────────────────────────────────
    pub credentials: CredentialServiceImpl<InMemoryStore>,
    pub defaults: DefaultsServiceImpl<InMemoryStore>,
    pub licenses: LicenseServiceImpl<InMemoryStore>,
    pub guardrails: GuardrailServiceImpl<InMemoryStore>,
    pub quotas: QuotaServiceImpl<InMemoryStore>,
    pub retention: RetentionServiceImpl<InMemoryStore>,
    pub route_policies: RoutePolicyServiceImpl<InMemoryStore>,
    pub run_cost_alerts: RunCostAlertServiceImpl<InMemoryStore>,
    pub run_sla: RunSlaServiceImpl<InMemoryStore>,
    pub budgets: BudgetServiceImpl<InMemoryStore>,

    // ── Notifications & operators ─────────────────────────────────────────
    pub notifications: NotificationServiceImpl<InMemoryStore>,
    pub operator_profiles: OperatorProfileServiceImpl<InMemoryStore>,
    pub workspace_memberships: WorkspaceMembershipServiceImpl<InMemoryStore>,
    pub audits: AuditServiceImpl<InMemoryStore>,
    pub tool_invocations: crate::services::ToolInvocationServiceImpl<InMemoryStore>,

    // ── Resource sharing ──────────────────────────────────────────────────
    pub resource_sharing: ResourceSharingServiceImpl<InMemoryStore>,

    // ── Fabric (FlowFabric bridge) ─────────────────────────────────────
    // Stored as `dyn Any` to avoid cairn-runtime → cairn-fabric cycle.
    // Downcast to `cairn_fabric::FabricServices` via `fabric::<T>()`.
    pub fabric: Option<Arc<dyn Any + Send + Sync>>,

    // ── Decision layer (RFC 019) ─────────────────────────────────────────
    pub decisions: std::sync::Arc<crate::decisions::DecisionServiceImpl>,
    /// Arc-wrapped decision service for injection into the execute phase.
    pub decision_service: std::sync::Arc<dyn crate::decisions::DecisionService>,

    // ── Hot-reloadable configuration ──────────────────────────────────────
    /// Typed accessors for model settings and operational knobs.
    ///
    /// Reads from the DefaultsService store first (changeable via API),
    /// falls back to env vars, then hardcoded defaults.
    pub runtime_config: std::sync::Arc<crate::runtime_config::RuntimeConfig>,
}

impl InMemoryServices {
    /// Create a bundle wired to an existing store with caller-supplied core
    /// services.
    ///
    /// Cairn-app uses this to install Fabric-backed adapters for runs,
    /// tasks, and sessions in default (production) builds. Every other
    /// service still hangs off the shared in-memory store, so provider
    /// bindings, evals, approvals, etc. remain identical.
    pub fn with_store_and_core(
        store: Arc<InMemoryStore>,
        runs: Arc<dyn RunService>,
        tasks: Arc<dyn TaskService>,
        sessions: Arc<dyn SessionService>,
    ) -> Self {
        let decisions = Arc::new(crate::decisions::DecisionServiceImpl::new());
        let decision_service: Arc<dyn crate::decisions::DecisionService> = decisions.clone();

        Self {
            runs,
            tasks,
            sessions,
            tenants: TenantServiceImpl::new(store.clone()),
            workspaces: WorkspaceServiceImpl::new(store.clone()),
            projects: ProjectServiceImpl::new(store.clone()),
            approvals: ApprovalServiceImpl::new(store.clone()),
            approval_policies: ApprovalPolicyServiceImpl::new(store.clone()),
            checkpoints: CheckpointServiceImpl::new(store.clone()),
            prompt_assets: PromptAssetServiceImpl::new(store.clone()),
            prompt_releases: PromptReleaseServiceImpl::new(store.clone()),
            prompt_versions: PromptVersionServiceImpl::new(store.clone()),
            ingest_jobs: IngestJobServiceImpl::new(store.clone()),
            eval_runs: EvalRunServiceImpl::new(store.clone()),
            mailbox: MailboxServiceImpl::new(store.clone()),
            signals: SignalServiceImpl::new(store.clone()),
            signal_router: SignalRouterServiceImpl::new(store.clone()),
            channels: ChannelServiceImpl::new(store.clone()),
            external_workers: ExternalWorkerServiceImpl::new(store.clone()),
            observability: LlmObservabilityServiceImpl::new(store.clone()),
            provider_bindings: ProviderBindingServiceImpl::new(store.clone()),
            provider_connections: ProviderConnectionServiceImpl::new(store.clone()),
            provider_health: ProviderHealthServiceImpl::new(store.clone()),
            provider_pools: ProviderConnectionPoolServiceImpl::new(store.clone()),
            provider_registry: std::sync::Arc::new(ProviderRegistry::new(store.clone())),
            credentials: CredentialServiceImpl::new(store.clone()),
            defaults: DefaultsServiceImpl::new(store.clone()),
            licenses: LicenseServiceImpl::new(store.clone()),
            guardrails: GuardrailServiceImpl::new(store.clone()),
            quotas: QuotaServiceImpl::new(store.clone()),
            retention: RetentionServiceImpl::new(store.clone()),
            route_policies: RoutePolicyServiceImpl::new(store.clone()),
            run_cost_alerts: RunCostAlertServiceImpl::new(store.clone()),
            run_sla: RunSlaServiceImpl::new(store.clone()),
            budgets: BudgetServiceImpl::new(store.clone()),
            notifications: NotificationServiceImpl::new(store.clone()),
            operator_profiles: OperatorProfileServiceImpl::new(store.clone()),
            workspace_memberships: WorkspaceMembershipServiceImpl::new(store.clone()),
            audits: AuditServiceImpl::new(store.clone()),
            tool_invocations: ToolInvocationServiceImpl::new(store.clone()),
            resource_sharing: ResourceSharingServiceImpl::new(store.clone()),
            fabric: None,
            decisions,
            decision_service,
            runtime_config: std::sync::Arc::new(crate::runtime_config::RuntimeConfig::new(
                store.clone(),
            )),
            store,
        }
    }

    /// Downcast the Fabric services to a concrete type.
    ///
    /// Returns `None` if no fabric was configured or if `T` doesn't match.
    /// Typical usage: `services.fabric::<cairn_fabric::FabricServices>()`.
    pub fn fabric<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.fabric.as_ref().and_then(|f| f.downcast_ref::<T>())
    }
}
