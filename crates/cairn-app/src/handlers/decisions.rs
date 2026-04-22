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

/// POST /v1/decisions/evaluate — evaluate a decision request through the
/// full RFC 019 8-step pipeline.
///
/// Wrapper over `DecisionService::evaluate` that exposes the decision
/// layer to operators and integration tests without requiring an
/// in-process orchestrator run. Decisions that end up cached are
/// persisted to the event log so they survive restart (RFC 020
/// §"Decision Cache Survival").
///
/// The request body accepts:
/// ```json
/// {
///   "kind": { "ToolInvocation": { "tool_name": "grep_search",
///                                  "effect": "Observational" } },
///   "principal": { "type": "system" },   // optional, defaults to system
///   "subject": { ... },                  // optional, derived from kind when absent
///   "tenant_id": "default",              // optional scope fields
///   "workspace_id": "default",
///   "project_id": "default",
///   "correlation_id": "..."              // optional, generated when absent
/// }
/// ```
/// The response is `{ "decision_id": ..., "outcome": ..., "source": ..., "cached": bool }`.
pub(crate) async fn evaluate_decision_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal_auth): Extension<AuthPrincipal>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    use cairn_domain::decisions::{DecisionKind, DecisionRequest, DecisionSubject, Principal};
    use cairn_domain::ids::CorrelationId;

    // ── kind (required) ──────────────────────────────────────────────────
    // Accept either the serde-tagged shape (`{ "kind": "...", "data": {...} }`)
    // passed through verbatim, or a flat shape where the caller puts the
    // discriminator and payload fields at the top level (easier for operators
    // and integration tests). We try the strict shape first.
    let kind_val = match body.get("kind") {
        Some(k) => k.clone(),
        None => return bad_request_response("missing 'kind' field"),
    };
    let kind: DecisionKind = match &kind_val {
        serde_json::Value::String(disc) => {
            // Flat shape: `{ "kind": "tool_invocation", "tool_name": ..., "effect": ... }`.
            let data = serde_json::Value::Object(
                body.as_object()
                    .map(|o| {
                        o.iter()
                            .filter(|(k, _)| {
                                !matches!(
                                    k.as_str(),
                                    "kind"
                                        | "principal"
                                        | "subject"
                                        | "tenant_id"
                                        | "workspace_id"
                                        | "project_id"
                                        | "correlation_id"
                                )
                            })
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect()
                    })
                    .unwrap_or_default(),
            );
            let tagged = serde_json::json!({ "kind": disc, "data": data });
            match serde_json::from_value(tagged) {
                Ok(k) => k,
                Err(e) => return bad_request_response(format!("invalid 'kind' payload: {e}")),
            }
        }
        serde_json::Value::Object(_) => match serde_json::from_value(kind_val) {
            Ok(k) => k,
            Err(e) => return bad_request_response(format!("invalid 'kind': {e}")),
        },
        _ => return bad_request_response("'kind' must be a string or object"),
    };

    // ── principal ────────────────────────────────────────────────────────
    // Caller-supplied principal is allowed for backwards-compatibility
    // (e.g. the orchestrator re-POSTing its own `Principal::Run` from a
    // host-internal call), but when omitted we derive it from the
    // authenticated `AuthPrincipal` so operator calls are not silently
    // recorded as `Principal::System`.
    let principal: Principal = match body.get("principal") {
        Some(p) => match serde_json::from_value(p.clone()) {
            Ok(p) => p,
            Err(e) => return bad_request_response(format!("invalid 'principal': {e}")),
        },
        None => match &principal_auth {
            AuthPrincipal::Operator { operator_id, .. } => Principal::Operator {
                operator_id: operator_id.clone(),
            },
            AuthPrincipal::ServiceAccount { .. } | AuthPrincipal::System => Principal::System,
        },
    };

    // ── subject (optional, derive from kind when absent) ─────────────────
    let subject: DecisionSubject = match body.get("subject") {
        Some(s) => match serde_json::from_value(s.clone()) {
            Ok(s) => s,
            Err(e) => return bad_request_response(format!("invalid 'subject': {e}")),
        },
        None => match &kind {
            DecisionKind::ToolInvocation { tool_name, .. } => DecisionSubject::ToolCall {
                tool_name: tool_name.clone(),
                args: serde_json::json!({}),
            },
            DecisionKind::ProviderCall { model_id, .. } => DecisionSubject::ProviderCall {
                model_id: model_id.clone(),
            },
            _ => DecisionSubject::Resource {
                resource_type: "unknown".to_owned(),
                resource_id: "unknown".to_owned(),
            },
        },
    };

    // ── scope ────────────────────────────────────────────────────────────
    // RFC 008 scope ownership: the request's tenant/workspace/project MUST
    // match the authenticated principal's tenant. Anything else would let
    // a caller write cached decisions under another tenant's scope —
    // cross-tenant cache pollution. System principal (no tenant binding)
    // is allowed to target any tenant — it's the runtime itself, not a
    // user-driven request.
    let params: HashMap<String, String> = body
        .as_object()
        .map(|o| {
            o.iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let scope = default_project_scope(&params);
    if let Some(tenant) = principal_auth.tenant() {
        if scope.tenant_id.as_str() != tenant.tenant_id.as_str() {
            return AppApiError::new(
                StatusCode::FORBIDDEN,
                "scope_forbidden",
                format!(
                    "caller tenant '{}' cannot evaluate decisions under tenant '{}'",
                    tenant.tenant_id,
                    scope.tenant_id.as_str()
                ),
            )
            .into_response();
        }
    }

    // ── correlation_id ───────────────────────────────────────────────────
    let correlation_id = body
        .get("correlation_id")
        .and_then(|v| v.as_str())
        .map(|s| CorrelationId::new(s.to_owned()))
        .unwrap_or_else(|| {
            CorrelationId::new(format!(
                "cor_{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis())
                    .unwrap_or(0)
            ))
        });

    let request = DecisionRequest {
        kind,
        principal,
        subject,
        scope,
        cost_estimate: None,
        requested_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        correlation_id,
    };

    match state.runtime.decisions.evaluate(request).await {
        Ok(result) => {
            let cached = matches!(
                result.event,
                cairn_domain::decisions::DecisionEvent::DecisionRecorded {
                    cached_for: Some(_),
                    ..
                }
            );
            let source = match &result.event {
                cairn_domain::decisions::DecisionEvent::DecisionRecorded { source, .. } => {
                    serde_json::to_value(source).unwrap_or(serde_json::Value::Null)
                }
                _ => serde_json::Value::Null,
            };
            let cache_hit = source
                .get("source")
                .and_then(|v| v.as_str())
                .map(|s| s == "cache_hit")
                .unwrap_or(false);
            let original_decision_id = source
                .get("original_decision_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned());
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "decision_id": result.decision_id,
                    "outcome": result.outcome,
                    "source": source,
                    "cached": cached,
                    "cache_hit": cache_hit,
                    "original_decision_id": original_decision_id,
                })),
            )
                .into_response()
        }
        Err(e) => decision_error_response(e),
    }
}

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
