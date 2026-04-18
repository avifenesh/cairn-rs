use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use cairn_domain::decisions::{
    ActorRef, DecisionKey, DecisionPolicy, DecisionRequest, DecisionScopeRef,
};
use cairn_domain::ids::PolicyId;
use cairn_domain::ProjectKey;
use cairn_runtime::decisions::{CacheEntryState, CachedDecisionSummary};
use cairn_runtime::{DecisionError, DecisionResult, DecisionService, TriggerEvent};
use cairn_store::{InMemoryStore, UsageCounters};
use cairn_workspace::{SandboxEvent, SandboxEventSink};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Clone)]
pub struct UsageSandboxEventSink {
    store: Arc<InMemoryStore>,
    inner: Arc<dyn SandboxEventSink>,
}

impl UsageSandboxEventSink {
    pub fn new(store: Arc<InMemoryStore>, inner: Arc<dyn SandboxEventSink>) -> Self {
        Self { store, inner }
    }
}

impl SandboxEventSink for UsageSandboxEventSink {
    fn publish(&self, event: SandboxEvent) {
        if let SandboxEvent::SandboxProvisioned { project, .. } = &event {
            self.store.increment_sandbox_provision_count(project);
        }
        self.inner.publish(event);
    }
}

#[derive(Clone)]
pub struct UsageMeteredDecisionService {
    inner: Arc<dyn DecisionService>,
    store: Arc<InMemoryStore>,
}

impl UsageMeteredDecisionService {
    pub fn new(inner: Arc<dyn DecisionService>, store: Arc<InMemoryStore>) -> Self {
        Self { inner, store }
    }
}

#[async_trait]
impl DecisionService for UsageMeteredDecisionService {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResult, DecisionError> {
        let scope = request.scope.clone();
        let result = self.inner.evaluate(request).await?;
        self.store.increment_decision_evaluation_count(&scope);
        Ok(result)
    }

    fn policy_for_kind(&self, kind_tag: &str) -> Option<DecisionPolicy> {
        self.inner.policy_for_kind(kind_tag)
    }

    async fn cache_lookup(&self, key: &DecisionKey) -> Result<CacheEntryState, DecisionError> {
        self.inner.cache_lookup(key).await
    }

    async fn invalidate(
        &self,
        decision_id: &cairn_domain::ids::DecisionId,
        reason: &str,
        invalidated_by: ActorRef,
    ) -> Result<(), DecisionError> {
        self.inner
            .invalidate(decision_id, reason, invalidated_by)
            .await
    }

    async fn invalidate_by_scope(
        &self,
        scope: &DecisionScopeRef,
        kind_filter: Option<&str>,
        reason: &str,
        invalidated_by: ActorRef,
    ) -> Result<u32, DecisionError> {
        self.inner
            .invalidate_by_scope(scope, kind_filter, reason, invalidated_by)
            .await
    }

    async fn invalidate_by_rule(
        &self,
        rule_id: &PolicyId,
        reason: &str,
        invalidated_by: ActorRef,
    ) -> Result<u32, DecisionError> {
        self.inner
            .invalidate_by_rule(rule_id, reason, invalidated_by)
            .await
    }

    async fn list_cached(
        &self,
        scope: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<CachedDecisionSummary>, DecisionError> {
        self.inner.list_cached(scope, limit).await
    }

    async fn get_decision(
        &self,
        decision_id: &cairn_domain::ids::DecisionId,
    ) -> Result<Option<cairn_domain::decisions::DecisionEvent>, DecisionError> {
        self.inner.get_decision(decision_id).await
    }
}

pub fn record_trigger_fire_usage(
    store: &InMemoryStore,
    project: &ProjectKey,
    events: &[TriggerEvent],
) {
    for event in events {
        if matches!(event, TriggerEvent::TriggerFired { .. }) {
            store.increment_trigger_fire_count(project);
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UsageTelemetryResponse {
    pub projects: Vec<ProjectUsageRow>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectUsageRow {
    pub tenant_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub run_count: u64,
    pub event_count: u64,
    pub sandbox_provision_count: u64,
    pub decision_evaluation_count: u64,
    pub trigger_fire_count: u64,
}

fn project_rows(snapshot: HashMap<ProjectKey, UsageCounters>) -> Vec<ProjectUsageRow> {
    let mut rows: Vec<ProjectUsageRow> = snapshot
        .into_iter()
        .map(|(project, counters)| ProjectUsageRow {
            tenant_id: project.tenant_id.to_string(),
            workspace_id: project.workspace_id.to_string(),
            project_id: project.project_id.to_string(),
            run_count: counters.run_count,
            event_count: counters.event_count,
            sandbox_provision_count: counters.sandbox_provision_count,
            decision_evaluation_count: counters.decision_evaluation_count,
            trigger_fire_count: counters.trigger_fire_count,
        })
        .collect();

    rows.sort_by(|left, right| {
        (&left.tenant_id, &left.workspace_id, &left.project_id).cmp(&(
            &right.tenant_id,
            &right.workspace_id,
            &right.project_id,
        ))
    });
    rows
}

pub async fn get_usage_telemetry_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(UsageTelemetryResponse {
        projects: project_rows(state.runtime.store.usage_snapshot()),
    })
}

// Boots `AppState` via `BootstrapConfig`; same HMAC-fail-loud reason as
// `repo_routes::tests` — gate on `in-memory-runtime`.
#[cfg(all(test, feature = "in-memory-runtime"))]
mod tests {
    use super::{
        get_usage_telemetry_handler, project_rows, record_trigger_fire_usage,
        UsageMeteredDecisionService, UsageSandboxEventSink, UsageTelemetryResponse,
    };
    use crate::AppState;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use axum::Router;
    use cairn_api::bootstrap::BootstrapConfig;
    use cairn_domain::decisions::{
        DecisionKind, DecisionRequest, DecisionSubject, Principal, ToolEffect,
    };
    use cairn_domain::ids::{CorrelationId, OperatorId, TriggerId};
    use cairn_domain::{
        EventEnvelope, EventId, EventSource, OnExhaustion, ProjectKey, RunCreated, RunId, SignalId,
        TaskId,
    };
    use cairn_runtime::{DecisionService, StubDecisionService, TriggerEvent};
    use cairn_store::{EventLog, InMemoryStore};
    use cairn_workspace::{
        BufferedSandboxEventSink, HostCapabilityRequirements, RepoId, SandboxBase, SandboxEvent,
        SandboxEventSink, SandboxPolicy, SandboxStrategy, SandboxStrategyRequest,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

    fn sample_project(project_id: &str) -> ProjectKey {
        ProjectKey::new("tenant", "workspace", project_id)
    }

    fn sample_policy() -> SandboxPolicy {
        SandboxPolicy {
            strategy: SandboxStrategyRequest::Preferred(SandboxStrategy::Overlay),
            base: SandboxBase::Repo {
                repo_id: RepoId::new("openai/cairn"),
                starting_ref: None,
            },
            credentials: Vec::new(),
            network_egress: None,
            memory_limit_bytes: None,
            cpu_weight: None,
            disk_quota_bytes: None,
            wall_clock_limit: None,
            on_resource_exhaustion: OnExhaustion::Destroy,
            preserve_on_failure: true,
            required_host_caps: HostCapabilityRequirements::default(),
        }
    }

    fn sample_decision_request(project: ProjectKey) -> DecisionRequest {
        DecisionRequest {
            kind: DecisionKind::ToolInvocation {
                tool_name: "memory_search".to_string(),
                effect: ToolEffect::Observational,
            },
            principal: Principal::Operator {
                operator_id: OperatorId::new("operator"),
            },
            subject: DecisionSubject::ToolCall {
                tool_name: "memory_search".to_string(),
                args: serde_json::json!({"query": "status"}),
            },
            scope: project,
            cost_estimate: None,
            requested_at: 1,
            correlation_id: CorrelationId::new("corr_usage"),
        }
    }

    #[test]
    fn project_rows_are_sorted() {
        let mut snapshot = HashMap::new();
        snapshot.insert(sample_project("b"), cairn_store::UsageCounters::default());
        snapshot.insert(sample_project("a"), cairn_store::UsageCounters::default());

        let rows = project_rows(snapshot);
        assert_eq!(rows[0].project_id, "a");
        assert_eq!(rows[1].project_id, "b");
    }

    #[test]
    fn sandbox_sink_counts_provision_events() {
        let store = Arc::new(InMemoryStore::new());
        let inner = Arc::new(BufferedSandboxEventSink::default());
        let sink = UsageSandboxEventSink::new(store.clone(), inner.clone());
        let project = sample_project("sandbox");

        sink.publish(SandboxEvent::SandboxProvisioned {
            sandbox_id: cairn_workspace::SandboxId::new("sbx_usage"),
            run_id: RunId::new("run_usage"),
            task_id: Some(TaskId::new("task_usage")),
            project: project.clone(),
            strategy: SandboxStrategy::Overlay,
            base_revision: Some("main".to_string()),
            policy: sample_policy(),
            path: PathBuf::from("/tmp/sbx_usage"),
            duration_ms: 5,
            provisioned_at: 10,
        });

        let snapshot = store.usage_snapshot();
        assert_eq!(
            snapshot
                .get(&project)
                .expect("project counters should exist")
                .sandbox_provision_count,
            1
        );
        assert_eq!(inner.drain().len(), 1);
    }

    #[tokio::test]
    async fn decision_wrapper_counts_successful_evaluations() {
        let store = Arc::new(InMemoryStore::new());
        let project = sample_project("decision");
        let service =
            UsageMeteredDecisionService::new(Arc::new(StubDecisionService::new()), store.clone());

        let result = service
            .evaluate(sample_decision_request(project.clone()))
            .await
            .expect("decision should succeed");

        assert!(matches!(
            result.event,
            cairn_domain::decisions::DecisionEvent::DecisionRecorded { .. }
        ));
        assert_eq!(
            store
                .usage_snapshot()
                .get(&project)
                .expect("project counters should exist")
                .decision_evaluation_count,
            1
        );
    }

    #[test]
    fn trigger_fire_usage_counts_only_fired_events() {
        let store = InMemoryStore::new();
        let project = sample_project("trigger");

        record_trigger_fire_usage(
            &store,
            &project,
            &[
                TriggerEvent::TriggerFired {
                    trigger_id: TriggerId::new("trigger_fire"),
                    signal_id: SignalId::new("signal_fire"),
                    signal_type: "signal.test".to_string(),
                    run_id: RunId::new("run_fire"),
                    chain_depth: 1,
                    fired_at: 100,
                },
                TriggerEvent::TriggerSkipped {
                    trigger_id: TriggerId::new("trigger_skip"),
                    signal_id: SignalId::new("signal_skip"),
                    reason: cairn_runtime::services::trigger_service::SkipReason::ConditionMismatch,
                    skipped_at: 101,
                },
            ],
        );

        assert_eq!(
            store
                .usage_snapshot()
                .get(&project)
                .expect("project counters should exist")
                .trigger_fire_count,
            1
        );
    }

    #[tokio::test]
    async fn usage_route_returns_store_snapshot() {
        let state = Arc::new(AppState::new(BootstrapConfig::default()).await.unwrap());
        let project = sample_project("usage-api");

        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_usage"),
                EventSource::Runtime,
                cairn_domain::RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: cairn_domain::SessionId::new("sess_usage"),
                    run_id: RunId::new("run_usage"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            )])
            .await
            .expect("event append should succeed");
        state
            .runtime
            .store
            .increment_sandbox_provision_count(&project);
        state
            .runtime
            .store
            .increment_decision_evaluation_count(&project);
        state.runtime.store.increment_trigger_fire_count(&project);

        let app = Router::new()
            .route("/v1/telemetry/usage", get(get_usage_telemetry_handler))
            .with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/telemetry/usage")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("route should respond");

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        let payload: UsageTelemetryResponse =
            serde_json::from_slice(&body).expect("payload should deserialize");
        let row = payload
            .projects
            .iter()
            .find(|row| row.project_id == "usage-api")
            .expect("usage row should exist");

        assert_eq!(row.run_count, 1);
        assert_eq!(row.event_count, 1);
        assert_eq!(row.sandbox_provision_count, 1);
        assert_eq!(row.decision_evaluation_count, 1);
        assert_eq!(row.trigger_fire_count, 1);
    }
}
