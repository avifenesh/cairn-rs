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
#[cfg(feature = "in-memory-runtime")]
use crate::services::{RunServiceImpl, SessionServiceImpl, TaskServiceImpl};
use crate::sessions::SessionService;
use crate::tasks::TaskService;
use crate::ProviderRegistry;

/// Bundled runtime services backed by `InMemoryStore`.
///
/// Core execution fields (`runs`, `tasks`, `sessions`) are `Arc<dyn Trait>`
/// so cairn-app can swap the in-memory impl for
/// `Fabric{Run,Task,Session}ServiceAdapter` at boot when
/// `CAIRN_FABRIC_ENABLED=1`. All other fields remain concrete
/// `*ServiceImpl<InMemoryStore>` — they back non-execution surfaces
/// (approvals, evals, provider bindings, etc.) that FF does not manage.
pub struct InMemoryServices {
    /// The shared append-only event log + synchronous projections.
    pub store: Arc<InMemoryStore>,

    // ── Core runtime ───────────────────────────────────────────────────────
    //
    // Trait-object fields so the Fabric adapter can be swapped in at boot
    // when `CAIRN_FABRIC_ENABLED=1`. Cairn-app's `AppState::new` picks the
    // concrete impl (in-memory vs FabricRunServiceAdapter et al.). Handlers
    // call trait methods through these fields unchanged either way.
    //
    // `InMemoryServices::new()` / `with_store()` default to the in-memory
    // impl; `with_store_and_core(store, runs, tasks, sessions)` lets callers
    // inject the Fabric adapter (or any other `RunService` / `TaskService` /
    // `SessionService` impl).
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
    // Recovery is NOT on this struct. FF's 14 background scanners
    // (DelayedPromoter, LeaseExpiryScanner, AttemptTimeoutScanner,
    // ExecutionDeadlineScanner, SuspensionTimeoutScanner,
    // PendingWaitpointExpiryScanner, BudgetResetScanner, BudgetReconciler,
    // QuotaReconciler, DependencyReconciler, FlowProjector,
    // IndexReconciler, RetentionTrimmer, UnblockScanner) own recovery
    // unconditionally — whether CAIRN_FABRIC_ENABLED is set or not, there is
    // no cairn-side recovery sweep worth running. The pre-Fabric
    // `RecoveryServiceImpl` was removed in the finalization round.
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
    /// Alias for audits — cairn-app uses this name.
    pub audit: AuditServiceImpl<InMemoryStore>,
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
    /// Create a fully-wired bundle backed by a fresh `InMemoryStore`.
    ///
    /// Only available under the `in-memory-runtime` feature. Default builds
    /// must use [`Self::with_store_and_core`] and inject Fabric-backed
    /// adapters for `runs` / `tasks` / `sessions` — the production path.
    #[cfg(feature = "in-memory-runtime")]
    pub fn new() -> Self {
        let store = Arc::new(InMemoryStore::new());
        Self::with_store(store)
    }

    /// Create a bundle wired to an existing store, defaulting
    /// runs/tasks/sessions to the in-memory impls.
    ///
    /// Only available under the `in-memory-runtime` feature — the in-memory
    /// Run/Task/Session backings carry no correctness guarantees and exist
    /// for local tinkering and tests. Production callers use
    /// [`Self::with_store_and_core`] with Fabric adapters.
    #[cfg(feature = "in-memory-runtime")]
    pub fn with_store(store: Arc<InMemoryStore>) -> Self {
        let runs: Arc<dyn RunService> = Arc::new(RunServiceImpl::new(store.clone()));
        let tasks: Arc<dyn TaskService> = Arc::new(TaskServiceImpl::new(store.clone()));
        let sessions: Arc<dyn SessionService> = Arc::new(SessionServiceImpl::new(store.clone()));
        Self::with_store_and_core(store, runs, tasks, sessions)
    }

    /// Create a bundle wired to an existing store with caller-supplied core
    /// services.
    ///
    /// Cairn-app uses this to install Fabric-backed adapters for runs,
    /// tasks, and sessions when `CAIRN_FABRIC_ENABLED` is set. Every other
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
            audit: AuditServiceImpl::new(store.clone()),
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

    /// Create a bundle wired to an existing store with Fabric services attached.
    ///
    /// The `fabric` argument is type-erased to avoid a cairn-runtime -> cairn-fabric
    /// cyclic dependency. Callers pass `Arc<cairn_fabric::FabricServices>` and
    /// retrieve it later via `fabric::<T>()`.
    ///
    /// Only available under the `in-memory-runtime` feature because it
    /// starts from `Self::with_store` which needs the in-memory impls. The
    /// production path builds via `Self::with_store_and_core` directly.
    #[cfg(feature = "in-memory-runtime")]
    pub fn with_fabric(store: Arc<InMemoryStore>, fabric: Arc<dyn Any + Send + Sync>) -> Self {
        let mut services = Self::with_store(store);
        services.fabric = Some(fabric);
        services
    }

    /// Downcast the Fabric services to a concrete type.
    ///
    /// Returns `None` if no fabric was configured or if `T` doesn't match.
    /// Typical usage: `services.fabric::<cairn_fabric::FabricServices>()`.
    pub fn fabric<T: Any + Send + Sync>(&self) -> Option<&T> {
        self.fabric.as_ref().and_then(|f| f.downcast_ref::<T>())
    }
}

#[cfg(feature = "in-memory-runtime")]
impl Default for InMemoryServices {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(test, feature = "in-memory-runtime"))]
mod tests {
    use super::InMemoryServices;
    use crate::decisions::DecisionService;
    use cairn_domain::decisions::{
        DecisionEvent, DecisionKind, DecisionOutcome, DecisionRequest, DecisionSource,
        DecisionSubject, Principal, ToolEffect,
    };
    use cairn_domain::ids::{CorrelationId, RunId};
    use cairn_domain::ProjectKey;

    fn observational_request() -> DecisionRequest {
        DecisionRequest {
            kind: DecisionKind::ToolInvocation {
                tool_name: "grep_search".into(),
                effect: ToolEffect::Observational,
            },
            principal: Principal::Run {
                run_id: RunId::new("run_1"),
            },
            subject: DecisionSubject::ToolCall {
                tool_name: "grep_search".into(),
                args: serde_json::json!({"pattern": "TODO"}),
            },
            scope: ProjectKey::new("t", "w", "p"),
            cost_estimate: None,
            requested_at: 1_700_000_000_000,
            correlation_id: CorrelationId::new("cor_aggregate"),
        }
    }

    #[tokio::test]
    async fn decision_service_handles_share_cache() {
        let runtime = InMemoryServices::new();
        let request = observational_request();
        let scope = request.scope.clone();

        let initial = runtime
            .decision_service
            .evaluate(request.clone())
            .await
            .unwrap();
        assert_eq!(initial.outcome, DecisionOutcome::Allowed);

        let cached = runtime.decisions.list_cached(&scope, 10).await.unwrap();
        assert!(
            cached
                .iter()
                .any(|entry| entry.decision_id == initial.decision_id),
            "expected cache entries from the injected decision service to be visible through runtime.decisions",
        );

        let second = runtime.decisions.evaluate(request).await.unwrap();
        if let DecisionEvent::DecisionRecorded { source, .. } = &second.event {
            assert!(
                matches!(source, DecisionSource::CacheHit { .. }),
                "expected runtime.decisions to reuse cache populated via runtime.decision_service",
            );
        } else {
            panic!("expected DecisionRecorded");
        }
    }
}
