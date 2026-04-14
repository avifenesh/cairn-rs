//! `InMemoryServices` — the bundled runtime service aggregate for cairn-app.
//!
//! Provides a single injectable struct that wires all runtime services against
//! a shared `InMemoryStore`. cairn-app constructs one instance at startup and
//! passes it to all HTTP handlers via `Arc<AppState>`.

use std::sync::Arc;

use cairn_store::InMemoryStore;

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
    QuotaServiceImpl, RecoveryServiceImpl, RetentionServiceImpl, RoutePolicyServiceImpl,
    RunCostAlertServiceImpl, RunServiceImpl, RunSlaServiceImpl, SessionServiceImpl,
    SignalRouterServiceImpl, SignalServiceImpl, TaskServiceImpl, TenantServiceImpl,
    WorkspaceMembershipServiceImpl, WorkspaceServiceImpl,
};
use crate::ProviderRegistry;

/// Bundled runtime services backed by `InMemoryStore`.
///
/// Every field is a concrete `*ServiceImpl<InMemoryStore>` so that
/// cairn-app handlers can call service methods without extra type juggling.
pub struct InMemoryServices {
    /// The shared append-only event log + synchronous projections.
    pub store: Arc<InMemoryStore>,

    // ── Core runtime ───────────────────────────────────────────────────────
    pub runs: RunServiceImpl<InMemoryStore>,
    pub tasks: TaskServiceImpl<InMemoryStore>,
    pub sessions: SessionServiceImpl<InMemoryStore>,
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

    // ── Recovery & observability ───────────────────────────────────────────
    pub recovery: RecoveryServiceImpl<InMemoryStore>,
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
    pub fn new() -> Self {
        let store = Arc::new(InMemoryStore::new());
        Self::with_store(store)
    }

    /// Create a bundle wired to an existing store (useful for testing).
    pub fn with_store(store: Arc<InMemoryStore>) -> Self {
        let decisions = Arc::new(crate::decisions::DecisionServiceImpl::new());
        let decision_service: Arc<dyn crate::decisions::DecisionService> = decisions.clone();

        Self {
            runs: RunServiceImpl::new(store.clone()),
            tasks: TaskServiceImpl::new(store.clone()),
            sessions: SessionServiceImpl::new(store.clone()),
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
            recovery: RecoveryServiceImpl::new(store.clone()),
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
            decisions,
            decision_service,
            runtime_config: std::sync::Arc::new(crate::runtime_config::RuntimeConfig::new(
                store.clone(),
            )),
            store,
        }
    }
}

impl Default for InMemoryServices {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
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
