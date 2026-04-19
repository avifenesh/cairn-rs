//! Decision cache handlers (RFC 019).
//!
//! Extracted from `lib.rs` — contains list/get/invalidate/bulk-invalidate
//! decision cache endpoints and the decision_error_response helper.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::auth::AuthPrincipal;
use cairn_runtime::DecisionService;

use crate::errors::{bad_request_response, AppApiError};
use crate::handlers::admin::audit_actor_id;
use crate::state::AppState;

// ── Helpers ─────────────────────────────────────────────────────────────────

pub(crate) fn decision_error_response(
    err: cairn_runtime::DecisionError,
) -> axum::response::Response {
    match &err {
        cairn_runtime::DecisionError::Internal(_) => AppApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "decision_error",
            err.to_string(),
        )
        .into_response(),
        cairn_runtime::DecisionError::InvalidRequest(_) => {
            AppApiError::new(StatusCode::BAD_REQUEST, "invalid_request", err.to_string())
                .into_response()
        }
    }
}

pub(crate) fn default_project_scope(params: &HashMap<String, String>) -> cairn_domain::ProjectKey {
    cairn_domain::ProjectKey::new(
        params
            .get("tenant_id")
            .map(|s| s.as_str())
            .unwrap_or("default"),
        params
            .get("workspace_id")
            .map(|s| s.as_str())
            .unwrap_or("default"),
        params
            .get("project_id")
            .map(|s| s.as_str())
            .unwrap_or("default"),
    )
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// GET /v1/decisions — list recent decisions.
pub(crate) async fn list_decisions_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(200);
    let scope = default_project_scope(&params);
    match state.runtime.decisions.list_cached(&scope, limit).await {
        Ok(items) => (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response(),
        Err(e) => decision_error_response(e),
    }
}

/// GET /v1/decisions/cache — list active cached decisions (learned rules).
pub(crate) async fn list_decision_cache_handler(
    State(state): State<Arc<AppState>>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(50)
        .min(200);
    let scope = default_project_scope(&params);
    match state.runtime.decisions.list_cached(&scope, limit).await {
        Ok(items) => (StatusCode::OK, Json(serde_json::json!({ "items": items }))).into_response(),
        Err(e) => decision_error_response(e),
    }
}

/// GET /v1/decisions/:id — drill into a specific decision.
pub(crate) async fn get_decision_handler(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    use cairn_domain::DecisionId;
    match state
        .runtime
        .decisions
        .get_decision(&DecisionId::new(id))
        .await
    {
        Ok(Some(event)) => (StatusCode::OK, Json(event)).into_response(),
        Ok(None) => AppApiError::new(StatusCode::NOT_FOUND, "not_found", "decision not found")
            .into_response(),
        Err(e) => decision_error_response(e),
    }
}

/// POST /v1/decisions/:id/invalidate — invalidate a specific cached decision.
pub(crate) async fn invalidate_decision_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(id): Path<String>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use cairn_domain::decisions::ActorRef;
    use cairn_domain::DecisionId;
    let reason = body
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("operator_invalidation")
        .to_owned();
    // T6a-H7: real principal for audit, not the hardcoded "operator".
    match state
        .runtime
        .decisions
        .invalidate(
            &DecisionId::new(id),
            &reason,
            ActorRef::Operator {
                operator_id: cairn_domain::OperatorId::new(audit_actor_id(&principal)),
            },
        )
        .await
    {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({ "invalidated": true })),
        )
            .into_response(),
        Err(e) => decision_error_response(e),
    }
}

/// POST /v1/decisions/invalidate — bulk invalidation by scope.
pub(crate) async fn bulk_invalidate_decisions_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use cairn_domain::decisions::{ActorRef, DecisionScopeRef};
    let scope: DecisionScopeRef = match body.get("scope") {
        Some(s) => match serde_json::from_value(s.clone()) {
            Ok(scope) => scope,
            Err(e) => {
                return bad_request_response(format!("invalid scope: {e}"));
            }
        },
        None => {
            return bad_request_response("missing 'scope' field");
        }
    };
    let kind_filter = body
        .get("kind")
        .and_then(|v| v.as_str())
        .filter(|k| *k != "all");
    let reason = body
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("bulk_invalidation")
        .to_owned();
    match state
        .runtime
        .decisions
        .invalidate_by_scope(
            &scope,
            kind_filter,
            &reason,
            ActorRef::Operator {
                operator_id: cairn_domain::OperatorId::new(audit_actor_id(&principal)),
            },
        )
        .await
    {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({ "invalidated_count": count })),
        )
            .into_response(),
        Err(e) => decision_error_response(e),
    }
}

/// POST /v1/decisions/invalidate-by-rule — selective invalidation via rule ID.
pub(crate) async fn invalidate_by_rule_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use cairn_domain::decisions::ActorRef;
    use cairn_domain::PolicyId;
    let rule_id = match body.get("rule_id").and_then(|v| v.as_str()) {
        Some(id) => PolicyId::new(id),
        None => {
            return bad_request_response("missing 'rule_id' field");
        }
    };
    let reason = body
        .get("reason")
        .and_then(|v| v.as_str())
        .unwrap_or("policy_rule_changed")
        .to_owned();
    match state
        .runtime
        .decisions
        .invalidate_by_rule(&rule_id, &reason, ActorRef::SystemPolicyChange)
        .await
    {
        Ok(count) => (
            StatusCode::OK,
            Json(serde_json::json!({ "invalidated_count": count })),
        )
            .into_response(),
        Err(e) => decision_error_response(e),
    }
}
