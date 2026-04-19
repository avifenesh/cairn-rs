//! Provider health, budget, binding, pool, connection, route policy,
//! and guardrail handlers.
//!
//! Extracted from `lib.rs` — contains all provider-related CRUD and
//! operational endpoints.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use utoipa::ToSchema;

use cairn_api::http::{ApiError, ListResponse};
use cairn_domain::policy::{GuardrailRule, GuardrailSubjectType};
use cairn_domain::providers::{
    OperationKind, ProviderBudget, ProviderBudgetPeriod, ProviderHealthRecord, RoutePolicyRule,
};
use cairn_domain::{
    ProjectKey, ProviderBindingId, ProviderConnectionId, ProviderModelId, TenantId,
};
use cairn_runtime::{
    BudgetService, CredentialService, DefaultsService, GuardrailService, ProviderBindingService,
    ProviderConnectionService, ProviderHealthService, RoutePolicyService,
};
use cairn_store::projections::RoutePolicyReadModel;
use cairn_store::EventLog;

use crate::errors::{
    now_ms, require_feature, runtime_error_response, store_error_response, AppApiError,
};
use crate::extractors::TenantScope;
use crate::state::AppState;

const DEFAULT_TENANT_ID: &str = "default_tenant";

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct TenantScopedQuery {
    pub tenant_id: String,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Clone, Debug, Default, serde::Deserialize)]
pub(crate) struct OptionalTenantScopedQuery {
    pub tenant_id: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl OptionalTenantScopedQuery {
    pub(crate) fn tenant_id(&self) -> &str {
        self.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID)
    }
}

#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
pub(crate) struct CreateProviderConnectionRequest {
    pub tenant_id: String,
    pub provider_connection_id: String,
    pub provider_family: String,
    pub adapter_type: String,
    #[serde(default)]
    pub supported_models: Vec<String>,
    #[serde(default)]
    pub credential_id: Option<String>,
    #[serde(default)]
    pub endpoint_url: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct ManualProviderHealthCheckRequest {
    pub latency_ms: Option<u64>,
    pub success: bool,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SetProviderHealthScheduleRequest {
    pub interval_ms: u64,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SetProviderBudgetRequest {
    pub tenant_id: String,
    pub period: ProviderBudgetPeriod,
    pub limit_micros: u64,
    pub alert_threshold_percent: u32,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct SetProviderRetryPolicyRequest {
    pub max_attempts: u32,
    pub backoff_ms: u64,
    pub retryable_error_classes: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateProviderPoolRequest {
    pub pool_id: String,
    pub max_connections: u32,
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct AddPoolConnectionRequest {
    pub connection_id: String,
}

#[derive(serde::Deserialize)]
pub(crate) struct UpdateProviderConnectionRequest {
    pub provider_family: String,
    pub adapter_type: String,
    pub supported_models: Vec<String>,
    pub endpoint_url: Option<String>,
    pub credential_id: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CostRankingQuery {
    pub tenant_id: Option<String>,
}

#[derive(Clone, Debug, serde::Deserialize, ToSchema)]
pub(crate) struct CreateProviderBindingRequest {
    pub tenant_id: String,
    pub workspace_id: String,
    pub project_id: String,
    pub provider_connection_id: String,
    #[schema(value_type = String)]
    pub operation_kind: OperationKind,
    pub provider_model_id: String,
    pub estimated_cost_micros: Option<u64>,
}

impl CreateProviderBindingRequest {
    pub(crate) fn project(&self) -> ProjectKey {
        ProjectKey::new(
            self.tenant_id.as_str(),
            self.workspace_id.as_str(),
            self.project_id.as_str(),
        )
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) struct CreateRoutePolicyRuleRequest {
    #[serde(default)]
    pub rule_id: String,
    #[serde(default)]
    pub policy_id: String,
    #[serde(default)]
    pub priority: u32,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub capability: Option<String>,
    #[serde(default)]
    pub preferred_model_ids: Vec<String>,
    #[serde(default)]
    pub fallback_model_ids: Vec<String>,
    #[serde(default)]
    pub max_cost_micros: Option<u64>,
    #[serde(default)]
    pub require_provider_ids: Vec<String>,
}

impl From<CreateRoutePolicyRuleRequest> for RoutePolicyRule {
    fn from(r: CreateRoutePolicyRuleRequest) -> Self {
        Self {
            rule_id: if r.rule_id.is_empty() {
                r.capability.clone().unwrap_or_else(|| "rule".to_owned())
            } else {
                r.rule_id
            },
            policy_id: r.policy_id,
            priority: r.priority,
            description: r.description.or(r.capability),
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateRoutePolicyRequest {
    pub tenant_id: String,
    pub name: String,
    pub rules: Vec<CreateRoutePolicyRuleRequest>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct CreateGuardrailPolicyRequest {
    pub tenant_id: String,
    pub name: String,
    pub rules: Vec<GuardrailRule>,
}

#[derive(Clone, Debug, serde::Deserialize)]
pub(crate) struct EvaluateGuardrailPolicyRequest {
    pub tenant_id: String,
    pub subject_type: GuardrailSubjectType,
    pub subject_id: Option<String>,
    pub action: String,
}

// ── Handlers ────────────────────────────────────────────────────────────────

pub(crate) async fn list_provider_health_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TenantScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .provider_health
        .list(
            &TenantId::new(query.tenant_id),
            query.limit.unwrap_or(100),
            query.offset.unwrap_or(0),
        )
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse::<ProviderHealthRecord> {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_provider_budgets_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TenantScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .budgets
        .list_budgets(&TenantId::new(query.tenant_id))
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse::<ProviderBudget> {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn set_provider_budget_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SetProviderBudgetRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .budgets
        .set_budget(
            TenantId::new(body.tenant_id),
            body.period,
            body.limit_micros,
            body.alert_threshold_percent,
        )
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn manual_provider_health_check_handler(
    State(state): State<Arc<AppState>>,
    _role: crate::extractors::AdminRoleGuard,
    Path(connection_id): Path<String>,
    Json(body): Json<ManualProviderHealthCheckRequest>,
) -> impl IntoResponse {
    // T6a-H12: provider health is tenant-shared state. A hostile tenant
    // could otherwise poison the shared provider's health record,
    // triggering fallback routing for every other tenant. Gate on
    // AdminRoleGuard until the service gains a principal-aware
    // `record_check_for_tenant` variant.
    match state
        .runtime
        .provider_health
        .record_check(
            &ProviderConnectionId::new(connection_id),
            body.latency_ms.unwrap_or(0),
            body.success,
        )
        .await
    {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn recover_provider_handler(
    State(state): State<Arc<AppState>>,
    Path(connection_id): Path<String>,
) -> impl IntoResponse {
    match state
        .runtime
        .provider_health
        .mark_recovered(&ProviderConnectionId::new(connection_id))
        .await
    {
        Ok(record) => (StatusCode::OK, Json(record)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn set_provider_health_schedule_handler(
    State(state): State<Arc<AppState>>,
    Path(connection_id): Path<String>,
    Json(body): Json<SetProviderHealthScheduleRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .provider_health
        .schedule_health_check(&ProviderConnectionId::new(connection_id), body.interval_ms)
        .await
    {
        Ok(schedule) => (StatusCode::OK, Json(schedule)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn get_provider_health_schedule_handler(
    State(state): State<Arc<AppState>>,
    Path(connection_id): Path<String>,
) -> impl IntoResponse {
    use cairn_store::projections::ProviderHealthScheduleReadModel;
    match ProviderHealthScheduleReadModel::get_schedule(
        state.runtime.store.as_ref(),
        &connection_id,
    )
    .await
    {
        Ok(Some(schedule)) => (StatusCode::OK, Json(schedule)).into_response(),
        Ok(None) => AppApiError::new(StatusCode::NOT_FOUND, "not_found", "schedule not found")
            .into_response(),
        Err(err) => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "store_error",
            err.to_string(),
        )
        .into_response(),
    }
}

pub(crate) async fn set_provider_retry_policy_handler(
    State(state): State<Arc<AppState>>,
    tenant_scope: TenantScope,
    Path(connection_id): Path<String>,
    Json(body): Json<SetProviderRetryPolicyRequest>,
) -> impl IntoResponse {
    use cairn_domain::{providers::RetryPolicy, ProviderRetryPolicySet, RuntimeEvent};
    let event = cairn_runtime::make_envelope(RuntimeEvent::ProviderRetryPolicySet(
        ProviderRetryPolicySet {
            connection_id: ProviderConnectionId::new(connection_id),
            tenant_id: tenant_scope.tenant_id().clone(),
            policy: RetryPolicy {
                max_attempts: body.max_attempts,
                backoff_ms: body.backoff_ms,
                retryable_error_classes: body.retryable_error_classes,
            },
            set_at_ms: now_ms(),
        },
    ));
    match state.runtime.store.append(&[event]).await {
        Ok(_) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn run_provider_health_checks_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match state.runtime.provider_health.run_due_health_checks().await {
        Ok(records) => (
            StatusCode::OK,
            Json(ListResponse::<ProviderHealthRecord> {
                items: records,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn create_provider_pool_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProviderPoolRequest>,
) -> impl IntoResponse {
    use cairn_runtime::ProviderConnectionPoolService;
    let tenant_id = TenantId::new(body.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID));
    match state
        .runtime
        .provider_pools
        .create_pool(tenant_id, body.pool_id, body.max_connections)
        .await
    {
        Ok(pool) => (StatusCode::CREATED, Json(pool)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_provider_pools_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TenantScopedQuery>,
) -> impl IntoResponse {
    use cairn_runtime::ProviderConnectionPoolService;
    match state
        .runtime
        .provider_pools
        .list_pools(&TenantId::new(query.tenant_id))
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn add_pool_connection_handler(
    State(state): State<Arc<AppState>>,
    Path(pool_id): Path<String>,
    Json(body): Json<AddPoolConnectionRequest>,
) -> impl IntoResponse {
    use cairn_runtime::ProviderConnectionPoolService;
    match state
        .runtime
        .provider_pools
        .add_connection(&pool_id, ProviderConnectionId::new(body.connection_id))
        .await
    {
        Ok(pool) => (StatusCode::CREATED, Json(pool)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn remove_pool_connection_handler(
    State(state): State<Arc<AppState>>,
    Path((pool_id, conn_id)): Path<(String, String)>,
) -> impl IntoResponse {
    use cairn_runtime::ProviderConnectionPoolService;
    match state
        .runtime
        .provider_pools
        .remove_connection(&pool_id, &ProviderConnectionId::new(conn_id))
        .await
    {
        Ok(pool) => (StatusCode::OK, Json(pool)).into_response(),
        Err(err) => runtime_error_response(err),
    }
}

pub(crate) async fn list_provider_connections_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TenantScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .provider_connections
        .list(
            &TenantId::new(query.tenant_id),
            query.limit.unwrap_or(100),
            query.offset.unwrap_or(0),
        )
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn provider_registry_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let snapshot = state.runtime.provider_registry.snapshot();
    let catalog = static_provider_registry_catalog();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "connections": snapshot.connections,
            "fallbacks": snapshot.fallbacks,
            "catalog": catalog,
        })),
    )
        .into_response()
}

pub(crate) fn static_provider_registry_catalog() -> Vec<serde_json::Value> {
    cairn_domain::provider_registry::all()
        .iter()
        .map(|provider| {
            serde_json::json!({
                "id": provider.id,
                "name": provider.name,
                "api_base": provider.api_base,
                "api_format": format!("{:?}", provider.api_format).to_lowercase(),
                "default_model": provider.default_model,
                "available": provider.is_available(),
                "requires_key": provider.requires_key(),
                "env_keys": provider.env_keys,
                "models": provider.models.iter().map(|model| serde_json::json!({
                    "id": model.id,
                    "context_window": model.context_window,
                    "capabilities": {
                        "streaming": model.capabilities.streaming,
                        "tool_use": model.capabilities.tool_use,
                        "vision": model.capabilities.vision,
                        "thinking": model.capabilities.thinking,
                    },
                    "input_cost_per_1m": model.input_cost_per_1m,
                    "output_cost_per_1m": model.output_cost_per_1m,
                })).collect::<Vec<_>>(),
            })
        })
        .collect()
}

#[utoipa::path(
    post,
    path = "/v1/providers/connections",
    tag = "providers",
    request_body = CreateProviderConnectionRequest,
    responses(
        (status = 201, description = "Provider connection created", body = crate::ProviderConnectionRecordDoc),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 422, description = "Unprocessable entity", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError)
    )
)]
pub(crate) async fn create_provider_connection_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProviderConnectionRequest>,
) -> impl IntoResponse {
    use cairn_domain::MULTI_PROVIDER;
    use cairn_runtime::ProviderConnectionConfig;

    if let Some(denied) = require_feature(&state.config, MULTI_PROVIDER) {
        return denied;
    }

    let before = crate::handlers::sse::current_event_head(&state).await;
    let conn_id = body.provider_connection_id.clone();
    let credential_id = body.credential_id.clone();
    let endpoint_url = body.endpoint_url.clone();

    match state
        .runtime
        .provider_connections
        .create(
            TenantId::new(body.tenant_id),
            ProviderConnectionId::new(body.provider_connection_id),
            ProviderConnectionConfig {
                provider_family: body.provider_family,
                adapter_type: body.adapter_type,
                supported_models: body.supported_models,
            },
        )
        .await
    {
        Ok(record) => {
            if let Some(cred_id) = credential_id {
                let key = format!("provider_credential_{conn_id}");
                let _ = state
                    .runtime
                    .defaults
                    .set(
                        cairn_domain::Scope::System,
                        "system".to_owned(),
                        key,
                        serde_json::json!(cred_id),
                    )
                    .await;
            }
            if let Some(url) = endpoint_url {
                let key = format!("provider_endpoint_{conn_id}");
                let _ = state
                    .runtime
                    .defaults
                    .set(
                        cairn_domain::Scope::System,
                        "system".to_owned(),
                        key,
                        serde_json::json!(url),
                    )
                    .await;
            }
            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            (StatusCode::CREATED, Json(record)).into_response()
        }
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn resolve_provider_key_handler(
    State(state): State<Arc<AppState>>,
    Path(connection_id): Path<String>,
) -> impl IntoResponse {
    let conn_id = ProviderConnectionId::new(&connection_id);
    let _connection = match state.runtime.provider_connections.get(&conn_id).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return AppApiError::new(
                StatusCode::NOT_FOUND,
                "not_found",
                "provider connection not found",
            )
            .into_response();
        }
        Err(err) => return runtime_error_response(err),
    };

    let credential_id_str = connection_id.as_str();
    let cred_key = format!("provider_credential_{credential_id_str}");
    let system_project = cairn_domain::ProjectKey::new("system", "system", "system");
    match state.runtime.defaults.resolve(&system_project, &cred_key).await {
        Ok(Some(setting)) => {
            if let Some(cred_id) = setting.as_str() {
                let credential_id = cairn_domain::CredentialId::new(cred_id);
                match state.runtime.credentials.get(&credential_id).await {
                    Ok(Some(record)) if record.active => {
                        (StatusCode::OK, Json(serde_json::json!({
                            "connection_id": connection_id,
                            "credential_id": cred_id,
                            "has_key": true,
                            "provider_id": record.provider_id,
                        }))).into_response()
                    }
                    Ok(Some(_)) => {
                        AppApiError::new(StatusCode::GONE, "credential_revoked", "linked credential has been revoked")
                            .into_response()
                    }
                    Ok(None) => {
                        AppApiError::new(StatusCode::NOT_FOUND, "credential_not_found", "linked credential not found")
                            .into_response()
                    }
                    Err(err) => runtime_error_response(err),
                }
            } else {
                AppApiError::new(StatusCode::NOT_FOUND, "no_credential", "no credential linked to this connection")
                    .into_response()
            }
        }
        _ => {
            AppApiError::new(StatusCode::NOT_FOUND, "no_credential", "no credential linked to this connection — store API key via POST /v1/admin/tenants/:id/credentials then link it")
                .into_response()
        }
    }
}

pub(crate) async fn update_provider_connection_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateProviderConnectionRequest>,
) -> impl IntoResponse {
    use cairn_runtime::ProviderConnectionConfig;

    let conn_id = ProviderConnectionId::new(&id);
    let before = crate::handlers::sse::current_event_head(&state).await;

    let config = ProviderConnectionConfig {
        provider_family: body.provider_family,
        adapter_type: body.adapter_type,
        supported_models: body.supported_models,
    };

    match state
        .runtime
        .provider_connections
        .update(&conn_id, config)
        .await
    {
        Ok(record) => {
            if let Some(url) = body.endpoint_url {
                let key = format!("provider_endpoint_{id}");
                let _ = state
                    .runtime
                    .defaults
                    .set(
                        cairn_domain::Scope::System,
                        "system".to_owned(),
                        key,
                        serde_json::json!(url),
                    )
                    .await;
            }
            if let Some(cred_id) = body.credential_id {
                let key = format!("provider_credential_{id}");
                let _ = state
                    .runtime
                    .defaults
                    .set(
                        cairn_domain::Scope::System,
                        "system".to_owned(),
                        key,
                        serde_json::json!(cred_id),
                    )
                    .await;
            }
            crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
            (StatusCode::OK, Json(record)).into_response()
        }
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn delete_provider_connection_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let conn_id = ProviderConnectionId::new(&id);
    let before = crate::handlers::sse::current_event_head(&state).await;
    match state.runtime.provider_connections.get(&conn_id).await {
        Ok(Some(_)) => {
            let event = cairn_domain::EventEnvelope::for_runtime_event(
                cairn_domain::EventId::new(format!("evt_del_conn_{}", now_ms())),
                cairn_domain::EventSource::Runtime,
                cairn_domain::RuntimeEvent::ProviderConnectionRegistered(
                    cairn_domain::events::ProviderConnectionRegistered {
                        tenant: cairn_domain::tenancy::TenantKey::new("default"),
                        provider_connection_id: conn_id,
                        provider_family: String::new(),
                        adapter_type: String::new(),
                        supported_models: vec![],
                        status: cairn_domain::providers::ProviderConnectionStatus::Disabled,
                        registered_at: now_ms(),
                    },
                ),
            );
            match state.runtime.store.append(&[event]).await {
                Ok(_) => {
                    crate::handlers::sse::publish_runtime_frames_since(&state, before).await;
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({ "deleted": true, "connection_id": id })),
                    )
                        .into_response()
                }
                Err(err) => store_error_response(err),
            }
        }
        Ok(None) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "provider connection not found",
        )
        .into_response(),
        Err(err) => runtime_error_response(err),
    }
}

#[utoipa::path(
    get,
    path = "/v1/providers/bindings",
    tag = "providers",
    responses(
        (status = 200, description = "Provider bindings listed", body = crate::ProviderBindingListResponseDoc),
        (status = 400, description = "Invalid request", body = ApiError),
        (status = 401, description = "Unauthorized", body = ApiError),
        (status = 500, description = "Internal server error", body = ApiError)
    )
)]
pub(crate) async fn list_provider_bindings_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<OptionalTenantScopedQuery>,
) -> impl IntoResponse {
    match state
        .runtime
        .provider_bindings
        .list(
            &TenantId::new(query.tenant_id()),
            query.limit.unwrap_or(100),
            query.offset.unwrap_or(0),
        )
        .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn get_binding_cost_stats_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    use cairn_store::projections::ProviderBindingCostStatsReadModel;
    match ProviderBindingCostStatsReadModel::get(
        state.runtime.store.as_ref(),
        &ProviderBindingId::new(id),
    )
    .await
    {
        Ok(Some(stats)) => (StatusCode::OK, Json(stats)).into_response(),
        Ok(None) => AppApiError::new(
            StatusCode::NOT_FOUND,
            "not_found",
            "no cost stats for binding",
        )
        .into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn list_binding_cost_ranking_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<CostRankingQuery>,
) -> impl IntoResponse {
    use cairn_store::projections::ProviderBindingCostStatsReadModel;
    let tenant_id = TenantId::new(query.tenant_id.as_deref().unwrap_or(DEFAULT_TENANT_ID));
    match ProviderBindingCostStatsReadModel::list_by_tenant(
        state.runtime.store.as_ref(),
        &tenant_id,
    )
    .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => store_error_response(err),
    }
}

pub(crate) async fn create_provider_binding_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateProviderBindingRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .provider_bindings
        .create(
            body.project(),
            ProviderConnectionId::new(body.provider_connection_id),
            body.operation_kind,
            ProviderModelId::new(body.provider_model_id),
            body.estimated_cost_micros,
        )
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn list_route_policies_handler(
    State(state): State<Arc<AppState>>,
    Query(query): Query<TenantScopedQuery>,
) -> impl IntoResponse {
    match RoutePolicyReadModel::list_by_tenant(
        state.runtime.store.as_ref(),
        &TenantId::new(query.tenant_id),
        query.limit.unwrap_or(100),
        query.offset.unwrap_or(0),
    )
    .await
    {
        Ok(items) => (
            StatusCode::OK,
            Json(ListResponse {
                items,
                has_more: false,
            }),
        )
            .into_response(),
        Err(err) => {
            tracing::error!("list_route_policies failed: {err}");
            AppApiError::new(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                err.to_string(),
            )
            .into_response()
        }
    }
}

pub(crate) async fn create_route_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateRoutePolicyRequest>,
) -> impl IntoResponse {
    let domain_rules: Vec<RoutePolicyRule> = body.rules.into_iter().map(Into::into).collect();
    match state
        .runtime
        .route_policies
        .create(TenantId::new(body.tenant_id), body.name, domain_rules)
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn create_guardrail_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateGuardrailPolicyRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .guardrails
        .create_policy(TenantId::new(body.tenant_id), body.name, body.rules)
        .await
    {
        Ok(record) => (StatusCode::CREATED, Json(record)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

pub(crate) async fn evaluate_guardrail_policy_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<EvaluateGuardrailPolicyRequest>,
) -> impl IntoResponse {
    match state
        .runtime
        .guardrails
        .evaluate(
            TenantId::new(body.tenant_id),
            body.subject_type,
            body.subject_id,
            body.action,
        )
        .await
    {
        Ok(decision) => (StatusCode::OK, Json(decision)).into_response(),
        Err(err) => AppApiError::new(StatusCode::BAD_REQUEST, "bad_request", err.to_string())
            .into_response(),
    }
}

#[allow(dead_code)]
pub(crate) async fn check_provider_health_handler(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let _ = state.runtime.provider_health.run_due_health_checks().await;

    let tenant_id = TenantId::new(DEFAULT_TENANT_ID);
    match state
        .runtime
        .provider_health
        .list(&tenant_id, 1000, 0)
        .await
    {
        Ok(records) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "items":    records,
                "has_more": false,
                "checked_at_ms": std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
            })),
        )
            .into_response(),
        Err(err) => runtime_error_response(err),
    }
}
