//! Eval handlers: runs, datasets, baselines, rubrics, scorecards, matrices,
//! trend/winner/export/report endpoints, and comparison utilities.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use cairn_domain::policy;
use cairn_domain::{
    EvalRunId, EventEnvelope, EventId, EventSource, ProjectId, ProjectKey, PromptAssetId,
    PromptReleaseId, PromptVersionId, ProviderBindingId, RouteDecisionId, RuntimeEvent, TenantId,
    WorkspaceKey, EVAL_MATRICES,
};
use cairn_evals::{
    EvalMetrics, EvalRun as ProductEvalRun, EvalSubjectKind, GuardrailMatrix,
    PromptComparisonMatrix, ProviderRoutingMatrix, ProviderRoutingRow, RubricDimension,
    SkillHealthMatrix,
};
use cairn_runtime::PromptAssetService;
use cairn_store::projections::PromptReleaseReadModel;
use cairn_store::EventLog;
use std::collections::HashMap;
use std::sync::Arc;

use cairn_api::http::ListResponse;

use crate::{
    bad_request_response, parse_eval_subject_kind, require_feature, runtime_error_response,
    store_error_response, AppApiError, AppState, OptionalProjectScopedQuery, ProjectScopedQuery,
    DEFAULT_TENANT_ID, DEFAULT_WORKSPACE_ID,
};

// ── DTOs ─────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct CreateEvalRunRequest {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    eval_run_id: String,
    subject_kind: String,
    evaluator_type: String,
    prompt_asset_id: Option<String>,
    prompt_version_id: Option<String>,
    prompt_release_id: Option<String>,
    created_by: Option<String>,
    dataset_id: Option<String>,
    /// Optional rubric the operator intends to score this run against.
    /// Validated to exist at create time so that the form cannot submit
    /// dangling references; scoring itself is invoked later via
    /// `POST /v1/evals/runs/:id/score-rubric`.
    rubric_id: Option<String>,
    /// Optional baseline that this run will be compared against.
    /// Validated to exist at create time. Comparison is invoked later
    /// via `POST /v1/evals/runs/:id/compare-baseline`.
    baseline_id: Option<String>,
}

impl CreateEvalRunRequest {
    #[allow(dead_code)]
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CompleteEvalRunRequest {
    metrics: EvalMetrics,
    cost: Option<f64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateEvalDatasetRequest {
    tenant_id: String,
    name: String,
    subject_kind: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateEvalBaselineRequest {
    tenant_id: String,
    name: String,
    prompt_asset_id: String,
    metrics: EvalMetrics,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct AddEvalDatasetEntryRequest {
    input: serde_json::Value,
    expected_output: Option<serde_json::Value>,
    tags: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateEvalRubricRequest {
    tenant_id: String,
    name: String,
    dimensions: Vec<RubricDimension>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ListEvalDatasetsQuery {
    tenant_id: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ScoreEvalRunRequest {
    metrics: EvalMetrics,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ScoreEvalRubricRequest {
    rubric_id: String,
    actual_outputs: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct EvalCompareQuery {
    run_ids: Option<String>,
}

impl EvalCompareQuery {
    fn run_ids(&self) -> Vec<EvalRunId> {
        self.run_ids
            .as_deref()
            .map(crate::parse_csv_values)
            .unwrap_or_default()
            .into_iter()
            .map(EvalRunId::new)
            .collect()
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PromptComparisonMatrixQuery {
    tenant_id: String,
    asset_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct PermissionMatrixQuery {
    tenant_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SkillHealthMatrixQuery {
    tenant_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct MemoryQualityMatrixQuery {
    project_id: String,
    tenant_id: String,
    workspace_id: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct GuardrailMatrixQuery {
    tenant_id: String,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct EvalCompareRow {
    metric: String,
    values: HashMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct EvalCompareResponse {
    run_ids: Vec<String>,
    rows: Vec<EvalCompareRow>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct PromptAssetSummary {
    asset_id: String,
    asset_name: String,
    total_eval_runs: u32,
    latest_task_success_rate: f64,
    /// One of: "improving", "degrading", "stable", "no_data"
    trend: String,
    active_release_id: Option<String>,
    best_eval_run_id: Option<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct EvalDashboard {
    generated_at_ms: u64,
    prompt_assets: Vec<PromptAssetSummary>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct EvalTrendQuery {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    metric: String,
    days: Option<u32>,
}

impl EvalTrendQuery {
    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    fn tenant_id(&self) -> TenantId {
        TenantId::new(self.tenant_id.as_str())
    }

    fn days(&self) -> u32 {
        self.days.unwrap_or(30)
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub(crate) struct EvalWinnerResponse {
    eval_run_id: String,
    prompt_release_id: String,
    prompt_version_id: String,
    task_success_rate: Option<f64>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct EvalExportQuery {
    tenant_id: String,
    workspace_id: String,
    project_id: String,
    format: Option<String>,
}

impl EvalExportQuery {
    fn tenant_id(&self) -> TenantId {
        TenantId::new(self.tenant_id.as_str())
    }

    fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }

    #[allow(dead_code)]
    fn format(&self) -> &str {
        self.format.as_deref().unwrap_or("json")
    }
}

/// `POST /v1/evals/runs/:id/compare-baseline`
/// Compare an eval run against the locked baseline for its prompt asset.
/// Optionally accepts `{baseline_run_id}` in the body; if omitted the service
/// selects the canonical baseline automatically.
#[derive(serde::Deserialize, Default)]
pub(crate) struct CompareEvalBaselineRequest {
    #[allow(dead_code)]
    baseline_run_id: Option<String>, // reserved for future explicit-baseline support
}

// ── Helpers ──────────────────────────────────────────────────────────────────

pub(crate) fn compute_trend(scores: &[f64]) -> &'static str {
    if scores.len() < 2 {
        return "no_data";
    }
    let recent_start = scores.len().saturating_sub(3);
    let previous_end = recent_start;
    let previous_start = previous_end.saturating_sub(3);
    let recent3 = &scores[recent_start..];
    let previous3 = &scores[previous_start..previous_end];
    if previous3.is_empty() {
        return "stable";
    }
    let recent_avg: f64 = recent3.iter().sum::<f64>() / recent3.len() as f64;
    let previous_avg: f64 = previous3.iter().sum::<f64>() / previous3.len() as f64;
    if recent_avg - previous_avg > 0.05 {
        "improving"
    } else if previous_avg - recent_avg > 0.05 {
        "degrading"
    } else {
        "stable"
    }
}

pub(crate) fn eval_metric_rows(run_ids: &[String], runs: &[ProductEvalRun]) -> Vec<EvalCompareRow> {
    type EvalMetricExtractor = fn(&EvalMetrics) -> Option<serde_json::Value>;

    let metrics: [(&str, EvalMetricExtractor); 10] = [
        ("task_success_rate", |m: &EvalMetrics| {
            m.task_success_rate.map(serde_json::Value::from)
        }),
        ("latency_p50_ms", |m: &EvalMetrics| {
            m.latency_p50_ms.map(serde_json::Value::from)
        }),
        ("latency_p99_ms", |m: &EvalMetrics| {
            m.latency_p99_ms.map(serde_json::Value::from)
        }),
        ("cost_per_run", |m: &EvalMetrics| {
            m.cost_per_run.map(serde_json::Value::from)
        }),
        ("policy_pass_rate", |m: &EvalMetrics| {
            m.policy_pass_rate.map(serde_json::Value::from)
        }),
        ("retrieval_hit_at_k", |m: &EvalMetrics| {
            m.retrieval_hit_at_k.map(serde_json::Value::from)
        }),
        ("citation_coverage", |m: &EvalMetrics| {
            m.citation_coverage.map(serde_json::Value::from)
        }),
        ("source_diversity", |m: &EvalMetrics| {
            m.source_diversity.map(serde_json::Value::from)
        }),
        ("retrieval_latency_ms", |m: &EvalMetrics| {
            m.retrieval_latency_ms.map(serde_json::Value::from)
        }),
        ("retrieval_cost", |m: &EvalMetrics| {
            m.retrieval_cost.map(serde_json::Value::from)
        }),
    ];

    metrics
        .into_iter()
        .map(|(name, value_for)| {
            let values = run_ids
                .iter()
                .map(|run_id| {
                    let value = runs
                        .iter()
                        .find(|run| run.eval_run_id.as_str() == run_id)
                        .and_then(|run| value_for(&run.metrics))
                        .unwrap_or(serde_json::Value::Null);
                    (run_id.clone(), value)
                })
                .collect();
            EvalCompareRow {
                metric: name.to_owned(),
                values,
            }
        })
        .collect()
}

// ── Handlers ─────────────────────────────────────────────────────────────────

pub(crate) async fn list_eval_runs_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    let project_id = query.project().project_id;
    let limit = query.limit.unwrap_or(100);
    let offset = query.offset.unwrap_or(0);
    let mut items = state.evals.list_by_project(&project_id);
    let has_more = items.len() > offset.saturating_add(limit);
    items = items.into_iter().skip(offset).take(limit).collect();
    (StatusCode::OK, Json(ListResponse { has_more, items })).into_response()
}

pub(crate) async fn get_eval_run_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.evals.get(&EvalRunId::new(id)) {
        Some(run) => (StatusCode::OK, Json(run)).into_response(),
        None => AppApiError::new(StatusCode::NOT_FOUND, "not_found", "eval run not found")
            .into_response(),
    }
}

pub(crate) async fn list_eval_datasets_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListEvalDatasetsQuery>,
) -> impl IntoResponse {
    let tenant_id = TenantId::new(
        query
            .tenant_id
            .unwrap_or_else(|| DEFAULT_TENANT_ID.to_owned()),
    );
    (
        StatusCode::OK,
        Json(ListResponse {
            items: state.eval_datasets.list(&tenant_id),
            has_more: false,
        }),
    )
        .into_response()
}

pub(crate) async fn create_eval_dataset_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateEvalDatasetRequest>,
) -> impl IntoResponse {
    let subject_kind = match parse_eval_subject_kind(&body.subject_kind) {
        Ok(subject_kind) => subject_kind,
        Err(err) => return bad_request_response(err),
    };
    let dataset =
        state
            .eval_datasets
            .create(TenantId::new(body.tenant_id), body.name, subject_kind);
    (StatusCode::CREATED, Json(dataset)).into_response()
}

pub(crate) async fn get_eval_dataset_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.eval_datasets.get(&id) {
        Some(dataset) => (StatusCode::OK, Json(dataset)).into_response(),
        None => AppApiError::new(StatusCode::NOT_FOUND, "not_found", "eval dataset not found")
            .into_response(),
    }
}

pub(crate) async fn add_eval_dataset_entry_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<AddEvalDatasetEntryRequest>,
) -> impl IntoResponse {
    match state
        .eval_datasets
        .add_entry(&id, body.input, body.expected_output, body.tags)
    {
        Ok(dataset) => (StatusCode::CREATED, Json(dataset)).into_response(),
        Err(err) => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", err.to_string()).into_response()
        }
    }
}

pub(crate) async fn list_eval_baselines_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListEvalDatasetsQuery>,
) -> impl IntoResponse {
    let tenant_id = TenantId::new(
        query
            .tenant_id
            .unwrap_or_else(|| DEFAULT_TENANT_ID.to_owned()),
    );
    (
        StatusCode::OK,
        Json(ListResponse {
            items: state.eval_baselines.list(&tenant_id),
            has_more: false,
        }),
    )
        .into_response()
}

pub(crate) async fn list_eval_rubrics_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ListEvalDatasetsQuery>,
) -> impl IntoResponse {
    let tenant_id = TenantId::new(
        query
            .tenant_id
            .unwrap_or_else(|| DEFAULT_TENANT_ID.to_owned()),
    );
    (
        StatusCode::OK,
        Json(ListResponse {
            items: state.eval_rubrics.list(&tenant_id),
            has_more: false,
        }),
    )
        .into_response()
}

pub(crate) async fn create_eval_baseline_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateEvalBaselineRequest>,
) -> impl IntoResponse {
    let baseline = state.eval_baselines.set_baseline(
        TenantId::new(body.tenant_id),
        body.name,
        PromptAssetId::new(body.prompt_asset_id),
        body.metrics,
    );
    (StatusCode::CREATED, Json(baseline)).into_response()
}

pub(crate) async fn get_eval_baseline_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.eval_baselines.get(&id) {
        Some(baseline) => (StatusCode::OK, Json(baseline)).into_response(),
        None => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "eval baseline not found",
        )
        .into_response(),
    }
}

pub(crate) async fn create_eval_rubric_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateEvalRubricRequest>,
) -> impl IntoResponse {
    let rubric =
        state
            .eval_rubrics
            .create(TenantId::new(body.tenant_id), body.name, body.dimensions);
    (StatusCode::CREATED, Json(rubric)).into_response()
}

pub(crate) async fn get_eval_rubric_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.eval_rubrics.get(&id) {
        Some(rubric) => (StatusCode::OK, Json(rubric)).into_response(),
        None => AppApiError::new(StatusCode::NOT_FOUND, "not_found", "eval rubric not found")
            .into_response(),
    }
}

pub(crate) async fn create_eval_run_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateEvalRunRequest>,
) -> impl IntoResponse {
    let domain_subject_kind = match parse_eval_subject_kind(&body.subject_kind) {
        Ok(subject_kind) => subject_kind,
        Err(err) => return bad_request_response(err),
    };
    // Convert cairn_domain::EvalSubjectKind to cairn_evals::EvalSubjectKind via serde.
    let subject_kind: EvalSubjectKind =
        serde_json::from_value(serde_json::to_value(domain_subject_kind).unwrap_or_default())
            .unwrap_or(EvalSubjectKind::PromptRelease);

    // Validate linked artifacts exist AND belong to the request's tenant.
    // Without the tenant check, an operator could bind a run to another
    // tenant's dataset/rubric/baseline simply by guessing its id.
    let request_tenant = body.tenant_id.as_str();
    if let Some(dataset_id) = body.dataset_id.as_deref() {
        match state.eval_datasets.get(dataset_id) {
            Some(d) if d.tenant_id.as_str() == request_tenant => {}
            Some(_) | None => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "eval dataset not found",
                )
                .into_response();
            }
        }
    }
    if let Some(rubric_id) = body.rubric_id.as_deref() {
        match state.eval_rubrics.get(rubric_id) {
            Some(r) if r.tenant_id.as_str() == request_tenant => {}
            Some(_) | None => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "eval rubric not found",
                )
                .into_response();
            }
        }
    }
    if let Some(baseline_id) = body.baseline_id.as_deref() {
        match state.eval_baselines.get(baseline_id) {
            Some(b) if b.tenant_id.as_str() == request_tenant => {}
            Some(_) | None => {
                return AppApiError::new(
                    StatusCode::NOT_FOUND,
                    "not_found",
                    "eval baseline not found",
                )
                .into_response();
            }
        }
    }

    let eval_run_id = EvalRunId::new(body.eval_run_id.clone());
    let project_id_domain = ProjectId::new(body.project_id.clone());
    let project_key = ProjectKey::new(
        body.tenant_id.as_str(),
        body.workspace_id.as_str(),
        body.project_id.as_str(),
    );

    // Idempotency guard (Copilot review on PR #227): the EvalRunStarted event
    // is persisted with a deterministic event_id (`eval_create_<run_id>`), so
    // the event_log's UNIQUE(event_id) constraint would 500 on every legitimate
    // client retry. Detect the duplicate upfront and return the existing run
    // (200). Critically: scope the match by project — a bare eval_run_id is
    // just a string and could collide across tenants, so we refuse to return
    // another tenant's run.
    if let Some(existing) = state.evals.get(&eval_run_id) {
        if existing.project_id == project_id_domain {
            return (StatusCode::OK, Json(existing)).into_response();
        }
        return AppApiError::new(
            StatusCode::CONFLICT,
            "conflict",
            format!(
                "eval_run_id {} already exists in another project",
                eval_run_id.as_str()
            ),
        )
        .into_response();
    }

    // Build the EvalRunStarted event and persist it to the event log FIRST.
    // Event-log is the durable source of truth; the in-memory `EvalRunService`
    // is a projection that `replay_evals` rebuilds on boot. Writing to memory
    // before the event-log would leave divergent state on append-failure and
    // make concurrent retries observe a "half-created" run (Copilot review).
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let ev = EventEnvelope::for_runtime_event(
        EventId::new(format!("eval_create_{}", eval_run_id.as_str())),
        EventSource::Runtime,
        cairn_domain::RuntimeEvent::EvalRunStarted(cairn_domain::events::EvalRunStarted {
            project: project_key,
            eval_run_id: eval_run_id.clone(),
            subject_kind: body.subject_kind.clone(),
            evaluator_type: body.evaluator_type.clone(),
            started_at: now,
            prompt_asset_id: body.prompt_asset_id.as_deref().map(PromptAssetId::new),
            prompt_version_id: body.prompt_version_id.as_deref().map(PromptVersionId::new),
            prompt_release_id: body.prompt_release_id.as_deref().map(PromptReleaseId::new),
            created_by: body
                .created_by
                .as_deref()
                .map(cairn_domain::OperatorId::new),
            // Issue #220 (dataset) + #223 (rubric + baseline): persist bindings
            // so `replay_evals` can restore them on restart. Before this the
            // linkage lived only in the in-memory `EvalsService` and was lost
            // on reboot.
            dataset_id: body.dataset_id.clone(),
            rubric_id: body.rubric_id.clone(),
            baseline_id: body.baseline_id.clone(),
        }),
    );
    if let Err(e) = state.runtime.store.append(&[ev]).await {
        tracing::error!(
            %eval_run_id,
            "failed to persist EvalRunStarted event: {e}"
        );
        return AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            format!("failed to persist eval run: {e}"),
        )
        .into_response();
    }

    // Event log persisted — now mutate the in-memory projection.
    let mut run = state.evals.create_run(
        eval_run_id.clone(),
        project_id_domain,
        subject_kind,
        body.evaluator_type.clone(),
        body.prompt_asset_id.as_deref().map(PromptAssetId::new),
        body.prompt_version_id.as_deref().map(PromptVersionId::new),
        body.prompt_release_id.as_deref().map(PromptReleaseId::new),
        body.created_by
            .as_deref()
            .map(cairn_domain::OperatorId::new),
    );
    // The run was just created above; set_* can only fail if the in-memory
    // projection is inconsistent with the event we just appended. Treat that
    // as an internal error so the response reflects persisted state (Copilot
    // review on PR #227).
    if let Some(dataset_id) = body.dataset_id.as_deref() {
        if let Err(err) = state
            .evals
            .set_dataset_id(&eval_run_id, dataset_id.to_owned())
        {
            tracing::error!(
                %eval_run_id,
                dataset_id = %dataset_id,
                "in-memory set_dataset_id failed after event-log append: {err}"
            );
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                format!("failed to attach dataset to eval run: {err}"),
            )
            .into_response();
        }
    }
    if let Some(rubric_id) = body.rubric_id.as_deref() {
        if let Err(err) = state
            .evals
            .set_rubric_id(&eval_run_id, rubric_id.to_owned())
        {
            tracing::error!(
                %eval_run_id,
                rubric_id = %rubric_id,
                "in-memory set_rubric_id failed after event-log append: {err}"
            );
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                format!("failed to attach rubric to eval run: {err}"),
            )
            .into_response();
        }
    }
    if let Some(baseline_id) = body.baseline_id.as_deref() {
        if let Err(err) = state
            .evals
            .set_baseline_id(&eval_run_id, baseline_id.to_owned())
        {
            tracing::error!(
                %eval_run_id,
                baseline_id = %baseline_id,
                "in-memory set_baseline_id failed after event-log append: {err}"
            );
            return AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                format!("failed to attach baseline to eval run: {err}"),
            )
            .into_response();
        }
    }
    // Re-fetch so the response body reflects any bindings applied above.
    if body.dataset_id.is_some() || body.rubric_id.is_some() || body.baseline_id.is_some() {
        if let Some(updated) = state.evals.get(&eval_run_id) {
            run = updated;
        }
    }

    (StatusCode::CREATED, Json(run)).into_response()
}

pub(crate) async fn start_eval_run_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.evals.start_run(&EvalRunId::new(id)) {
        Ok(run) => (StatusCode::OK, Json(run)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn complete_eval_run_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<CompleteEvalRunRequest>,
) -> impl IntoResponse {
    match state
        .evals
        .complete_run(&EvalRunId::new(id), body.metrics, body.cost)
    {
        Ok(run) => (StatusCode::OK, Json(run)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn score_eval_run_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ScoreEvalRunRequest>,
) -> impl IntoResponse {
    match state.evals.record_score(&EvalRunId::new(id), body.metrics) {
        Ok(run) => (StatusCode::OK, Json(run)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn score_eval_rubric_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<ScoreEvalRubricRequest>,
) -> impl IntoResponse {
    match state
        .eval_rubrics
        .score_against_rubric(&EvalRunId::new(id), &body.rubric_id, &body.actual_outputs)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn compare_eval_baseline_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state
        .eval_baselines
        .compare_to_baseline(&EvalRunId::new(id))
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn get_eval_dashboard_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
) -> impl IntoResponse {
    let project_key = query.project();
    let _workspace_key = WorkspaceKey::new(
        query.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID),
        query
            .workspace_id
            .as_deref()
            .unwrap_or(DEFAULT_WORKSPACE_ID),
    );
    let project_id = project_key.project_id.clone();

    let assets = match state
        .runtime
        .prompt_assets
        .list_by_project(&project_key, 500, 0)
        .await
    {
        Ok(a) => a,
        Err(err) => return runtime_error_response(err),
    };

    let all_runs = state.evals.list_by_project(&project_id);

    let all_releases = match PromptReleaseReadModel::list_by_project(
        state.runtime.store.as_ref(),
        &project_key,
        1000,
        0,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => return store_error_response(err),
    };

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let prompt_assets = assets
        .into_iter()
        .map(|asset| {
            let asset_runs: Vec<_> = all_runs
                .iter()
                .filter(|r| {
                    r.prompt_asset_id.as_ref().map(|id| id.as_str())
                        == Some(asset.prompt_asset_id.as_str())
                })
                .collect();

            let total_eval_runs = asset_runs.len() as u32;

            // Completed runs sorted by completed_at, collecting task_success_rate scores
            let mut completed: Vec<_> = asset_runs
                .iter()
                .filter(|r| r.completed_at.is_some())
                .collect();
            completed.sort_by_key(|r| r.completed_at.unwrap_or(0));

            let scores: Vec<f64> = completed
                .iter()
                .filter_map(|r| r.metrics.task_success_rate)
                .collect();

            let latest_task_success_rate = scores.last().copied().unwrap_or(0.0);
            let trend = compute_trend(&scores).to_owned();

            let best_eval_run_id = completed
                .iter()
                .max_by(|a, b| {
                    a.metrics
                        .task_success_rate
                        .unwrap_or(0.0)
                        .partial_cmp(&b.metrics.task_success_rate.unwrap_or(0.0))
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|r| r.eval_run_id.to_string());

            let active_release_id = all_releases
                .iter()
                .find(|r| {
                    r.prompt_asset_id.as_str() == asset.prompt_asset_id.as_str()
                        && r.state == "active"
                })
                .map(|r| r.prompt_release_id.to_string());

            PromptAssetSummary {
                asset_id: asset.prompt_asset_id.to_string(),
                asset_name: asset.name.clone(),
                total_eval_runs,
                latest_task_success_rate,
                trend,
                active_release_id,
                best_eval_run_id,
            }
        })
        .collect();

    (
        StatusCode::OK,
        Json(EvalDashboard {
            generated_at_ms: now,
            prompt_assets,
        }),
    )
        .into_response()
}

pub(crate) async fn compare_eval_runs_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EvalCompareQuery>,
) -> impl IntoResponse {
    let run_ids = query.run_ids();
    if run_ids.is_empty() {
        return bad_request_response("run_ids is required");
    }

    let mut runs = Vec::new();
    for run_id in &run_ids {
        let Some(run) = state.evals.get(run_id) else {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                format!("eval run not found: {run_id}"),
            )
            .into_response();
        };
        runs.push(run);
    }

    let run_id_strings: Vec<String> = run_ids.iter().map(ToString::to_string).collect();
    let response = EvalCompareResponse {
        rows: eval_metric_rows(&run_id_strings, &runs),
        run_ids: run_id_strings,
    };
    (StatusCode::OK, Json(response)).into_response()
}

pub(crate) async fn get_prompt_comparison_matrix_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PromptComparisonMatrixQuery>,
) -> impl IntoResponse {
    if let Some(denied) = require_feature(&state.config, EVAL_MATRICES) {
        return denied;
    }
    let matrix: PromptComparisonMatrix = state.evals.build_prompt_comparison_matrix(
        &ProjectId::new(query.tenant_id),
        &PromptAssetId::new(query.asset_id),
    );
    (StatusCode::OK, Json(matrix)).into_response()
}

pub(crate) async fn get_permission_matrix_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<PermissionMatrixQuery>,
) -> impl IntoResponse {
    use cairn_evals::matrices::{EvalMetrics, PermissionMatrix, PermissionRow};
    if let Some(denied) = require_feature(&state.config, EVAL_MATRICES) {
        return denied;
    }
    let tenant_id = TenantId::new(query.tenant_id);
    // Build permission rows from stored guardrail policies.
    let policies = match cairn_store::projections::GuardrailReadModel::list_policies(
        state.runtime.store.as_ref(),
        &tenant_id,
        1000,
        0,
    )
    .await
    {
        Ok(p) => p,
        Err(err) => return store_error_response(err),
    };

    let rows: Vec<PermissionRow> = policies
        .iter()
        .flat_map(|policy| {
            policy.rules.iter().map(|rule| {
                let pass_rate = match rule.effect {
                    policy::GuardrailRuleEffect::Allow => 1.0_f64,
                    policy::GuardrailRuleEffect::Deny => 0.0_f64,
                    _ => 0.5_f64,
                };
                PermissionRow {
                    project_id: ProjectId::new(""),
                    policy_id: cairn_domain::PolicyId::new(policy.policy_id.as_str()),
                    mode: format!("{:?}", rule.effect).to_lowercase(),
                    capability: rule.action.clone(),
                    eval_run_id: cairn_domain::EvalRunId::new(""),
                    metrics: EvalMetrics {
                        policy_pass_rate: Some(pass_rate),
                        ..Default::default()
                    },
                }
            })
        })
        .collect();

    (StatusCode::OK, Json(PermissionMatrix { rows })).into_response()
}

pub(crate) async fn get_memory_quality_matrix_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<MemoryQualityMatrixQuery>,
) -> impl IntoResponse {
    use cairn_domain::ProjectKey;
    if let Some(denied) = require_feature(&state.config, EVAL_MATRICES) {
        return denied;
    }
    let project = ProjectKey::new(
        query.tenant_id.as_str(),
        query.workspace_id.as_str(),
        query.project_id.as_str(),
    );
    match state.evals.build_memory_quality_matrix(&project).await {
        Ok(matrix) => (
            StatusCode::OK,
            Json::<cairn_evals::MemorySourceQualityMatrix>(matrix),
        )
            .into_response(),
        Err(err) => {
            tracing::error!("build_memory_quality_matrix failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        }
    }
}

pub(crate) async fn get_guardrail_matrix_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<GuardrailMatrixQuery>,
) -> impl IntoResponse {
    if let Some(denied) = require_feature(&state.config, EVAL_MATRICES) {
        return denied;
    }
    match state
        .evals
        .build_guardrail_matrix(&TenantId::new(query.tenant_id))
        .await
    {
        Ok(matrix) => (StatusCode::OK, Json::<GuardrailMatrix>(matrix)).into_response(),
        Err(err) => {
            tracing::error!("build_guardrail_matrix failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        }
    }
}

pub(crate) async fn get_skill_health_matrix_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SkillHealthMatrixQuery>,
) -> impl IntoResponse {
    if let Some(denied) = require_feature(&state.config, EVAL_MATRICES) {
        return denied;
    }
    match state
        .evals
        .build_skill_health_matrix(&TenantId::new(query.tenant_id))
        .await
    {
        Ok(matrix) => (StatusCode::OK, Json::<SkillHealthMatrix>(matrix)).into_response(),
        Err(err) => {
            tracing::error!("build_skill_health_matrix failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        }
    }
}

pub(crate) async fn get_provider_routing_matrix_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<SkillHealthMatrixQuery>,
) -> impl IntoResponse {
    let tenant_id = TenantId::new(&query.tenant_id);

    // Read the event log to find ProviderCallCompleted events for this tenant.
    let all_events = match state.runtime.store.read_stream(None, 10_000).await {
        Ok(events) => events,
        Err(err) => return store_error_response(err),
    };

    // Aggregate per-binding: (total_cost_micros, success_count, total_count)
    let mut binding_stats: std::collections::HashMap<String, (ProviderBindingId, u64, u64, u64)> =
        std::collections::HashMap::new();

    for stored in &all_events {
        if let RuntimeEvent::ProviderCallCompleted(e) = &stored.envelope.payload {
            if e.project.tenant_id != tenant_id {
                continue;
            }
            let key = e.provider_binding_id.as_str().to_owned();
            let entry = binding_stats
                .entry(key)
                .or_insert_with(|| (e.provider_binding_id.clone(), 0, 0, 0));
            entry.1 += e.cost_micros.unwrap_or(0);
            entry.3 += 1;
            if e.status == cairn_domain::providers::ProviderCallStatus::Succeeded {
                entry.2 += 1;
            }
        }
    }

    if binding_stats.is_empty() {
        return (StatusCode::OK, Json(ProviderRoutingMatrix { rows: vec![] })).into_response();
    }

    // Find the project_id used in the provider calls (to look up eval runs).
    let provider_project_id = all_events.iter().find_map(|e| {
        if let RuntimeEvent::ProviderCallCompleted(ev) = &e.envelope.payload {
            if ev.project.tenant_id == tenant_id {
                return Some(ev.project.project_id.clone());
            }
        }
        None
    });

    // Find the latest eval run for this project to associate with the rows.
    let eval_run_id = provider_project_id
        .and_then(|pid| {
            state
                .evals
                .list_by_project(&pid)
                .into_iter()
                .next()
                .map(|r| r.eval_run_id)
        })
        .unwrap_or_else(|| EvalRunId::new("unknown"));

    let rows: Vec<ProviderRoutingRow> = binding_stats
        .into_values()
        .map(|(binding_id, cost_micros, successes, total)| {
            let success_rate = if total > 0 {
                successes as f64 / total as f64
            } else {
                0.0
            };
            ProviderRoutingRow {
                project_id: cairn_domain::ProjectId::new(&query.tenant_id),
                route_decision_id: RouteDecisionId::new(""),
                provider_binding_id: Some(binding_id),
                eval_run_id: eval_run_id.clone(),
                metrics: EvalMetrics::default(),
                total_cost_micros: cost_micros,
                success_rate,
            }
        })
        .collect();

    (StatusCode::OK, Json(ProviderRoutingMatrix { rows })).into_response()
}

pub(crate) async fn get_scorecard_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalProjectScopedQuery>,
    Path(asset_id): Path<String>,
) -> impl IntoResponse {
    let scorecard = state
        .evals
        .build_scorecard(&query.project().project_id, &PromptAssetId::new(asset_id));
    (StatusCode::OK, Json(scorecard)).into_response()
}

pub(crate) async fn get_eval_asset_trend_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EvalTrendQuery>,
    Path(asset_id): Path<String>,
) -> impl IntoResponse {
    let _project = query.project();
    let metric = query.metric.clone();
    let days = query.days();
    let tenant_id = query.tenant_id();
    match state.evals.get_trend(
        tenant_id.as_str(),
        &PromptAssetId::new(asset_id),
        metric,
        days,
    ) {
        Ok(points) => (StatusCode::OK, Json(points)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn get_eval_asset_winner_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<ProjectScopedQuery>,
    Path(asset_id): Path<String>,
) -> impl IntoResponse {
    let scorecard = state.evals.build_scorecard(
        &ProjectId::new(query.project_id),
        &PromptAssetId::new(asset_id),
    );
    let Some(best) = scorecard.entries.first() else {
        return AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "no completed eval runs for prompt asset",
        )
        .into_response();
    };

    (
        StatusCode::OK,
        Json(EvalWinnerResponse {
            eval_run_id: best.eval_run_id.to_string(),
            prompt_release_id: best.prompt_release_id.to_string(),
            prompt_version_id: best.prompt_version_id.to_string(),
            task_success_rate: best.metrics.task_success_rate,
        }),
    )
        .into_response()
}

pub(crate) async fn get_eval_asset_export_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EvalExportQuery>,
    Path(asset_id): Path<String>,
) -> impl IntoResponse {
    let prompt_asset_id = PromptAssetId::new(asset_id);
    // Export runs for this asset, filtered by project_id from query params.
    let project_id = ProjectId::new(query.project_id.as_str());
    let mut runs_for_asset: Vec<cairn_evals::scorecards::EvalRun> = state
        .evals
        .export_runs(&project_id, 10000)
        .into_iter()
        .filter(|r| r.prompt_asset_id.as_ref() == Some(&prompt_asset_id))
        .collect();
    runs_for_asset.sort_by_key(|r| r.eval_run_id.as_str().to_owned());

    if query.format.as_deref() == Some("csv") {
        let mut csv = String::from(
            "eval_run_id,prompt_release_id,task_success_rate,latency_p50_ms,cost_per_run,completed_at\n",
        );
        for run in &runs_for_asset {
            csv.push_str(&format!(
                "{},{},{},{},{},{}\n",
                run.eval_run_id,
                run.prompt_release_id
                    .as_ref()
                    .map(|r| r.as_str())
                    .unwrap_or(""),
                run.metrics
                    .task_success_rate
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                run.metrics
                    .latency_p50_ms
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                run.metrics
                    .cost_per_run
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                run.completed_at.unwrap_or(0),
            ));
        }
        return (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/csv")],
            csv,
        )
            .into_response();
    }

    (StatusCode::OK, Json(runs_for_asset)).into_response()
}

pub(crate) async fn get_eval_asset_report_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<EvalExportQuery>,
    Path(asset_id): Path<String>,
) -> impl IntoResponse {
    let _project = query.project();
    let report = state
        .evals
        .generate_report(query.tenant_id().as_str(), &PromptAssetId::new(asset_id));
    (StatusCode::OK, Json(report)).into_response()
}

pub(crate) async fn compare_eval_run_baseline_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<Json<CompareEvalBaselineRequest>>,
) -> impl IntoResponse {
    // `baseline_run_id` in the body is accepted for forward-compat but the
    // service currently selects the baseline from the locked asset record.
    let _ = body; // suppress unused warning until explicit-baseline is wired
    match state
        .eval_baselines
        .compare_to_baseline(&EvalRunId::new(id))
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

/// `POST /v1/evals/runs/:id/score-rubric`
/// Score an eval run against a rubric. Identical contract to
/// `score_eval_rubric_handler`; this is the REST-style alias registered at
/// `/v1/evals/runs/:id/score-rubric`.
pub(crate) async fn score_eval_run_with_rubric_handler(
    state: State<Arc<AppState>>,
    path: Path<String>,
    body: Json<ScoreEvalRubricRequest>,
) -> impl IntoResponse {
    score_eval_rubric_handler(state, path, body).await
}
