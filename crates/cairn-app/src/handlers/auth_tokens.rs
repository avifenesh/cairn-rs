//! Auth token CRUD handlers.
//!
//! Extracted from `lib.rs` — contains create, list, and delete
//! operator API token endpoints.

use std::sync::Arc;

use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};

use cairn_api::auth::AuthPrincipal;

use crate::errors::bad_request_response;
use crate::extractors::is_admin_principal;
use crate::state::AppState;
use crate::tokens::OperatorTokenRecord;

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub(crate) struct CreateAuthTokenRequest {
    pub operator_id: String,
    pub tenant_id: String,
    pub name: String,
    pub expires_at: Option<u64>,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// `POST /v1/auth/tokens` — create an operator API token.
/// Only the admin service account or System principal may call this.
/// Returns the raw token once — it cannot be retrieved again.
pub(crate) async fn create_auth_token_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreateAuthTokenRequest>,
) -> impl IntoResponse {
    if !is_admin_principal(&principal) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({
                "error": "forbidden",
                "detail": "only the admin token may create operator tokens"
            })),
        )
            .into_response();
    }
    if body.operator_id.trim().is_empty() {
        return bad_request_response("operator_id must not be empty");
    }
    if body.name.trim().is_empty() {
        return bad_request_response("name must not be empty");
    }

    let token_id = format!("tok_{}", uuid::Uuid::new_v4().simple());
    let raw_token = format!("sk_{}", uuid::Uuid::new_v4().simple());
    let created_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let record = OperatorTokenRecord {
        token_id: token_id.clone(),
        operator_id: body.operator_id.clone(),
        tenant_id: body.tenant_id.clone(),
        name: body.name.clone(),
        created_at,
        expires_at: body.expires_at,
    };

    state.service_tokens.register(
        raw_token.clone(),
        AuthPrincipal::Operator {
            operator_id: cairn_domain::ids::OperatorId::new(&body.operator_id),
            tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new(
                &body.tenant_id,
            )),
        },
    );
    state.operator_tokens.insert(raw_token.clone(), record);

    (
        StatusCode::CREATED,
        Json(serde_json::json!({
            "token":       raw_token,
            "token_id":    token_id,
            "operator_id": body.operator_id,
            "tenant_id":   body.tenant_id,
            "name":        body.name,
            "created_at":  created_at,
            "expires_at":  body.expires_at,
        })),
    )
        .into_response()
}

/// `GET /v1/auth/tokens` — list operator tokens (raw token redacted).
pub(crate) async fn list_auth_tokens_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
) -> impl IntoResponse {
    if !is_admin_principal(&principal) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "forbidden" })),
        )
            .into_response();
    }
    let tokens: Vec<serde_json::Value> = state
        .operator_tokens
        .list()
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "token_id":    r.token_id,
                "operator_id": r.operator_id,
                "tenant_id":   r.tenant_id,
                "name":        r.name,
                "created_at":  r.created_at,
                "expires_at":  r.expires_at,
                "token":       "[redacted]",
            })
        })
        .collect();
    let total = tokens.len();
    Json(serde_json::json!({ "tokens": tokens, "total": total })).into_response()
}

/// `DELETE /v1/auth/tokens/:id` — revoke an operator token by token_id.
pub(crate) async fn delete_auth_token_handler(
    State(state): State<Arc<AppState>>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(token_id): Path<String>,
) -> impl IntoResponse {
    if !is_admin_principal(&principal) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({ "error": "forbidden" })),
        )
            .into_response();
    }
    let raw = match state.operator_tokens.raw_token(&token_id) {
        Some(t) => t,
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({
                    "error": "not_found", "token_id": token_id
                })),
            )
                .into_response();
        }
    };
    state.service_tokens.revoke(&raw);
    state.operator_tokens.remove(&token_id);
    (
        StatusCode::OK,
        Json(serde_json::json!({ "revoked": true, "token_id": token_id })),
    )
        .into_response()
}
