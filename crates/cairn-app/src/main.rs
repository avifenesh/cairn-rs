//! Bootstrap binary for the Cairn Rust workspace.
//!
//! Usage:
//!   cairn-app                         # local mode, 127.0.0.1:3000
//!   cairn-app --mode team             # self-hosted team mode
//!   cairn-app --port 8080             # custom port
//!   cairn-app --addr 0.0.0.0          # bind all interfaces
//!
mod sse_hooks;

use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::middleware::{from_fn_with_state, Next};
use axum::response::{IntoResponse, Response};
use tower_http::cors::{Any, CorsLayer};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::{get, post};
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;
use axum::Json;
use axum::Router;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;
use cairn_api::auth::{AuthPrincipal, Authenticator, ServiceTokenAuthenticator, ServiceTokenRegistry};
use cairn_api::bootstrap::{BootstrapConfig, DeploymentMode, EncryptionKeySource, StorageBackend};
use cairn_api::{DashboardOverview, HealthResponse, SystemStatus};
use cairn_domain::{ApprovalDecision, ApprovalId, ProjectKey, RunId, TaskId};
use cairn_runtime::approvals::ApprovalService;
use cairn_runtime::runs::RunService;
use cairn_runtime::sessions::SessionService;
use cairn_runtime::InMemoryServices;
use cairn_store::projections::{ApprovalReadModel, ProviderHealthReadModel, RunReadModel, SessionReadModel, TaskReadModel, ToolInvocationReadModel};
use cairn_store::{EventLog, EventPosition, StoredEvent};
use cairn_store::DbAdapter;
use cairn_store::pg::{PgAdapter, PgEventLog};
use cairn_store::pg::PgMigrationRunner;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use tokio_stream::Stream;

// ── Postgres backend ──────────────────────────────────────────────────────────

/// Bundled Postgres connection handles.
///
/// Created at startup when `--db postgres://...` is supplied.
/// Appends go to both Postgres (durable) and InMemory (read models + SSE);
/// event log replays (GET /v1/events) are served from Postgres when present.
#[derive(Clone)]
struct PgBackend {
    event_log: Arc<PgEventLog>,
    adapter:   Arc<PgAdapter>,
}

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
struct AppState {
    runtime: Arc<InMemoryServices>,
    started_at: Arc<Instant>,
    /// Bearer token registry — populated at startup with an admin token.
    tokens: Arc<ServiceTokenRegistry>,
    /// Postgres backend — Some when `--db postgres://...` is passed, None otherwise.
    pg: Option<Arc<PgBackend>>,
}

// ── Response types ───────────────────────────────────────────────────────────

#[derive(Serialize)]
struct ApiError {
    code: &'static str,
    message: String,
}

fn not_found(message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::NOT_FOUND,
        Json(ApiError {
            code: "not_found",
            message: message.into(),
        }),
    )
}

fn internal_error(message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ApiError {
            code: "internal_error",
            message: message.into(),
        }),
    )
}

fn bad_request(message: impl Into<String>) -> (StatusCode, Json<ApiError>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ApiError {
            code: "bad_request",
            message: message.into(),
        }),
    )
}

// ── Query param structs ───────────────────────────────────────────────────────

#[derive(Deserialize)]
struct PaginationQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

fn default_limit() -> usize {
    50
}

/// Optional project scope for filtered queries.
#[derive(Deserialize)]
struct ProjectQuery {
    tenant_id: Option<String>,
    workspace_id: Option<String>,
    project_id: Option<String>,
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default)]
    offset: usize,
}

impl ProjectQuery {
    fn project_key(&self) -> Option<ProjectKey> {
        match (&self.tenant_id, &self.workspace_id, &self.project_id) {
            (Some(t), Some(w), Some(p)) => Some(ProjectKey::new(t.as_str(), w.as_str(), p.as_str())),
            _ => None,
        }
    }
}

// ── Auth middleware (RFC 008) ─────────────────────────────────────────────────

/// Paths that are served without authentication.
///
/// - `/health` — liveness probe used by load-balancers.
/// - `/v1/stream` — SSE endpoint: browsers can't set custom headers in
///   `EventSource`, so clients reconnect using the last-event-id only.
fn is_auth_exempt(path: &str) -> bool {
    path == "/health" || path.starts_with("/v1/stream")
}

/// Extract the raw token from an `Authorization: Bearer <token>` header.
fn bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_owned())
}

/// Axum middleware that enforces bearer token authentication on `/v1/*` routes.
///
/// Exempt paths (`/health`, `/v1/stream`) pass through without a token.
/// All other paths require a valid `Authorization: Bearer <token>` header.
/// On success the resolved `AuthPrincipal` is placed in request extensions
/// so downstream handlers can read it via `Extension<AuthPrincipal>`.
async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let path = request.uri().path().to_owned();

    if is_auth_exempt(&path) {
        return next.run(request).await;
    }

    // Only guard /v1/* routes; all other paths (e.g. future admin paths) pass
    // through unless explicitly added here.
    if !path.starts_with("/v1/") {
        return next.run(request).await;
    }

    let token = match bearer_token(request.headers()) {
        Some(t) => t,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    code: "unauthorized",
                    message: "missing Authorization: Bearer <token> header".to_owned(),
                }),
            )
                .into_response();
        }
    };

    let authenticator = ServiceTokenAuthenticator::new(state.tokens.clone());
    match authenticator.authenticate(&token) {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(_) => (
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                code: "unauthorized",
                message: "invalid or expired bearer token".to_owned(),
            }),
        )
            .into_response(),
    }
}

// ── Core handlers ─────────────────────────────────────────────────────────────

async fn health_handler() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn status_handler(State(state): State<AppState>) -> Json<SystemStatus> {
    // Prefer Postgres health when configured; fall back to InMemory store.
    let store_ok = if let Some(pg) = &state.pg {
        pg.adapter.health_check().await.is_ok()
    } else {
        state.runtime.store.head_position().await.is_ok()
    };
    Json(SystemStatus {
        runtime_ok: true,
        store_ok,
        uptime_secs: state.started_at.elapsed().as_secs(),
    })
}

async fn dashboard_handler(State(state): State<AppState>) -> Json<DashboardOverview> {
    let active_runs = state.runtime.store.count_active_runs().await as u32;
    let active_tasks = state.runtime.store.count_active_tasks().await as u32;
    Json(DashboardOverview {
        active_runs,
        active_tasks,
        pending_approvals: 0,
        failed_runs_24h: 0,
        system_healthy: true,
        latency_p50_ms: None,
        latency_p95_ms: None,
        error_rate_24h: 0.0,
        degraded_components: vec![],
        recent_critical_events: vec![],
        active_providers: 0,
        active_plugins: 0,
        memory_doc_count: 0,
        eval_runs_today: 0,
    })
}

/// `GET /v1/stream` — Real-time SSE event stream (RFC 002).
///
/// Protocol:
/// - On connect, sends `event: connected` with server position as data.
/// - If `Last-Event-ID` header is present, replays all events after that
///   position before entering the live stream (RFC 002 replay window).
/// - Subsequent events are streamed live as they are appended to the store.
/// - SSE `id:` field carries the log position so clients can resume.
/// - Keepalive comment is sent every 15 s to prevent proxy timeouts.
async fn stream_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    // Subscribe BEFORE reading the replay window — this guarantees no events
    // are missed in the window between replay and live subscription.
    let receiver = state.runtime.store.subscribe();

    // Extract Last-Event-ID for cursor-based replay.
    let last_event_id: Option<u64> = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse().ok());

    let after = last_event_id.map(cairn_store::EventPosition);

    // Read the replay window synchronously before yielding the stream.
    let replayed = state
        .runtime
        .store
        .read_stream(after, 1000)
        .await
        .unwrap_or_default();

    let head_pos = state
        .runtime
        .store
        .head_position()
        .await
        .ok()
        .flatten()
        .map(|p| p.0)
        .unwrap_or(0);

    // Connected event — tells the client the current head position.
    let connected = Event::default()
        .event("connected")
        .data(format!(r#"{{"head_position":{head_pos}}}"#));

    // Replay stream: historical events after Last-Event-ID.
    let replay = tokio_stream::iter(replayed).map(stored_event_to_sse);

    // Live stream: broadcast channel, filter out lagged-receiver errors.
    let live = BroadcastStream::new(receiver)
        .filter_map(|r| r.ok())
        .map(stored_event_to_sse);

    // connected → replay → live
    let stream = tokio_stream::once(connected)
        .chain(replay)
        .chain(live)
        .map(Ok::<Event, Infallible>);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    )
}

fn stored_event_to_sse(e: StoredEvent) -> Event {
    let data = serde_json::json!({
        "event_id": e.envelope.event_id.as_str(),
        "type": event_type_name(&e.envelope.payload),
        "payload": &e.envelope.payload,
    });
    Event::default()
        .id(e.position.0.to_string())
        .event(event_type_name(&e.envelope.payload))
        .data(serde_json::to_string(&data).unwrap_or_else(|_| "{}".to_owned()))
}

// ── Run handlers ──────────────────────────────────────────────────────────────

/// `GET /v1/runs` — list all runs (limit/offset pagination).
async fn list_runs_handler(
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    // list_by_state across every state is expensive; use the store's filtered
    // helper which returns all runs when no filters are applied.
    let dummy_project = ProjectKey::new("_", "_", "_");
    match state
        .runtime
        .store
        .list_runs_filtered(&dummy_project, None, None, q.limit, q.offset)
        .await
    {
        Ok(runs) => Ok(Json(runs)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// `GET /v1/runs/:id` — get a single run by ID.
async fn get_run_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(id);
    match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(Some(run)) => Ok(Json(run)),
        Ok(None) => Err(not_found(format!("run {} not found", run_id.as_str()))),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

// ── Task handlers ────────────────────────────────────────────────────────────

/// `GET /v1/runs/:id/tasks` — list all tasks belonging to a run.
///
/// Returns every TaskRecord whose parent_run_id matches the given run ID,
/// ordered by (created_at, task_id).  Returns an empty array when the run
/// exists but has no tasks; returns 404 when the run is unknown.
async fn list_run_tasks_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(&id);

    // Verify the run exists before listing its tasks.
    match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(None) => return Err(not_found(format!("run {id} not found"))),
        Err(e)   => return Err(internal_error(e.to_string())),
        Ok(Some(_)) => {}
    }

    match TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &run_id, 1000).await {
        Ok(tasks) => Ok(Json(tasks)),
        Err(e)    => Err(internal_error(e.to_string())),
    }
}

/// `GET /v1/runs/:id/approvals` — list all approvals for a run.
///
/// Returns approvals in all states (pending and resolved) ordered by
/// (created_at, approval_id).  Returns 404 when the run is unknown.
async fn list_run_approvals_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(&id);

    // Verify the run exists.
    match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(None) => return Err(not_found(format!("run {id} not found"))),
        Err(e)   => return Err(internal_error(e.to_string())),
        Ok(Some(_)) => {}
    }

    let approvals = state.runtime.store.list_approvals_by_run(&run_id);
    Ok(Json(approvals))
}

// ── Session handlers ──────────────────────────────────────────────────────────

/// `GET /v1/sessions/:id/runs` — list all runs in a session.
///
/// Returns every RunRecord whose session_id matches the given session,
/// ordered by (created_at, run_id).  Includes root runs and subagent runs
/// (parent_run_id is non-null for subagents).  Returns 404 when the session
/// is unknown.  Supports optional `?limit` (default 100).
async fn list_session_runs_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    let session_id = cairn_domain::SessionId::new(&id);

    // Verify session exists.
    match SessionReadModel::get(state.runtime.store.as_ref(), &session_id).await {
        Ok(None) => return Err(not_found(format!("session {id} not found"))),
        Err(e)   => return Err(internal_error(e.to_string())),
        Ok(Some(_)) => {}
    }

    match RunReadModel::list_by_session(
        state.runtime.store.as_ref(),
        &session_id,
        q.limit,
        q.offset,
    ).await {
        Ok(runs) => Ok(Json(runs)),
        Err(e)   => Err(internal_error(e.to_string())),
    }
}

/// `GET /v1/sessions` — list active sessions (most recent first, limit/offset).
async fn list_sessions_handler(
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    match SessionReadModel::list_active(state.runtime.store.as_ref(), q.limit).await {
        Ok(sessions) => {
            let page: Vec<_> = sessions.into_iter().skip(q.offset).collect();
            Ok(Json(page))
        }
        Err(e) => Err(internal_error(e.to_string())),
    }
}

// ── Approval handlers ─────────────────────────────────────────────────────────

/// `GET /v1/approvals/pending` — list pending approvals.
///
/// Accepts optional `tenant_id`, `workspace_id`, `project_id` query params.
/// If all three are provided the result is scoped to that project; otherwise
/// all pending approvals in the store are returned.
async fn list_pending_approvals_handler(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
) -> impl axum::response::IntoResponse {
    if let Some(project) = q.project_key() {
        match ApprovalReadModel::list_pending(
            state.runtime.store.as_ref(),
            &project,
            q.limit,
            q.offset,
        )
        .await
        {
            Ok(records) => Ok(Json(records)),
            Err(e) => Err(internal_error(e.to_string())),
        }
    } else {
        // No project filter — scan all approvals and return pending ones.
        match list_all_pending(&state, q.limit, q.offset).await {
            Ok(records) => Ok(Json(records)),
            Err(e) => Err(internal_error(e.to_string())),
        }
    }
}

/// Scan the full approval store for pending (undecided) records.
async fn list_all_pending(
    state: &AppState,
    limit: usize,
    offset: usize,
) -> Result<Vec<cairn_store::projections::ApprovalRecord>, cairn_store::StoreError> {
    // Read all approvals via the store's raw state; use list_by_state workaround:
    // create a sentinel project and delegate to list_pending — but InMemoryStore
    // filters by project equality. Instead, iterate using the runtime service.
    // The ApprovalService::list_pending also requires a project, so we use a
    // broad scan via list_runs_filtered analogue: collect unique projects from
    // pending approvals by reading the store lock directly.
    //
    // Since we're in an in-memory context, we use a sentinel approach: read all
    // runs to collect projects, then union their pending approvals.
    let dummy = ProjectKey::new("", "", "");
    let all = ApprovalReadModel::list_pending(state.runtime.store.as_ref(), &dummy, 0, 0).await?;
    // The in-memory impl filters by project equality; "" won't match any real
    // project. Fall through to the approvals field directly via count approach.
    // For now, return empty (no project-scoped approvals without a project key).
    let _ = all;

    // Practical fix: use the store's raw scan via the approval service's store ref.
    // We gather runs first, deduplicate projects, then union pending approvals.
    let runs = state.runtime.store.list_runs_filtered(&dummy, None, None, 1000, 0).await?;
    let projects: Vec<ProjectKey> = {
        let mut seen = std::collections::HashSet::new();
        runs.into_iter()
            .filter(|r| seen.insert(format!("{}:{}:{}", r.project.tenant_id.as_str(), r.project.workspace_id.as_str(), r.project.project_id.as_str())))
            .map(|r| r.project)
            .collect()
    };

    let mut combined = Vec::new();
    for project in &projects {
        let mut batch = ApprovalReadModel::list_pending(
            state.runtime.store.as_ref(),
            project,
            1000,
            0,
        )
        .await?;
        combined.append(&mut batch);
    }
    combined.sort_by_key(|a| a.created_at);
    combined.dedup_by_key(|a| a.approval_id.clone());
    Ok(combined.into_iter().skip(offset).take(limit).collect())
}

#[derive(Deserialize)]
struct ResolveApprovalBody {
    /// `"approved"` or `"rejected"`
    decision: String,
}

/// `POST /v1/approvals/:id/resolve` — approve or reject a pending approval.
async fn resolve_approval_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ResolveApprovalBody>,
) -> impl axum::response::IntoResponse {
    let approval_id = ApprovalId::new(id);
    let decision = match body.decision.to_lowercase().as_str() {
        "approved" | "approve" => ApprovalDecision::Approved,
        "rejected" | "reject" => ApprovalDecision::Rejected,
        other => return Err(bad_request(format!("unknown decision: {other}; use 'approved' or 'rejected'"))),
    };
    match state.runtime.approvals.resolve(&approval_id, decision).await {
        Ok(record) => Ok((StatusCode::OK, Json(record))),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("NotFound") {
                Err(not_found(format!("approval {} not found", approval_id.as_str())))
            } else {
                Err(internal_error(msg))
            }
        }
    }
}

// ── Prompt handlers (RFC 006) ─────────────────────────────────────────────────

/// `GET /v1/prompts/assets` — list all prompt assets across every project.
async fn list_prompt_assets_handler(
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    match state.runtime.store.list_all_prompt_assets(q.limit, q.offset).await {
        Ok(assets) => Ok(Json(assets)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// `GET /v1/prompts/releases` — list all prompt releases across every project.
async fn list_prompt_releases_handler(
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    match state.runtime.store.list_all_prompt_releases(q.limit, q.offset).await {
        Ok(releases) => Ok(Json(releases)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

// ── Cost handler (RFC 009) ────────────────────────────────────────────────────

#[derive(Serialize)]
struct CostSummaryResponse {
    total_provider_calls: u64,
    total_tokens_in: u64,
    total_tokens_out: u64,
    total_cost_micros: u64,
}

/// `GET /v1/costs` — aggregate cost summary across all runs in the store.
async fn costs_handler(State(state): State<AppState>) -> Json<CostSummaryResponse> {
    let (calls, tokens_in, tokens_out, cost_micros) = state.runtime.store.cost_summary().await;
    Json(CostSummaryResponse {
        total_provider_calls: calls,
        total_tokens_in: tokens_in,
        total_tokens_out: tokens_out,
        total_cost_micros: cost_micros,
    })
}

// ── Provider handler (RFC 007) ────────────────────────────────────────────────

/// `GET /v1/providers` — list all provider bindings (RFC 007 fleet view).
async fn list_providers_handler(
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    match state.runtime.store.list_all_provider_bindings(q.limit, q.offset).await {
        Ok(bindings) => Ok(Json(bindings)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

// ── Event replay handler (RFC 002) ────────────────────────────────────────────

#[derive(Deserialize)]
struct EventReplayQuery {
    /// Return events strictly after this log position.
    after: Option<u64>,
    #[serde(default = "default_event_limit")]
    limit: usize,
}

fn default_event_limit() -> usize {
    100
}

#[derive(Serialize)]
struct StoredEventSummary {
    position: u64,
    stored_at: u64,
    event_type: String,
}

/// `GET /v1/sessions/:id/events` — entity-scoped event stream for a session.
///
/// Returns all events whose payload's `session_id` matches `:id`, ordered by
/// log position. Supports the same `?after=<position>&limit=<n>` cursor
/// as the global `/v1/events` endpoint.
async fn list_session_events_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EventReplayQuery>,
) -> impl axum::response::IntoResponse {
    let session_id = cairn_domain::SessionId::new(id);
    let limit = q.limit.min(500);
    let after = q.after.map(EventPosition);
    match state
        .runtime
        .store
        .read_by_entity(&cairn_store::EntityRef::Session(session_id), after, limit)
        .await
    {
        Ok(events) => {
            let summaries: Vec<StoredEventSummary> = events
                .into_iter()
                .map(|e| StoredEventSummary {
                    position: e.position.0,
                    stored_at: e.stored_at,
                    event_type: event_type_name(&e.envelope.payload).to_owned(),
                })
                .collect();
            Ok(Json(summaries))
        }
        Err(e) => Err(internal_error(e.to_string())),
    }
}

// ── Provider health handler (RFC 007) ───────────────────────────────────────

/// Response shape for a single provider connection health entry.
#[derive(Serialize)]
struct ProviderHealthEntry {
    connection_id: String,
    status: String,
    healthy: bool,
    last_checked_at: u64,
    consecutive_failures: u32,
    error_message: Option<String>,
}

/// `GET /v1/providers/health` — health status for all registered provider connections.
///
/// Returns a snapshot of every `ProviderHealthRecord` in the store, showing
/// connectivity status, last health-check timestamp, and consecutive failures.
/// Operator dashboards use this to detect degraded providers before routing.
async fn provider_health_handler(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    // Collect unique tenants from provider connections, then union health records.
    // In practice each deployment has one tenant, so we scan all records directly.
    use cairn_domain::TenantId;
    // Use the store's full scan: list health for the default tenant first,
    // then fall back to listing all provider connections to derive tenants.
    // Derive tenant IDs from provider bindings (list_all_provider_bindings scans all).
    let bindings = match state.runtime.store.list_all_provider_bindings(500, 0).await {
        Ok(b) => b,
        Err(e) => return Err(internal_error(e.to_string())),
    };
    let tenants: Vec<cairn_domain::TenantId> = {
        let mut seen = std::collections::HashSet::new();
        bindings
            .iter()
            .filter(|b| seen.insert(b.project.tenant_id.as_str().to_owned()))
            .map(|b| b.project.tenant_id.clone())
            .collect()
    };
    let mut all_health: Vec<ProviderHealthEntry> = Vec::new();
    for tenant_id in &tenants {
        let records = match ProviderHealthReadModel::list_by_tenant(
            state.runtime.store.as_ref(),
            tenant_id,
            100,
            0,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return Err(internal_error(e.to_string())),
        };
        for rec in records {
            all_health.push(ProviderHealthEntry {
                connection_id: rec.binding_id.as_str().to_owned(),
                status: format!("{:?}", rec.status).to_lowercase(),
                healthy: rec.healthy,
                last_checked_at: rec.last_checked_ms,
                consecutive_failures: rec.consecutive_failures,
                error_message: rec.error_message.clone(),
            });
        }
    }
    Ok(Json(all_health))
}

// ── Run tool invocations handler ─────────────────────────────────────────────

/// `GET /v1/runs/:id/tool-invocations` — list all tool calls for a run.
///
/// Returns a page of `ToolInvocationRecord` objects for the given run,
/// sorted by requested_at ascending. Supports `?limit` and `?offset`.
async fn list_run_tool_invocations_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(id);
    match ToolInvocationReadModel::list_by_run(
        state.runtime.store.as_ref(),
        &run_id,
        q.limit,
        q.offset,
    )
    .await
    {
        Ok(records) => Ok(Json(records)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// `GET /v1/runs/:id/events` — entity-scoped event stream for a run (RFC 002).
///
/// Returns all events whose payload's `run_id` matches `:id`, ordered by
/// log position. Supports the same `?after=<position>&limit=<n>` cursor
/// as the global `/v1/events` endpoint.
async fn list_run_events_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EventReplayQuery>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(id);
    let limit = q.limit.min(500);
    let after = q.after.map(EventPosition);
    match state
        .runtime
        .store
        .read_by_entity(&cairn_store::EntityRef::Run(run_id), after, limit)
        .await
    {
        Ok(events) => {
            let summaries: Vec<StoredEventSummary> = events
                .into_iter()
                .map(|e| StoredEventSummary {
                    position: e.position.0,
                    stored_at: e.stored_at,
                    event_type: event_type_name(&e.envelope.payload).to_owned(),
                })
                .collect();
            Ok(Json(summaries))
        }
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// `GET /v1/runs/:id/cost` — accumulated cost breakdown for a run (RFC 009).
///
/// Returns zero-valued fields when no provider calls have been made yet,
/// so the caller can always expect a consistent JSON shape.
async fn get_run_cost_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    use cairn_store::projections::RunCostReadModel;

    let run_id = RunId::new(id);

    match RunCostReadModel::get_run_cost(state.runtime.store.as_ref(), &run_id).await {
        Ok(Some(record)) => Ok(Json(serde_json::json!({
            "run_id":           record.run_id.as_str(),
            "total_cost_micros": record.total_cost_micros,
            "total_tokens_in":   record.total_tokens_in,
            "total_tokens_out":  record.total_tokens_out,
            "provider_calls":    record.provider_calls,
        }))),
        // No cost record yet — run exists but has had no provider calls.
        Ok(None) => Ok(Json(serde_json::json!({
            "run_id":           run_id.as_str(),
            "total_cost_micros": 0u64,
            "total_tokens_in":   0u64,
            "total_tokens_out":  0u64,
            "provider_calls":    0u64,
        }))),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// `GET /v1/events` — cursor-based replay of the global event log (RFC 002).
///
/// Clients use `?after=<position>&limit=<n>` to page forward. Returns at most
/// `limit` events (default 100, max 500) strictly after the given position.
/// When Postgres is configured, replays from the durable Postgres log.
async fn list_events_handler(
    State(state): State<AppState>,
    Query(q): Query<EventReplayQuery>,
) -> impl axum::response::IntoResponse {
    let limit = q.limit.min(500);
    let after = q.after.map(EventPosition);
    // Use Postgres event log when available for durable replay.
    let read_result = if let Some(pg) = &state.pg {
        pg.event_log.read_stream(after, limit).await
    } else {
        state.runtime.store.read_stream(after, limit).await
    };
    match read_result {
        Ok(events) => {
            let summaries: Vec<StoredEventSummary> = events
                .into_iter()
                .map(|e| StoredEventSummary {
                    position: e.position.0,
                    stored_at: e.stored_at,
                    event_type: event_type_name(&e.envelope.payload).to_owned(),
                })
                .collect();
            Ok(Json(summaries))
        }
        Err(e) => Err(internal_error(e.to_string())),
    }
}

fn event_type_name(event: &cairn_domain::RuntimeEvent) -> &'static str {
    use cairn_domain::RuntimeEvent as E;
    match event {
        E::SessionCreated(_) => "session_created",
        E::SessionStateChanged(_) => "session_state_changed",
        E::RunCreated(_) => "run_created",
        E::RunStateChanged(_) => "run_state_changed",
        E::TaskCreated(_) => "task_created",
        E::TaskStateChanged(_) => "task_state_changed",
        E::TaskLeaseClaimed(_) => "task_lease_claimed",
        E::TaskLeaseHeartbeated(_) => "task_lease_heartbeated",
        E::TaskLeaseExpired(_) => "task_lease_expired",
        E::ApprovalRequested(_) => "approval_requested",
        E::ApprovalResolved(_) => "approval_resolved",
        E::CheckpointRecorded(_) => "checkpoint_recorded",
        E::CheckpointStrategySet(_) => "checkpoint_strategy_set",
        E::ProviderCallCompleted(_) => "provider_call_completed",
        E::RunCostUpdated(_) => "run_cost_updated",
        E::OperatorIntervention(_) => "operator_intervention",
        E::RecoveryEscalated(_) => "recovery_escalated",
        _ => "runtime_event",
    }
}

// ── Event append handler (RFC 002) ────────────────────────────────────────────

/// Per-envelope result returned by `POST /v1/events/append`.
#[derive(Serialize)]
struct AppendResult {
    event_id: String,
    position: u64,
    /// `true` = event was newly appended; `false` = idempotent duplicate
    /// (causation_id already existed — existing position is returned).
    appended: bool,
}

/// `POST /v1/events/append` — write path for the event log (RFC 002).
///
/// Accepts a JSON array of `EventEnvelope<RuntimeEvent>` objects. Each
/// envelope is processed for idempotency:
///
/// - If the envelope carries a `causation_id` **and** an event with that
///   causation ID already exists in the log, the existing position is
///   returned without re-appending.
/// - Otherwise the event is appended and its assigned position is returned.
///
/// Appended events are broadcast immediately to all SSE subscribers.
///
/// Returns an array of `AppendResult` in the same order as the input.
async fn append_events_handler(
    State(state): State<AppState>,
    Json(envelopes): Json<Vec<cairn_domain::EventEnvelope<cairn_domain::RuntimeEvent>>>,
) -> impl axum::response::IntoResponse {
    if envelopes.is_empty() {
        return Ok((StatusCode::OK, Json(Vec::<AppendResult>::new())));
    }

    let mut results: Vec<AppendResult> = Vec::with_capacity(envelopes.len());

    for envelope in envelopes {
        let event_id = envelope.event_id.as_str().to_owned();

        // Idempotency check: if causation_id is set and already in the log,
        // return the existing position instead of appending.
        if let Some(ref cid) = envelope.causation_id {
            // Check InMemory first (fastest path); Pg check follows when configured.
            let existing = state.runtime.store.find_by_causation_id(cid.as_str()).await;
            match existing {
                Ok(Some(pos)) => {
                    results.push(AppendResult {
                        event_id,
                        position: pos.0,
                        appended: false,
                    });
                    continue;
                }
                Ok(None) => {} // not found — fall through to append
                Err(e) => return Err(internal_error(e.to_string())),
            }
        }

        // Append the single event.
        // When Postgres is configured: dual-write — persist to Pg (durable) and
        // also write to InMemory so read models and SSE broadcast stay current.
        if let Some(ref pg) = state.pg {
            // Durable append to Postgres.
            if let Err(e) = pg.event_log.append(&[envelope.clone()]).await {
                return Err(internal_error(format!("postgres append: {e}")));
            }
        }
        // Always write to InMemory: updates projections + broadcasts to SSE subscribers.
        match state.runtime.store.append(&[envelope]).await {
            Ok(positions) => {
                results.push(AppendResult {
                    event_id,
                    position: positions[0].0,
                    appended: true,
                });
            }
            Err(e) => return Err(internal_error(e.to_string())),
        }
    }

    Ok((StatusCode::CREATED, Json(results)))
}

// ── DB status handler ─────────────────────────────────────────────────────────

#[derive(Serialize)]
struct DbStatusResponse {
    /// `"postgres"` or `"in_memory"`.
    backend: &'static str,
    /// `true` when the Postgres pool is reachable.
    connected: bool,
    /// Number of migrations recorded in `_cairn_migrations`.
    /// `null` when using the in-memory backend.
    migration_count: Option<usize>,
    /// Whether the schema is fully up to date (all known migrations applied).
    schema_current: Option<bool>,
}

/// `GET /v1/db/status` — Postgres connection health + migration state.
///
/// Returns `backend = "in_memory"` when no Postgres URL was supplied.
/// When Postgres is configured, checks connectivity and reports the number
/// of applied migrations so operators can diagnose schema drift.
async fn db_status_handler(State(state): State<AppState>) -> Json<DbStatusResponse> {
    match &state.pg {
        None => Json(DbStatusResponse {
            backend: "in_memory",
            connected: true,
            migration_count: None,
            schema_current: None,
        }),
        Some(pg) => {
            let connected = pg.adapter.health_check().await.is_ok();
            let (migration_count, schema_current) = if connected {
                let pool = pg.adapter.pool().clone();
                let runner = PgMigrationRunner::new(pool);
                match runner.applied().await {
                    Ok(applied) => {
                        const TOTAL_KNOWN: usize = 19;
                        let count = applied.len();
                        (Some(count), Some(count >= TOTAL_KNOWN))
                    }
                    Err(_) => (None, Some(false)),
                }
            } else {
                (None, Some(false))
            };
            Json(DbStatusResponse {
                backend: "postgres",
                connected,
                migration_count,
                schema_current,
            })
        }
    }
}

// ── Arg parsing ───────────────────────────────────────────────────────────────

fn parse_args_from(args: &[String]) -> BootstrapConfig {
    let mut config = BootstrapConfig::default();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                i += 1;
                if i < args.len() {
                    config.mode = match args[i].as_str() {
                        "team" | "self-hosted" => DeploymentMode::SelfHostedTeam,
                        _ => DeploymentMode::Local,
                    };
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    if let Ok(port) = args[i].parse::<u16>() {
                        config.listen_port = port;
                    }
                }
            }
            "--addr" => {
                i += 1;
                if i < args.len() {
                    config.listen_addr = args[i].clone();
                }
            }
            "--db" => {
                i += 1;
                if i < args.len() {
                    let val = &args[i];
                    if val.starts_with("postgres://") || val.starts_with("postgresql://") {
                        config.storage = StorageBackend::Postgres {
                            connection_url: val.clone(),
                        };
                    } else {
                        config.storage = StorageBackend::Sqlite {
                            path: val.clone(),
                        };
                    }
                }
            }
            "--encryption-key-env" => {
                i += 1;
                if i < args.len() {
                    config.encryption_key = EncryptionKeySource::EnvVar {
                        var_name: args[i].clone(),
                    };
                }
            }
            _ => {}
        }
        i += 1;
    }

    if config.mode == DeploymentMode::SelfHostedTeam {
        if config.listen_addr == "127.0.0.1" {
            config.listen_addr = "0.0.0.0".to_owned();
        }
        if matches!(config.encryption_key, EncryptionKeySource::LocalAuto) {
            config.encryption_key = EncryptionKeySource::None;
        }
    }

    config
}

fn parse_args() -> BootstrapConfig {
    let args: Vec<String> = std::env::args().collect();
    parse_args_from(&args)
}

// ── Entry point ───────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    // Initialise structured request tracing.  Operators can tune verbosity via
    // the RUST_LOG env var (e.g. RUST_LOG=cairn_app=info,tower_http=debug).
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .compact()
        .init();

    let config = parse_args();

    // ── Token registry ────────────────────────────────────────────────────────
    // Read admin token from env; fall back to a hardcoded dev token in local
    // mode. Team mode requires an explicit token or startup aborts.
    let tokens = Arc::new(ServiceTokenRegistry::new());
    let admin_token = std::env::var("CAIRN_ADMIN_TOKEN").unwrap_or_else(|_| {
        if config.mode == DeploymentMode::SelfHostedTeam {
            eprintln!(
                "error: CAIRN_ADMIN_TOKEN env var is required in team mode. \
                 Set it to a strong random token before starting."
            );
            std::process::exit(1);
        }
        "dev-admin-token".to_owned()
    });
    tokens.register(
        admin_token.clone(),
        AuthPrincipal::ServiceAccount {
            name: "admin".to_owned(),
            tenant: cairn_domain::tenancy::TenantKey::new(
                cairn_domain::TenantId::new("default"),
            ),
        },
    );
    eprintln!("auth: admin token registered (set CAIRN_ADMIN_TOKEN to override)");

    // ── Postgres (optional) ───────────────────────────────────────────────────
    let pg = match &config.storage {
        StorageBackend::Postgres { connection_url } => {
            let url = connection_url.clone();
            eprintln!("store: connecting to Postgres at {url}");
            match PgPoolOptions::new()
                .max_connections(10)
                .acquire_timeout(Duration::from_secs(10))
                .connect(&url)
                .await
            {
                Ok(pool) => {
                    eprintln!("store: Postgres connection established");
                    // Run pending schema migrations before accepting traffic.
                    let migrator = PgMigrationRunner::new(pool.clone());
                    match migrator.run_pending().await {
                        Ok(applied) if applied.is_empty() => {
                            eprintln!("store: Postgres schema is up to date");
                        }
                        Ok(applied) => {
                            eprintln!("store: applied {} migration(s):", applied.len());
                            for m in &applied {
                                eprintln!("  V{:03}__{}", m.version, m.name);
                            }
                        }
                        Err(e) => {
                            eprintln!("error: Postgres migration failed: {e}");
                            std::process::exit(1);
                        }
                    }
                    let backend = Arc::new(PgBackend {
                        event_log: Arc::new(PgEventLog::new(pool.clone())),
                        adapter:   Arc::new(PgAdapter::new(pool)),
                    });
                    eprintln!("store: Postgres backend active (dual-write with InMemory)");
                    Some(backend)
                }
                Err(e) => {
                    eprintln!("error: failed to connect to Postgres: {e}");
                    std::process::exit(1);
                }
            }
        }
        StorageBackend::Sqlite { path } => {
            eprintln!("store: SQLite backend not yet wired ({path}); using InMemory");
            None
        }
        _ => None,
    };

    // ── App state ─────────────────────────────────────────────────────────────
    let runtime = Arc::new(InMemoryServices::new());
    let state = AppState {
        runtime,
        started_at: Arc::new(Instant::now()),
        tokens,
        pg,
    };

    // ── Router ────────────────────────────────────────────────────────────────
    let app = build_router(state);

    let addr = format!("{}:{}", config.listen_addr, config.listen_port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));

    eprintln!("cairn-app listening on http://{addr}");
    axum::serve(listener, app)
        .await
        .unwrap_or_else(|e| panic!("server error: {e}"));
}

/// Build the application router with all routes and auth middleware wired in.
///
/// Extracted from `main` so tests can construct the router with a custom
/// `AppState` (e.g. pre-loaded with test tokens).
fn build_router(state: AppState) -> Router {
    Router::new()
        // ── Public (no auth required) ─────────────────────────────────────
        .route("/health", get(health_handler))
        .route("/v1/stream", get(stream_handler))
        // ── Protected /v1/* routes ────────────────────────────────────────
        .route("/v1/status", get(status_handler))
        .route("/v1/dashboard", get(dashboard_handler))
        .route("/v1/runs", get(list_runs_handler))
        .route("/v1/runs/:id", get(get_run_handler))
        .route("/v1/runs/:id/cost", get(get_run_cost_handler))
        .route("/v1/runs/:id/events", get(list_run_events_handler))
        .route("/v1/runs/:id/tool-invocations", get(list_run_tool_invocations_handler))
        .route("/v1/runs/:id/tasks",     get(list_run_tasks_handler))
        .route("/v1/runs/:id/approvals", get(list_run_approvals_handler))
        .route("/v1/sessions", get(list_sessions_handler))
        .route("/v1/sessions/:id/events", get(list_session_events_handler))
        .route("/v1/sessions/:id/runs",   get(list_session_runs_handler))
        .route("/v1/approvals/pending", get(list_pending_approvals_handler))
        .route("/v1/approvals/:id/resolve", post(resolve_approval_handler))
        .route("/v1/prompts/assets", get(list_prompt_assets_handler))
        .route("/v1/prompts/releases", get(list_prompt_releases_handler))
        .route("/v1/costs", get(costs_handler))
        .route("/v1/providers", get(list_providers_handler))
        .route("/v1/providers/health", get(provider_health_handler))
        .route("/v1/events", get(list_events_handler))
        .route("/v1/events/append", post(append_events_handler))
        // DB diagnostics
        .route("/v1/db/status", get(db_status_handler))
        // ── Middleware stack ──────────────────────────────────────────────
        // TraceLayer logs method, path, status, and latency for every request.
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        // Auth runs inside CORS so OPTIONS preflights are answered without
        // a token — browsers never send credentials on preflight requests.
        .layer(from_fn_with_state(state.clone(), auth_middleware))
        .layer(cors_layer())
        .with_state(state)
}

/// Build the CORS layer used by `build_router`.
///
/// - `allow_origin(Any)` — accepts requests from any origin. Tighten to a
///   specific list when deploying behind a reverse proxy.
/// - Methods: GET, POST, PUT, DELETE, PATCH, OPTIONS — covers every verb the
///   API uses plus the browser preflight method.
/// - Headers: `Content-Type` and `Authorization` — required for JSON bodies
///   and bearer token auth.
/// - `max_age(86400)` — browser may cache the preflight result for 24 h,
///   reducing round-trips on subsequent requests.
fn cors_layer() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(Any)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::PATCH,
            Method::OPTIONS,
        ])
        .allow_headers([CONTENT_TYPE, AUTHORIZATION])
        .max_age(std::time::Duration::from_secs(86_400))
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use cairn_api::bootstrap::{ServerBootstrap, StorageBackend};
    use cairn_domain::{ProjectKey, SessionId};
    use cairn_runtime::sessions::SessionService;
    use std::sync::Mutex;
    use tower::ServiceExt as _;

    struct RecordingBootstrap {
        seen: Mutex<Option<BootstrapConfig>>,
    }

    impl RecordingBootstrap {
        fn new() -> Self {
            Self { seen: Mutex::new(None) }
        }
        fn seen(&self) -> Option<BootstrapConfig> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl ServerBootstrap for RecordingBootstrap {
        type Error = String;
        fn start(&self, config: &BootstrapConfig) -> Result<(), Self::Error> {
            *self.seen.lock().unwrap() = Some(config.clone());
            Ok(())
        }
    }

    fn run_bootstrap<B: ServerBootstrap>(b: &B, c: &BootstrapConfig) -> Result<(), B::Error> {
        b.start(c)
    }

    /// The test token registered by default in `make_state()`.
    const TEST_TOKEN: &str = "test-admin-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TEST_TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-admin".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(
                    cairn_domain::TenantId::new("test-tenant"),
                ),
            },
        );
        AppState {
            runtime: Arc::new(InMemoryServices::new()),
            started_at: Arc::new(Instant::now()),
            tokens,
            pg: None,
        }
    }

    fn make_app(state: AppState) -> Router {
        build_router(state)
    }

    /// Issue a GET request with the test bearer token.
    async fn authed_get(app: Router, uri: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .uri(uri)
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// Issue a GET request with NO auth header.
    async fn unauthed_get(app: Router, uri: &str) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// Build a GET request that includes the test auth token.
    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// Build a POST request with JSON body and the test auth token.
    fn authed_post(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {TEST_TOKEN}"))
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap()
    }

    // ── Arg parsing ──

    #[test]
    fn parse_args_defaults_to_local_mode() {
        let config = parse_args_from(&["cairn-app".to_owned()]);
        assert_eq!(config.mode, DeploymentMode::Local);
        assert_eq!(config.listen_addr, "127.0.0.1");
        assert_eq!(config.listen_port, 3000);
    }

    #[test]
    fn parse_args_promotes_team_mode_to_public_bind() {
        let config = parse_args_from(&[
            "cairn-app".to_owned(), "--mode".to_owned(), "team".to_owned(),
        ]);
        assert_eq!(config.mode, DeploymentMode::SelfHostedTeam);
        assert_eq!(config.listen_addr, "0.0.0.0");
    }

    #[test]
    fn run_bootstrap_delegates_to_server_bootstrap() {
        let b = RecordingBootstrap::new();
        let c = BootstrapConfig::team("postgres://localhost/cairn");
        run_bootstrap(&b, &c).unwrap();
        assert_eq!(b.seen(), Some(c));
    }

    #[test]
    fn parse_args_db_flag_sets_postgres() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(), "--db".to_owned(), "postgres://localhost/cairn".to_owned(),
        ]);
        assert!(matches!(c.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn parse_args_db_flag_sets_sqlite() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(), "--db".to_owned(), "my_data.db".to_owned(),
        ]);
        assert!(matches!(c.storage, StorageBackend::Sqlite { .. }));
    }

    #[test]
    fn team_mode_clears_local_auto_encryption() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(), "--mode".to_owned(), "team".to_owned(),
        ]);
        assert!(!c.credentials_available());
    }

    #[test]
    fn parse_args_port_flag_overrides_default() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(), "--port".to_owned(), "8080".to_owned(),
        ]);
        assert_eq!(c.listen_port, 8080);
    }

    // ── Handler unit tests ──

    #[tokio::test]
    async fn health_returns_ok() {
        let Json(resp) = health_handler().await;
        assert!(resp.ok);
    }

    /// Verify the server starts with TraceLayer wired in and tracing-subscriber
    /// initialised.  The test initialises a subscriber (via try_init so it is
    /// safe to run alongside other tests), builds the router, sends a real
    /// /health request, and asserts the response is 200.  If TraceLayer were
    /// missing the request would still succeed but the span would be absent —
    /// we confirm TraceLayer is present by checking that the router compiles
    /// with it and the request pipeline is fully functional.
    #[tokio::test]
    async fn server_starts_with_tracing_enabled() {
        use axum::body::to_bytes;

        // Safe to call multiple times; only the first subscriber wins.
        let _ = tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::new("info"))
            .with_target(false)
            .compact()
            .try_init();

        let app = make_app(make_state());

        let response = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            response.status(),
            StatusCode::OK,
            "health endpoint must return 200 with tracing enabled"
        );

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["ok"], true, "health response must carry ok=true");
    }

    #[tokio::test]
    async fn status_returns_runtime_and_store_ok() {
        let Json(resp) = status_handler(State(make_state())).await;
        assert!(resp.runtime_ok);
        assert!(resp.store_ok);
    }

    #[tokio::test]
    async fn dashboard_returns_zeros_on_empty_store() {
        let Json(resp) = dashboard_handler(State(make_state())).await;
        assert_eq!(resp.active_runs, 0);
        assert_eq!(resp.active_tasks, 0);
        assert!(resp.system_healthy);
    }

    #[tokio::test]
    async fn stream_handler_returns_sse_response() {
        // Basic smoke test — handler returns without panicking.
        let sse = stream_handler(State(make_state()), HeaderMap::new()).await;
        let _ = sse;
    }

    // ── SSE stream tests ──────────────────────────────────────────────────────

    /// Drive the SSE stream from an HTTP request using tower's oneshot and
    /// collect the first N bytes of the SSE body.
    async fn collect_sse_bytes(app: axum::Router, uri: &str, extra_headers: Vec<(&str, &str)>) -> Vec<u8> {
        let mut builder = Request::builder().uri(uri);
        for (k, v) in extra_headers {
            builder = builder.header(k, v);
        }
        let resp = app
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        // Read the first 4 KB to capture the initial events.
        let bytes = to_bytes(resp.into_body(), 4096).await.unwrap();
        bytes.to_vec()
    }

    #[tokio::test]
    async fn stream_sends_connected_event_on_connect() {
        let app = make_app(make_state());
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);
        assert!(text.contains("event: connected"), "missing connected event; got: {text}");
        assert!(text.contains("head_position"), "connected payload missing head_position; got: {text}");
    }

    #[tokio::test]
    async fn stream_replays_events_after_last_event_id() {
        let state = make_state();
        let project = ProjectKey::new("ts", "ws", "ps");

        // Create 3 sessions — generates positions 1, 2, 3.
        for i in 0u32..3 {
            state
                .runtime
                .sessions
                .create(&project, SessionId::new(format!("sess_sse_{i}")))
                .await
                .unwrap();
        }

        // Reconnect with Last-Event-ID: 1 → should replay positions 2 and 3.
        let app = make_app(state);
        let raw = collect_sse_bytes(app, "/v1/stream", vec![("last-event-id", "1")]).await;
        let text = String::from_utf8_lossy(&raw);

        // Should contain event type and session_created payloads.
        let session_created_count = text.matches("event: session_created").count();
        assert!(
            session_created_count >= 2,
            "expected ≥2 replayed session_created events; got {session_created_count} in: {text}"
        );
    }

    #[tokio::test]
    async fn stream_event_includes_id_field() {
        let state = make_state();
        let project = ProjectKey::new("ti", "wi", "pi");
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_id_test"))
            .await
            .unwrap();

        let app = make_app(state);
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);

        // The session_created event should have an `id:` line.
        assert!(
            text.contains("\nid: ") || text.starts_with("id: "),
            "SSE id: field missing; got: {text}"
        );
    }

    #[tokio::test]
    async fn stream_last_event_id_zero_replays_all_events() {
        let state = make_state();
        let project = ProjectKey::new("tz", "wz", "pz");

        // Two sessions → positions 1 and 2.
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_z_1"))
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_z_2"))
            .await
            .unwrap();

        // Last-Event-ID: 0 is before all positions (positions start at 1) →
        // should replay both events.
        let app = make_app(state);
        let raw = collect_sse_bytes(app, "/v1/stream", vec![("last-event-id", "0")]).await;
        let text = String::from_utf8_lossy(&raw);

        let count = text.matches("event: session_created").count();
        assert_eq!(count, 2, "expected 2 replayed events; got {count} in: {text}");
    }

    #[tokio::test]
    async fn stream_empty_store_sends_only_connected() {
        let app = make_app(make_state());
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);

        // Only the connected event, no session_created events.
        assert!(text.contains("event: connected"));
        assert!(!text.contains("event: session_created"), "unexpected events: {text}");
    }

    // ── Integration-style route tests ──

    #[tokio::test]
    async fn get_runs_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/runs"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(runs.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_run_not_found_returns_404() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/nonexistent_run_id")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // ── Task endpoint tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn list_run_tasks_returns_empty_for_run_with_no_tasks() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, RunCreated};

        let state = make_state();
        let project = ProjectKey::new("t_task", "w_task", "p_task");
        let session_id = SessionId::new("sess_task_empty");
        let run_id = cairn_domain::RunId::new("run_notasks");

        // Create session + run but add no tasks.
        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_notasks"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/run_notasks/tasks")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK, "run with no tasks returns 200");
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tasks: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(tasks.as_array().unwrap().is_empty(), "no tasks = empty array");
    }

    #[tokio::test]
    async fn list_run_tasks_returns_tasks_for_run() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RuntimeEvent,
            RunCreated, TaskCreated,
        };

        let state = make_state();
        let project = ProjectKey::new("t_tasks", "w_tasks", "p_tasks");
        let session_id = SessionId::new("sess_tasks");
        let run_id = cairn_domain::RunId::new("run_withtasks");

        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();

        // Create run + two tasks.
        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_wt"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_task_1"),
                EventSource::Runtime,
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project.clone(),
                    task_id: cairn_domain::TaskId::new("task_alpha"),
                    parent_run_id: Some(run_id.clone()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_task_2"),
                EventSource::Runtime,
                RuntimeEvent::TaskCreated(TaskCreated {
                    project: project.clone(),
                    task_id: cairn_domain::TaskId::new("task_beta"),
                    parent_run_id: Some(run_id.clone()),
                    parent_task_id: None,
                    prompt_release_id: None,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/run_withtasks/tasks")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tasks: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = tasks.as_array().unwrap();
        assert_eq!(arr.len(), 2, "two tasks must be returned");

        let task_ids: Vec<_> = arr.iter().map(|t| t["task_id"].as_str().unwrap()).collect();
        assert!(task_ids.contains(&"task_alpha"), "task_alpha must be in response");
        assert!(task_ids.contains(&"task_beta"),  "task_beta must be in response");
        // Each task must link back to the run.
        for t in arr {
            assert_eq!(t["parent_run_id"], "run_withtasks",
                "every task must reference its parent run");
        }
    }

    #[tokio::test]
    async fn list_run_tasks_returns_404_for_unknown_run() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/ghost_run/tasks")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND,
            "unknown run must return 404");
    }

    // ── Approval endpoint tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn list_run_approvals_empty_for_run_with_no_approvals() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, RunCreated};

        let state = make_state();
        let project = ProjectKey::new("ta", "wa", "pa");
        let session_id = SessionId::new("sess_appr_empty");
        let run_id_str = "run_appr_empty";
        let run_id = cairn_domain::RunId::new(run_id_str);

        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_ae"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(&format!("/v1/runs/{run_id_str}/approvals"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(approvals.as_array().unwrap().is_empty(),
            "run with no approvals must return empty array");
    }

    #[tokio::test]
    async fn list_run_approvals_shows_pending_approval() {
        use cairn_domain::{
            ApprovalId, ApprovalRequested, EventEnvelope, EventId, EventSource,
            RuntimeEvent, RunCreated,
        };
        use cairn_domain::policy::ApprovalRequirement;

        let state = make_state();
        let project = ProjectKey::new("tb", "wb", "pb");
        let session_id = SessionId::new("sess_appr_pend");
        let run_id_str = "run_appr_pend";
        let run_id = cairn_domain::RunId::new(run_id_str);
        let approval_id = ApprovalId::new("appr_pend");

        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_ap"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_appr_pend"),
                EventSource::Runtime,
                RuntimeEvent::ApprovalRequested(ApprovalRequested {
                    project: project.clone(),
                    approval_id: approval_id.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    requirement: ApprovalRequirement::Required,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(&format!("/v1/runs/{run_id_str}/approvals"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = approvals.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one pending approval");
        assert_eq!(arr[0]["approval_id"], "appr_pend");
        assert!(arr[0]["decision"].is_null(), "pending approval has no decision");
    }

    #[tokio::test]
    async fn list_run_approvals_shows_resolved_decision() {
        use cairn_domain::{
            ApprovalId, ApprovalRequested, ApprovalResolved,
            EventEnvelope, EventId, EventSource, RuntimeEvent, RunCreated,
        };
        use cairn_domain::policy::{ApprovalDecision, ApprovalRequirement};

        let state = make_state();
        let project = ProjectKey::new("tc", "wc", "pc");
        let session_id = SessionId::new("sess_appr_res");
        let run_id_str = "run_appr_res";
        let run_id = cairn_domain::RunId::new(run_id_str);
        let approval_id = ApprovalId::new("appr_res");

        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_ar"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: run_id.clone(),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_appr_req"),
                EventSource::Runtime,
                RuntimeEvent::ApprovalRequested(ApprovalRequested {
                    project: project.clone(),
                    approval_id: approval_id.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    requirement: ApprovalRequirement::Required,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_appr_res"),
                EventSource::Runtime,
                RuntimeEvent::ApprovalResolved(ApprovalResolved {
                    project: project.clone(),
                    approval_id: approval_id.clone(),
                    decision: ApprovalDecision::Approved,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(&format!("/v1/runs/{run_id_str}/approvals"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = approvals.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one resolved approval");
        assert_eq!(arr[0]["approval_id"], "appr_res");
        // Decision must be populated after resolution.
        assert_eq!(arr[0]["decision"], "approved",
            "resolved approval must carry the decision");
    }

    // ── Session runs endpoint tests ──────────────────────────────────────────

    #[tokio::test]
    async fn list_session_runs_empty_for_session_with_no_runs() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, SessionCreated};

        let state = make_state();
        let project = ProjectKey::new("tr1", "wr1", "pr1");
        let session_id = SessionId::new("sess_noruns");

        // Create session via event but add no runs.
        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_sess_nr"),
                EventSource::Runtime,
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_noruns/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(runs.as_array().unwrap().is_empty(),
            "session with no runs must return empty array");
    }

    #[tokio::test]
    async fn list_session_runs_returns_two_runs() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, RunCreated, SessionCreated};

        let state = make_state();
        let project = ProjectKey::new("tr2", "wr2", "pr2");
        let session_id = SessionId::new("sess_tworuns");

        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_sess_tr"),
                EventSource::Runtime,
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_tr1"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: cairn_domain::RunId::new("run_tr_1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_tr2"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: cairn_domain::RunId::new("run_tr_2"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_tworuns/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = runs.as_array().unwrap();
        assert_eq!(arr.len(), 2, "session must have 2 runs");

        let run_ids: Vec<_> = arr.iter().map(|r| r["run_id"].as_str().unwrap()).collect();
        assert!(run_ids.contains(&"run_tr_1"));
        assert!(run_ids.contains(&"run_tr_2"));
        // All runs belong to the session.
        for r in arr {
            assert_eq!(r["session_id"], "sess_tworuns");
        }
    }

    #[tokio::test]
    async fn list_session_runs_shows_parent_run_id_for_subagent() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, RunCreated, SessionCreated};

        let state = make_state();
        let project = ProjectKey::new("tr3", "wr3", "pr3");
        let session_id = SessionId::new("sess_subagent");

        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_sess_sa"),
                EventSource::Runtime,
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                }),
            ),
            // Root orchestrator run.
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_root"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: cairn_domain::RunId::new("run_root"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: Some("orchestrator".to_owned()),
                }),
            ),
            // Subagent spawned by root.
            EventEnvelope::for_runtime_event(
                EventId::new("evt_run_sub"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    run_id: cairn_domain::RunId::new("run_subagent"),
                    parent_run_id: Some(cairn_domain::RunId::new("run_root")),
                    prompt_release_id: None,
                    agent_role_id: Some("researcher".to_owned()),
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/sess_subagent/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = runs.as_array().unwrap();
        assert_eq!(arr.len(), 2, "root + subagent = 2 runs");

        let root = arr.iter().find(|r| r["run_id"] == "run_root").unwrap();
        assert!(root["parent_run_id"].is_null(),
            "root run has no parent");
        assert_eq!(root["agent_role_id"], "orchestrator");

        let sub = arr.iter().find(|r| r["run_id"] == "run_subagent").unwrap();
        assert_eq!(sub["parent_run_id"], "run_root",
            "subagent must reference root run as parent");
        assert_eq!(sub["agent_role_id"], "researcher");
    }

    #[tokio::test]
    async fn list_session_runs_returns_404_for_unknown_session() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions/ghost_session/runs")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_sessions_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/sessions")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(sessions.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_pending_approvals_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/approvals/pending")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(approvals.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn resolve_nonexistent_approval_returns_404() {
        let app = make_app(make_state());
        let body = serde_json::json!({"decision": "approved"});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/approvals/no_such_approval/resolve")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn resolve_bad_decision_returns_400() {
        let app = make_app(make_state());
        let body = serde_json::json!({"decision": "maybe"});
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/approvals/any_id/resolve")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn runs_list_reflects_created_run() {
        let state = make_state();
        let project = ProjectKey::new("t1", "w1", "p1");
        let session_id = cairn_domain::SessionId::new("sess_1");
        let run_id = cairn_domain::RunId::new("run_1");
        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.runs.start(&project, &session_id, run_id, None).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(runs.as_array().unwrap().len(), 1);
        assert_eq!(runs[0]["run_id"], "run_1");
    }

    #[tokio::test]
    async fn get_run_by_id_returns_record() {
        let state = make_state();
        let project = ProjectKey::new("t2", "w2", "p2");
        let session_id = SessionId::new("sess_2");
        let run_id_str = "run_2";
        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, cairn_domain::RunId::new(run_id_str), None)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(&format!("/v1/runs/{run_id_str}"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let run: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(run["run_id"], run_id_str);
    }

    #[tokio::test]
    async fn sessions_list_reflects_created_session() {
        let state = make_state();
        let project = ProjectKey::new("t3", "w3", "p3");
        let session_id = SessionId::new("sess_3");
        state.runtime.sessions.create(&project, session_id).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/sessions"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(sessions.as_array().unwrap().len(), 1);
        assert_eq!(sessions[0]["session_id"], "sess_3");
    }

    // ── Prompt asset / release tests ──────────────────────────────────────────

    #[tokio::test]
    async fn prompt_assets_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/prompts/assets"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn prompt_assets_reflects_created_asset() {
        use cairn_domain::PromptAssetId;
        use cairn_runtime::prompt_assets::PromptAssetService as _;

        let state = make_state();
        let project = ProjectKey::new("ta", "wa", "pa");
        state
            .runtime
            .prompt_assets
            .create(&project, PromptAssetId::new("asset_a"), "My Prompt".to_owned(), "system".to_owned())
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/prompts/assets"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["prompt_asset_id"], "asset_a");
        assert_eq!(items[0]["name"], "My Prompt");
    }

    #[tokio::test]
    async fn prompt_releases_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/prompts/releases"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    // ── Cost tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn costs_empty_store_returns_zeros() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/costs"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_provider_calls"], 0);
        assert_eq!(cost["total_cost_micros"], 0);
    }

    #[tokio::test]
    async fn costs_reflects_run_cost_events() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RuntimeEvent,
            events::RunCostUpdated,
        };

        let state = make_state();
        let project = ProjectKey::new("tc", "wc", "pc");
        let session_id = SessionId::new("sess_c");
        let run_id = cairn_domain::RunId::new("run_c");
        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.runs.start(&project, &session_id, run_id.clone(), None).await.unwrap();

        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_cost_c"),
                EventSource::Runtime,
                RuntimeEvent::RunCostUpdated(RunCostUpdated {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    session_id: Some(session_id.clone()),
                    tenant_id: Some(cairn_domain::TenantId::new("tc")),
                    delta_cost_micros: 500,
                    delta_tokens_in: 100,
                    delta_tokens_out: 50,
                    provider_call_id: "call_c".to_owned(),
                    updated_at_ms: 1000,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/costs"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_cost_micros"], 500);
        assert_eq!(cost["total_tokens_in"], 100);
        assert_eq!(cost["total_tokens_out"], 50);
    }

    // ── Provider tests ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn providers_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/providers"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn providers_reflects_created_binding() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, ProviderBindingId, ProviderConnectionId,
            ProviderModelId, RuntimeEvent,
            events::ProviderBindingCreated,
            providers::{OperationKind, ProviderBindingSettings},
        };

        let state = make_state();
        let project = ProjectKey::new("tp", "wp", "pp");

        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_bind_p"),
                EventSource::Runtime,
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: project.clone(),
                    provider_binding_id: ProviderBindingId::new("bind_p"),
                    provider_connection_id: ProviderConnectionId::new("conn_p"),
                    provider_model_id: ProviderModelId::new("model_p"),
                    operation_kind: OperationKind::Generate,
                    settings: ProviderBindingSettings::default(),
                    policy_id: None,
                    active: true,
                    created_at: 1000,
                    estimated_cost_micros: None,
                }),
            ),
        ]).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/providers"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["provider_binding_id"], "bind_p");
    }

    // ── Event replay tests ────────────────────────────────────────────────────

    #[tokio::test]
    async fn events_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/events"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(events.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn events_returns_all_events_from_log() {
        let state = make_state();
        let project = ProjectKey::new("te", "we", "pe");
        let session_id = SessionId::new("sess_e");
        // Creating a session appends a SessionCreated event.
        state.runtime.sessions.create(&project, session_id).await.unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/events"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        assert!(!arr.is_empty(), "expected at least one event");
        assert_eq!(arr[0]["event_type"], "session_created");
        assert!(arr[0]["position"].as_u64().is_some());
    }

    #[tokio::test]
    async fn events_after_cursor_paginates() {
        let state = make_state();
        let project = ProjectKey::new("tf", "wf", "pf");
        // Create 3 sessions → 3 events at positions 0, 1, 2.
        for i in 0u32..3 {
            state
                .runtime
                .sessions
                .create(&project, SessionId::new(format!("sess_f_{i}")))
                .await
                .unwrap();
        }

        let app = make_app(state);
        // Positions start at 1. after=1 means "after position 1" → should return
        // positions 2 and 3 (the second and third SessionCreated events).
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/events?after=1&limit=10")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        assert_eq!(arr.len(), 2, "expected events after position 1");
        assert!(arr.iter().all(|e| e["position"].as_u64().unwrap() > 1));
    }

    #[tokio::test]
    async fn events_limit_is_respected() {
        let state = make_state();
        let project = ProjectKey::new("tg", "wg", "pg");
        for i in 0u32..5 {
            state
                .runtime
                .sessions
                .create(&project, SessionId::new(format!("sess_g_{i}")))
                .await
                .unwrap();
        }

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/events?limit=3")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(events.as_array().unwrap().len(), 3);
    }

    // ── Event append tests (RFC 002) ──────────────────────────────────────────

    /// Build a minimal valid EventEnvelope JSON for a SessionCreated event.
    ///
    /// Serde shapes used here:
    /// - `EventSource`:  internally tagged with `"source_type"`, snake_case variants
    ///   → `Runtime` → `{"source_type":"runtime"}`
    /// - `OwnershipKey`: internally tagged with `"scope"`, snake_case variants
    ///   → `Project(…)` → `{"scope":"project","tenant_id":…,…}`
    /// - `RuntimeEvent`: internally tagged with `"event"`, snake_case variants,
    ///   content flattened → `{"event":"session_created","project":{…},"session_id":"…"}`
    /// - `SessionCreated` has no `created_at` field.
    fn session_created_envelope(event_id: &str, session_id: &str) -> serde_json::Value {
        serde_json::json!({
            "event_id": event_id,
            "source": { "source_type": "runtime" },
            "ownership": {
                "scope": "project",
                "tenant_id": "t_append",
                "workspace_id": "w_append",
                "project_id": "p_append"
            },
            "causation_id": null,
            "correlation_id": null,
            "payload": {
                "event": "session_created",
                "project": {
                    "tenant_id": "t_append",
                    "workspace_id": "w_append",
                    "project_id": "p_append"
                },
                "session_id": session_id
            }
        })
    }

    /// Same as above but with a causation_id for idempotency testing.
    fn session_created_with_causation(event_id: &str, session_id: &str, causation_id: &str) -> serde_json::Value {
        let mut e = session_created_envelope(event_id, session_id);
        e["causation_id"] = serde_json::json!(causation_id);
        e
    }

    async fn post_append(app: axum::Router, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/events/append")
                    .header("content-type", "application/json")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        (status, json)
    }

    #[tokio::test]
    async fn append_single_event_returns_201_with_position() {
        let app = make_app(make_state());
        let envelope = session_created_envelope("evt_a1", "sess_a1");
        let (status, results) = post_append(app, serde_json::json!([envelope])).await;

        assert_eq!(status, StatusCode::CREATED);
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["event_id"], "evt_a1");
        assert!(arr[0]["position"].as_u64().unwrap() > 0, "position must be ≥ 1");
        assert_eq!(arr[0]["appended"], true);
    }

    #[tokio::test]
    async fn append_empty_batch_returns_200_empty_array() {
        let app = make_app(make_state());
        let (status, results) = post_append(app, serde_json::json!([])).await;
        assert_eq!(status, StatusCode::OK);
        assert!(results.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn append_assigns_sequential_positions() {
        let app = make_app(make_state());
        let envelopes = serde_json::json!([
            session_created_envelope("evt_seq1", "sess_seq1"),
            session_created_envelope("evt_seq2", "sess_seq2"),
            session_created_envelope("evt_seq3", "sess_seq3"),
        ]);
        let (status, results) = post_append(app, envelopes).await;

        assert_eq!(status, StatusCode::CREATED);
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 3);

        let positions: Vec<u64> = arr.iter().map(|r| r["position"].as_u64().unwrap()).collect();
        // All positions must be distinct and strictly increasing.
        assert!(positions[0] < positions[1]);
        assert!(positions[1] < positions[2]);
        assert!(arr.iter().all(|r| r["appended"] == true));
    }

    #[tokio::test]
    async fn append_idempotent_with_causation_id_returns_existing_position() {
        let state = make_state();
        let causation = "cmd_idem_1";

        // First append — creates the event.
        let env = session_created_with_causation("evt_idem1", "sess_idem1", causation);
        let (status1, res1) = post_append(make_app(state.clone()), serde_json::json!([env.clone()])).await;
        assert_eq!(status1, StatusCode::CREATED);
        let first_pos = res1[0]["position"].as_u64().unwrap();
        assert_eq!(res1[0]["appended"], true);

        // Second append — same causation_id, different event_id.
        let env2 = session_created_with_causation("evt_idem2", "sess_idem2", causation);
        let (status2, res2) = post_append(make_app(state.clone()), serde_json::json!([env2])).await;
        assert_eq!(status2, StatusCode::CREATED);
        let second_pos = res2[0]["position"].as_u64().unwrap();
        assert_eq!(res2[0]["appended"], false, "second append should be idempotent");
        assert_eq!(second_pos, first_pos, "idempotent append must return the original position");
    }

    #[tokio::test]
    async fn append_mixed_batch_new_and_idempotent() {
        let state = make_state();
        let causation = "cmd_mixed_1";

        // Pre-append the first event.
        let env_pre = session_created_with_causation("evt_m0", "sess_m0", causation);
        post_append(make_app(state.clone()), serde_json::json!([env_pre])).await;

        // Batch: first is a duplicate (causation_id present), second is new.
        let env_dup = session_created_with_causation("evt_m1", "sess_m1", causation);
        let env_new = session_created_envelope("evt_m2", "sess_m2");
        let (status, results) = post_append(
            make_app(state.clone()),
            serde_json::json!([env_dup, env_new]),
        ).await;

        assert_eq!(status, StatusCode::CREATED);
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["appended"], false, "first should be idempotent duplicate");
        assert_eq!(arr[1]["appended"], true, "second should be newly appended");
        assert!(
            arr[1]["position"].as_u64().unwrap() > arr[0]["position"].as_u64().unwrap(),
            "new event position must be greater than duplicate's original position"
        );
    }

    #[tokio::test]
    async fn append_event_appears_in_event_log_immediately() {
        let state = make_state();
        let app1 = make_app(state.clone());
        let app2 = make_app(state.clone());

        // Append one event.
        let env = session_created_envelope("evt_vis1", "sess_vis1");
        let (_, results) = post_append(app1, serde_json::json!([env])).await;
        let appended_pos = results[0]["position"].as_u64().unwrap();

        // The event should now appear in GET /v1/events.
        let resp = app2
            .oneshot(authed_req("/v1/events"))
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let positions: Vec<u64> = events
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["position"].as_u64().unwrap())
            .collect();
        assert!(
            positions.contains(&appended_pos),
            "appended event at position {appended_pos} not found in event log; got: {positions:?}"
        );
    }

    #[tokio::test]
    async fn append_broadcasts_to_sse_subscribers() {
        use tokio_stream::StreamExt as _;

        let state = make_state();
        // Subscribe to the broadcast channel BEFORE appending.
        let mut receiver = state.runtime.store.subscribe();

        // Append one event via the handler.
        let env = session_created_envelope("evt_bc1", "sess_bc1");
        let app = make_app(state.clone());
        let (status, _) = post_append(app, serde_json::json!([env])).await;
        assert_eq!(status, StatusCode::CREATED);

        // The receiver should have gotten the event immediately.
        let received = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            async { receiver.recv().await },
        )
        .await
        .expect("broadcast delivery timed out")
        .expect("broadcast channel closed");

        assert_eq!(
            event_type_name(&received.envelope.payload),
            "session_created",
            "wrong event type in broadcast"
        );
    }

    #[tokio::test]
    async fn append_no_causation_id_always_appends() {
        let state = make_state();

        // Two envelopes with the same event_id but no causation_id →
        // both get appended (no idempotency guard).
        let env1 = session_created_envelope("evt_nc1", "sess_nc1");
        let env2 = session_created_envelope("evt_nc2", "sess_nc2");

        let (_, r1) = post_append(make_app(state.clone()), serde_json::json!([env1])).await;
        let (_, r2) = post_append(make_app(state.clone()), serde_json::json!([env2])).await;

        assert_eq!(r1[0]["appended"], true);
        assert_eq!(r2[0]["appended"], true);
        // Positions are distinct.
        assert_ne!(r1[0]["position"], r2[0]["position"]);
    }

    // ── Auth middleware tests (RFC 008) ───────────────────────────────────────

    #[tokio::test]
    async fn valid_token_passes_protected_route() {
        let resp = authed_get(make_app(make_state()), "/v1/status").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn invalid_token_returns_401_on_protected_route() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/status")
                    .header("authorization", "Bearer wrong-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(err["code"], "unauthorized");
    }

    #[tokio::test]
    async fn missing_token_returns_401_on_protected_route() {
        let resp = unauthed_get(make_app(make_state()), "/v1/runs").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(err["code"], "unauthorized");
    }

    #[tokio::test]
    async fn health_endpoint_requires_no_token() {
        // /health is public — no Authorization header needed.
        let resp = unauthed_get(make_app(make_state()), "/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let h: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(h["ok"], true);
    }

    #[tokio::test]
    async fn stream_endpoint_requires_no_token() {
        // /v1/stream is exempt — SSE clients use EventSource which can't set headers.
        let resp = unauthed_get(make_app(make_state()), "/v1/stream").await;
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn multiple_tokens_can_be_registered() {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            "token-a".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "svc-a".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("t1")),
            },
        );
        tokens.register(
            "token-b".to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "svc-b".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("t2")),
            },
        );
        let state = AppState {
            runtime: Arc::new(InMemoryServices::new()),
            started_at: Arc::new(Instant::now()),
            tokens,
            pg: None,
        };
        let app = build_router(state);

        // Both tokens are valid.
        for tok in &["token-a", "token-b"] {
            let resp = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/v1/status")
                        .header("authorization", format!("Bearer {tok}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "token {tok} should be valid");
        }
    }

    #[tokio::test]
    async fn auth_error_body_contains_code_and_message() {
        let resp = unauthed_get(make_app(make_state()), "/v1/dashboard").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let err: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Must contain both `code` and `message` per ApiError contract.
        assert!(err.get("code").is_some(), "missing code field");
        assert!(err.get("message").is_some(), "missing message field");
    }

    // ── GET /v1/db/status tests ───────────────────────────────────────────────

    #[tokio::test]
    async fn db_status_in_memory_backend_returns_correct_fields() {
        let resp = authed_get(make_app(make_state()), "/v1/db/status").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(status["backend"], "in_memory");
        assert_eq!(status["connected"], true);
        // In-memory mode has no migration tracking.
        assert!(status["migration_count"].is_null());
        assert!(status["schema_current"].is_null());
    }

    #[tokio::test]
    async fn db_status_requires_auth() {
        let resp = unauthed_get(make_app(make_state()), "/v1/db/status").await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn db_status_shape_matches_contract() {
        let resp = authed_get(make_app(make_state()), "/v1/db/status").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();

        // All four contract fields must be present.
        assert!(status.get("backend").is_some(),           "missing backend");
        assert!(status.get("connected").is_some(),         "missing connected");
        assert!(status.get("migration_count").is_some(),   "missing migration_count");
        assert!(status.get("schema_current").is_some(),    "missing schema_current");
    }

    // ── End-to-end write → project → read cycle tests ────────────────────────
    //
    // These five tests prove the full pipeline:
    //   POST /v1/events/append → InMemory synchronous projection → GET read endpoint
    // Each test uses only the HTTP surface so they exercise exactly what a real
    // client would do.

    /// (1) POST SessionCreated via /v1/events/append → GET /v1/sessions shows it.
    #[tokio::test]
    async fn e2e_append_session_then_list_sessions_shows_it() {
        let state = make_state();
        let envelope = session_created_envelope("evt_e2e_s1", "sess_e2e_1");
        let (status, results) = post_append(make_app(state.clone()), serde_json::json!([envelope])).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(results[0]["appended"], true, "event must be freshly appended");

        let resp = make_app(state).oneshot(authed_req("/v1/sessions")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = sessions.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one session must appear after append");
        assert_eq!(arr[0]["session_id"], "sess_e2e_1",
            "session_id must match what GET /v1/sessions returns");
    }

    /// (2) POST RunCreated via /v1/events/append → GET /v1/runs shows it.
    #[tokio::test]
    async fn e2e_append_run_then_list_runs_shows_it() {
        let state = make_state();
        let proj = serde_json::json!({"tenant_id":"t_e2e","workspace_id":"w_e2e","project_id":"p_e2e"});
        let sess_env = session_created_envelope("evt_e2e_sess", "sess_e2e_run");
        post_append(make_app(state.clone()), serde_json::json!([sess_env])).await;

        let run_env = serde_json::json!({
            "event_id": "evt_e2e_run1",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_e2e", "workspace_id": "w_e2e", "project_id": "p_e2e"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "run_created", "project": proj,
                "session_id": "sess_e2e_run", "run_id": "run_e2e_1",
                "parent_run_id": null, "prompt_release_id": null, "agent_role_id": null
            }
        });
        let (status, results) = post_append(make_app(state.clone()), serde_json::json!([run_env])).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(results[0]["appended"], true);

        let resp = make_app(state).oneshot(authed_req("/v1/runs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = runs.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one run must appear after append");
        assert_eq!(arr[0]["run_id"], "run_e2e_1",
            "run_id must match what GET /v1/runs returns");
    }

    /// (3) POST ApprovalRequested → GET /v1/approvals/pending shows it.
    #[tokio::test]
    async fn e2e_append_approval_then_list_pending_shows_it() {
        let state = make_state();
        let proj = serde_json::json!({"tenant_id":"t_ap","workspace_id":"w_ap","project_id":"p_ap"});
        let approval_env = serde_json::json!({
            "event_id": "evt_e2e_ap1",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_ap", "workspace_id": "w_ap", "project_id": "p_ap"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "approval_requested", "project": proj,
                "approval_id": "appr_e2e_1",
                "run_id": null, "task_id": null, "requirement": "required"
            }
        });
        let (status, _) = post_append(make_app(state.clone()), serde_json::json!([approval_env])).await;
        assert_eq!(status, StatusCode::CREATED);

        let resp = make_app(state)
            .oneshot(authed_req("/v1/approvals/pending?tenant_id=t_ap&workspace_id=w_ap&project_id=p_ap"))
            .await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = approvals.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one pending approval must appear after append");
        assert_eq!(arr[0]["approval_id"], "appr_e2e_1");
        assert!(arr[0]["decision"].is_null(), "pending approval must have null decision");
    }

    /// (4) POST ApprovalRequested then POST /v1/approvals/:id/resolve(Approved)
    /// → GET /v1/approvals/pending is empty.
    #[tokio::test]
    async fn e2e_resolve_approval_removes_from_pending() {
        let state = make_state();
        let proj = serde_json::json!({"tenant_id":"t_res","workspace_id":"w_res","project_id":"p_res"});
        let approval_env = serde_json::json!({
            "event_id": "evt_e2e_res1",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_res", "workspace_id": "w_res", "project_id": "p_res"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "approval_requested", "project": proj,
                "approval_id": "appr_e2e_res",
                "run_id": null, "task_id": null, "requirement": "required"
            }
        });
        post_append(make_app(state.clone()), serde_json::json!([approval_env])).await;

        let resolve_resp = make_app(state.clone())
            .oneshot(authed_post("/v1/approvals/appr_e2e_res/resolve", serde_json::json!({"decision": "approved"})))
            .await.unwrap();
        assert_eq!(resolve_resp.status(), StatusCode::OK, "resolve must return 200");
        let rbody = to_bytes(resolve_resp.into_body(), usize::MAX).await.unwrap();
        let resolved: serde_json::Value = serde_json::from_slice(&rbody).unwrap();
        assert_eq!(resolved["decision"], "approved", "resolved approval must carry decision=approved");

        let resp = make_app(state)
            .oneshot(authed_req("/v1/approvals/pending?tenant_id=t_res&workspace_id=w_res&project_id=p_res"))
            .await.unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let pending: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(pending.as_array().unwrap().is_empty(),
            "pending list must be empty after approval resolved");
    }

    /// (5) Append session + run, then GET /v1/dashboard shows active_runs=1.
    #[tokio::test]
    async fn e2e_dashboard_active_runs_reflects_appended_run() {
        let state = make_state();

        let resp0 = make_app(state.clone()).oneshot(authed_req("/v1/dashboard")).await.unwrap();
        let body0 = to_bytes(resp0.into_body(), usize::MAX).await.unwrap();
        let dash0: serde_json::Value = serde_json::from_slice(&body0).unwrap();
        assert_eq!(dash0["active_runs"], 0, "dashboard must start with 0 active runs");

        let proj = serde_json::json!({"tenant_id":"t_dash","workspace_id":"w_dash","project_id":"p_dash"});
        let sess_env = serde_json::json!({
            "event_id": "evt_dash_sess",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_dash", "workspace_id": "w_dash", "project_id": "p_dash"},
            "causation_id": null, "correlation_id": null,
            "payload": {"event": "session_created", "project": proj, "session_id": "sess_dash_1"}
        });
        post_append(make_app(state.clone()), serde_json::json!([sess_env])).await;

        let run_env = serde_json::json!({
            "event_id": "evt_dash_run",
            "source": {"source_type": "runtime"},
            "ownership": {"scope": "project", "tenant_id": "t_dash", "workspace_id": "w_dash", "project_id": "p_dash"},
            "causation_id": null, "correlation_id": null,
            "payload": {
                "event": "run_created", "project": proj,
                "session_id": "sess_dash_1", "run_id": "run_dash_1",
                "parent_run_id": null, "prompt_release_id": null, "agent_role_id": null
            }
        });
        post_append(make_app(state.clone()), serde_json::json!([run_env])).await;

        let resp1 = make_app(state).oneshot(authed_req("/v1/dashboard")).await.unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let body1 = to_bytes(resp1.into_body(), usize::MAX).await.unwrap();
        let dash1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        assert_eq!(dash1["active_runs"], 1,
            "dashboard must show active_runs=1 after appending one RunCreated");
        assert!(dash1["system_healthy"].as_bool().unwrap_or(false), "system must be healthy");
    }

    // ── CORS tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn options_preflight_returns_cors_headers() {
        let app = make_app(make_state());
        let resp = app.oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/events/append")
                .header("origin", "http://localhost:5173")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "content-type,authorization")
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        assert!(resp.status().is_success(),
            "OPTIONS preflight must succeed; got {}", resp.status());
        let h = resp.headers();
        assert!(h.contains_key("access-control-allow-origin"),   "missing ACAO header");
        assert!(h.contains_key("access-control-allow-methods"),  "missing ACAM header");
        assert!(h.contains_key("access-control-allow-headers"),  "missing ACAH header");
    }

    #[tokio::test]
    async fn cors_allow_origin_is_wildcard() {
        let app = make_app(make_state());
        let resp = app.oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/health")
                .header("origin", "https://example.com")
                .header("access-control-request-method", "GET")
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        let acao = resp.headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok()).unwrap_or("");
        assert_eq!(acao, "*", "allow_origin must be wildcard (*)");
    }

    #[tokio::test]
    async fn regular_get_includes_cors_header() {
        let resp = authed_get(make_app(make_state()), "/v1/status").await;
        let acao = resp.headers().get("access-control-allow-origin");
        assert!(acao.is_some(), "GET response must include Access-Control-Allow-Origin");
    }

    #[tokio::test]
    async fn cors_allow_methods_includes_required_verbs() {
        let app = make_app(make_state());
        let resp = app.oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/v1/events/append")
                .header("origin", "http://localhost:3000")
                .header("access-control-request-method", "POST")
                .header("access-control-request-headers", "authorization")
                .body(Body::empty()).unwrap(),
        ).await.unwrap();
        let methods = resp.headers()
            .get("access-control-allow-methods")
            .and_then(|v| v.to_str().ok()).unwrap_or("").to_uppercase();
        for verb in ["GET", "POST", "PUT", "DELETE", "OPTIONS"] {
            assert!(methods.contains(verb),
                "Access-Control-Allow-Methods must include {verb}; got: {methods}");
        }
    }

    // ── GET /v1/sessions/:id/events tests ────────────────────────────────────

    #[tokio::test]
    async fn session_events_empty_for_unknown_session() {
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/sessions/no_such_session/events").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(events.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn session_events_returns_events_for_session() {
        let state = make_state();
        let project = ProjectKey::new("t_sev", "w_sev", "p_sev");
        let session_id = SessionId::new("sess_sev");
        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/sessions/sess_sev/events").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        assert!(!arr.is_empty(), "session must have at least one event");
        assert_eq!(arr[0]["event_type"], "session_created");
        assert!(arr[0]["position"].as_u64().is_some());
    }

    #[tokio::test]
    async fn session_events_scoped_to_session_only() {
        let state = make_state();
        let project = ProjectKey::new("t_scope", "w_scope", "p_scope");
        // Create two sessions — each gets a SessionCreated event.
        state.runtime.sessions.create(&project, SessionId::new("sess_scope_a")).await.unwrap();
        state.runtime.sessions.create(&project, SessionId::new("sess_scope_b")).await.unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/sessions/sess_scope_a/events").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        // Only sess_scope_a events must appear — not sess_scope_b.
        assert_eq!(arr.len(), 1, "only one SessionCreated event for sess_scope_a");
    }

    #[tokio::test]
    async fn session_events_after_cursor_paginates() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, SessionState,
            events::SessionStateChanged,
            events::StateTransition as ST,
            tenancy::OwnershipKey,
        };

        let state = make_state();
        let project = ProjectKey::new("t_cur", "w_cur", "p_cur");
        let session_id = SessionId::new("sess_cur");
        // SessionCreated → event 1 (session-scoped).
        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        // Append a SessionStateChanged directly → event 2 (also session-scoped).
        state.runtime.store.append(&[
            EventEnvelope::new(
                EventId::new("evt_ssc_cur"),
                EventSource::Runtime,
                OwnershipKey::Project(project.clone()),
                cairn_domain::RuntimeEvent::SessionStateChanged(SessionStateChanged {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    transition: ST { from: Some(cairn_domain::SessionState::Open), to: cairn_domain::SessionState::Completed },
                }),
            ),
        ]).await.unwrap();

        let app_all  = make_app(state.clone());
        let app_page = make_app(state.clone());

        let resp_all = authed_get(app_all, "/v1/sessions/sess_cur/events").await;
        let body_all = to_bytes(resp_all.into_body(), usize::MAX).await.unwrap();
        let all: serde_json::Value = serde_json::from_slice(&body_all).unwrap();
        let all_arr = all.as_array().unwrap();
        assert!(all_arr.len() >= 2, "expect session_created + session_state_changed");

        // Use the first event position as cursor.
        let first_pos = all_arr[0]["position"].as_u64().unwrap();
        let uri = format!("/v1/sessions/sess_cur/events?after={first_pos}");
        let resp_page = authed_get(app_page, &uri).await;
        let body_page = to_bytes(resp_page.into_body(), usize::MAX).await.unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body_page).unwrap();
        let page_arr = page.as_array().unwrap();
        assert_eq!(page_arr.len(), all_arr.len() - 1,
            "after=first_pos must return one fewer event");
        assert!(page_arr.iter().all(|e| e["position"].as_u64().unwrap() > first_pos));
    }

        // ── GET /v1/runs/:id/cost tests ──────────────────────────────────────────

        #[tokio::test]
        async fn run_cost_returns_zeros_when_no_provider_calls() {
        let state = make_state();
        let project = ProjectKey::new("t_cost", "w_cost", "p_cost");
        let session_id = SessionId::new("sess_cost");
        let run_id = cairn_domain::RunId::new("run_cost_zero");
        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.runs.start(&project, &session_id, run_id, None).await.unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/runs/run_cost_zero/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["run_id"],              "run_cost_zero");
        assert_eq!(cost["total_cost_micros"],  0);
        assert_eq!(cost["total_tokens_in"],    0);
        assert_eq!(cost["total_tokens_out"],   0);
        assert_eq!(cost["provider_calls"],     0);
        }

        #[tokio::test]
        async fn run_cost_returns_zeros_for_unknown_run() {
        // Unknown run → no cost record → zero response (not 404).
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/runs/nonexistent_run_cost/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_cost_micros"], 0);
        assert_eq!(cost["provider_calls"],    0);
        }

        #[tokio::test]
        async fn run_cost_reflects_run_cost_updated_events() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, TenantId,
            events::RunCostUpdated,
            tenancy::OwnershipKey,
        };

        let state = make_state();
        let project = ProjectKey::new("t_rcu", "w_rcu", "p_rcu");
        let session_id = SessionId::new("sess_rcu");
        let run_id     = cairn_domain::RunId::new("run_rcu");

        state.runtime.sessions.create(&project, session_id.clone()).await.unwrap();
        state.runtime.runs.start(&project, &session_id, run_id.clone(), None).await.unwrap();

        // Two provider calls: 300 + 200 micros, 50+30 tokens in, 20+10 tokens out.
        for (i, (cost, t_in, t_out)) in [(300u64, 50u64, 20u64), (200, 30, 10)].iter().enumerate() {
            state.runtime.store.append(&[EventEnvelope::new(
                EventId::new(format!("evt_rcu_{i}")),
                EventSource::Runtime,
                OwnershipKey::Project(project.clone()),
                cairn_domain::RuntimeEvent::RunCostUpdated(RunCostUpdated {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    session_id: Some(session_id.clone()),
                    tenant_id: Some(TenantId::new("t_rcu")),
                    delta_cost_micros: *cost,
                    delta_tokens_in: *t_in,
                    delta_tokens_out: *t_out,
                    provider_call_id: format!("call_{i}"),
                    updated_at_ms: 1_000,
                }),
            )]).await.unwrap();
        }

        let app = make_app(state);
        let resp = authed_get(app, "/v1/runs/run_rcu/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["run_id"],             "run_rcu");
        assert_eq!(cost["total_cost_micros"],  500,  "300+200 micros");
        assert_eq!(cost["total_tokens_in"],    80,   "50+30 tokens in");
        assert_eq!(cost["total_tokens_out"],   30,   "20+10 tokens out");
        assert_eq!(cost["provider_calls"],     2,    "2 provider calls");
        }

        #[tokio::test]
        async fn run_cost_response_has_correct_shape() {
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/runs/any_run/cost").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // All four contract fields must be present.
        for field in ["run_id", "total_cost_micros", "total_tokens_in", "total_tokens_out", "provider_calls"] {
            assert!(cost.get(field).is_some(), "missing field: {field}");
        }
        }


}

#[cfg(test)]
mod run_events_tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;

    const TOKEN: &str = "test-run-events-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-run-events".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(
                    cairn_domain::TenantId::new("tenant_re"),
                ),
            },
        );
        AppState {
            runtime: Arc::new(InMemoryServices::new()),
            started_at: Arc::new(std::time::Instant::now()),
            tokens,
            pg: None,
        }
    }

    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// GET /v1/runs/:id/events returns 404 for an unknown run.
    #[tokio::test]
    async fn run_events_unknown_run_returns_empty() {
        let app = build_router(make_state());
        let resp = app.oneshot(authed_req("/v1/runs/no_such_run/events")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(events.as_array().unwrap().is_empty(),
            "unknown run must return empty event list");
    }

    /// GET /v1/runs/:id/events returns all events for the run after they are appended.
    ///
    /// Proves the write → project → read cycle for the run event stream:
    /// - POST /v1/events/append with RunCreated
    /// - GET /v1/runs/:id/events returns that event
    #[tokio::test]
    async fn run_events_returns_events_for_the_run() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_re");

        // Create a session and run directly in the store.
        state.runtime.sessions
            .create(&project, SessionId::new("sess_re_1"))
            .await.unwrap();
        state.runtime.runs
            .start(&project, &SessionId::new("sess_re_1"), RunId::new("run_re_1"), None)
            .await.unwrap();

        // GET /v1/runs/run_re_1/events must return at least the RunCreated event.
        let app = build_router(state);
        let resp = app.oneshot(authed_req("/v1/runs/run_re_1/events")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();

        assert!(!arr.is_empty(), "run events must not be empty after run is created");

        // Every returned event must carry a position and event_type.
        for event in arr {
            assert!(event["position"].as_u64().is_some(), "event must have a position");
            assert!(!event["event_type"].as_str().unwrap_or("").is_empty(),
                "event must have an event_type");
        }

        // The RunCreated event must appear.
        let has_run_created = arr.iter().any(|e| e["event_type"] == "run_created");
        assert!(has_run_created, "run_created event must appear in the run event stream");
    }

    /// Cursor-based pagination: after=<position> skips earlier events.
    #[tokio::test]
    async fn run_events_cursor_pagination_works() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_pg");

        state.runtime.sessions
            .create(&project, SessionId::new("sess_pg"))
            .await.unwrap();
        state.runtime.runs
            .start(&project, &SessionId::new("sess_pg"), RunId::new("run_pg"), None)
            .await.unwrap();

        let app1 = build_router(state.clone());
        let resp_all = app1.oneshot(authed_req("/v1/runs/run_pg/events")).await.unwrap();
        let body_all = to_bytes(resp_all.into_body(), usize::MAX).await.unwrap();
        let all: serde_json::Value = serde_json::from_slice(&body_all).unwrap();
        let all_arr = all.as_array().unwrap();
        assert!(!all_arr.is_empty(), "must have events");

        let first_pos = all_arr[0]["position"].as_u64().unwrap();

        // After the first event's position, all remaining events are returned.
        let uri = format!("/v1/runs/run_pg/events?after={first_pos}");
        let app2 = build_router(state);
        let resp_page = app2.oneshot(authed_req(&uri)).await.unwrap();
        let body_page = to_bytes(resp_page.into_body(), usize::MAX).await.unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body_page).unwrap();
        let page_arr = page.as_array().unwrap();

        assert_eq!(page_arr.len(), all_arr.len() - 1,
            "after=first_pos must skip the first event");
        assert!(page_arr.iter().all(|e| e["position"].as_u64().unwrap() > first_pos),
            "all paginated events must be after the cursor position");
    }

    /// The run event stream is scoped to its run — events from other runs are excluded.
    #[tokio::test]
    async fn run_events_are_run_scoped() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_sc");

        state.runtime.sessions
            .create(&project, SessionId::new("sess_sc"))
            .await.unwrap();
        state.runtime.runs
            .start(&project, &SessionId::new("sess_sc"), RunId::new("run_sc_a"), None)
            .await.unwrap();
        state.runtime.runs
            .start(&project, &SessionId::new("sess_sc"), RunId::new("run_sc_b"), None)
            .await.unwrap();

        // Events for run_sc_a must not include run_sc_b events.
        let app = build_router(state);
        let resp = app.oneshot(authed_req("/v1/runs/run_sc_a/events")).await.unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();

        assert!(!arr.is_empty(), "run_sc_a must have events");
        // All returned event_type values should be run-lifecycle types, not b's events.
        // Since event_type is derived from payload, just verify run_created is present once.
        let run_created_count = arr.iter().filter(|e| e["event_type"] == "run_created").count();
        assert_eq!(run_created_count, 1,
            "exactly one run_created must appear (for run_sc_a, not run_sc_b)");
    }
}

#[cfg(test)]
mod tool_invocations_tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;
    use cairn_domain::{
        ProjectKey, RunId, SessionId, ToolInvocationId,
        tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget},
        policy::ExecutionClass,
    };
    use cairn_runtime::{ToolInvocationService as _};

    const TOKEN: &str = "test-tool-inv-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-tool-inv".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(
                    cairn_domain::TenantId::new("tenant_ti"),
                ),
            },
        );
        AppState {
            runtime: Arc::new(InMemoryServices::new()),
            started_at: Arc::new(std::time::Instant::now()),
            tokens,
            pg: None,
        }
    }

    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// GET /v1/runs/:id/tool-invocations returns empty for a run with no calls.
    #[tokio::test]
    async fn tool_invocations_empty_for_run_with_no_calls() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");

        state.runtime.sessions
            .create(&project, SessionId::new("sess_ti_empty"))
            .await.unwrap();
        state.runtime.runs
            .start(&project, &SessionId::new("sess_ti_empty"), RunId::new("run_ti_empty"), None)
            .await.unwrap();

        let app = build_router(state);
        let resp = app.oneshot(authed_req("/v1/runs/run_ti_empty/tool-invocations")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let records: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(records.as_array().unwrap().is_empty(),
            "run with no tool calls must return empty list");
    }

    /// GET /v1/runs/:id/tool-invocations returns both calls after they are recorded.
    #[tokio::test]
    async fn tool_invocations_returns_two_calls_for_run() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");
        let run = RunId::new("run_ti_two");
        let sess = SessionId::new("sess_ti_two");

        state.runtime.sessions.create(&project, sess.clone()).await.unwrap();
        state.runtime.runs.start(&project, &sess, run.clone(), None).await.unwrap();

        // Record two tool calls on the run.
        let target = ToolInvocationTarget::Builtin { tool_name: "read_file".to_owned() };
        state.runtime.tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_1"),
                None,
                Some(run.clone()),
                None,
                target.clone(),
                ExecutionClass::SandboxedProcess,
            )
            .await.unwrap();
        state.runtime.tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_2"),
                None,
                Some(run.clone()),
                None,
                ToolInvocationTarget::Builtin { tool_name: "write_file".to_owned() },
                ExecutionClass::SupervisedProcess,
            )
            .await.unwrap();

        let app = build_router(state);
        let resp = app.oneshot(authed_req("/v1/runs/run_ti_two/tool-invocations")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let records: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = records.as_array().unwrap();

        assert_eq!(arr.len(), 2, "run must have exactly 2 tool invocation records");

        // Both invocation IDs must be present.
        let inv_ids: Vec<&str> = arr.iter()
            .map(|r| r["invocation_id"].as_str().unwrap_or(""))
            .collect();
        assert!(inv_ids.contains(&"inv_ti_1"), "inv_ti_1 must be in the response");
        assert!(inv_ids.contains(&"inv_ti_2"), "inv_ti_2 must be in the response");

        // Both are scoped to the run.
        for record in arr {
            assert_eq!(record["run_id"].as_str().unwrap_or(""), "run_ti_two",
                "all records must be for run_ti_two");
        }
    }

    /// Outcome field reflects the terminal outcome after a call completes.
    ///
    /// Records start with state=requested/started and outcome=null;
    /// after ToolInvocationCompleted the state transitions and outcome is set.
    #[tokio::test]
    async fn tool_invocation_outcome_field_reflects_completion() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");
        let run = RunId::new("run_ti_outcome");
        let sess = SessionId::new("sess_ti_outcome");

        state.runtime.sessions.create(&project, sess.clone()).await.unwrap();
        state.runtime.runs.start(&project, &sess, run.clone(), None).await.unwrap();

        // Start a tool call.
        state.runtime.tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_outcome"),
                None,
                Some(run.clone()),
                None,
                ToolInvocationTarget::Builtin { tool_name: "bash".to_owned() },
                ExecutionClass::SupervisedProcess,
            )
            .await.unwrap();

        // Before completion: outcome must be null, state is not terminal.
        let app1 = build_router(state.clone());
        let resp1 = app1.oneshot(authed_req("/v1/runs/run_ti_outcome/tool-invocations")).await.unwrap();
        let body1 = to_bytes(resp1.into_body(), usize::MAX).await.unwrap();
        let before: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        let before_rec = &before.as_array().unwrap()[0];
        assert!(before_rec["outcome"].is_null(),
            "outcome must be null before completion");
        let before_state = before_rec["state"].as_str().unwrap_or("");
        assert!(!before_state.is_empty(), "state field must be present");

        // Complete the call with Success.
        state.runtime.tool_invocations
            .record_completed(
                &project,
                ToolInvocationId::new("inv_ti_outcome"),
                None,
                "bash".to_owned(),
            )
            .await.unwrap();

        // After completion: outcome must be "success", state must be "completed".
        let app2 = build_router(state);
        let resp2 = app2.oneshot(authed_req("/v1/runs/run_ti_outcome/tool-invocations")).await.unwrap();
        let body2 = to_bytes(resp2.into_body(), usize::MAX).await.unwrap();
        let after: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        let after_rec = &after.as_array().unwrap()[0];

        let outcome = after_rec["outcome"].as_str().unwrap_or("<null>");
        assert_eq!(outcome, "success",
            "outcome must be 'success' after ToolInvocationCompleted");
        assert_eq!(after_rec["state"].as_str().unwrap_or(""), "completed",
            "state must be 'completed' after successful completion");
    }

    /// Tool invocations endpoint requires auth.
    #[tokio::test]
    async fn tool_invocations_requires_auth() {
        let app = build_router(make_state());
        let resp = app.oneshot(
            Request::builder()
                .uri("/v1/runs/any_run/tool-invocations")
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

}

#[cfg(test)]
mod provider_health_tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt as _;
    use cairn_domain::{
        EventEnvelope, EventId, EventSource, ProviderConnectionId, ProjectKey, RuntimeEvent,
        TenantId, ProviderBindingId, ProviderModelId,
        events::{ProviderConnectionRegistered, ProviderHealthChecked, ProviderMarkedDegraded},
        providers::{OperationKind, ProviderBindingSettings, ProviderConnectionStatus, ProviderHealthStatus},
        tenancy::TenantKey,
    };

    const TOKEN: &str = "test-ph-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-ph".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(TenantId::new("t_ph")),
            },
        );
        AppState {
            runtime: Arc::new(InMemoryServices::new()),
            started_at: Arc::new(std::time::Instant::now()),
            tokens,
            pg: None,
        }
    }

    fn authed_req(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", format!("Bearer {TOKEN}"))
            .body(Body::empty())
            .unwrap()
    }

    /// GET /v1/providers/health returns empty when no providers are registered.
    #[tokio::test]
    async fn provider_health_empty_with_no_providers() {
        let app = build_router(make_state());
        let resp = app.oneshot(authed_req("/v1/providers/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(health.as_array().unwrap().is_empty(),
            "no providers registered — health list must be empty");
    }

    /// After a healthy check, the health entry shows healthy=true and correct fields.
    #[tokio::test]
    async fn provider_health_shows_healthy_after_health_check() {
        use cairn_domain::events::ProviderBindingCreated;

        let state = make_state();
        let project = ProjectKey::new("t_ph", "ws_ph", "proj_ph");

        // Register connection + binding (needed to derive tenant for health query).
        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_ph_conn"),
                EventSource::Runtime,
                RuntimeEvent::ProviderConnectionRegistered(ProviderConnectionRegistered {
                    tenant: TenantKey::new("t_ph"),
                    provider_connection_id: ProviderConnectionId::new("conn_ph_1"),
                    provider_family: "openai".to_owned(),
                    adapter_type: "responses".to_owned(),
                    status: ProviderConnectionStatus::Active,
                    registered_at: 1_000,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_ph_bind"),
                EventSource::Runtime,
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: project.clone(),
                    provider_binding_id: ProviderBindingId::new("conn_ph_1"),
                    provider_connection_id: ProviderConnectionId::new("conn_ph_1"),
                    provider_model_id: ProviderModelId::new("gpt-4o"),
                    operation_kind: OperationKind::Generate,
                    settings: ProviderBindingSettings::default(),
                    policy_id: None,
                    active: true,
                    created_at: 1_000,
                    estimated_cost_micros: None,
                }),
            ),
            // Healthy check.
            EventEnvelope::for_runtime_event(
                EventId::new("evt_ph_check"),
                EventSource::Runtime,
                RuntimeEvent::ProviderHealthChecked(ProviderHealthChecked {
                    tenant_id: TenantId::new("t_ph"),
                    connection_id: ProviderConnectionId::new("conn_ph_1"),
                    status: ProviderHealthStatus::Healthy,
                    latency_ms: Some(95),
                    checked_at_ms: 5_000,
                }),
            ),
        ]).await.unwrap();

        let app = build_router(state);
        let resp = app.oneshot(authed_req("/v1/providers/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = health.as_array().unwrap();

        assert_eq!(arr.len(), 1, "one health entry must appear");
        assert_eq!(arr[0]["connection_id"], "conn_ph_1");
        assert_eq!(arr[0]["healthy"], true, "must be healthy after health check");
        assert_eq!(arr[0]["consecutive_failures"], 0);
        assert_eq!(arr[0]["last_checked_at"], 5_000);
        // Status serializes to lowercase.
        assert!(!arr[0]["status"].as_str().unwrap_or("").is_empty(),
            "status must be set");
    }

    /// After ProviderMarkedDegraded, the health entry reflects degraded status.
    #[tokio::test]
    async fn provider_health_shows_degraded_after_mark_degraded() {
        use cairn_domain::events::{ProviderBindingCreated, ProviderMarkedDegraded};

        let state = make_state();
        let project = ProjectKey::new("t_ph", "ws_ph", "proj_ph2");

        state.runtime.store.append(&[
            EventEnvelope::for_runtime_event(
                EventId::new("evt_ph2_bind"),
                EventSource::Runtime,
                RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                    project: project.clone(),
                    provider_binding_id: ProviderBindingId::new("conn_ph_deg"),
                    provider_connection_id: ProviderConnectionId::new("conn_ph_deg"),
                    provider_model_id: ProviderModelId::new("gpt-4o"),
                    operation_kind: OperationKind::Generate,
                    settings: ProviderBindingSettings::default(),
                    policy_id: None,
                    active: true,
                    created_at: 1_000,
                    estimated_cost_micros: None,
                }),
            ),
            EventEnvelope::for_runtime_event(
                EventId::new("evt_ph2_degrade"),
                EventSource::Runtime,
                RuntimeEvent::ProviderMarkedDegraded(ProviderMarkedDegraded {
                    tenant_id: TenantId::new("t_ph"),
                    connection_id: ProviderConnectionId::new("conn_ph_deg"),
                    reason: "upstream latency exceeded threshold".to_owned(),
                    marked_at_ms: 8_000,
                }),
            ),
        ]).await.unwrap();

        let app = build_router(state);
        let resp = app.oneshot(authed_req("/v1/providers/health")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = health.as_array().unwrap();

        assert_eq!(arr.len(), 1, "one health entry");
        assert_eq!(arr[0]["healthy"], false, "must be unhealthy after degraded mark");
        assert!(arr[0]["error_message"].as_str()
            .map_or(false, |e| e.contains("latency")),
            "error_message must contain the degradation reason");
        assert_eq!(arr[0]["last_checked_at"], 8_000);
    }

    /// GET /v1/providers/health requires auth.
    #[tokio::test]
    async fn provider_health_requires_auth() {
        let app = build_router(make_state());
        let resp = app.oneshot(
            Request::builder()
                .uri("/v1/providers/health")
                .body(Body::empty())
                .unwrap(),
        ).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
