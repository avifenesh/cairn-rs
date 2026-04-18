//! Binary-specific HTTP handlers: version, changelog, webhook test,
//! rate-limit, task, session, batch, approval, tool invocations,
//! notifications, system info, LLM traces, system role, entitlements,
//! templates, bundles, and Prometheus metrics.

#[allow(unused_imports)]
use crate::*;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
#[allow(unused_imports)]
use cairn_api::auth::{AuthPrincipal, ServiceTokenAuthenticator};
use cairn_api::bootstrap::DeploymentMode;
use cairn_domain::{ApprovalDecision, ApprovalId, ProjectKey, RunId, TaskId};
use cairn_runtime::approvals::ApprovalService;
use cairn_store::projections::{
    ApprovalReadModel, LlmCallTraceReadModel, RunReadModel, SessionReadModel, TaskReadModel,
    ToolInvocationReadModel,
};
use serde::{Deserialize, Serialize};

// ── Version + changelog ──────────────────────────────────────────────────────

/// The canonical application version — sourced from Cargo.toml at compile time.
pub(crate) const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Static changelog.  Add a new entry for every published release.
pub(crate) const CHANGELOG: &str = r##"[
  {
    "version": "0.1.0",
    "date":    "2026-04-09",
    "changes": [
      "Initial release of cairn-rs",
      "30 operator UI pages (Dashboard, Sessions, Runs, Tasks, Approvals, Prompts, Traces, Memory, Sources, Costs, Evals, Graph, Audit Log, Providers, Plugins, Credentials, Channels, Playground, API Docs, Settings, Profile, Eval Comparison, Workers, Orchestration, Deployment, Metrics, Logs, Test Harness, Cost Calculator, Workspaces)",
      "56+ REST API routes across Health, Sessions, Runs, Tasks, Approvals, Providers, Memory, Events, Evals, Admin groups",
      "Event-sourced runtime: 56+ domain event types, append-only log, idempotent commands",
      "Real-time SSE event stream with Last-Event-ID replay",
      "Multi-tenant isolation: tenant / workspace / project hierarchy with RBAC",
      "Human-in-the-loop approval workflows",
      "Provider-agnostic LLM integration: any OpenAI-compatible endpoint, Ollama, Bedrock, Vertex AI",
      "Provider registry with 13 known providers, model router, and test-connection endpoint",
      "Stream accumulator for SSE event parsing (adopted from Cersei SDK)",
      "ToolContext with session/run awareness, working directory, and extensions type-map",
      "#[derive(Tool)] proc macro for declarative tool definitions",
      "6-level PermissionLevel (None, ReadOnly, Write, Execute, Dangerous, Forbidden)",
      "Hook system: PreToolUse, PostToolUse, PreModelTurn, PostModelTurn, Stop, Error events",
      "Credential store with AES-256-GCM encryption, tenant-scoped, key rotation",
      "Built-in eval framework with rubrics, baselines, bandit experiments",
      "Cost tracking and token metering per call, run, and session",
      "Knowledge retrieval: ingest, chunking, multi-factor scoring, graph expansion",
      "Per-IP and per-token rate limiting (100/1000 req/min) with X-RateLimit-* headers",
      "Batch operations: POST /v1/runs/batch, POST /v1/tasks/batch/cancel",
      "Static OpenAPI 3.0 spec at /v1/openapi.json and Swagger UI at /v1/docs",
      "Session and run export (JSON download) and import",
      "SQLite and Postgres persistence backends",
      "Embedded React UI served from the binary (rust-embed)",
      "GitHub Actions CI: fmt, clippy (-D warnings), tests, UI build",
      "Pre-push hook with smart Rust/frontend change detection"
    ]
  }
]"##;

/// Middleware: inject `X-Cairn-Version` on every response so clients can
/// inspect the server version without a separate request.
pub(crate) async fn version_header_middleware(req: Request<Body>, next: Next) -> Response {
    let mut resp = next.run(req).await;
    if let Ok(v) = axum::http::HeaderValue::from_str(APP_VERSION) {
        resp.headers_mut().insert("X-Cairn-Version", v);
    }
    resp
}

/// `GET /v1/changelog` — release notes as a JSON array.
/// Public endpoint — no auth required.
pub(crate) async fn changelog_handler() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        CHANGELOG,
    )
}

// ── Webhook test ─────────────────────────────────────────────────────────────

#[derive(serde::Deserialize)]
pub(crate) struct TestWebhookRequest {
    url: String,
    event_type: String,
}

#[derive(serde::Serialize)]
pub(crate) struct TestWebhookResponse {
    success: bool,
    status_code: u16,
    latency_ms: u64,
}

pub(crate) async fn test_webhook_handler(
    axum::Json(body): axum::Json<TestWebhookRequest>,
) -> impl IntoResponse {
    let payload = serde_json::json!({
        "event_type": body.event_type,
        "source":     "cairn-rs",
        "test":       true,
        "timestamp":  std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
        "message":    format!("Test notification for event '{}'", body.event_type),
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();

    let start = std::time::Instant::now();
    let result = client
        .post(&body.url)
        .header("Content-Type", "application/json")
        .header("User-Agent", "cairn-rs/webhook-test")
        .json(&payload)
        .send()
        .await;
    let latency_ms = start.elapsed().as_millis() as u64;

    match result {
        Ok(resp) => {
            let status_code = resp.status().as_u16();
            axum::Json(TestWebhookResponse {
                success: resp.status().is_success(),
                status_code,
                latency_ms,
            })
        }
        Err(_) => axum::Json(TestWebhookResponse {
            success: false,
            status_code: 0,
            latency_ms,
        }),
    }
}

// ── Rate-limit middleware ─────────────────────────────────────────────────────

/// Extract the best available identity key for rate-limiting.
///
/// Prefers the Bearer token (stable across IPs) so token holders share
/// a single 1 000 req/min bucket.  Falls back to `X-Forwarded-For` or the
/// socket peer address when no token is present (100 req/min bucket).
pub(crate) fn rate_limit_key(req: &Request<Body>) -> (String, u32) {
    let token = req
        .headers()
        .get(AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(|t| t.trim().to_owned());

    if let Some(tok) = token {
        return (format!("tok:{tok}"), RATE_LIMIT_TOKEN);
    }

    let ip = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());

    (format!("ip:{ip}"), RATE_LIMIT_IP)
}

/// `GET /v1/rate-limit` — return current limits and remaining quota.
pub(crate) async fn rate_limit_status_handler(
    State(state): State<AppState>,
    req: Request<Body>,
) -> impl IntoResponse {
    let (key, limit) = rate_limit_key(&req);
    let table = state.rate_limits.lock().unwrap_or_else(|e| e.into_inner());
    let (count, reset_in_secs) = match table.get(&key) {
        Some(bucket) if bucket.window_start.elapsed() < RATE_WINDOW => {
            let elapsed = bucket.window_start.elapsed();
            let reset_in = RATE_WINDOW
                .checked_sub(elapsed)
                .unwrap_or(RATE_WINDOW)
                .as_secs();
            (bucket.count, reset_in)
        }
        _ => (0, RATE_WINDOW.as_secs()),
    };
    let remaining = limit.saturating_sub(count);
    axum::Json(serde_json::json!({
        "limit_per_minute":     limit,
        "used_this_minute":     count,
        "remaining_this_minute": remaining,
        "reset_in_seconds":     reset_in_secs,
        "policy": {
            "token_limit_per_minute": RATE_LIMIT_TOKEN,
            "ip_limit_per_minute":    RATE_LIMIT_IP,
            "window_seconds":         RATE_WINDOW.as_secs(),
        },
    }))
}

// ── Task handlers ────────────────────────────────────────────────────────────

/// `GET /v1/runs/:id/tasks` — list all tasks belonging to a run.
pub(crate) async fn list_run_tasks_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(&id);

    // Verify the run exists before listing its tasks.
    match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(None) => return Err(not_found(format!("run {id} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
        Ok(Some(_)) => {}
    }

    match TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &run_id, 1000).await {
        Ok(tasks) => Ok(Json(tasks)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateTaskBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    task_id: Option<String>,
}

pub(crate) async fn create_run_task_handler(
    State(state): State<AppState>,
    Path(run_id_str): Path<String>,
    Json(body): Json<CreateTaskBody>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(&run_id_str);

    let run = match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(None) => return Err(not_found(format!("run {run_id_str} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
        Ok(Some(r)) => r,
    };

    let task_id = TaskId::new(
        body.task_id
            .as_deref()
            .unwrap_or(&uuid::Uuid::new_v4().to_string()),
    );

    match state
        .runtime
        .tasks
        .submit(&run.project, task_id, Some(run_id), None, 0)
        .await
    {
        Ok(record) => {
            let mut value = serde_json::to_value(&record).unwrap_or_default();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("name".to_owned(), serde_json::json!(body.name));
                obj.insert(
                    "description".to_owned(),
                    serde_json::json!(body.description),
                );
                obj.insert(
                    "metadata".to_owned(),
                    body.metadata.unwrap_or(serde_json::json!({})),
                );
            }
            Ok((StatusCode::CREATED, Json(value)))
        }
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// `POST /v1/tasks/:id/start` — transition a claimed (leased) task to running.
pub(crate) async fn start_task_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let task_id = TaskId::new(id.clone());
    match state.runtime.tasks.start(&task_id).await {
        Ok(record) => Ok((StatusCode::OK, Json(record))),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("NotFound") {
                Err(not_found(format!("task {id} not found")))
            } else {
                Err(bad_request(msg))
            }
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct FailTaskBody {
    error: String,
}

pub(crate) async fn fail_task_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<FailTaskBody>,
) -> impl axum::response::IntoResponse {
    let task_id = TaskId::new(id.clone());
    match state
        .runtime
        .tasks
        .fail(&task_id, cairn_domain::FailureClass::ExecutionError)
        .await
    {
        Ok(record) => {
            let mut value = serde_json::to_value(&record).unwrap_or_default();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("error".to_owned(), serde_json::json!(body.error));
            }
            Ok((StatusCode::OK, Json(value)))
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("NotFound") {
                Err(not_found(format!("task {id} not found")))
            } else {
                Err(bad_request(msg))
            }
        }
    }
}

/// `GET /v1/runs/:id/approvals` — list all approvals for a run.
pub(crate) async fn list_run_approvals_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(&id);

    // Verify the run exists.
    match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(None) => return Err(not_found(format!("run {id} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
        Ok(Some(_)) => {}
    }

    let approvals = state.runtime.store.list_approvals_by_run(&run_id);
    Ok(Json(approvals))
}

// ── Session handlers ──────────────────────────────────────────────────────────

pub(crate) async fn list_session_runs_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    let session_id = cairn_domain::SessionId::new(&id);

    // Verify session exists.
    match SessionReadModel::get(state.runtime.store.as_ref(), &session_id).await {
        Ok(None) => return Err(not_found(format!("session {id} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
        Ok(Some(_)) => {}
    }

    match RunReadModel::list_by_session(
        state.runtime.store.as_ref(),
        &session_id,
        q.limit,
        q.offset,
    )
    .await
    {
        Ok(runs) => Ok(Json(runs)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

#[derive(Deserialize)]
pub(crate) struct CreateRunBody {
    pub(crate) tenant_id: Option<String>,
    pub(crate) workspace_id: Option<String>,
    pub(crate) project_id: Option<String>,
    pub(crate) session_id: Option<String>,
    pub(crate) run_id: Option<String>,
    pub(crate) parent_run_id: Option<String>,
}

// ── Batch handlers ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct BatchCreateRunsBody {
    runs: Vec<CreateRunBody>,
}

/// `POST /v1/runs/batch` — create multiple runs in one request.
pub(crate) async fn batch_create_runs_handler(
    State(state): State<AppState>,
    Json(body): Json<BatchCreateRunsBody>,
) -> impl axum::response::IntoResponse {
    if body.runs.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "code": "bad_request",
                "message": "runs array must not be empty",
            })),
        )
            .into_response();
    }

    let mut results: Vec<serde_json::Value> = Vec::with_capacity(body.runs.len());

    for run_body in body.runs {
        let project = ProjectKey::new(
            run_body.tenant_id.as_deref().unwrap_or("default"),
            run_body.workspace_id.as_deref().unwrap_or("default"),
            run_body.project_id.as_deref().unwrap_or("default"),
        );
        let session_id =
            cairn_domain::SessionId::new(run_body.session_id.as_deref().unwrap_or("session_1"));
        let run_id = RunId::new(
            run_body
                .run_id
                .as_deref()
                .unwrap_or(&uuid::Uuid::new_v4().to_string()),
        );
        let parent_run_id = run_body.parent_run_id.as_deref().map(RunId::new);
        match state
            .runtime
            .runs
            .start(&project, &session_id, run_id, parent_run_id)
            .await
        {
            Ok(record) => results.push(serde_json::json!({ "ok": true,  "run": record })),
            Err(e) => results.push(serde_json::json!({ "ok": false, "error": e.to_string() })),
        }
    }

    let all_ok = results.iter().all(|r| r["ok"].as_bool().unwrap_or(false));
    let status = if all_ok {
        StatusCode::CREATED
    } else {
        StatusCode::MULTI_STATUS
    };
    (status, Json(serde_json::json!({ "results": results }))).into_response()
}

#[derive(Deserialize)]
pub(crate) struct BatchCancelTasksBody {
    task_ids: Vec<String>,
}

/// `POST /v1/tasks/batch/cancel` — cancel multiple tasks in one call.
pub(crate) async fn batch_cancel_tasks_handler(
    State(state): State<AppState>,
    Json(body): Json<BatchCancelTasksBody>,
) -> impl axum::response::IntoResponse {
    if body.task_ids.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({
                "code": "bad_request",
                "message": "task_ids array must not be empty",
            })),
        )
            .into_response();
    }

    let mut cancelled: u32 = 0;
    let mut failed: Vec<serde_json::Value> = Vec::new();

    for raw_id in body.task_ids {
        let task_id = TaskId::new(&raw_id);
        match state.runtime.tasks.cancel(&task_id).await {
            Ok(_) => cancelled += 1,
            Err(e) => {
                let reason = e.to_string();
                failed.push(serde_json::json!({ "id": raw_id, "reason": reason }));
            }
        }
    }

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "cancelled": cancelled,
            "failed":    failed,
        })),
    )
        .into_response()
}

// ── Approval handlers ─────────────────────────────────────────────────────────

/// `GET /v1/approvals/pending` — list pending approvals.
pub(crate) async fn list_pending_approvals_handler(
    State(state): State<AppState>,
    Query(q): Query<ProjectQuery>,
) -> impl axum::response::IntoResponse {
    // Use total approval count (all states) as the pagination denominator.
    let total = state.runtime.store.count_all_approvals();
    let hdrs = pagination_headers("/v1/approvals/pending", total, q.offset, q.limit);
    if let Some(project) = q.project_key() {
        match ApprovalReadModel::list_pending(
            state.runtime.store.as_ref(),
            &project,
            q.limit,
            q.offset,
        )
        .await
        {
            Ok(records) => Ok((hdrs, Json(records))),
            Err(e) => Err(internal_error(e.to_string())),
        }
    } else {
        match list_all_pending(&state, q.limit, q.offset).await {
            Ok(records) => Ok((hdrs, Json(records))),
            Err(e) => Err(internal_error(e.to_string())),
        }
    }
}

/// Scan the full approval store for pending (undecided) records across all
/// projects.  Uses a direct store method instead of filtering by project key.
pub(crate) async fn list_all_pending(
    state: &AppState,
    limit: usize,
    offset: usize,
) -> Result<Vec<cairn_store::projections::ApprovalRecord>, cairn_store::StoreError> {
    Ok(state
        .runtime
        .store
        .list_all_pending_approvals(limit, offset))
}

#[derive(Deserialize)]
pub(crate) struct ResolveApprovalBody {
    /// `"approved"` or `"rejected"`
    decision: String,
    /// Optional free-text explanation logged alongside the decision.
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct ResolveApprovalResponse {
    #[serde(flatten)]
    record: cairn_store::projections::ApprovalRecord,
    /// Echo of the reason supplied in the request body (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// `POST /v1/approvals/:id/resolve` — approve or reject a pending approval.
pub(crate) async fn resolve_approval_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<ResolveApprovalBody>,
) -> impl axum::response::IntoResponse {
    if let Err(msg) = validate::check_all(&[
        validate::require_id("id", &id),
        validate::max_len_str("decision", &body.decision, 32),
        validate::max_len("reason", &body.reason, validate::MAX_DESC_LEN),
    ]) {
        return Err(bad_request(msg));
    }

    let approval_id = ApprovalId::new(id);
    let decision = match body.decision.to_lowercase().as_str() {
        "approved" | "approve" => ApprovalDecision::Approved,
        "rejected" | "reject" => ApprovalDecision::Rejected,
        other => {
            return Err(bad_request(format!(
                "unknown decision: {other}; use 'approved' or 'rejected'"
            )));
        }
    };
    match state
        .runtime
        .approvals
        .resolve(&approval_id, decision)
        .await
    {
        Ok(record) => Ok((
            StatusCode::OK,
            Json(ResolveApprovalResponse {
                record,
                reason: body.reason,
            }),
        )),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("not found") || msg.contains("NotFound") {
                Err(not_found(format!(
                    "approval {} not found",
                    approval_id.as_str()
                )))
            } else {
                Err(internal_error(msg))
            }
        }
    }
}

// ── Run tool invocations handler ─────────────────────────────────────────────

/// `GET /v1/runs/:id/tool-invocations` — list all tool calls for a run.
pub(crate) async fn list_run_tool_invocations_handler(
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

// ── Notification handlers ─────────────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct NotifListQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
pub(crate) struct NotifListResponse {
    notifications: Vec<Notification>,
    unread_count: usize,
}

/// `GET /v1/notifications?limit=50` — list recent notifications.
pub(crate) async fn list_notifications_handler(
    State(state): State<AppState>,
    Query(q): Query<NotifListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    let buf = state
        .notifications
        .read()
        .unwrap_or_else(|e| e.into_inner());
    let notifications: Vec<Notification> = buf.list(limit).into_iter().cloned().collect();
    let unread_count = buf.unread_count();
    Json(NotifListResponse {
        notifications,
        unread_count,
    })
}

/// `POST /v1/notifications/:id/read` — mark one notification as read.
pub(crate) async fn mark_notification_read_handler(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let found = state
        .notifications
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .mark_read(&id);
    if found {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                code: "not_found",
                message: format!("notification {id} not found"),
            }),
        )
            .into_response()
    }
}

/// `POST /v1/notifications/read-all` — mark all notifications as read.
pub(crate) async fn mark_all_notifications_read_handler(
    State(state): State<AppState>,
) -> impl IntoResponse {
    state
        .notifications
        .write()
        .unwrap_or_else(|e| e.into_inner())
        .mark_all_read();
    StatusCode::NO_CONTENT
}

// ── System info handler ───────────────────────────────────────────────────────

/// `GET /v1/system/info` — comprehensive system information.
pub(crate) async fn system_info_handler(
    State(state): State<AppState>,
) -> impl axum::response::IntoResponse {
    let deployment_mode = match state.mode {
        DeploymentMode::SelfHostedTeam => "self_hosted_team",
        DeploymentMode::Local => "local",
    };
    let store_type = if state.pg.is_some() {
        "postgres"
    } else if state.sqlite.is_some() {
        "sqlite"
    } else {
        "in_memory"
    };

    // Mask the admin token — only reveal whether it is set and how long it is.
    let admin_token_set = std::env::var("CAIRN_ADMIN_TOKEN").is_ok();
    let ollama_host = std::env::var("OLLAMA_HOST").unwrap_or_default();
    let ollama_host_display = if ollama_host.is_empty() {
        "not configured".to_owned()
    } else {
        ollama_host.clone()
    };

    Json(serde_json::json!({
        "version":      env!("CARGO_PKG_VERSION"),
        "rust_version": env!("CARGO_PKG_VERSION"),   // CARGO_PKG_VERSION is the crate version
        "build_date":   concat!(env!("CARGO_PKG_VERSION"), " (build date not embedded)"),
        "git_commit":   option_env!("GIT_COMMIT").unwrap_or("dev"),
        "os":           std::env::consts::OS,
        "arch":         std::env::consts::ARCH,

        "features": {
            "sse_buffer_size":       NOTIF_RING_SIZE * 50,   // approx SSE ring (10 000)
            "rate_limit_per_minute": RATE_LIMIT_TOKEN,
            "ip_rate_limit_per_minute": RATE_LIMIT_IP,
            "max_body_size_mb":      10,
            "websocket_enabled":     true,
            "ollama_connected":      state.ollama.is_some(),
            "store_type":            store_type,
            "postgres_enabled":      state.pg.is_some(),
            "sqlite_enabled":        state.sqlite.is_some(),
            "notification_buffer":   NOTIF_RING_SIZE,
        },

        "environment": {
            "admin_token_set":   admin_token_set,
            "ollama_host":       ollama_host_display,
            "listen_addr":       "see server startup log",
            "deployment_mode":   deployment_mode,
            "uptime_seconds":    state.started_at.elapsed().as_secs(),
        }
    }))
}

// ── LLM trace handlers (GAP-010) ─────────────────────────────────────────────

/// `GET /v1/traces` — all recent LLM call traces (operator view, limit 500).
pub(crate) async fn list_all_traces_handler(
    State(state): State<AppState>,
    Query(q): Query<PaginationQuery>,
) -> impl axum::response::IntoResponse {
    let limit = q.limit.min(500);
    // Fetch all traces to obtain the true total, then page.
    match LlmCallTraceReadModel::list_all_traces(state.runtime.store.as_ref(), usize::MAX).await {
        Ok(all_traces) => {
            let total = all_traces.len();
            let traces: Vec<_> = all_traces.into_iter().skip(q.offset).take(limit).collect();
            Ok((
                pagination_headers("/v1/traces", total, q.offset, limit),
                Json(serde_json::json!({ "traces": traces })),
            ))
        }
        Err(e) => Err(internal_error(e.to_string())),
    }
}

// ── RFC 011: Server role handler ─────────────────────────────────────────────

/// `GET /v1/system/role` — returns the current process role.
pub(crate) async fn system_role_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "role": state.process_role.as_str(),
        "serves_http": state.process_role.serves_http(),
        "runs_workers": state.process_role.runs_workers(),
    }))
}

// ── RFC 014: Entitlement handlers ────────────────────────────────────────────

#[derive(Deserialize)]
pub(crate) struct TenantQuery {
    #[serde(default)]
    tenant_id: Option<String>,
}

/// `GET /v1/entitlements` — current plan + usage + limits for the default tenant.
pub(crate) async fn entitlements_handler(
    State(state): State<AppState>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    let tenant_id = q.tenant_id.as_deref().unwrap_or("default");
    match state.entitlements.get_usage(tenant_id) {
        Some(report) => Ok(Json(report)),
        None => Err(not_found(format!(
            "no plan assigned to tenant '{tenant_id}'"
        ))),
    }
}

/// `GET /v1/entitlements/usage` — detailed usage breakdown.
pub(crate) async fn entitlements_usage_handler(
    State(state): State<AppState>,
    Query(q): Query<TenantQuery>,
) -> impl IntoResponse {
    let tenant_id = q.tenant_id.as_deref().unwrap_or("default");
    match state.entitlements.get_detailed_usage(tenant_id) {
        Some(report) => Ok(Json(report)),
        None => Err(not_found(format!(
            "no plan assigned to tenant '{tenant_id}'"
        ))),
    }
}

/// `POST /v1/bundles/import` — import a bundle into a target project.
pub(crate) async fn bundle_import_handler(
    Json(body): Json<bundles::ImportRequest>,
) -> impl IntoResponse {
    if let Err(msg) = validate::check_all(&[validate::require_id("project_id", &body.project_id)]) {
        return Err(bad_request(msg));
    }
    if body.existing_ids.len() > 10_000 {
        return Err(bad_request(
            "existing_ids exceeds maximum of 10,000 entries",
        ));
    }

    // Validate the bundle.
    let validation = bundles::validate_bundle(&body.bundle);
    if !validation.valid {
        return Err(bad_request(format!(
            "bundle validation failed: {}",
            validation
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.field, e.message))
                .collect::<Vec<_>>()
                .join("; ")
        )));
    }

    // Plan the import with conflict resolution.
    let existing: std::collections::HashSet<String> = body.existing_ids.into_iter().collect();
    let plan = bundles::plan_import(
        &body.bundle,
        &body.project_id,
        &existing,
        body.conflict_strategy,
    );

    // Execute the plan.
    let result = bundles::execute_import(&plan);
    Ok(Json(result))
}

// ── RFC 012: Template handlers ───────────────────────────────────────────────

/// `GET /v1/templates` — list all available starter templates.
pub(crate) async fn list_templates_handler(
    State(state): State<AppState>,
) -> Json<Vec<templates::TemplateSummary>> {
    Json(state.templates.list())
}

/// `GET /v1/templates/:id` — get full template detail with file contents.
pub(crate) async fn get_template_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.templates.get(&id) {
        Some(t) => Ok(Json(t.clone())),
        None => Err(not_found(format!("template not found: {id}"))),
    }
}

/// `POST /v1/templates/:id/apply` — apply a template to a project.
pub(crate) async fn apply_template_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<templates::ApplyRequest>,
) -> impl IntoResponse {
    match state.templates.apply(&id, &body.project_id) {
        Some(result) => Ok(Json(result)),
        None => Err(not_found(format!("template not found: {id}"))),
    }
}

// ── Prometheus metrics handler ───────────────────────────────────────────────

/// `GET /v1/metrics/prometheus` — Prometheus exposition format (text/plain).
///
/// Compatible with Prometheus scrape configs and Grafana data sources.
pub(crate) async fn metrics_prometheus_handler(State(state): State<AppState>) -> impl IntoResponse {
    let m = state.metrics.read().unwrap_or_else(|e| e.into_inner());
    let mut out = String::with_capacity(1024);

    out.push_str("# HELP cairn_http_requests_total Total HTTP requests handled\n");
    out.push_str("# TYPE cairn_http_requests_total counter\n");
    out.push_str(&format!(
        "cairn_http_requests_total {}\n\n",
        m.total_requests
    ));

    out.push_str("# HELP cairn_http_requests_by_path_total Requests grouped by path\n");
    out.push_str("# TYPE cairn_http_requests_by_path_total counter\n");
    let mut paths: Vec<(&String, &u64)> = m.requests_by_path.iter().collect();
    paths.sort_by_key(|(p, _)| p.as_str());
    for (path, count) in &paths {
        let safe = path.replace('\\', "\\\\").replace('"', "\\\"");
        out.push_str(&format!(
            "cairn_http_requests_by_path_total{{path=\"{safe}\"}} {count}\n"
        ));
    }
    out.push('\n');

    out.push_str("# HELP cairn_http_latency_ms HTTP request latency quantiles (ms)\n");
    out.push_str("# TYPE cairn_http_latency_ms gauge\n");
    out.push_str(&format!(
        "cairn_http_latency_ms{{quantile=\"0.50\"}} {}\n",
        m.percentile(50.0)
    ));
    out.push_str(&format!(
        "cairn_http_latency_ms{{quantile=\"0.95\"}} {}\n",
        m.percentile(95.0)
    ));
    out.push_str(&format!(
        "cairn_http_latency_ms{{quantile=\"0.99\"}} {}\n",
        m.percentile(99.0)
    ));
    out.push_str(&format!(
        "cairn_http_latency_ms{{quantile=\"avg\"}} {}\n",
        m.avg_latency_ms()
    ));
    out.push('\n');

    out.push_str("# HELP cairn_http_error_rate Fraction of requests with 4xx/5xx status\n");
    out.push_str("# TYPE cairn_http_error_rate gauge\n");
    out.push_str(&format!("cairn_http_error_rate {:.6}\n\n", m.error_rate()));

    out.push_str("# HELP cairn_http_errors_by_status Error count by HTTP status code\n");
    out.push_str("# TYPE cairn_http_errors_by_status counter\n");
    let mut statuses: Vec<(&u16, &u64)> = m.errors_by_status.iter().collect();
    statuses.sort_by_key(|(s, _)| *s);
    for (status, count) in &statuses {
        out.push_str(&format!(
            "cairn_http_errors_by_status{{status=\"{status}\"}} {count}\n"
        ));
    }

    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        out,
    )
}

// ── OpenTelemetry OTLP trace export ──────────────────────────────────────────

/// Query params for `GET /v1/traces/export`.
#[derive(Deserialize)]
pub(crate) struct TraceExportQuery {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

/// Convert a UUID string (with hyphens) to a 32-char lowercase hex trace ID.
fn uuid_to_trace_id(uuid: &str) -> String {
    uuid.replace('-', "")
}

/// Derive an 8-byte (16 hex char) span ID from the request ID.
fn uuid_to_span_id(uuid: &str) -> String {
    let hex = uuid.replace('-', "");
    hex[hex.len().saturating_sub(16)..].to_owned()
}

/// Format a Unix-nanosecond timestamp as the string integer OTLP expects.
fn ns_to_otlp_time(ns: u64) -> String {
    ns.to_string()
}

/// Build a single OTLP JSON span from a `cairn_app::tokens::RequestLogEntry`.
fn log_entry_to_otlp_span(entry: &cairn_app::tokens::RequestLogEntry) -> serde_json::Value {
    let trace_id = uuid_to_trace_id(&entry.request_id);
    let span_id = uuid_to_span_id(&entry.request_id);

    let start_ns = entry.start_time_unix_ns;
    let end_ns = start_ns + entry.latency_ms * 1_000_000;

    // OTLP span status: 1 = OK, 2 = ERROR
    let (status_code, status_msg) = if entry.status >= 500 {
        (2i32, "Internal Server Error")
    } else if entry.status >= 400 {
        (2i32, "Client Error")
    } else {
        (1i32, "")
    };

    // Build attributes following OpenTelemetry HTTP semantic conventions.
    let mut attrs = vec![
        serde_json::json!({ "key": "http.method",       "value": { "stringValue": entry.method } }),
        serde_json::json!({ "key": "http.target",       "value": { "stringValue": entry.path  } }),
        serde_json::json!({ "key": "http.status_code",  "value": { "intValue": entry.status.to_string() } }),
        serde_json::json!({ "key": "http.flavor",       "value": { "stringValue": "1.1" } }),
        serde_json::json!({ "key": "net.host.name",     "value": { "stringValue": "cairn-app" } }),
        serde_json::json!({ "key": "cairn.request_id",  "value": { "stringValue": entry.request_id } }),
        serde_json::json!({ "key": "cairn.latency_ms",  "value": { "intValue": entry.latency_ms.to_string() } }),
    ];
    if let Some(q) = &entry.query {
        attrs.push(serde_json::json!({ "key": "http.url", "value": { "stringValue": format!("{}?{}", entry.path, q) } }));
    }

    serde_json::json!({
        "traceId":    trace_id,
        "spanId":     span_id,
        "parentSpanId": "",
        "name":       format!("{} {}", entry.method, entry.path),
        "kind":       2,             // SPAN_KIND_SERVER
        "startTimeUnixNano": ns_to_otlp_time(start_ns),
        "endTimeUnixNano":   ns_to_otlp_time(end_ns),
        "attributes": attrs,
        "droppedAttributesCount": 0,
        "events":     [],
        "droppedEventsCount": 0,
        "links":      [],
        "droppedLinksCount": 0,
        "status": {
            "code":    status_code,
            "message": status_msg,
        }
    })
}

/// `GET /v1/traces/export?format=otlp&limit=200`
pub(crate) async fn export_otlp_handler(
    State(state): State<AppState>,
    Query(q): Query<TraceExportQuery>,
) -> impl IntoResponse {
    // Only "otlp" format supported; reject unknown formats.
    if let Some(ref fmt) = q.format {
        if fmt != "otlp" {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": format!("unsupported format '{fmt}'; only 'otlp' is supported"),
                })),
            )
                .into_response();
        }
    }

    let limit = q.limit.unwrap_or(200).min(2_000);

    let entries: Vec<cairn_app::tokens::RequestLogEntry> = match state.request_log.read() {
        Ok(log) => log.tail(limit, &[]).into_iter().cloned().collect(),
        Err(_) => vec![],
    };

    let spans: Vec<serde_json::Value> = entries.iter().map(log_entry_to_otlp_span).collect();

    let body = serde_json::json!({
        "resourceSpans": [{
            "resource": {
                "attributes": [
                    { "key": "service.name",      "value": { "stringValue": "cairn-app" } },
                    { "key": "service.version",   "value": { "stringValue": env!("CARGO_PKG_VERSION") } },
                    { "key": "service.namespace",  "value": { "stringValue": "cairn-rs" } },
                    { "key": "telemetry.sdk.name",     "value": { "stringValue": "cairn-native" } },
                    { "key": "telemetry.sdk.language", "value": { "stringValue": "rust" } },
                ]
            },
            "scopeSpans": [{
                "scope": {
                    "name":    "cairn.http",
                    "version": env!("CARGO_PKG_VERSION"),
                },
                "spans": spans,
            }]
        }]
    });

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        Json(body),
    )
        .into_response()
}
