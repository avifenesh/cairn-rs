//! HTTP error response helpers and utility functions.

use axum::extract::rejection::JsonRejection;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use cairn_api::bootstrap::{BootstrapConfig, DeploymentMode, StorageBackend};
use cairn_api::http::ApiError;
use cairn_domain::tool_invocation::ToolInvocationState;
use cairn_domain::{
    DefaultFeatureGate, Entitlement, EntitlementSet, EventEnvelope, EventId, EventSource,
    FeatureGate, FeatureGateResult, OperatorId, ProductTier, ProjectId, RunState, RuntimeEvent,
    SessionState, TaskState, TenantId,
};
use cairn_evals::EvalRunService as ProductEvalRunService;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct AppApiError {
    pub(crate) status: StatusCode,
    pub(crate) error: ApiError,
}

impl AppApiError {
    pub(crate) fn new(
        status: StatusCode,
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            error: ApiError {
                status_code: status.as_u16(),
                code: code.into(),
                message: message.into(),
                request_id: None,
            },
        }
    }
}

impl IntoResponse for AppApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.error)).into_response()
    }
}

pub(crate) fn unauthorized_api_error() -> AppApiError {
    AppApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "unauthorized")
}

pub(crate) fn tenant_scope_mismatch_error() -> AppApiError {
    AppApiError::new(
        StatusCode::FORBIDDEN,
        "tenant_scope_mismatch",
        "requested project does not belong to authenticated tenant",
    )
}

pub(crate) fn query_rejection_error(message: impl Into<String>) -> AppApiError {
    AppApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        message,
    )
}

pub(crate) fn forbidden_api_error(message: impl Into<String>) -> AppApiError {
    AppApiError::new(StatusCode::FORBIDDEN, "forbidden", message)
}

pub(crate) fn validation_error_response(message: impl Into<String>) -> Response {
    AppApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        message,
    )
    .into_response()
}

pub(crate) fn bad_request_response(message: impl Into<String>) -> axum::response::Response {
    validation_error_response(message)
}

pub(crate) fn memory_api_error_response(err: String) -> Response {
    if err.starts_with("memory not found:") {
        return AppApiError::new(StatusCode::NOT_FOUND, "not_found", err).into_response();
    }

    if err.starts_with("invalid memory status:") {
        return bad_request_response(err);
    }

    AppApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", err).into_response()
}

pub(crate) fn runtime_error_response(err: cairn_runtime::RuntimeError) -> axum::response::Response {
    match err {
        cairn_runtime::RuntimeError::NotFound { .. } => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", err.to_string()).into_response()
        }
        cairn_runtime::RuntimeError::Conflict { .. } => {
            AppApiError::new(StatusCode::CONFLICT, "conflict", err.to_string()).into_response()
        }
        cairn_runtime::RuntimeError::DependencyConflict { .. } => AppApiError::new(
            StatusCode::CONFLICT,
            "dependency_conflict",
            err.to_string(),
        )
        .into_response(),
        cairn_runtime::RuntimeError::PolicyDenied { .. } => {
            AppApiError::new(StatusCode::FORBIDDEN, "permission_denied", err.to_string())
                .into_response()
        }
        cairn_runtime::RuntimeError::QuotaExceeded { .. } => AppApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "quota_exceeded",
            err.to_string(),
        )
        .into_response(),
        cairn_runtime::RuntimeError::InvalidTransition { .. }
        | cairn_runtime::RuntimeError::LeaseExpired { .. }
        | cairn_runtime::RuntimeError::Validation { .. } => {
            validation_error_response(err.to_string())
        }
        cairn_runtime::RuntimeError::Store(store_err) => store_error_response(store_err),
        cairn_runtime::RuntimeError::Internal(_) => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            err.to_string(),
        )
        .into_response(),
    }
}

pub(crate) fn store_error_response(err: cairn_store::StoreError) -> Response {
    match err {
        cairn_store::StoreError::NotFound { .. } => {
            AppApiError::new(StatusCode::NOT_FOUND, "not_found", err.to_string()).into_response()
        }
        cairn_store::StoreError::Conflict { .. } => {
            AppApiError::new(StatusCode::CONFLICT, "conflict", err.to_string()).into_response()
        }
        cairn_store::StoreError::Connection(_)
        | cairn_store::StoreError::Migration(_)
        | cairn_store::StoreError::Serialization(_)
        | cairn_store::StoreError::Internal(_) => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            err.to_string(),
        )
        .into_response(),
    }
}

pub(crate) fn json_rejection_response(err: JsonRejection) -> Response {
    AppApiError::new(
        StatusCode::UNPROCESSABLE_ENTITY,
        "validation_error",
        err.body_text(),
    )
    .into_response()
}

pub(crate) fn parse_run_state(value: &str) -> Result<RunState, String> {
    serde_json::from_value::<RunState>(serde_json::Value::String(value.to_owned()))
        .map_err(|_| format!("invalid run status: {value}"))
}

pub(crate) fn parse_session_state(value: &str) -> Result<SessionState, String> {
    serde_json::from_value::<SessionState>(serde_json::Value::String(value.to_owned()))
        .map_err(|_| format!("invalid session status: {value}"))
}

pub(crate) fn parse_task_state(value: &str) -> Result<TaskState, String> {
    serde_json::from_value::<TaskState>(serde_json::Value::String(value.to_owned()))
        .map_err(|_| format!("invalid task state: {value}"))
}

pub(crate) fn parse_eval_subject_kind(
    value: &str,
) -> Result<cairn_domain::EvalSubjectKind, String> {
    serde_json::from_value::<cairn_domain::EvalSubjectKind>(serde_json::Value::String(
        value.to_owned(),
    ))
    .map_err(|_| format!("invalid eval subject_kind: {value}"))
}

pub(crate) fn parse_tool_invocation_state(value: &str) -> Result<ToolInvocationState, String> {
    serde_json::from_value::<ToolInvocationState>(serde_json::Value::String(value.to_owned()))
        .map_err(|_| format!("invalid tool invocation state: {value}"))
}

pub(crate) fn latest_eval_score_for_release(
    evals: &ProductEvalRunService,
    release: &cairn_store::projections::PromptReleaseRecord,
) -> Option<f64> {
    let mut runs = evals
        .list_by_project(&ProjectId::new(release.project.project_id.as_str()))
        .into_iter()
        .filter(|run| run.prompt_release_id.as_ref() == Some(&release.prompt_release_id))
        .collect::<Vec<_>>();
    runs.sort_by_key(|run| run.completed_at.unwrap_or(run.created_at));
    runs.into_iter()
        .rev()
        .find_map(|run| run.metrics.task_success_rate)
}

pub(crate) fn deployment_mode_tier(mode: DeploymentMode) -> ProductTier {
    match mode {
        DeploymentMode::Local => ProductTier::LocalEval,
        DeploymentMode::SelfHostedTeam => ProductTier::TeamSelfHosted,
    }
}

/// Build the active EntitlementSet for the current deployment config.
/// Self-hosted team mode gets DeploymentTier by default. Local in-memory dev
/// runs also get DeploymentTier when credentials are available so operator
/// flows can exercise credential management without a paid license.
pub(crate) fn local_dev_deployment_entitlements(config: &BootstrapConfig) -> bool {
    matches!(config.mode, DeploymentMode::Local)
        && matches!(config.storage, StorageBackend::InMemory)
        && config.credentials_available()
}

pub(crate) fn app_entitlements(config: &BootstrapConfig) -> EntitlementSet {
    let tier = deployment_mode_tier(config.mode);
    let base = EntitlementSet::new(TenantId::new("bootstrap"), tier);
    if local_dev_deployment_entitlements(config) {
        return base.with_entitlement(Entitlement::DeploymentTier);
    }
    match config.mode {
        DeploymentMode::SelfHostedTeam => base.with_entitlement(Entitlement::DeploymentTier),
        DeploymentMode::Local => base,
    }
}

/// Check a feature gate, returning a 403 response if the feature is not allowed.
pub(crate) fn require_feature(config: &BootstrapConfig, feature: &str) -> Option<Response> {
    let gate = DefaultFeatureGate::v1_defaults();
    match gate.check(&app_entitlements(config), feature) {
        FeatureGateResult::Allowed => None,
        FeatureGateResult::Denied { reason } | FeatureGateResult::Degraded { reason } => Some(
            (
                StatusCode::FORBIDDEN,
                Json(serde_json::json!({
                    "error": reason,
                    "code": "entitlement_required"
                })),
            )
                .into_response(),
        ),
    }
}

pub(crate) fn deployment_mode_label(mode: DeploymentMode) -> &'static str {
    match mode {
        DeploymentMode::Local => "local",
        DeploymentMode::SelfHostedTeam => "self_hosted_team",
    }
}

pub(crate) fn storage_backend_label(storage: &StorageBackend) -> &'static str {
    match storage {
        StorageBackend::InMemory => "memory",
        StorageBackend::Sqlite { .. } => "sqlite",
        StorageBackend::Postgres { .. } => "postgres",
    }
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) fn operator_event_envelope(payload: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
    EventEnvelope::for_runtime_event(
        EventId::new(format!("evt_operator_{}", Uuid::new_v4())),
        EventSource::Operator {
            operator_id: OperatorId::new("operator_api"),
        },
        payload,
    )
}
