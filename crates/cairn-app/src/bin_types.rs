//! Shared request/response types for binary-specific routes.

use axum::http::StatusCode;
use axum::Json;
use cairn_domain::ProjectKey;
use serde::{Deserialize, Serialize};

// ── Request-ID type ──────────────────────────────────────────────────────────

/// A per-request correlation ID stored in request extensions.
///
/// Populated by `metrics_middleware` before calling `next.run()` so every
/// downstream handler and future middleware can read it without re-extracting
/// from the response (which is unavailable until after the handler returns).
///
/// Preference order:
///   1. Client-supplied `X-Request-ID` header (validated: ASCII, ≤ 128 chars).
///   2. Freshly generated UUID v4.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(crate) struct RequestId(pub(crate) String);

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub(crate) struct ApiError {
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

pub(crate) fn not_found(message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            code: "not_found",
            message: message.into(),
        }),
    )
}

pub(crate) fn internal_error(message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            code: "internal_error",
            message: message.into(),
        }),
    )
}

pub(crate) fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            code: "bad_request",
            message: message.into(),
        }),
    )
}

pub(crate) fn forbidden(message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::FORBIDDEN,
        Json(ApiError {
            code: "forbidden",
            message: message.into(),
        }),
    )
}

// ── Pagination headers ────────────────────────────────────────────────────────

/// Build the four standard pagination response headers.
///
/// Returned as an `AppendHeaders` value that axum can include in a response
/// tuple alongside the body — e.g. `Ok((pagination_headers(...), Json(page)))`.
///
/// - `X-Total-Count` — total items across all pages
/// - `X-Page`        — 1-based current page number
/// - `X-Per-Page`    — items per page (the effective limit)
/// - `Link`          — RFC 5988 next/last relations for cursor navigation
pub(crate) fn pagination_headers(
    path: &str,
    total: usize,
    offset: usize,
    limit: usize,
) -> axum::response::AppendHeaders<[(String, String); 4]> {
    let per_page = limit.max(1);
    let page = offset / per_page + 1;
    let last_page = total.max(1).div_ceil(per_page);
    let has_next = offset + per_page < total;

    let link = if has_next {
        format!(
            "<{path}?page={next}>; rel=\"next\", <{path}?page={last}>; rel=\"last\"",
            next = page + 1,
            last = last_page,
        )
    } else {
        format!("<{path}?page={last_page}>; rel=\"last\"")
    };

    axum::response::AppendHeaders([
        ("X-Total-Count".to_owned(), total.to_string()),
        ("X-Page".to_owned(), page.to_string()),
        ("X-Per-Page".to_owned(), per_page.to_string()),
        ("Link".to_owned(), link),
    ])
}

// ── Query param structs ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    #[serde(default = "default_limit")]
    pub(crate) limit: usize,
    #[serde(default)]
    pub(crate) offset: usize,
}

pub(crate) fn default_limit() -> usize {
    50
}

/// Optional project scope for filtered queries.
#[derive(Deserialize)]
pub(crate) struct ProjectQuery {
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
    #[serde(default = "default_limit")]
    pub(crate) limit: usize,
    #[serde(default)]
    pub(crate) offset: usize,
}

impl ProjectQuery {
    pub(crate) fn project_key(&self) -> Option<ProjectKey> {
        match (&self.tenant_id, &self.workspace_id, &self.project_id) {
            (Some(t), Some(w), Some(p)) => {
                Some(ProjectKey::new(t.as_str(), w.as_str(), p.as_str()))
            }
            _ => None,
        }
    }
}
