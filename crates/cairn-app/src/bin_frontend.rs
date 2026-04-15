//! OpenAPI spec, Swagger UI, and embedded frontend.

#[allow(unused_imports)]
use crate::*;

use axum::http::StatusCode;
use axum::response::IntoResponse;
use rust_embed::RustEmbed;

// ── OpenAPI spec + Swagger UI ─────────────────────────────────────────────────

/// `GET /v1/openapi.json` — OpenAPI 3.0 specification.
pub(crate) async fn openapi_json_handler() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        openapi_spec::OPENAPI_JSON,
    )
}

/// `GET /v1/docs` — Swagger UI (CDN-hosted, points at /v1/openapi.json).
pub(crate) async fn swagger_ui_handler() -> impl IntoResponse {
    (
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        openapi_spec::SWAGGER_UI_HTML,
    )
}

// ── Embedded frontend (ui/dist/) ──────────────────────────────────────────────
//
// In debug builds rust-embed reads files from disk at request time so you can
// update ui/dist/ without recompiling.  In release builds the files are baked
// into the binary — single-binary deployment with no external assets needed.

#[derive(RustEmbed)]
#[folder = "../../ui/dist"]
pub(crate) struct FrontendAssets;

/// Serve an embedded frontend file, falling back to `index.html` for any path
/// not found (SPA client-side routing).  API routes registered before this
/// fallback continue to take priority.
pub(crate) async fn serve_frontend(uri: axum::http::Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Empty path → index.html
    let path = if path.is_empty() { "index.html" } else { path };

    match FrontendAssets::get(path) {
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(axum::http::header::CONTENT_TYPE, mime.as_ref().to_owned())],
                file.data.to_vec(),
            )
                .into_response()
        }
        // SPA fallback: any unknown path returns index.html so React Router
        // handles client-side navigation (e.g. #settings, #runs).
        None => match FrontendAssets::get("index.html") {
            Some(index) => (
                [(
                    axum::http::header::CONTENT_TYPE,
                    "text/html; charset=utf-8".to_owned(),
                )],
                index.data.to_vec(),
            )
                .into_response(),
            None => StatusCode::NOT_FOUND.into_response(),
        },
    }
}

// Prometheus metrics handler → bin_handlers.rs
// Server role handler → bin_handlers.rs
// Entitlement, bundle, template handlers → bin_handlers.rs
