//! HTTP middleware: authentication, rate limiting, observability.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::{
    body::{to_bytes, Body},
    extract::{MatchedPath, Request, State},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use cairn_api::auth::Authenticator;
use cairn_api::auth::{AuthPrincipal, ServiceTokenAuthenticator};
use cairn_domain::{ProjectKey, PromptReleaseId, WorkspaceId, WorkspaceKey, WorkspaceRole};
use cairn_runtime::set_current_trace_id;
use cairn_runtime::WorkspaceService;
use cairn_store::projections::{PromptReleaseReadModel, WorkspaceMembershipReadModel};
use uuid::Uuid;

use crate::errors::{
    bad_request_response, forbidden_api_error, now_ms, runtime_error_response,
    store_error_response, AppApiError,
};
use crate::state::{AppState, RateLimitBucket};
use crate::tokens::RequestLogEntry;
use crate::CreateRunRequest;

// ── Auth middleware ─────────────────────────────────────────────────────────

pub(crate) async fn auth_middleware(
    State(state): State<Arc<AppState>>,
    mut request: Request,
    next: Next,
) -> Response {
    if auth_exempt_path(request.uri().path()) {
        return next.run(request).await;
    }

    let Some(token) = bearer_token(&request) else {
        return unauthorized_response();
    };

    let authenticator = ServiceTokenAuthenticator::new(state.service_tokens.clone());
    let Ok(principal) = authenticator.authenticate(token) else {
        return unauthorized_response();
    };

    if let Some(tenant) = principal.tenant() {
        request.extensions_mut().insert(tenant.tenant_id.clone());
    }

    if let Err(response) = attach_workspace_role(&state, &principal, &mut request).await {
        return response;
    }

    request.extensions_mut().insert(principal);

    next.run(request).await
}

// ── Rate limiting ───────────────────────────────────────────────────────────

/// Per-token rate limit: 1 000 requests per 60-second window.
pub(crate) const RL_TOKEN_LIMIT: u32 = 1_000;
/// Per-IP rate limit (when no bearer token): 100 requests per 60-second window.
pub(crate) const RL_IP_LIMIT: u32 = 100;
/// Sliding-window duration in milliseconds.
pub(crate) const RL_WINDOW_MS: u64 = 60_000;

pub(crate) async fn rate_limit_middleware(
    State(rate_limits): State<Arc<Mutex<HashMap<String, RateLimitBucket>>>>,
    request: Request,
    next: Next,
) -> Response {
    // Skip health/readiness probes — these must never be rate-limited.
    if matches!(
        request.uri().path(),
        "/health" | "/ready" | "/metrics" | "/version"
    ) {
        return next.run(request).await;
    }

    let now = now_ms();

    // Derive rate-limit key + per-key limit.
    // Token-authenticated requests get a higher allowance (1 000/min).
    // Unauthenticated requests are keyed by IP (100/min).
    let (key, limit) = if let Some(token) = bearer_token(&request) {
        (format!("tok:{token}"), RL_TOKEN_LIMIT)
    } else if let Some(ip) = request_rate_limit_key(&request) {
        (format!("ip:{ip}"), RL_IP_LIMIT)
    } else {
        // No identifiable key — let the request through.
        return next.run(request).await;
    };

    let (remaining, reset_secs, exceeded) = {
        let mut buckets = match rate_limits.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };

        let bucket = buckets.entry(key).or_insert(RateLimitBucket {
            count: 0,
            window_started_ms: now,
        });

        // Reset the window if it has elapsed.
        if now.saturating_sub(bucket.window_started_ms) >= RL_WINDOW_MS {
            *bucket = RateLimitBucket {
                count: 0,
                window_started_ms: now,
            };
        }

        let reset_secs = (RL_WINDOW_MS - now.saturating_sub(bucket.window_started_ms))
            .max(1)
            .div_ceil(1000);

        if bucket.count >= limit {
            (0u32, reset_secs, true)
        } else {
            bucket.count += 1;
            (limit.saturating_sub(bucket.count), reset_secs, false)
        }
    };

    if exceeded {
        let mut response = AppApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "rate_limited",
            "rate limit exceeded",
        )
        .into_response();
        let headers = response.headers_mut();
        if let Ok(v) = HeaderValue::from_str(&reset_secs.to_string()) {
            headers.insert(header::RETRY_AFTER, v);
        }
        headers.insert("x-ratelimit-limit", HeaderValue::from(limit));
        headers.insert("x-ratelimit-remaining", HeaderValue::from(0u32));
        headers.insert("x-ratelimit-reset", HeaderValue::from(reset_secs));
        return response;
    }

    let mut response = next.run(request).await;

    // Attach rate-limit headers to every successful response.
    let headers = response.headers_mut();
    headers.insert("x-ratelimit-limit", HeaderValue::from(limit));
    headers.insert("x-ratelimit-remaining", HeaderValue::from(remaining));
    headers.insert("x-ratelimit-reset", HeaderValue::from(reset_secs));

    response
}

// ── Request ID / tracing ────────────────────────────────────────────────────

/// RFC 011 extension types for tracing context in request extensions.
#[derive(Clone, Debug)]
pub(crate) struct RequestId(#[allow(dead_code)] pub(crate) String);
#[derive(Clone, Debug)]
pub(crate) struct TraceId(#[allow(dead_code)] String);
#[derive(Clone, Debug)]
pub(crate) struct SpanId(#[allow(dead_code)] String);

pub(crate) async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    // Accept an incoming X-Trace-Id or generate a new one (RFC 011).
    let trace_id = request
        .headers()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let request_id = Uuid::new_v4().to_string();
    // Span ID: first 8 hex chars of the request UUID (no extra dep needed).
    let span_id = request_id
        .chars()
        .filter(|c| c.is_ascii_hexdigit())
        .take(8)
        .collect::<String>();

    // Propagate trace context to extensions so handlers can read it.
    request.extensions_mut().insert(TraceId(trace_id.clone()));
    request.extensions_mut().insert(SpanId(span_id.clone()));
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    // Set thread-local so make_envelope() attaches trace_id to events.
    set_current_trace_id(&trace_id);

    let mut response = next.run(request).await;

    // Clear after handler completes.
    set_current_trace_id("");

    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response
            .headers_mut()
            .insert(header::HeaderName::from_static("x-request-id"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&trace_id) {
        response
            .headers_mut()
            .insert(header::HeaderName::from_static("x-trace-id"), value);
    }
    if let Ok(value) = HeaderValue::from_str(&span_id) {
        response
            .headers_mut()
            .insert(header::HeaderName::from_static("x-span-id"), value);
    }
    response
}

// ── Auth helpers ────────────────────────────────────────────────────────────

pub(crate) fn auth_exempt_path(path: &str) -> bool {
    // Public infra endpoints
    if matches!(
        path,
        "/health"
            | "/healthz"
            | "/ready"
            | "/metrics"
            | "/version"
            | "/v1/onboarding/templates"
            | "/openapi.json"
            | "/docs"
            | "/v1/stream"
            | "/v1/docs"
            | "/v1/webhooks/github"
    ) {
        return true;
    }
    // Dynamic webhook receivers — all integration webhooks use their own
    // verification (HMAC, etc.) instead of bearer token auth.
    if path.starts_with("/v1/webhooks/") {
        return true;
    }

    // The embedded React UI must load without auth — it has its own LoginPage
    // that collects the token client-side before making API calls.
    // Exempt: static assets and any non-/v1/ path (SPA fallback to index.html).
    if path == "/"
        || path == "/index.html"
        || path == "/favicon.svg"
        || path.starts_with("/assets/")
        || !path.starts_with("/v1/")
    {
        return true;
    }

    false
}

pub(crate) fn request_rate_limit_key(request: &Request) -> Option<String> {
    request
        .headers()
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn bearer_token(request: &Request) -> Option<&str> {
    // 1. Standard Authorization: Bearer <token> header
    if let Some(header) = request.headers().get(axum::http::header::AUTHORIZATION) {
        if let Ok(value) = header.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                return Some(token);
            }
        }
    }
    // 2. Query-param fallback: ?token=<token>  (for SSE EventSource which
    //    cannot set custom headers).
    if let Some(query) = request.uri().query() {
        for pair in query.split('&') {
            if let Some(val) = pair.strip_prefix("token=") {
                if !val.is_empty() {
                    return Some(val);
                }
            }
        }
    }
    None
}

pub(crate) fn principal_member_id(principal: &AuthPrincipal) -> Option<&str> {
    match principal {
        AuthPrincipal::Operator { operator_id, .. } => Some(operator_id.as_str()),
        AuthPrincipal::ServiceAccount { name, .. } => Some(name.as_str()),
        AuthPrincipal::System => None,
    }
}

// ── Workspace role inference ────────────────────────────────────────────────

pub(crate) async fn lookup_workspace_role(
    state: &AppState,
    principal: &AuthPrincipal,
    workspace_key: &WorkspaceKey,
) -> Result<Option<WorkspaceRole>, Response> {
    let Some(member_id) = principal_member_id(principal) else {
        return Ok(None);
    };

    WorkspaceMembershipReadModel::get_member(state.runtime.store.as_ref(), workspace_key, member_id)
        .await
        .map(|membership| membership.map(|membership| membership.role))
        .map_err(store_error_response)
}

pub(crate) async fn infer_workspace_role_for_request(
    state: &AppState,
    principal: &AuthPrincipal,
    request: &mut Request,
) -> Result<Option<WorkspaceRole>, Response> {
    let path = request.uri().path().to_owned();
    let method = request.method().clone();

    if method == axum::http::Method::POST && path == "/v1/runs" {
        let owned_request = std::mem::replace(request, Request::new(Body::empty()));
        let (parts, body) = owned_request.into_parts();
        let bytes = to_bytes(body, 10 * 1024 * 1024)
            .await
            .map_err(|_| bad_request_response("invalid request body"))?;
        *request = Request::from_parts(parts, Body::from(bytes.clone()));

        let Ok(payload) = serde_json::from_slice::<CreateRunRequest>(&bytes) else {
            return Ok(None);
        };
        let workspace_key = WorkspaceKey::new(payload.tenant_id, payload.workspace_id);
        return lookup_workspace_role(state, principal, &workspace_key).await;
    }

    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if method == axum::http::Method::POST
        && segments.len() == 5
        && segments[0] == "v1"
        && segments[1] == "admin"
        && segments[2] == "workspaces"
        && segments[4] == "members"
    {
        let workspace = state
            .runtime
            .workspaces
            .get(&WorkspaceId::new(segments[3]))
            .await
            .map_err(runtime_error_response)?
            .ok_or_else(|| {
                AppApiError::new(StatusCode::NOT_FOUND, "not_found", "workspace not found")
                    .into_response()
            })?;
        let workspace_key = WorkspaceKey::new(workspace.tenant_id, workspace.workspace_id);
        return lookup_workspace_role(state, principal, &workspace_key).await;
    }

    if method == axum::http::Method::POST
        && segments.len() == 5
        && segments[0] == "v1"
        && segments[1] == "prompts"
        && segments[2] == "releases"
        && segments[4] == "activate"
    {
        let release = PromptReleaseReadModel::get(
            state.runtime.store.as_ref(),
            &PromptReleaseId::new(segments[3]),
        )
        .await
        .map_err(store_error_response)?;
        if let Some(release) = release {
            return lookup_workspace_role(state, principal, &release.project.workspace_key()).await;
        }
    }

    Ok(None)
}

pub(crate) async fn attach_workspace_role(
    state: &AppState,
    principal: &AuthPrincipal,
    request: &mut Request,
) -> Result<(), Response> {
    if let Some(role) = infer_workspace_role_for_request(state, principal, request).await? {
        request.extensions_mut().insert(role);
    }
    Ok(())
}

pub(crate) async fn ensure_workspace_role_for_project(
    state: &AppState,
    principal: &AuthPrincipal,
    project: &ProjectKey,
    minimum_role: WorkspaceRole,
) -> Result<(), Response> {
    let Some(role) = lookup_workspace_role(state, principal, &project.workspace_key()).await?
    else {
        return Ok(());
    };
    if !role.has_at_least(minimum_role) {
        return Err(forbidden_api_error("insufficient workspace role").into_response());
    }
    Ok(())
}

// ── Observability ───────────────────────────────────────────────────────────

pub(crate) async fn observability_middleware(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().as_str().to_owned();
    let path = request
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str)
        .unwrap_or_else(|| request.uri().path())
        .to_owned();
    let query = request.uri().query().map(String::from);
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|r| r.0.clone())
        .unwrap_or_default();
    let start = Instant::now();
    let response = next.run(request).await;
    let latency_ms = start.elapsed().as_millis() as u64;
    let status = response.status().as_u16();

    state
        .metrics
        .record_request(&method, &path, status, latency_ms);

    // Write structured log entry to the request log ring buffer.
    let level = if status >= 500 {
        "error"
    } else if status >= 400 {
        "warn"
    } else {
        "info"
    };
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let message = format!("{method} {path} -> {status} ({latency_ms}ms)");
    if let Ok(mut log) = state.request_log.write() {
        log.push(RequestLogEntry {
            timestamp,
            level,
            message,
            request_id,
            method: method.clone(),
            path: path.clone(),
            query,
            status,
            latency_ms,
        });
    }

    refresh_activity_metrics(state.as_ref()).await;

    response
}

pub(crate) async fn refresh_activity_metrics(state: &AppState) {
    let active_runs = state.runtime.store.count_active_runs().await;
    let active_tasks = state.runtime.store.count_active_tasks().await;
    state
        .metrics
        .set_active_counts(active_runs as usize, active_tasks as usize);
}

// ── Private helpers ─────────────────────────────────────────────────────────

fn unauthorized_response() -> Response {
    AppApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "unauthorized").into_response()
}
