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
