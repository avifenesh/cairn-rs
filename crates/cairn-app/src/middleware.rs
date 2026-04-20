//! HTTP middleware: authentication, rate limiting, observability.

use std::{
    sync::{atomic::Ordering, Arc},
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
#[cfg(feature = "metrics-core")]
use cairn_runtime::TenantService;
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
    if auth_exempt_path_method(request.uri().path(), request.method()) {
        return next.run(request).await;
    }

    let Some(token) = bearer_token(&request) else {
        return unauthorized_response();
    };

    let authenticator = ServiceTokenAuthenticator::new(state.service_tokens.clone());
    let Ok(principal) = authenticator.authenticate(&token) else {
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
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let rate_limits = state.rate_limits.clone();
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

        // T6b-C6: amortized time-based eviction. We cache the last sweep
        // timestamp on the state and only sweep once per window, so an
        // attacker keeping the map at the threshold can't trigger a
        // full O(N) scan on every request. A HashSet of "rotate bearer
        // strings" still grows up to 10k entries in the worst case
        // before the minute-boundary sweep — bounded and recoverable.
        let last_sweep = state.rate_limit_last_sweep_ms.load(Ordering::Relaxed);
        if now.saturating_sub(last_sweep) >= RL_WINDOW_MS {
            let evict_before = now.saturating_sub(RL_WINDOW_MS * 2);
            buckets.retain(|_, b| b.window_started_ms >= evict_before);
            state.rate_limit_last_sweep_ms.store(now, Ordering::Relaxed);
        }

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

#[allow(dead_code)] // retained for test coverage; production uses the method variant
pub(crate) fn auth_exempt_path(path: &str) -> bool {
    auth_exempt_path_method(path, &axum::http::Method::GET)
}

/// Same as [`auth_exempt_path`] but aware of HTTP method — the SPA
/// fallback exemption only covers GET, so a POST to an unknown
/// non-/v1/ path still goes through auth and 404s.
pub(crate) fn auth_exempt_path_method(path: &str, method: &axum::http::Method) -> bool {
    // T6b-C1: strict allowlist. Pre-fix, `!path.starts_with("/v1/")` made
    // ANY non-/v1/ path public, regardless of method — so a POST to
    // /internal/* or /debug/* would bypass auth. Normalize case +
    // require method=GET for the SPA-fallback exemption.
    let normalized = path.to_ascii_lowercase();
    let normalized = normalized.as_str();

    // Public infra endpoints.
    // NOTE: `/v1/stream` is NOT on this list — the SSE handler MUST
    // validate its `?token=` query param through the same
    // ServiceTokenAuthenticator and filter events by tenant (T6b-C7).
    if matches!(
        normalized,
        "/health"
            | "/healthz"
            | "/ready"
            | "/metrics"
            | "/version"
            | "/v1/onboarding/templates"
            | "/openapi.json"
            | "/docs"
            | "/v1/docs"
    ) {
        return true;
    }
    // Integration webhook receivers verify their own HMAC — bearer auth
    // doesn't apply here. Each integration's handler MUST fail-closed
    // when the signature check fails.
    if normalized.starts_with("/v1/webhooks/") {
        return true;
    }

    // Embedded React UI assets + SPA client-side routes. The SPA has
    // its own LoginPage that collects the bearer token client-side
    // before making API calls. We only exempt GET — POST/PATCH/DELETE
    // to any unknown path must still go through auth.
    if method != axum::http::Method::GET {
        return false;
    }
    if matches!(
        normalized,
        "/" | "/index.html"
            | "/favicon.svg"
            | "/favicon.ico"
            | "/robots.txt"
            | "/.well-known/agent.json"
    ) || normalized.starts_with("/assets/")
    {
        return true;
    }
    // SPA client-side routes: any GET that is NOT under /v1/ (and
    // isn't in the explicit list above) falls through to
    // `serve_frontend`, which returns index.html so React Router can
    // handle navigation. This is the only remaining wildcard, and it
    // is method-gated to GET.
    !normalized.starts_with("/v1/")
}

/// T6b-C2: redact credential-looking query params before the query
/// string is stored in telemetry. Keys matched case-insensitively
/// against a denylist after percent-decoding (so `?%74oken=secret`
/// — a URL-encoded `token=` — is also caught).
pub(crate) fn scrub_credentials_in_query(query: &str) -> String {
    const REDACTED_KEYS: &[&str] = &["token", "api_key", "apikey", "password", "secret", "bearer"];

    // Parse each pair ONCE through the urlencoded decoder, compare the
    // decoded key against the denylist, and rebuild the scrubbed query
    // (urlencoded-decoded — the point is ops visibility, not byte-exact
    // round-trip).
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
        .map(|(k, v)| {
            let k = k.into_owned();
            let v_scrubbed = if REDACTED_KEYS.iter().any(|r| r.eq_ignore_ascii_case(&k)) {
                "REDACTED".to_owned()
            } else {
                v.into_owned()
            };
            (k, v_scrubbed)
        })
        .collect();
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&pairs)
        .finish()
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

pub(crate) fn bearer_token(request: &Request) -> Option<String> {
    // 1. Standard `Authorization: Bearer <token>` header.
    if let Some(header) = request.headers().get(axum::http::header::AUTHORIZATION) {
        if let Ok(value) = header.to_str() {
            if let Some(token) = value.strip_prefix("Bearer ") {
                let trimmed = token.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
        }
    }
    // 2. Query-param fallback: `?token=<token>` for SSE EventSource
    //    which can't set custom headers.
    //
    //    T6b-H2: percent-decode properly (so `%2B` becomes `+`), pick
    //    only the FIRST `token` key (duplicates are rejected), and
    //    reject whitespace-only values.
    if let Some(query) = request.uri().query() {
        for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
            if k.as_ref() == "token" {
                let v = v.trim();
                if v.is_empty() {
                    return None;
                }
                return Some(v.to_owned());
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
    // T6b-C2: scrub credential-ish query params before they hit the
    // request-log ring buffer. The SSE EventSource path uses
    // `?token=<bearer>` because browsers can't set custom headers on
    // SSE connections — the raw bearer would otherwise be visible via
    // `GET /v1/admin/logs` and durably flushed to disk on shutdown.
    let query = request.uri().query().map(scrub_credentials_in_query);
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|r| r.0.clone())
        .unwrap_or_default();
    let start_time_unix_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
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
            start_time_unix_ns,
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

    #[cfg(feature = "metrics-core")]
    {
        // Projection lag: for the in-memory store this is always 0 because
        // projections are applied synchronously inside `append`. The gauge
        // exists so Postgres / SQLite backends (which can lag) have a
        // ready-made series. Not reading `head_position` to call out the
        // 0-is-structural nature; update when async projections land.
        state.metrics.set_projection_lag(0);

        // Tenant-scoped queue depths. Bounded to the first 200 tenants to
        // keep the metrics handler latency predictable on large installs;
        // operators who need per-tenant visibility past that point should
        // move to an async collector.
        if let Ok(tenants) = state.runtime.tenants.list(200, 0).await {
            for t in tenants {
                let tenant_id = t.tenant_id.as_str();
                let runs = state
                    .runtime
                    .store
                    .count_active_runs_for_tenant(&t.tenant_id)
                    .await;
                let tasks = state
                    .runtime
                    .store
                    .count_active_tasks_for_tenant(&t.tenant_id)
                    .await;
                let pending = state
                    .runtime
                    .store
                    .count_pending_approvals_for_tenant(&t.tenant_id)
                    .await;
                state
                    .metrics
                    .set_tenant_queue_depth(tenant_id, runs, tasks, pending);
            }
        }
    }
}

// ── Private helpers ─────────────────────────────────────────────────────────

fn unauthorized_response() -> Response {
    AppApiError::new(StatusCode::UNAUTHORIZED, "unauthorized", "unauthorized").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    // ── auth_exempt_path ───────────────────────────────────────────────

    #[test]
    fn exempt_infra_endpoints() {
        for path in &[
            "/health",
            "/healthz",
            "/ready",
            "/metrics",
            "/version",
            "/openapi.json",
            "/docs",
        ] {
            assert!(auth_exempt_path(path), "{path} should be auth-exempt");
        }
    }

    #[test]
    fn exempt_public_api_endpoints() {
        assert!(auth_exempt_path("/v1/onboarding/templates"));
        assert!(auth_exempt_path("/v1/docs"));
        // T6b-C7: /v1/stream is no longer blanket-exempt; the SSE
        // handler validates its own ?token= query param.
        assert!(!auth_exempt_path("/v1/stream"));
    }

    #[test]
    fn spa_fallback_only_exempts_get() {
        use axum::http::Method;
        // GET on a SPA-fallback path is exempt (serves index.html).
        assert!(auth_exempt_path_method("/settings", &Method::GET));
        // POST / PATCH / DELETE on the same path must still hit auth.
        assert!(!auth_exempt_path_method("/settings", &Method::POST));
        assert!(!auth_exempt_path_method("/settings", &Method::PATCH));
        assert!(!auth_exempt_path_method("/settings", &Method::DELETE));
    }

    #[test]
    fn case_insensitive_exempt_matching() {
        // /V1/RUNS should behave the same as /v1/runs — i.e. NOT
        // exempt (it's under /v1/, case-insensitive).
        assert!(!auth_exempt_path("/V1/runs"));
        assert!(!auth_exempt_path("/V1/Stream"));
    }

    #[test]
    fn exempt_webhook_paths() {
        assert!(auth_exempt_path("/v1/webhooks/github"));
        assert!(auth_exempt_path("/v1/webhooks/slack"));
        assert!(auth_exempt_path("/v1/webhooks/any-integration"));
    }

    #[test]
    fn exempt_static_ui_paths() {
        assert!(auth_exempt_path("/"));
        assert!(auth_exempt_path("/index.html"));
        assert!(auth_exempt_path("/favicon.svg"));
        assert!(auth_exempt_path("/assets/index-abc123.js"));
        assert!(auth_exempt_path("/assets/style.css"));
    }

    #[test]
    fn exempt_spa_fallback_non_v1() {
        // Any path that does NOT start with /v1/ is SPA fallback.
        assert!(auth_exempt_path("/settings"));
        assert!(auth_exempt_path("/runs/abc"));
        assert!(auth_exempt_path("/dashboard"));
    }

    #[test]
    fn not_exempt_v1_api_routes() {
        assert!(!auth_exempt_path("/v1/runs"));
        assert!(!auth_exempt_path("/v1/prompts"));
        assert!(!auth_exempt_path("/v1/admin/settings"));
        assert!(!auth_exempt_path("/v1/agents"));
        assert!(!auth_exempt_path("/v1/sessions"));
    }

    // ── bearer_token ───────────────────────────────────────────────────

    fn make_request(uri: &str, auth_header: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri(uri);
        if let Some(header) = auth_header {
            builder = builder.header("Authorization", header);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn bearer_from_auth_header() {
        let req = make_request("/v1/runs", Some("Bearer my-secret-token"));
        assert_eq!(bearer_token(&req), Some("my-secret-token".to_owned()));
    }

    #[test]
    fn bearer_from_query_param() {
        let req = make_request("/v1/stream?token=sse-token-123", None);
        assert_eq!(bearer_token(&req), Some("sse-token-123".to_owned()));
    }

    #[test]
    fn bearer_from_query_with_other_params() {
        let req = make_request("/v1/stream?follow=true&token=t1&limit=10", None);
        assert_eq!(bearer_token(&req), Some("t1".to_owned()));
    }

    #[test]
    fn bearer_header_takes_priority_over_query() {
        let req = make_request("/v1/stream?token=query-tok", Some("Bearer header-tok"));
        assert_eq!(bearer_token(&req), Some("header-tok".to_owned()));
    }

    #[test]
    fn bearer_none_when_missing() {
        let req = make_request("/v1/runs", None);
        assert_eq!(bearer_token(&req), None);
    }

    #[test]
    fn bearer_none_for_non_bearer_auth() {
        let req = make_request("/v1/runs", Some("Basic dXNlcjpwYXNz"));
        assert_eq!(bearer_token(&req), None);
    }

    #[test]
    fn bearer_none_for_empty_query_token() {
        let req = make_request("/v1/stream?token=", None);
        assert_eq!(bearer_token(&req), None);
    }

    // ── request_rate_limit_key ─────────────────────────────────────────

    #[test]
    fn rate_limit_key_from_forwarded_for() {
        let req = Request::builder()
            .uri("/v1/runs")
            .header("x-forwarded-for", "10.0.0.1")
            .body(Body::empty())
            .unwrap();
        assert_eq!(request_rate_limit_key(&req), Some("10.0.0.1".to_owned()));
    }

    #[test]
    fn rate_limit_key_none_without_header() {
        let req = Request::builder()
            .uri("/v1/runs")
            .body(Body::empty())
            .unwrap();
        assert_eq!(request_rate_limit_key(&req), None);
    }

    #[test]
    fn rate_limit_key_none_for_empty_header() {
        let req = Request::builder()
            .uri("/v1/runs")
            .header("x-forwarded-for", "  ")
            .body(Body::empty())
            .unwrap();
        assert_eq!(request_rate_limit_key(&req), None);
    }

    // ── principal_member_id ────────────────────────────────────────────

    use cairn_domain::ids::{OperatorId, TenantId};
    use cairn_domain::tenancy::TenantKey;

    fn test_tenant() -> TenantKey {
        TenantKey {
            tenant_id: TenantId::new("t"),
        }
    }

    #[test]
    fn member_id_for_operator() {
        let principal = AuthPrincipal::Operator {
            operator_id: OperatorId::new("op-1"),
            tenant: test_tenant(),
        };
        assert_eq!(principal_member_id(&principal), Some("op-1"));
    }

    #[test]
    fn member_id_for_service_account() {
        let principal = AuthPrincipal::ServiceAccount {
            name: "sa-ci".into(),
            tenant: test_tenant(),
        };
        assert_eq!(principal_member_id(&principal), Some("sa-ci"));
    }

    #[test]
    fn member_id_none_for_system() {
        assert_eq!(principal_member_id(&AuthPrincipal::System), None);
    }
}
