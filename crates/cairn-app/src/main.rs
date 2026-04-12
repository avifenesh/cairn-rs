//! Bootstrap binary for the Cairn Rust workspace.
//!
//! Usage:
//!   cairn-app                         # local mode, 127.0.0.1:3000
//!   cairn-app --mode team             # self-hosted team mode
//!   cairn-app --port 8080             # custom port
//!   cairn-app --addr 0.0.0.0          # bind all interfaces
//!
#[allow(dead_code)]
mod bundles;
#[allow(dead_code)]
mod entitlements;
mod openapi_spec;
#[allow(dead_code)]
mod sse_hooks;
#[allow(dead_code)]
mod templates;
#[allow(dead_code)]
mod validate;

use std::collections::HashMap;
use std::convert::Infallible;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::header::AUTHORIZATION;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Json;
use axum::Router;
use cairn_api::auth::{
    AuthPrincipal, Authenticator, ServiceTokenAuthenticator, ServiceTokenRegistry,
};
use cairn_api::bootstrap::{BootstrapConfig, DeploymentMode, EncryptionKeySource, StorageBackend};
use cairn_domain::{ApprovalDecision, ApprovalId, ProjectKey, RunId, TaskId};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_runtime::approvals::ApprovalService;
use cairn_runtime::provider_health::ProviderHealthService;
use cairn_runtime::sessions::SessionService;
use cairn_runtime::tasks::TaskService;
use cairn_runtime::{CredentialService, DefaultsService, RecoveryService};
use cairn_runtime::{InMemoryServices, OllamaEmbeddingProvider, OllamaModel, OllamaProvider};
use cairn_store::pg::PgMigrationRunner;
use cairn_store::pg::{PgAdapter, PgEventLog};
use cairn_store::projections::{
    ApprovalReadModel, LlmCallTraceReadModel, RunReadModel, SessionReadModel, TaskReadModel,
    ToolInvocationReadModel,
};
use cairn_store::sqlite::{SqliteAdapter, SqliteEventLog};
use cairn_store::DbAdapter;
use cairn_store::{EventLog, EventPosition};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use tower_http::trace::{DefaultMakeSpan, DefaultOnRequest, DefaultOnResponse, TraceLayer};
use tracing::Level;

// ── Postgres backend ──────────────────────────────────────────────────────────

/// Bundled Postgres connection handles.
///
/// Created at startup when `--db postgres://...` is supplied.
/// Appends go to both Postgres (durable) and InMemory (read models + SSE);
/// event log replays (GET /v1/events) are served from Postgres when present.
#[derive(Clone)]
struct PgBackend {
    event_log: Arc<PgEventLog>,
    adapter: Arc<PgAdapter>,
}

/// Bundled SQLite connection handles.
///
/// Created at startup when `--db sqlite:path` or a bare `.db` path is supplied.
/// Appends go to both SQLite (durable) and InMemory (read models + SSE).
#[derive(Clone)]
struct SqliteBackend {
    event_log: Arc<SqliteEventLog>,
    adapter: Arc<SqliteAdapter>,
    path: PathBuf,
}

// ── Rate limiting ─────────────────────────────────────────────────────────────

/// One sliding-window bucket per identity key (token or IP).
#[derive(Clone)]
struct RateBucket {
    /// Number of requests in the current 60-second window.
    count: u32,
    /// When the current window started (used to decide when to reset).
    window_start: Instant,
}
/// Shared rate-limit table.  Keyed by token (preferred) or IP address.
type RateLimitTable = Arc<Mutex<HashMap<String, RateBucket>>>;

/// Per-token limit: requests per minute.
const RATE_LIMIT_TOKEN: u32 = 1_000;
/// Per-IP limit when no token is present: requests per minute.
const RATE_LIMIT_IP: u32 = 100;
/// Window duration.
const RATE_WINDOW: Duration = Duration::from_secs(60);

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
/// Binary-specific state for routes not covered by the lib.rs catalog.
///
/// Shares `runtime` and `tokens` with `cairn_app::AppState` (same Arc).
/// Fields like `document_store`, `retrieval`, and `ingest` are served
/// exclusively by the catalog router and are NOT duplicated here.
struct AppState {
    runtime: Arc<InMemoryServices>,
    started_at: Arc<Instant>,
    tokens: Arc<ServiceTokenRegistry>,
    pg: Option<Arc<PgBackend>>,
    sqlite: Option<Arc<SqliteBackend>>,
    mode: DeploymentMode,
    /// Shared with lib.rs AppState — kept for seed_demo_data and dead handlers
    /// pending cleanup.
    #[allow(dead_code)]
    document_store: Arc<InMemoryDocumentStore>,
    #[allow(dead_code)]
    retrieval: Arc<InMemoryRetrieval>,
    #[allow(dead_code)]
    ingest: Arc<IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>>,
    ollama: Option<Arc<OllamaProvider>>,
    /// Heavy/generate provider: CAIRN_BRAIN_URL (gemma4 31B etc.)
    openai_compat_brain: Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    /// Light/embed+worker provider: CAIRN_WORKER_URL (qwen3.5, qwen3-embedding)
    openai_compat_worker: Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    /// OpenRouter provider: OPENROUTER_API_KEY activates https://openrouter.ai/api/v1
    openai_compat_openrouter: Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    /// Backward-compat alias: first of brain/worker/openrouter that is configured.
    openai_compat: Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    metrics: Arc<std::sync::RwLock<AppMetrics>>,
    rate_limits: RateLimitTable,
    request_log: Arc<std::sync::RwLock<RequestLogBuffer>>,
    notifications: Arc<std::sync::RwLock<NotificationBuffer>>,
    templates: Arc<templates::TemplateRegistry>,
    entitlements: Arc<entitlements::EntitlementService>,
    bedrock: Option<Arc<cairn_providers::backends::bedrock::Bedrock>>,
    process_role: cairn_api::bootstrap::ProcessRole,
}

// ── Notification buffer ───────────────────────────────────────────────────────

const NOTIF_RING_SIZE: usize = 200;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NotifType {
    ApprovalRequested,
    ApprovalResolved,
    RunCompleted,
    RunFailed,
    TaskStuck,
}

#[derive(Clone, Debug, Serialize)]
pub struct Notification {
    pub id: String,
    #[serde(rename = "type")]
    pub notif_type: NotifType,
    pub message: String,
    /// Entity ID the notification links to (run_id, approval_id, task_id, …).
    pub entity_id: Option<String>,
    /// Hash navigation target for the UI (e.g. "runs", "approvals").
    pub href: String,
    pub read: bool,
    pub created_at: u64,
}

pub struct NotificationBuffer {
    entries: std::collections::VecDeque<Notification>,
}

impl NotificationBuffer {
    fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::with_capacity(NOTIF_RING_SIZE),
        }
    }

    fn push(&mut self, n: Notification) {
        if self.entries.len() == NOTIF_RING_SIZE {
            self.entries.pop_front();
        }
        self.entries.push_back(n);
    }

    fn list(&self, limit: usize) -> Vec<&Notification> {
        self.entries
            .iter()
            .rev()
            .take(limit)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    fn mark_read(&mut self, id: &str) -> bool {
        if let Some(n) = self.entries.iter_mut().find(|n| n.id == id) {
            n.read = true;
            true
        } else {
            false
        }
    }

    fn mark_all_read(&mut self) {
        for n in &mut self.entries {
            n.read = true;
        }
    }

    fn unread_count(&self) -> usize {
        self.entries.iter().filter(|n| !n.read).count()
    }
}

// ── Request metrics ──────────────────────────────────────────────────────────

/// Rolling-window request metrics.  No external crates required.
///
/// Latency samples are stored in a fixed-size ring buffer; percentiles are
/// computed on-demand from a sorted copy of the buffer (cheap for N=1000).
const LATENCY_RING_SIZE: usize = 1_000;

struct AppMetrics {
    total_requests: u64,
    requests_by_path: std::collections::HashMap<String, u64>,
    errors_by_status: std::collections::HashMap<u16, u64>,
    /// Rolling window — LATENCY_RING_SIZE most-recent latencies in ms.
    latency_ring: std::collections::VecDeque<u64>,
}

impl AppMetrics {
    fn new() -> Self {
        Self {
            total_requests: 0,
            requests_by_path: std::collections::HashMap::new(),
            errors_by_status: std::collections::HashMap::new(),
            latency_ring: std::collections::VecDeque::with_capacity(LATENCY_RING_SIZE),
        }
    }
    fn percentile(&self, p: f64) -> u64 {
        if self.latency_ring.is_empty() {
            return 0;
        }
        let mut sorted: Vec<u64> = self.latency_ring.iter().copied().collect();
        sorted.sort_unstable();
        let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    fn avg_latency_ms(&self) -> u64 {
        if self.latency_ring.is_empty() {
            return 0;
        }
        self.latency_ring.iter().sum::<u64>() / self.latency_ring.len() as u64
    }

    fn error_rate(&self) -> f64 {
        if self.total_requests == 0 {
            return 0.0;
        }
        let errors: u64 = self.errors_by_status.values().sum();
        errors as f64 / self.total_requests as f64
    }
}

// ── Request log ring buffer ───────────────────────────────────────────────────

/// Maximum number of structured log entries retained in memory.
const LOG_RING_SIZE: usize = 2_000;

/// One structured request log entry.
#[derive(Clone, Serialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: &'static str,
    pub message: String,
    pub request_id: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub status: u16,
    pub latency_ms: u64,
    /// Wall-clock start time in Unix nanoseconds.  Used for OTLP span export.
    pub start_time_unix_ns: u64,
}

/// Fixed-capacity FIFO ring buffer of structured log entries.
pub struct RequestLogBuffer {
    entries: std::collections::VecDeque<LogEntry>,
}

impl RequestLogBuffer {
    fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::with_capacity(LOG_RING_SIZE),
        }
    }
    /// Return the last `n` entries whose level matches the filter (empty = all).
    fn tail(&self, n: usize, level_filter: &[&str]) -> Vec<&LogEntry> {
        self.entries
            .iter()
            .rev()
            .filter(|e| level_filter.is_empty() || level_filter.contains(&e.level))
            .take(n)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }
}

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
pub struct RequestId(pub String);

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
fn pagination_headers(
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
            (Some(t), Some(w), Some(p)) => {
                Some(ProjectKey::new(t.as_str(), w.as_str(), p.as_str()))
            }
            _ => None,
        }
    }
}

// ── Metrics middleware ────────────────────────────────────────────────────────

// ── Version + changelog ──────────────────────────────────────────────────────

/// The canonical application version — sourced from Cargo.toml at compile time.
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Static changelog.  Add a new entry for every published release.
const CHANGELOG: &str = r##"[
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
async fn version_header_middleware(req: Request<Body>, next: Next) -> Response {
    let mut resp = next.run(req).await;
    if let Ok(v) = axum::http::HeaderValue::from_str(APP_VERSION) {
        resp.headers_mut().insert("X-Cairn-Version", v);
    }
    resp
}

/// `GET /v1/changelog` — release notes as a JSON array.
/// Public endpoint — no auth required.
async fn changelog_handler() -> impl IntoResponse {
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
struct TestWebhookRequest {
    url: String,
    event_type: String,
}

#[derive(serde::Serialize)]
struct TestWebhookResponse {
    success: bool,
    status_code: u16,
    latency_ms: u64,
}

async fn test_webhook_handler(
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
fn rate_limit_key(req: &Request<Body>) -> (String, u32) {
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
async fn rate_limit_status_handler(
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

// ── Detailed health handler ───────────────────────────────────────────────────

/// Per-subsystem health entry.
#[derive(Serialize)]
struct CheckEntry {
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    models: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capacity: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rss_mb: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    heap_mb: Option<u64>,
}

#[derive(Serialize)]
struct DetailedHealthChecks {
    store: CheckEntry,
    ollama: CheckEntry,
    event_buffer: CheckEntry,
    memory: CheckEntry,
}

#[derive(Serialize)]
struct DetailedHealthResponse {
    status: &'static str,
    checks: DetailedHealthChecks,
    uptime_seconds: u64,
    version: &'static str,
    started_at: String,
    /// RFC 011: current process role.
    role: String,
}

/// Read resident set size from `/proc/self/status` (Linux only).
/// Returns (rss_kb, vm_size_kb).  Returns (0, 0) on other platforms.
fn read_proc_memory() -> (u64, u64) {
    #[cfg(target_os = "linux")]
    {
        if let Ok(text) = std::fs::read_to_string("/proc/self/status") {
            let mut rss = 0u64;
            let mut vm = 0u64;
            for line in text.lines() {
                if line.starts_with("VmRSS:") {
                    rss = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                } else if line.starts_with("VmSize:") {
                    vm = line
                        .split_whitespace()
                        .nth(1)
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                }
            }
            return (rss, vm);
        }
    }
    (0, 0)
}

/// `GET /v1/health/detailed` — deep health status for every subsystem.
async fn detailed_health_handler(State(state): State<AppState>) -> Json<DetailedHealthResponse> {
    // ── Store check ───────────────────────────────────────────────────────────
    let store_start = Instant::now();
    let store_ok = if let Some(pg) = &state.pg {
        pg.adapter.health_check().await.is_ok()
    } else if let Some(sq) = &state.sqlite {
        sq.adapter.health_check().await.is_ok()
    } else {
        state.runtime.store.head_position().await.is_ok()
    };
    let store_latency = store_start.elapsed().as_millis() as u64;

    let store_check = CheckEntry {
        status: if store_ok { "healthy" } else { "unhealthy" },
        latency_ms: Some(store_latency),
        models: None,
        size: None,
        capacity: None,
        rss_mb: None,
        heap_mb: None,
    };

    // ── Ollama check ──────────────────────────────────────────────────────────
    let ollama_check = if let Some(provider) = &state.ollama {
        let t = Instant::now();
        match provider.health_check().await {
            Ok(tags) => CheckEntry {
                status: "healthy",
                latency_ms: Some(t.elapsed().as_millis() as u64),
                models: Some(tags.models.len()),
                size: None,
                capacity: None,
                rss_mb: None,
                heap_mb: None,
            },
            Err(_) => CheckEntry {
                status: "unhealthy",
                latency_ms: None,
                models: None,
                size: None,
                capacity: None,
                rss_mb: None,
                heap_mb: None,
            },
        }
    } else {
        CheckEntry {
            status: "unconfigured",
            latency_ms: None,
            models: None,
            size: None,
            capacity: None,
            rss_mb: None,
            heap_mb: None,
        }
    };

    // ── Event buffer (not present in main.rs; always at capacity 0) ──────────
    // The SSE ring buffer lives in lib.rs AppState only.  For completeness we
    // report it as healthy with unknown size.
    let event_buffer_check = CheckEntry {
        status: "healthy",
        latency_ms: None,
        size: None,
        capacity: None,
        models: None,
        rss_mb: None,
        heap_mb: None,
    };

    // ── Process memory ────────────────────────────────────────────────────────
    let (rss_kb, _vm_kb) = read_proc_memory();
    let memory_check = CheckEntry {
        status: "healthy",
        rss_mb: Some(rss_kb / 1024),
        heap_mb: None, // allocator-level heap not easily available without jemalloc
        latency_ms: None,
        models: None,
        size: None,
        capacity: None,
    };

    // ── Overall status ────────────────────────────────────────────────────────
    let degraded = !store_ok || matches!(ollama_check.status, "unhealthy");

    let overall = if degraded { "degraded" } else { "healthy" };

    // ISO-8601 started_at from uptime
    let uptime = state.started_at.elapsed().as_secs();
    let started_secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(uptime);
    let started_at = format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        1970 + started_secs / 31_557_600, // approx — good enough for display
        ((started_secs % 31_557_600) / 2_629_800) + 1,
        ((started_secs % 2_629_800) / 86_400) + 1,
        (started_secs % 86_400) / 3_600,
        (started_secs % 3_600) / 60,
        started_secs % 60,
    );

    Json(DetailedHealthResponse {
        status: overall,
        checks: DetailedHealthChecks {
            store: store_check,
            ollama: ollama_check,
            event_buffer: event_buffer_check,
            memory: memory_check,
        },
        uptime_seconds: uptime,
        version: env!("CARGO_PKG_VERSION"),
        started_at,
        role: state.process_role.as_str().to_owned(),
    })
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

/// Query parameters accepted by `GET /v1/ws`.
#[derive(Deserialize)]
struct WsQueryParams {
    /// Bearer token — required because browsers can't set headers during WS upgrade.
    token: Option<String>,
}

/// `GET /v1/ws` — real-time event stream over WebSocket (RFC 002 companion).
///
/// ### Auth
/// Pass the admin token via `?token=<token>` query parameter.
/// Header-based auth cannot be used for WebSocket connections from browsers.
///
/// ### Client → Server messages (JSON)
/// ```json
/// { "type": "subscribe",  "event_types": ["run_created", "task_queued"] }
/// { "type": "ping" }
/// ```
///
/// ### Server → Client messages (JSON)
/// ```json
/// { "type": "connected",  "head_position": 42 }
/// { "type": "event",      "position": 43, "event_type": "run_created", "event_id": "...", "payload": {...} }
/// { "type": "pong" }
/// { "type": "warn",       "message": "lagged: missed N event(s)" }
/// ```
async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    Query(params): Query<WsQueryParams>,
) -> impl IntoResponse {
    // Authenticate via query param — same token registry as bearer auth.
    let token = match params.token {
        Some(t) if !t.is_empty() => t,
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let authenticator = ServiceTokenAuthenticator::new(state.tokens.clone());
    if authenticator.authenticate(&token).is_err() {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    ws.on_upgrade(move |socket| handle_ws_connection(socket, state))
}

/// Drive a single WebSocket connection to completion.
async fn handle_ws_connection(mut socket: WebSocket, state: AppState) {
    use tokio::sync::broadcast::error::RecvError;

    let mut receiver = state.runtime.store.subscribe();

    // Active event-type filter set by the client (None = all events).
    let mut filter: Option<Vec<String>> = None;

    // Send the "connected" handshake with the current log head position.
    let head_pos = state
        .runtime
        .store
        .head_position()
        .await
        .ok()
        .flatten()
        .map(|p| p.0)
        .unwrap_or(0);

    let connected = serde_json::json!({ "type": "connected", "head_position": head_pos });
    if socket
        .send(WsMessage::Text(connected.to_string()))
        .await
        .is_err()
    {
        return;
    }

    loop {
        tokio::select! {
            // ── Inbound from the browser ──────────────────────────────────
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                            match val.get("type").and_then(|t| t.as_str()) {
                                Some("subscribe") => {
                                    filter = val
                                        .get("event_types")
                                        .and_then(|e| e.as_array())
                                        .map(|arr| {
                                            arr.iter()
                                                .filter_map(|v| v.as_str().map(str::to_owned))
                                                .collect()
                                        });
                                }
                                Some("ping") => {
                                    let _ = socket
                                        .send(WsMessage::Text(r#"{"type":"pong"}"#.to_owned()))
                                        .await;
                                }
                                _ => {}
                            }
                        }
                    }
                    Some(Ok(WsMessage::Ping(data))) => {
                        let _ = socket.send(WsMessage::Pong(data)).await;
                    }
                    // Client closed or errored — end the task.
                    Some(Ok(WsMessage::Close(_))) | None | Some(Err(_)) => break,
                    _ => {}
                }
            }

            // ── Outbound broadcast events ─────────────────────────────────
            recv_result = receiver.recv() => {
                match recv_result {
                    Ok(event) => {
                        let event_type = event_type_name(&event.envelope.payload);

                        // Apply the client's subscription filter when set.
                        if let Some(ref types) = filter {
                            if !types.is_empty()
                                && !types.iter().any(|t| t.as_str() == event_type)
                            {
                                continue;
                            }
                        }

                        let msg = serde_json::json!({
                            "type":       "event",
                            "position":   event.position.0,
                            "event_type": event_type,
                            "event_id":   event.envelope.event_id.as_str(),
                            "payload":    &event.envelope.payload,
                        });

                        if socket
                            .send(WsMessage::Text(msg.to_string()))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    // Missed messages due to a slow consumer — notify and continue.
                    Err(RecvError::Lagged(n)) => {
                        let warn = serde_json::json!({
                            "type":    "warn",
                            "message": format!("lagged: missed {n} event(s)"),
                        });
                        let _ = socket
                            .send(WsMessage::Text(warn.to_string()))
                            .await;
                    }
                    // Broadcast channel dropped — server shutting down.
                    Err(RecvError::Closed) => break,
                }
            }
        }
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
        Err(e) => return Err(internal_error(e.to_string())),
        Ok(Some(_)) => {}
    }

    match TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &run_id, 1000).await {
        Ok(tasks) => Ok(Json(tasks)),
        Err(e) => Err(internal_error(e.to_string())),
    }
}

/// `POST /v1/runs/:id/tasks` — explicitly create a task within a run.
///
/// Submits the task through the `TaskService`, which appends a `TaskCreated`
/// event and projects it into the read model.  The new task starts in the
/// `queued` state, ready to be claimed by a worker.
///
/// Body: `{ "name": "...", "description": "...", "metadata": {...} }`
/// All fields except `name` are optional; `task_id` is auto-generated.
#[derive(Deserialize)]
struct CreateTaskBody {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    metadata: Option<serde_json::Value>,
    #[serde(default)]
    task_id: Option<String>,
}

async fn create_run_task_handler(
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
///
/// Valid only when the task is in `leased` state.  Appends a
/// `TaskStateChanged(leased → running)` event.
async fn start_task_handler(
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

/// `POST /v1/tasks/:id/fail` — mark a running or claimed task as failed.
///
/// Body: `{ "error": "reason string" }`
/// The `error` string is echoed in the response alongside the updated record.
#[derive(Deserialize)]
struct FailTaskBody {
    error: String,
}

async fn fail_task_handler(
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
        Err(e) => return Err(internal_error(e.to_string())),
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
struct CreateRunBody {
    tenant_id: Option<String>,
    workspace_id: Option<String>,
    project_id: Option<String>,
    session_id: Option<String>,
    run_id: Option<String>,
    parent_run_id: Option<String>,
}

// ── Batch handlers ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BatchCreateRunsBody {
    runs: Vec<CreateRunBody>,
}

/// `POST /v1/runs/batch` — create multiple runs in one request.
///
/// Each element in `runs` follows the same schema as `POST /v1/runs`.
/// Runs are created sequentially; the response preserves input order.
/// If any run fails the remainder still proceed — each result carries an
/// `ok` flag and either the created record or an `error` string.
async fn batch_create_runs_handler(
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
struct BatchCancelTasksBody {
    task_ids: Vec<String>,
}

/// `POST /v1/tasks/batch/cancel` — cancel multiple tasks in one call.
///
/// Tasks that are already terminal (completed, failed, canceled) are
/// counted as failures with an `already_terminal` reason rather than
/// as errors, so the caller can distinguish "nothing to cancel" from
/// actual service failures.
async fn batch_cancel_tasks_handler(
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

// ── Export / Import handlers ──────────────────────────────────────────────────

/// Current export format version.  Increment when the shape changes
/// incompatibly so importers can detect version mismatches early.
const EXPORT_VERSION: &str = "1.0";

/// Build an ISO-8601 timestamp string from the current system time.
fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    format!(
        "{}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        1970 + secs / 31_557_600,
        ((secs % 31_557_600) / 2_629_800) + 1,
        ((secs % 2_629_800) / 86_400) + 1,
        (secs % 86_400) / 3_600,
        (secs % 3_600) / 60,
        secs % 60,
    )
}

/// `GET /v1/runs/:id/export` — export a run with all its tasks and events.
///
/// The response is a JSON document suitable for archiving or importing into
/// another cairn instance.  The `Content-Disposition` header prompts browsers
/// to download it as `run-<id>.json`.
async fn export_run_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let run_id = RunId::new(&id);

    // Fetch the run record.
    let run = match RunReadModel::get(state.runtime.store.as_ref(), &run_id).await {
        Ok(Some(r)) => r,
        Ok(None) => return Err(not_found(format!("run {id} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // Fetch tasks.
    let tasks = match TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &run_id, 2000)
        .await
    {
        Ok(t) => t,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // Fetch events (summaries — full payloads are not stored by default).
    let events = match state
        .runtime
        .store
        .read_by_entity(&cairn_store::EntityRef::Run(run_id), None, 2000)
        .await
    {
        Ok(evts) => evts
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "position":   e.position.0,
                    "stored_at":  e.stored_at,
                    "event_type": event_type_name(&e.envelope.payload),
                    "event_id":   e.envelope.event_id.as_str(),
                })
            })
            .collect::<Vec<_>>(),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let body = serde_json::json!({
        "version":     EXPORT_VERSION,
        "type":        "run_export",
        "exported_at": now_iso8601(),
        "data": {
            "run":    run,
            "tasks":  tasks,
            "events": events,
        }
    });

    let filename = format!("run-{id}.json");
    let content_disposition = format!("attachment; filename=\"{filename}\"");
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&content_disposition)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

/// `GET /v1/sessions/:id/export` — export a session with all runs, tasks, events.
async fn export_session_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl axum::response::IntoResponse {
    let session_id = cairn_domain::SessionId::new(&id);

    // Fetch the session record.
    let session = match SessionReadModel::get(state.runtime.store.as_ref(), &session_id).await {
        Ok(Some(s)) => s,
        Ok(None) => return Err(not_found(format!("session {id} not found"))),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // All runs in this session.
    let runs = match RunReadModel::list_by_session(
        state.runtime.store.as_ref(),
        &session_id,
        500,
        0,
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    // All tasks for every run.
    let mut all_tasks: Vec<serde_json::Value> = Vec::new();
    for run in &runs {
        let rid = run.run_id.clone();
        if let Ok(tasks) =
            TaskReadModel::list_by_parent_run(state.runtime.store.as_ref(), &rid, 2000).await
        {
            for t in tasks {
                all_tasks.push(serde_json::to_value(t).unwrap_or(serde_json::Value::Null));
            }
        }
    }

    // Events for the session itself.
    let events = match state
        .runtime
        .store
        .read_by_entity(&cairn_store::EntityRef::Session(session_id), None, 2000)
        .await
    {
        Ok(evts) => evts
            .into_iter()
            .map(|e| {
                serde_json::json!({
                    "position":   e.position.0,
                    "stored_at":  e.stored_at,
                    "event_type": event_type_name(&e.envelope.payload),
                    "event_id":   e.envelope.event_id.as_str(),
                })
            })
            .collect::<Vec<_>>(),
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let body = serde_json::json!({
        "version":     EXPORT_VERSION,
        "type":        "session_export",
        "exported_at": now_iso8601(),
        "data": {
            "session": session,
            "runs":    runs,
            "tasks":   all_tasks,
            "events":  events,
        }
    });

    let filename = format!("session-{id}.json");
    let content_disposition = format!("attachment; filename=\"{filename}\"");
    let mut resp = (StatusCode::OK, Json(body)).into_response();
    resp.headers_mut().insert(
        axum::http::header::CONTENT_DISPOSITION,
        axum::http::HeaderValue::from_str(&content_disposition)
            .unwrap_or_else(|_| axum::http::HeaderValue::from_static("attachment")),
    );
    Ok(resp)
}

/// Import body for `POST /v1/sessions/import`.
#[derive(Deserialize)]
struct ImportSessionBody {
    version: Option<String>,
    #[serde(rename = "type")]
    export_type: Option<String>,
    data: Option<ImportSessionData>,
}

#[derive(Deserialize)]
struct ImportSessionData {
    session: Option<serde_json::Value>,
}

/// `POST /v1/sessions/import` — re-create a session from a session export.
///
/// Only the session record itself is re-created; runs, tasks, and events are
/// **not** replayed (that would require a full event-log replay which is out
/// of scope for this endpoint).  Returns the newly created session record.
async fn import_session_handler(
    State(state): State<AppState>,
    Json(body): Json<ImportSessionBody>,
) -> impl axum::response::IntoResponse {
    // Validate version
    if let Some(ref v) = body.version {
        if v != EXPORT_VERSION {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiError {
                    code: "version_mismatch",
                    message: format!(
                        "export version {v} is not supported; expected {EXPORT_VERSION}"
                    ),
                }),
            ));
        }
    }

    // Validate type
    match body.export_type.as_deref() {
        Some("session_export") | None => {}
        Some(t) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ApiError {
                    code: "wrong_export_type",
                    message: format!("expected 'session_export', got '{t}'"),
                }),
            ));
        }
    }

    let session_data = body.data.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "missing_data",
                message: "import body must include a 'data' field".to_owned(),
            }),
        )
    })?;

    let session_json = session_data.session.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                code: "missing_session",
                message: "'data.session' is required".to_owned(),
            }),
        )
    })?;

    // Extract fields from the exported session record.
    let session_id_str = session_json
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("imported_session");

    let project_obj = session_json.get("project");
    let tenant_id = project_obj
        .and_then(|p| p.get("tenant_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let workspace_id = project_obj
        .and_then(|p| p.get("workspace_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");
    let project_id = project_obj
        .and_then(|p| p.get("project_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("default");

    let project = ProjectKey::new(tenant_id, workspace_id, project_id);
    let session_id = cairn_domain::SessionId::new(session_id_str);

    match SessionService::create(&state.runtime.sessions, &project, session_id).await {
        Ok(record) => Ok((StatusCode::CREATED, Json(serde_json::json!(record)))),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                code: "create_failed",
                message: e.to_string(),
            }),
        )),
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
async fn list_all_pending(
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
struct ResolveApprovalBody {
    /// `"approved"` or `"rejected"`
    decision: String,
    /// Optional free-text explanation logged alongside the decision.
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Serialize)]
struct ResolveApprovalResponse {
    #[serde(flatten)]
    record: cairn_store::projections::ApprovalRecord,
    /// Echo of the reason supplied in the request body (if any).
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

/// `POST /v1/approvals/:id/resolve` — approve or reject a pending approval.
///
/// Body: `{ "decision": "approved" | "rejected", "reason": "<optional string>" }`
async fn resolve_approval_handler(
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
    // Use durable event log for replay when available (Postgres > SQLite > InMemory).
    let read_result = if let Some(pg) = &state.pg {
        pg.event_log.read_stream(after, limit).await
    } else if let Some(sq) = &state.sqlite {
        sq.event_log.read_stream(after, limit).await
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

        // ── Notification hook ──────────────────────────────────────────────────
        // Inspect each event and push a notification for operator-relevant ones.
        {
            use cairn_domain::lifecycle::RunState;
            use cairn_domain::RuntimeEvent as E;
            use std::time::SystemTime;
            let now_ms = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let notif_id = format!("notif-{}", &event_id[..event_id.len().min(16)]);

            let maybe_notif: Option<Notification> = match &envelope.payload {
                E::ApprovalRequested(e) => Some(Notification {
                    id: notif_id,
                    notif_type: NotifType::ApprovalRequested,
                    message: format!(
                        "Approval requested for {}",
                        e.run_id.as_ref().map(|r| r.as_str()).unwrap_or("a task"),
                    ),
                    entity_id: Some(e.approval_id.as_str().to_owned()),
                    href: "approvals".to_owned(),
                    read: false,
                    created_at: now_ms,
                }),
                E::ApprovalResolved(e) => Some(Notification {
                    id: notif_id,
                    notif_type: NotifType::ApprovalResolved,
                    message: format!(
                        "Approval {} — decision: {:?}",
                        e.approval_id.as_str(),
                        e.decision,
                    ),
                    entity_id: Some(e.approval_id.as_str().to_owned()),
                    href: "approvals".to_owned(),
                    read: false,
                    created_at: now_ms,
                }),
                E::RunStateChanged(e) => match &e.transition.to {
                    RunState::Completed => Some(Notification {
                        id: notif_id,
                        notif_type: NotifType::RunCompleted,
                        message: format!("Run {} completed", e.run_id.as_str()),
                        entity_id: Some(e.run_id.as_str().to_owned()),
                        href: format!("run/{}", e.run_id.as_str()),
                        read: false,
                        created_at: now_ms,
                    }),
                    RunState::Failed => Some(Notification {
                        id: notif_id,
                        notif_type: NotifType::RunFailed,
                        message: format!(
                            "Run {} failed{}",
                            e.run_id.as_str(),
                            e.failure_class
                                .as_ref()
                                .map(|f| format!(" ({f:?})"))
                                .unwrap_or_default(),
                        ),
                        entity_id: Some(e.run_id.as_str().to_owned()),
                        href: format!("run/{}", e.run_id.as_str()),
                        read: false,
                        created_at: now_ms,
                    }),
                    _ => None,
                },
                E::TaskStateChanged(e) => {
                    use cairn_domain::lifecycle::TaskState;
                    match &e.transition.to {
                        TaskState::DeadLettered | TaskState::RetryableFailed => {
                            Some(Notification {
                                id: notif_id,
                                notif_type: NotifType::TaskStuck,
                                message: format!(
                                    "Task {} is stuck ({:?})",
                                    e.task_id.as_str(),
                                    e.transition.to,
                                ),
                                entity_id: Some(e.task_id.as_str().to_owned()),
                                href: "tasks".to_owned(),
                                read: false,
                                created_at: now_ms,
                            })
                        }
                        _ => None,
                    }
                }
                _ => None,
            };

            if let Some(n) = maybe_notif {
                if let Ok(mut buf) = state.notifications.write() {
                    buf.push(n);
                }
            }
        }
        // ── End notification hook ──────────────────────────────────────────────

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
        // Dual-write: persist to durable backend first, then update InMemory
        // so projections and SSE broadcasts stay current.
        if let Some(ref pg) = state.pg {
            if let Err(e) = pg.event_log.append(std::slice::from_ref(&envelope)).await {
                return Err(internal_error(format!("postgres append: {e}")));
            }
        } else if let Some(ref sq) = state.sqlite {
            if let Err(e) = sq.event_log.append(std::slice::from_ref(&envelope)).await {
                return Err(internal_error(format!("sqlite append: {e}")));
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
    if let Some(pg) = &state.pg {
        let connected = pg.adapter.health_check().await.is_ok();
        let (migration_count, schema_current) = if connected {
            let pool = pg.adapter.pool().clone();
            let runner = PgMigrationRunner::new(pool);
            match runner.applied().await {
                Ok(applied) => {
                    const TOTAL_KNOWN: usize = 20;
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
    } else if let Some(sq) = &state.sqlite {
        let connected = sq.adapter.health_check().await.is_ok();
        Json(DbStatusResponse {
            backend: "sqlite",
            connected,
            migration_count: None, // SQLite uses single-shot migrate(), no versioned log
            schema_current: Some(connected),
        })
    } else {
        Json(DbStatusResponse {
            backend: "in_memory",
            connected: true,
            migration_count: None,
            schema_current: None,
        })
    }
}

/// `GET /v1/admin/audit-log?limit=100` — list audit log entries for the operator's tenant.
// ── Notification handlers ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct NotifListQuery {
    limit: Option<usize>,
}

#[derive(Serialize)]
struct NotifListResponse {
    notifications: Vec<Notification>,
    unread_count: usize,
}

/// `GET /v1/notifications?limit=50` — list recent notifications.
async fn list_notifications_handler(
    State(state): State<AppState>,
    Query(q): Query<NotifListQuery>,
) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(50).min(200);
    let buf = state
        .notifications
        .read()
        .expect("notification lock poisoned");
    let notifications: Vec<Notification> = buf.list(limit).into_iter().cloned().collect();
    let unread_count = buf.unread_count();
    Json(NotifListResponse {
        notifications,
        unread_count,
    })
}

/// `POST /v1/notifications/:id/read` — mark one notification as read.
async fn mark_notification_read_handler(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> impl IntoResponse {
    let found = state
        .notifications
        .write()
        .expect("notification lock poisoned")
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
async fn mark_all_notifications_read_handler(State(state): State<AppState>) -> impl IntoResponse {
    state
        .notifications
        .write()
        .expect("notification lock poisoned")
        .mark_all_read();
    StatusCode::NO_CONTENT
}

// ── Request log handler (now served by lib.rs catalog; kept for OTLP export) ─

#[allow(dead_code)]
#[derive(Deserialize)]
struct LogsQuery {
    /// Maximum entries to return (default 100, max 500).
    #[serde(default = "default_logs_limit")]
    limit: usize,
    /// Comma-separated level filter: "info", "warn", "error".  Omit = all levels.
    level: Option<String>,
}

#[allow(dead_code)]
fn default_logs_limit() -> usize {
    100
}

/// `GET /v1/admin/logs` — now served by the catalog-driven handler in lib.rs.
/// This handler is retained for the OTLP export endpoint which also reads request logs.
#[allow(dead_code)]
async fn list_request_logs_handler(
    State(state): State<AppState>,
    Query(q): Query<LogsQuery>,
) -> impl IntoResponse {
    let limit = q.limit.min(500);
    let level_filter: Vec<&'static str> = q
        .level
        .as_deref()
        .map(|s| {
            s.split(',')
                .filter_map(|l| match l.trim() {
                    "info" => Some("info"),
                    "warn" => Some("warn"),
                    "error" => Some("error"),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();

    let entries: Vec<LogEntry> = match state.request_log.read() {
        Ok(log) => log
            .tail(limit, &level_filter)
            .into_iter()
            .cloned()
            .collect(),
        Err(_) => vec![],
    };

    let total = entries.len();
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "entries": entries,
            "total":   total,
            "limit":   limit,
        })),
    )
}

// ── OpenTelemetry OTLP trace export ──────────────────────────────────────────

/// Query params for `GET /v1/traces/export`.
#[derive(Deserialize)]
struct TraceExportQuery {
    /// Export format.  Only `otlp` is supported today; kept for future extensibility.
    #[serde(default)]
    format: Option<String>,
    /// Maximum number of spans to include (default 200, max 2 000).
    #[serde(default)]
    limit: Option<usize>,
}

/// Convert a UUID string (with hyphens) to a 32-char lowercase hex trace ID.
/// OTLP requires exactly 32 hex characters (128-bit trace ID).
fn uuid_to_trace_id(uuid: &str) -> String {
    uuid.replace('-', "")
}

/// Derive an 8-byte (16 hex char) span ID from the request ID.
/// We take the last 16 hex chars of the trace ID so trace and span share a root.
fn uuid_to_span_id(uuid: &str) -> String {
    let hex = uuid.replace('-', "");
    hex[hex.len().saturating_sub(16)..].to_owned()
}

/// Format a Unix-nanosecond timestamp as the string integer OTLP expects.
fn ns_to_otlp_time(ns: u64) -> String {
    ns.to_string()
}

/// Build a single OTLP JSON span from a `LogEntry`.
fn log_entry_to_otlp_span(entry: &LogEntry) -> serde_json::Value {
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
///
/// Returns HTTP request spans formatted as OTLP (OpenTelemetry Protocol) JSON.
/// Compatible with Jaeger, Zipkin (via OTLP bridge), Grafana Tempo, and any
/// OTLP-capable tracing backend.
///
/// Each span represents one HTTP request processed by cairn-app:
/// - `traceId` / `spanId` derived from the internal request UUID
/// - `startTimeUnixNano` / `endTimeUnixNano` from wall-clock + latency
/// - Attributes follow the OpenTelemetry HTTP semantic conventions
///
/// The `resourceSpans[0].resource` identifies the service (`cairn-app`).
async fn export_otlp_handler(
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

    let entries: Vec<LogEntry> = match state.request_log.read() {
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

// ── Admin snapshot / restore ──────────────────────────────────────────────────

/// `POST /v1/admin/snapshot` — export the full InMemory event log as a
/// downloadable JSON attachment.
///
/// The snapshot contains all events in position order. Restoring it via
/// `POST /v1/admin/restore` will replay every event and rebuild all
/// projections from scratch, giving a consistent store state.
/// POST /v1/admin/rotate-token — rotate the admin bearer token at runtime.
///
/// Requires the current admin token in the Authorization header.
/// Body: `{ "new_token": "..." }` (min 16 chars).
///
/// The token registry is shared (same Arc) between the main.rs and lib.rs
/// routers, so a single revoke+register updates both.
async fn rotate_token_handler(
    State(state): State<AppState>,
    Json(body): Json<RotateTokenRequest>,
) -> impl IntoResponse {
    let new_token = body.new_token.trim().to_owned();
    if new_token.len() < 16 {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "new_token must be at least 16 characters"})),
        );
    }

    // Find and revoke the old admin token, then register the new one.
    let entries = state.tokens.all_entries();
    for (old_token, principal) in &entries {
        if let AuthPrincipal::ServiceAccount { name, .. } = principal {
            if name == "admin" {
                state.tokens.revoke(old_token);
                state.tokens.register(new_token, principal.clone());
                return (
                    StatusCode::OK,
                    Json(serde_json::json!({"status": "rotated"})),
                );
            }
        }
    }

    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(serde_json::json!({"error": "admin token not found in registry"})),
    )
}

#[derive(Deserialize)]
struct RotateTokenRequest {
    new_token: String,
}

async fn admin_snapshot_handler(State(state): State<AppState>) -> impl IntoResponse {
    let snap = state.runtime.store.dump_events();
    let json = match serde_json::to_vec_pretty(&snap) {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::response::Response::builder()
                    .status(500)
                    .body(axum::body::Body::from(e.to_string()))
                    .unwrap(),
            )
                .into_response();
        }
    };
    let filename = format!(
        "cairn-snapshot-{}.json",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    );
    axum::response::Response::builder()
        .status(200)
        .header("Content-Type", "application/json; charset=utf-8")
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"{filename}\""),
        )
        .header("X-Event-Count", snap.event_count.to_string())
        .body(axum::body::Body::from(json))
        .unwrap()
        .into_response()
}

async fn backup_handler(State(state): State<AppState>) -> impl IntoResponse {
    let Some(sqlite) = state.sqlite.as_ref() else {
        return not_found("SQLite backup is only available when the SQLite backend is active")
            .into_response();
    };

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let backup_path = PathBuf::from(format!("{}.backup-{timestamp}", sqlite.path.display()));

    let size_bytes = match tokio::fs::copy(&sqlite.path, &backup_path).await {
        Ok(bytes) => bytes,
        Err(error) => {
            return internal_error(format!(
                "failed to back up SQLite database {}: {error}",
                sqlite.path.display()
            ))
            .into_response();
        }
    };

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "backed_up",
            "path": backup_path.to_string_lossy(),
            "size_bytes": size_bytes,
        })),
    )
        .into_response()
}

/// `POST /v1/admin/restore` — restore from a snapshot uploaded as a JSON body.
///
/// Clears all in-memory state and replays the uploaded event log. Returns the
/// count of replayed events. This is irreversible — take a snapshot first.
async fn admin_restore_handler(
    State(state): State<AppState>,
    axum::Json(snap): axum::Json<cairn_store::snapshot::StoreSnapshot>,
) -> impl IntoResponse {
    let event_count = snap.event_count;
    let replayed = state.runtime.store.load_snapshot(snap);
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "ok":           true,
            "event_count":  event_count,
            "replayed":     replayed,
        })),
    )
}

// ── Projection rebuild + event inspection handlers ────────────────────────────

/// `POST /v1/admin/rebuild-projections` — replay the full event log through
/// all in-memory projections, rebuilding every read model from scratch.
///
/// This is the primary operational recovery tool: if a projection diverges
/// from the event log (e.g. after a bug fix), call this endpoint to restore
/// consistency without losing events.
///
/// Internally the handler performs a snapshot → restore cycle: it exports the
/// current event log and immediately replays it, which exercises
/// `apply_projection` on every stored event in order.
///
/// Returns: `{ events_replayed: N, duration_ms: N }`
async fn rebuild_projections_handler(State(state): State<AppState>) -> impl IntoResponse {
    let t0 = std::time::Instant::now();
    let snap = state.runtime.store.dump_events();
    let events_replayed = state.runtime.store.load_snapshot(snap);
    let duration_ms = t0.elapsed().as_millis() as u64;

    tracing::info!(events_replayed, duration_ms, "projections rebuilt");

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "events_replayed": events_replayed,
            "duration_ms":     duration_ms,
        })),
    )
}

/// `GET /v1/admin/event-count` — total event count and a per-type breakdown.
///
/// Returns: `{ total: N, by_type: { "session_created": 5, ... } }`
///
/// Useful for a quick health check on event log cardinality and for spotting
/// unexpected event type distributions.
async fn event_count_handler(State(state): State<AppState>) -> impl IntoResponse {
    let events = match state.runtime.store.read_stream(None, usize::MAX).await {
        Ok(v) => v,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let total = events.len() as u64;
    let mut by_type: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for ev in &events {
        *by_type
            .entry(event_type_name(&ev.envelope.payload).to_owned())
            .or_insert(0) += 1;
    }

    // Sort the breakdown for deterministic output.
    let mut sorted: Vec<(String, u64)> = by_type.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let by_type_obj: serde_json::Map<String, serde_json::Value> = sorted
        .into_iter()
        .map(|(k, v)| (k, serde_json::Value::Number(v.into())))
        .collect();

    Ok(Json(serde_json::json!({
        "total":   total,
        "by_type": serde_json::Value::Object(by_type_obj),
    })))
}

/// `GET /v1/admin/event-log?from=0&limit=100` — paginated raw event access.
///
/// Returns events in ascending position order.  Each event includes its
/// position, stored_at timestamp, event type name, and the full payload.
///
/// Query params:
/// - `from`  — return events with position ≥ this value (default: 0 = all)
/// - `limit` — max events per page (default: 100, max: 500)
///
/// Returns: `{ events: [...], total: N, has_more: bool }`
#[derive(Deserialize)]
struct EventLogQuery {
    #[serde(default)]
    from: u64,
    #[serde(default = "default_event_log_limit")]
    limit: usize,
}

fn default_event_log_limit() -> usize {
    100
}

async fn admin_event_log_handler(
    State(state): State<AppState>,
    Query(q): Query<EventLogQuery>,
) -> impl IntoResponse {
    let limit = q.limit.min(500);
    let after = if q.from > 0 {
        Some(EventPosition(q.from - 1))
    } else {
        None
    };

    let events = match state.runtime.store.read_stream(after, limit + 1).await {
        Ok(v) => v,
        Err(e) => return Err(internal_error(e.to_string())),
    };

    let has_more = events.len() > limit;
    let page: Vec<serde_json::Value> = events
        .into_iter()
        .take(limit)
        .map(|e| {
            serde_json::json!({
                "position":   e.position.0,
                "stored_at":  e.stored_at,
                "event_type": event_type_name(&e.envelope.payload),
                "event_id":   e.envelope.event_id.as_str(),
                "payload":    e.envelope.payload,
            })
        })
        .collect();

    let total = page.len();
    Ok(Json(serde_json::json!({
        "events":   page,
        "total":    total,
        "has_more": has_more,
    })))
}

// ── Ollama handler ────────────────────────────────────────────────────────────

/// `GET /v1/providers/ollama/models` — list models available in the local Ollama registry.
///
/// Returns `200` with a JSON array of model names when Ollama is configured and
/// reachable, `503` when Ollama is not wired (OLLAMA_HOST unset), and `502`
/// when the daemon cannot be reached at call time.
async fn ollama_models_handler(State(state): State<AppState>) -> impl IntoResponse {
    if let Some(provider) = &state.ollama {
        match provider.list_models().await {
            Ok(models) => {
                let names: Vec<&str> = models
                    .iter()
                    .map(|m: &OllamaModel| m.name.as_str())
                    .collect();
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({
                        "host":   provider.host(),
                        "models": names,
                        "count":  names.len(),
                    })),
                )
                    .into_response()
            }
            Err(e) => (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({
                    "error": format!("Ollama unreachable: {e}")
                })),
            )
                .into_response(),
        }
    } else {
        // No Ollama configured — return 503 so callers know to use provider connections instead.
        (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({
                "error": "Ollama not configured — set OLLAMA_HOST to enable local model management"
            })),
        )
            .into_response()
    }
}

// ── Provider connection discovery ─────────────────────────────────────────────

/// Unified model record returned by `discover-models`.
#[derive(serde::Serialize, Clone, Debug)]
struct DiscoveredModel {
    model_id: String,
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parameter_size: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quantization: Option<String>,
    /// Inferred capabilities: "generate", "embed", "rerank".
    capabilities: Vec<String>,
    /// Maximum context window in tokens, if known (from /v1/models or /api/show).
    #[serde(skip_serializing_if = "Option::is_none")]
    context_window_tokens: Option<u32>,
}

/// Query params shared by `discover-models` and `test`.
#[derive(serde::Deserialize, Default)]
struct DiscoverModelsQuery {
    /// Override endpoint URL (for ad-hoc discovery before connection is registered).
    endpoint_url: Option<String>,
    /// API key to use with `endpoint_url`.
    api_key: Option<String>,
    /// Override adapter type: "ollama" | "openai_compat" (inferred from connection if absent).
    adapter_type: Option<String>,
}

fn decrypt_provider_credential(
    record: &cairn_domain::credentials::CredentialRecord,
) -> Result<String, String> {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Key, Nonce};
    use sha2::{Digest, Sha256};

    let seed = record.key_id.as_deref().unwrap_or("cairn-local-test-key");
    let digest = Sha256::digest(seed.as_bytes());
    let mut key_material = [0u8; 32];
    key_material.copy_from_slice(&digest[..32]);

    let key = Key::<Aes256Gcm>::from_slice(&key_material);
    let cipher = Aes256Gcm::new(key);

    let encrypted_at_ms = record
        .encrypted_at_ms
        .ok_or_else(|| "credential missing encrypted_at_ms".to_owned())?;
    let nonce_digest = Sha256::digest(
        format!(
            "{}:{}:{encrypted_at_ms}",
            record.tenant_id.as_str(),
            record.provider_id
        )
        .as_bytes(),
    );
    let mut nonce_bytes = [0u8; 12];
    nonce_bytes.copy_from_slice(&nonce_digest[..12]);

    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher
        .decrypt(nonce, record.encrypted_value.as_ref())
        .map_err(|e| format!("credential decryption failed: {e}"))?;
    String::from_utf8(plaintext).map_err(|e| format!("credential plaintext invalid utf-8: {e}"))
}

async fn resolve_connection_probe_material(
    state: &AppState,
    connection_id: &str,
) -> (Option<String>, Option<String>) {
    let system_project = cairn_domain::ProjectKey::new("system", "system", "system");

    let endpoint_key = format!("provider_endpoint_{connection_id}");
    let endpoint_url = match state
        .runtime
        .defaults
        .resolve(&system_project, &endpoint_key)
        .await
    {
        Ok(Some(setting)) => setting
            .as_str()
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        _ => None,
    };

    let credential_key = format!("provider_credential_{connection_id}");
    let credential_id = match state
        .runtime
        .defaults
        .resolve(&system_project, &credential_key)
        .await
    {
        Ok(Some(setting)) => setting.as_str().map(str::to_owned),
        _ => None,
    };

    let api_key = match credential_id {
        Some(credential_id) => match state
            .runtime
            .credentials
            .get(&cairn_domain::CredentialId::new(credential_id))
            .await
        {
            Ok(Some(record)) if record.active => decrypt_provider_credential(&record).ok(),
            _ => None,
        },
        None => None,
    };

    // Resolve $ENV_VAR references — if the stored key starts with '$', treat it as an env var name.
    let api_key = api_key.map(|key| {
        if let Some(var_name) = key.strip_prefix('$') {
            std::env::var(var_name).unwrap_or(key)
        } else {
            key
        }
    });

    (endpoint_url, api_key)
}

/// `GET /v1/providers/connections/:id/discover-models`
///
/// Queries the live provider endpoint for available models.
///
/// - `ollama`        → `GET {host}/api/tags`
/// - `openai_compat` → `GET {base_url}/models`
///
/// Use `?endpoint_url=` for ad-hoc queries before registering the connection.
async fn discover_models_handler(
    State(state): State<AppState>,
    Path(connection_id): Path<String>,
    Query(query): Query<DiscoverModelsQuery>,
) -> impl IntoResponse {
    use cairn_domain::ProviderConnectionId;
    use cairn_store::projections::ProviderConnectionReadModel;

    let conn_id = ProviderConnectionId::new(connection_id.clone());
    let adapter_type =
        match ProviderConnectionReadModel::get(state.runtime.store.as_ref(), &conn_id).await {
            Ok(Some(rec)) => rec.adapter_type.to_lowercase(),
            Ok(None) => {
                if query.endpoint_url.is_none() {
                    return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({
                    "error": format!("provider connection '{connection_id}' not found"),
                    "hint": "pass ?endpoint_url=... to discover without a registered connection",
                }))).into_response();
                }
                query
                    .adapter_type
                    .clone()
                    .unwrap_or_else(|| "openai_compat".to_owned())
            }
            Err(e) => return internal_error(format!("store error: {e}")).into_response(),
        };
    // Allow query param to override stored adapter_type.
    let adapter_type = query
        .adapter_type
        .as_deref()
        .unwrap_or(&adapter_type)
        .to_lowercase();

    let (stored_endpoint, stored_api_key) =
        if query.endpoint_url.is_none() && query.api_key.is_none() {
            resolve_connection_probe_material(&state, &connection_id).await
        } else {
            (None, None)
        };

    if adapter_type == "ollama" {
        discover_ollama_models_live(
            &state,
            query.endpoint_url.as_deref().or(stored_endpoint.as_deref()),
        )
        .await
    } else {
        discover_openai_compat_models_live(
            &state,
            query.endpoint_url.as_deref().or(stored_endpoint.as_deref()),
            query.api_key.as_deref().or(stored_api_key.as_deref()),
        )
        .await
    }
}

async fn discover_ollama_models_live(
    state: &AppState,
    endpoint_override: Option<&str>,
) -> axum::response::Response {
    let host =
        match endpoint_override {
            Some(url) => url.trim_end_matches('/').to_owned(),
            None => match &state.ollama {
                Some(p) => p.host().to_owned(),
                None => return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": "Ollama not configured — set OLLAMA_HOST or pass ?endpoint_url="
                    })),
                )
                    .into_response(),
            },
        };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    match client.get(format!("{host}/api/tags")).send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(body) => {
                    let names: Vec<String> = body
                        .get("models")
                        .and_then(|m| m.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|m| m.get("name")?.as_str().map(str::to_owned))
                                .collect()
                        })
                        .unwrap_or_default();

                    // Call /api/show for each model to get num_ctx.
                    // Best-effort: silently ignore failures for individual models.
                    let mut models: Vec<DiscoveredModel> = Vec::with_capacity(names.len());
                    for name in &names {
                        let ctx = fetch_ollama_context_window(&client, &host, name).await;
                        models.push(ollama_name_to_discovered_with_ctx(name, ctx));
                    }

                    (
                        StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "provider": "ollama",
                            "endpoint": host,
                            "models":   models,
                        })),
                    )
                        .into_response()
                }
                Err(e) => internal_error(format!("parse error: {e}")).into_response(),
            }
        }
        Ok(resp) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Ollama returned HTTP {}", resp.status()),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Ollama unreachable: {e}"),
            })),
        )
            .into_response(),
    }
}

/// Call Ollama's `POST /api/show` for a single model and extract `num_ctx`.
///
/// Returns `None` on any error (network, parse, field absent) so discovery
/// can fall back to `known_context_window`.
async fn fetch_ollama_context_window(
    client: &reqwest::Client,
    host: &str,
    model_name: &str,
) -> Option<u32> {
    let resp = client
        .post(format!("{host}/api/show"))
        .json(&serde_json::json!({ "name": model_name }))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    // Ollama /api/show nests context length as model_info.*.context_length
    // or directly as model_info.llama.context_length.
    // Try a few paths before giving up.
    if let Some(n) = body
        .pointer("/model_info/llama.context_length")
        .or_else(|| body.pointer("/model_info/context_length"))
    {
        return n.as_u64().map(|v| v as u32);
    }
    // Older Ollama versions expose it under /parameters/num_ctx.
    if let Some(n) = body.pointer("/parameters/num_ctx") {
        return n.as_u64().map(|v| v as u32);
    }
    // Some versions put it under /details/parameter_size or model_info flatly.
    body.get("model_info")
        .and_then(|mi| mi.as_object())
        .and_then(|obj| obj.values().find_map(|v| v.as_u64().map(|n| n as u32)))
        .filter(|&n| n >= 512) // sanity: must be a plausible context size
}

async fn discover_openai_compat_models_live(
    state: &AppState,
    endpoint_override: Option<&str>,
    api_key_override: Option<&str>,
) -> axum::response::Response {
    let (base_url, api_key) = match endpoint_override {
        Some(url) => (url.trim_end_matches('/').to_owned(), api_key_override.map(str::to_owned).unwrap_or_default()),
        None => match &state.openai_compat {
            Some(p) => (p.base_url.as_str().trim_end_matches('/').to_owned(), std::env::var("OPENAI_COMPAT_API_KEY").unwrap_or_default()),
            None => return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                "error": "OpenAI-compat not configured — set OPENAI_COMPAT_BASE_URL or pass ?endpoint_url="
            }))).into_response(),
        },
    };
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let mut req = client.get(format!("{base_url}/models"));
    if !api_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {api_key}"));
    }
    match req.send().await {
        Ok(resp) if resp.status().is_success() => {
            match resp.json::<serde_json::Value>().await {
                Ok(body) => {
                    // Each item in `data` is a full model object — pass the
                    // whole object so we can extract context_length / max_model_len.
                    let models: Vec<DiscoveredModel> = body
                        .get("data")
                        .and_then(|d| d.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(openai_model_obj_to_discovered)
                                .collect()
                        })
                        .unwrap_or_default();
                    (
                        StatusCode::OK,
                        axum::Json(serde_json::json!({
                            "provider": "openai_compat",
                            "endpoint": base_url,
                            "models":   models,
                        })),
                    )
                        .into_response()
                }
                Err(e) => internal_error(format!("parse error: {e}")).into_response(),
            }
        }
        Ok(resp) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Provider returned HTTP {}", resp.status()),
            })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({
                "error": format!("Provider unreachable: {e}"),
            })),
        )
            .into_response(),
    }
}

fn ollama_name_to_discovered_with_ctx(name: &str, ctx_window: Option<u32>) -> DiscoveredModel {
    let (base, tag) = name.split_once(':').unwrap_or((name, ""));
    let mut parts = tag.split('-');
    let param_size = parts
        .next()
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    let quantization = parts
        .filter(|s| s.to_lowercase().starts_with('q') || s.contains('_'))
        .max_by_key(|s| s.len())
        .map(str::to_owned);
    let lower = base.to_lowercase();
    let capabilities =
        if lower.contains("embed") || lower.contains("nomic") || lower.contains("all-minilm") {
            vec!["embed".to_owned()]
        } else if lower.contains("rerank") {
            vec!["rerank".to_owned()]
        } else {
            vec!["generate".to_owned()]
        };
    // Fall back to well-known defaults when the provider didn't report context length.
    let ctx = ctx_window.or_else(|| known_context_window(name));
    DiscoveredModel {
        model_id: name.to_owned(),
        name: name.to_owned(),
        parameter_size: param_size,
        quantization,
        capabilities,
        context_window_tokens: ctx,
    }
}

/// Convert a full OpenAI-compat model JSON object (from /v1/models `data` array)
/// into a `DiscoveredModel`, extracting the context window if present.
fn openai_model_obj_to_discovered(obj: &serde_json::Value) -> Option<DiscoveredModel> {
    let id = obj.get("id")?.as_str()?;
    let lower = id.to_lowercase();
    let capabilities = if lower.contains("embed") || lower.contains("embedding") {
        vec!["embed".to_owned()]
    } else if lower.contains("rerank") {
        vec!["rerank".to_owned()]
    } else {
        vec!["generate".to_owned()]
    };
    // OpenAI-compat providers use various field names for context window.
    let ctx = obj
        .get("context_length")
        .or_else(|| obj.get("max_model_len"))
        .or_else(|| obj.get("context_window"))
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .or_else(|| known_context_window(id));
    Some(DiscoveredModel {
        model_id: id.to_owned(),
        name: id.to_owned(),
        parameter_size: None,
        quantization: None,
        capabilities,
        context_window_tokens: ctx,
    })
}

/// Return the known context window size for well-known model families.
///
/// Used as a fallback when the provider doesn't report context_length.
fn known_context_window(model_id: &str) -> Option<u32> {
    // Use the provider registry for known models.
    let ctx = cairn_domain::provider_registry::context_window_for(model_id);
    // 128_000 is the registry's default — treat as "unknown" so callers
    // can fall back to their own logic.
    if ctx != 128_000 {
        return Some(ctx as u32);
    }
    // Fallback: substring matching for models not in the static registry
    // (e.g. Ollama local models, niche open-source variants).
    let lower = model_id.to_lowercase();
    if lower.contains("qwen3-coder") {
        Some(262_144)
    } else if lower.contains("qwen") {
        Some(32_768)
    } else if lower.contains("llama-2") || lower.contains("llama2") {
        Some(4_096)
    } else if lower.contains("phi-2") || lower.contains("phi2") {
        Some(2_048)
    } else if lower.contains("nomic-embed") || lower.contains("all-minilm") {
        Some(8_192)
    } else {
        None
    }
}

/// Estimate input token count from text length (rough: 1 token ≈ 4 chars).
fn estimate_tokens(text: &str) -> u32 {
    ((text.len() as f64) / 4.0).ceil() as u32
}

/// Compute a safe `max_output_tokens` given the context window and input length.
///
/// Strategy:
/// - Reserve `input_estimate + safety_margin` tokens for input + overhead.
/// - Cap output at `context_window / 4` so one response can't consume the
///   entire window (leaves room for multi-turn history).
/// - Minimum output is always at least 256 tokens so short models don't
///   truncate prematurely.
///
/// Returns `None` if context_window is unknown (caller should use its own
/// default).
fn compute_max_output_tokens(context_window: u32, input_estimate: u32) -> u32 {
    const SAFETY_MARGIN: u32 = 512; // reserved for system prompt overhead
    let available = context_window.saturating_sub(input_estimate + SAFETY_MARGIN);
    let quarter_ctx = context_window / 4;
    available.min(quarter_ctx).max(256)
}

/// `GET /v1/providers/connections/:id/test`
///
/// Probes the provider endpoint and returns reachability + round-trip latency.
///
/// Response: `{ "ok": bool, "latency_ms": u64, "provider": str, "status": u16, "detail": str }`
async fn test_connection_handler(
    State(state): State<AppState>,
    Path(connection_id): Path<String>,
    Query(query): Query<DiscoverModelsQuery>,
) -> impl IntoResponse {
    use cairn_domain::ProviderConnectionId;
    use cairn_store::projections::ProviderConnectionReadModel;

    let conn_id = ProviderConnectionId::new(connection_id.clone());
    let adapter_type =
        match ProviderConnectionReadModel::get(state.runtime.store.as_ref(), &conn_id).await {
            Ok(Some(rec)) => rec.adapter_type.to_lowercase(),
            Ok(None) => {
                if query.endpoint_url.is_none() {
                    return (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({
                    "error": format!("provider connection '{connection_id}' not found"),
                    "hint": "pass ?endpoint_url=... to test without a registered connection",
                }))).into_response();
                }
                query
                    .adapter_type
                    .clone()
                    .unwrap_or_else(|| "openai_compat".to_owned())
            }
            Err(e) => return internal_error(format!("store error: {e}")).into_response(),
        };
    let adapter_type = query
        .adapter_type
        .as_deref()
        .unwrap_or(&adapter_type)
        .to_lowercase();

    let (stored_endpoint, stored_api_key) =
        if query.endpoint_url.is_none() && query.api_key.is_none() {
            resolve_connection_probe_material(&state, &connection_id).await
        } else {
            (None, None)
        };

    let (probe_url, auth_header) = if adapter_type == "ollama" {
        let host = query
            .endpoint_url
            .as_deref()
            .map(|u| u.trim_end_matches('/').to_owned())
            .or_else(|| stored_endpoint.clone())
            .or_else(|| state.ollama.as_ref().map(|p| p.host().to_owned()));
        match host {
            Some(h) => (format!("{h}/api/tags"), None),
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({ "error": "Ollama not configured" })),
                )
                    .into_response();
            }
        }
    } else if adapter_type.contains("bedrock") {
        // Bedrock: probe the runtime endpoint with a simple GET to check reachability.
        let region = std::env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_owned());
        let key = query
            .api_key
            .clone()
            .or_else(|| stored_api_key.clone())
            .or_else(|| std::env::var("BEDROCK_API_KEY").ok())
            .unwrap_or_default();
        let base = query
            .endpoint_url
            .as_deref()
            .map(|u| u.trim_end_matches('/').to_owned())
            .or_else(|| stored_endpoint.clone())
            .unwrap_or_else(|| format!("https://bedrock-runtime.{region}.amazonaws.com"));
        let auth = if key.is_empty() {
            None
        } else {
            Some(format!("Bearer {key}"))
        };
        // Probe the base URL — a 403 or 404 still means reachable.
        (base, auth)
    } else {
        let base = query
            .endpoint_url
            .as_deref()
            .map(|u| u.trim_end_matches('/').to_owned())
            .or_else(|| stored_endpoint.clone())
            .or_else(|| state.openai_compat.as_ref().map(|p| p.base_url.to_string()));
        match base {
            Some(b) => {
                let key = query
                    .api_key
                    .clone()
                    .or_else(|| stored_api_key.clone())
                    .or_else(|| std::env::var("OPENAI_COMPAT_API_KEY").ok())
                    .unwrap_or_default();
                let auth = if key.is_empty() {
                    None
                } else {
                    Some(format!("Bearer {key}"))
                };
                (format!("{b}/models"), auth)
            }
            None => {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({ "error": "Provider not configured" })),
                )
                    .into_response();
            }
        }
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .unwrap_or_default();
    let start = std::time::Instant::now();
    let mut req = client.get(&probe_url);
    if let Some(auth) = auth_header {
        req = req.header("Authorization", auth);
    }
    match req.send().await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            let status = resp.status().as_u16();
            // For Bedrock: any HTTP response (even 403) means endpoint is reachable.
            let ok = if adapter_type.contains("bedrock") {
                status != 0
            } else {
                resp.status().is_success()
            };
            let detail = if ok && resp.status().is_success() {
                "reachable"
            } else if ok {
                "reachable (auth required)"
            } else {
                "returned non-2xx"
            };
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "ok":         ok,
                    "latency_ms": latency_ms,
                    "provider":   adapter_type,
                    "status":     status,
                    "detail":     detail,
                })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::OK,
            axum::Json(serde_json::json!({
                "ok":         false,
                "latency_ms": start.elapsed().as_millis() as u64,
                "provider":   adapter_type,
                "status":     0u16,
                "detail":     format!("connection error: {e}"),
            })),
        )
            .into_response(),
    }
}

/// `POST /v1/providers/ollama/generate` — run a prompt through the local Ollama LLM.
///
/// Body: `{ "model": "llama3", "prompt": "Hello, world!" }`
/// Response: `{ "text", "model", "tokens_in", "tokens_out", "latency_ms" }`
///
/// Returns 503 when OLLAMA_HOST is not configured, 502 when the daemon is
/// unreachable, 500 on model errors.
#[derive(serde::Deserialize)]
struct OllamaGenerateRequest {
    model: Option<String>,
    /// Single-turn prompt (used when `messages` is absent).
    #[serde(default)]
    prompt: String,
    /// Multi-turn conversation history. When present, `prompt` is ignored.
    /// Each element must be `{"role": "user"|"assistant"|"system", "content": "..."}`.
    #[serde(default)]
    messages: Option<Vec<serde_json::Value>>,
    /// Explicit max output tokens override.  When set, bypasses dynamic budgeting.
    #[serde(default)]
    max_tokens: Option<u32>,
}

async fn ollama_generate_handler(
    State(state): State<AppState>,
    Json(body): Json<OllamaGenerateRequest>,
) -> impl IntoResponse {
    if let Err(msg) = validate::check_all(&[
        validate::valid_id("model", &body.model),
        validate::max_len_str("prompt", &body.prompt, validate::MAX_PROMPT_LEN),
    ]) {
        return bad_request(msg).into_response();
    }
    if body.prompt.is_empty() && body.messages.as_ref().is_none_or(|m| m.is_empty()) {
        return bad_request("prompt or messages is required").into_response();
    }

    let default_model = state.runtime.runtime_config.default_generate_model().await;
    let model_id = body.model.as_deref().unwrap_or(&default_model).to_owned();

    let provider: Arc<dyn cairn_domain::providers::GenerationProvider> = match state
        .runtime
        .provider_registry
        .resolve_generation_for_model(
            &cairn_domain::TenantId::new("default_tenant"),
            &model_id,
            cairn_runtime::ProviderResolutionPurpose::Generate,
        )
        .await
    {
        Ok(Some(provider)) => provider,
        Ok(None) => {
            let is_bedrock_model = model_id.contains('.');
            let is_brain_model = model_id.to_lowercase() == "openrouter/free"
                || model_id.to_lowercase().contains("gemma-3-27b")
                || model_id.to_lowercase().contains("qwen3-coder")
                || model_id.to_lowercase().contains("gemma-4")
                || model_id.to_lowercase().contains("gemma4")
                || model_id.to_lowercase().contains("cyankiwi")
                || model_id.to_lowercase().contains("brain");

            if is_bedrock_model {
                if let Some(ref bedrock) = state.bedrock {
                    bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else {
                    return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                            "error": "Bedrock provider not configured — set BEDROCK_API_KEY and BEDROCK_MODEL_ID"
                        }))).into_response();
                }
            } else if let Some(ref ollama) = state.ollama {
                ollama.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if is_brain_model {
                if let Some(ref brain) = state.openai_compat_brain {
                    brain.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else if let Some(ref worker) = state.openai_compat_worker {
                    worker.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else if let Some(ref or_) = state.openai_compat_openrouter {
                    or_.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else if let Some(ref bedrock) = state.bedrock {
                    bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
                } else {
                    return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                            "error": "Brain provider not configured — set CAIRN_BRAIN_URL, OPENROUTER_API_KEY, or BEDROCK_API_KEY"
                        }))).into_response();
                }
            } else if let Some(ref worker) = state.openai_compat_worker {
                worker.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if let Some(ref brain) = state.openai_compat_brain {
                brain.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if let Some(ref or_) = state.openai_compat_openrouter {
                or_.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else if let Some(ref bedrock) = state.bedrock {
                bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>
            } else {
                return (StatusCode::SERVICE_UNAVAILABLE, axum::Json(serde_json::json!({
                        "error": "No LLM provider configured — set OLLAMA_HOST, CAIRN_BRAIN_URL, CAIRN_WORKER_URL, OPENROUTER_API_KEY, or BEDROCK_API_KEY"
                    }))).into_response();
            }
        }
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    let messages = vec![serde_json::json!({
        "role":    "user",
        "content": body.prompt,
    })];

    // ── Dynamic token budgeting ───────────────────────────────────────────────
    // Estimate how many input tokens the prompt uses, then compute a safe
    // max_output_tokens that fits within the model's context window.
    //
    // Fallback chain:
    //   1. body.max_tokens (explicit caller override) — if set, honour it
    //   2. known_context_window(model_id)             — model-family defaults
    //   3. Hardcoded 8K conservative default           — unknown models
    let input_tokens = estimate_tokens(&body.prompt)
        + body
            .messages
            .as_ref()
            .map(|m| {
                m.iter()
                    .map(|msg| {
                        estimate_tokens(msg.get("content").and_then(|v| v.as_str()).unwrap_or(""))
                    })
                    .sum::<u32>()
            })
            .unwrap_or(0);

    let max_output_tokens: u32 = if let Some(explicit) = body.max_tokens {
        explicit
    } else {
        let ctx_window = known_context_window(&model_id).unwrap_or(8_192);
        compute_max_output_tokens(ctx_window, input_tokens)
    };

    let settings = cairn_domain::providers::ProviderBindingSettings {
        max_output_tokens: Some(max_output_tokens),
        ..Default::default()
    };
    let start = std::time::Instant::now();

    match provider.generate(&model_id, messages, &settings).await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "text":       resp.text,
                    "model":      resp.model_id,
                    "tokens_in":  resp.input_tokens,
                    "tokens_out": resp.output_tokens,
                    "latency_ms": latency_ms,
                })),
            )
                .into_response()
        }
        Err(e) => {
            let (status, msg) = match &e {
                cairn_domain::providers::ProviderAdapterError::TimedOut => {
                    (StatusCode::GATEWAY_TIMEOUT, e.to_string())
                }
                cairn_domain::providers::ProviderAdapterError::RateLimited => {
                    (StatusCode::TOO_MANY_REQUESTS, e.to_string())
                }
                cairn_domain::providers::ProviderAdapterError::TransportFailure(_) => {
                    (StatusCode::BAD_GATEWAY, e.to_string())
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
            (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
        }
    }
}

// ── Ollama embed handler ──────────────────────────────────────────────────────

/// `POST /v1/memory/embed` — embed a batch of texts using the local Ollama daemon.
///
/// Body: `{ "texts": ["text a", "text b"], "model": "nomic-embed-text" }`
///
/// Returns `{ "embeddings": [[...], [...]], "model": "...", "token_count": N }`.
///
/// Returns 503 when OLLAMA_HOST is not configured, 400 when `texts` is empty,
/// 502 when the daemon is unreachable.
#[derive(serde::Deserialize)]
struct OllamaEmbedRequest {
    texts: Vec<String>,
    model: Option<String>,
}

async fn ollama_embed_handler(
    State(state): State<AppState>,
    Json(body): Json<OllamaEmbedRequest>,
) -> impl IntoResponse {
    if body.texts.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": "texts must not be empty" })),
        )
            .into_response();
    }

    let default_ollama_embed = state
        .runtime
        .runtime_config
        .default_ollama_embed_model()
        .await;
    let default_compat_embed = state.runtime.runtime_config.default_embed_model().await;
    let model_id = body
        .model
        .as_deref()
        .unwrap_or_else(|| {
            if state.ollama.is_some() {
                &default_ollama_embed
            } else {
                &default_compat_embed
            }
        })
        .to_owned();

    let embedder: Arc<dyn cairn_domain::providers::EmbeddingProvider> = match state
        .runtime
        .provider_registry
        .resolve_embedding_for_model(&cairn_domain::TenantId::new("default_tenant"), &model_id)
        .await
    {
        Ok(Some(embedder)) => embedder,
        Ok(None) => {
            let model_id_lower = model_id.to_ascii_lowercase();
            let is_brain_model = model_id_lower == "openrouter/free"
                || model_id_lower.contains("gemma-3-27b")
                || model_id_lower.contains("qwen3-coder")
                || model_id_lower.contains("gemma-4")
                || model_id_lower.contains("gemma4")
                || model_id_lower.contains("cyankiwi")
                || model_id_lower.contains("brain");

            if let Some(ref ollama) = state.ollama {
                Arc::new(OllamaEmbeddingProvider::new(ollama.host()))
            } else if is_brain_model {
                if let Some(ref brain) = state.openai_compat_brain {
                    brain.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
                } else if let Some(ref worker) = state.openai_compat_worker {
                    worker.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
                } else if let Some(ref or_) = state.openai_compat_openrouter {
                    or_.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
                } else {
                    return (
                        StatusCode::SERVICE_UNAVAILABLE,
                        axum::Json(serde_json::json!({
                            "error": "No embedding provider configured — set OLLAMA_HOST, CAIRN_WORKER_URL, CAIRN_BRAIN_URL, or OPENROUTER_API_KEY"
                        })),
                    )
                        .into_response();
                }
            } else if let Some(ref worker) = state.openai_compat_worker {
                worker.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
            } else if let Some(ref brain) = state.openai_compat_brain {
                brain.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
            } else if let Some(ref or_) = state.openai_compat_openrouter {
                or_.clone() as Arc<dyn cairn_domain::providers::EmbeddingProvider>
            } else {
                return (
                    StatusCode::SERVICE_UNAVAILABLE,
                    axum::Json(serde_json::json!({
                        "error": "No embedding provider configured — set OLLAMA_HOST, CAIRN_WORKER_URL, CAIRN_BRAIN_URL, or OPENROUTER_API_KEY"
                    })),
                )
                    .into_response();
            }
        }
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    let start = std::time::Instant::now();
    match embedder.embed(&model_id, body.texts).await {
        Ok(resp) => {
            let latency_ms = start.elapsed().as_millis() as u64;
            (
                StatusCode::OK,
                axum::Json(serde_json::json!({
                    "embeddings":   resp.embeddings,
                    "model":        resp.model_id,
                    "token_count":  resp.token_count,
                    "latency_ms":   latency_ms,
                })),
            )
                .into_response()
        }
        Err(e) => {
            use cairn_domain::providers::ProviderAdapterError;
            let (status, msg) = match &e {
                ProviderAdapterError::TimedOut => (StatusCode::GATEWAY_TIMEOUT, e.to_string()),
                ProviderAdapterError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, e.to_string()),
                ProviderAdapterError::TransportFailure(_) => {
                    (StatusCode::BAD_GATEWAY, e.to_string())
                }
                _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
            };
            (status, axum::Json(serde_json::json!({ "error": msg }))).into_response()
        }
    }
}

/// `POST /v1/chat/stream` — stream tokens from any configured LLM provider via SSE.
///
/// Routes to the first available provider: Bedrock → Ollama → OpenAI-compat brain → worker → OpenRouter.
/// Body: `{ "model": "qwen3:8b", "prompt": "...", "messages": [...] }`
/// Emits SSE events:
///   - `event: token`  data: `{"text": "word "}`
///   - `event: done`   data: `{"latency_ms": N, "model": "..."}`
///   - `event: error`  data: `{"error": "..."}`
///
/// Clients read via `fetch()` + `ReadableStream` — no EventSource needed.
fn stream_generation_provider_as_sse(
    provider: Arc<dyn cairn_domain::providers::GenerationProvider>,
    model_id: String,
    messages: Vec<serde_json::Value>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(8);

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let settings = cairn_domain::providers::ProviderBindingSettings::default();
        match provider.generate(&model_id, messages, &settings).await {
            Ok(resp) => {
                let _ = tx
                    .send(Ok(Event::default()
                        .event("token")
                        .data(serde_json::json!({"text": resp.text}).to_string())))
                    .await;
                let _ = tx
                    .send(Ok(Event::default().event("done").data(
                        serde_json::json!({
                            "latency_ms": start.elapsed().as_millis() as u64,
                            "model": resp.model_id,
                            "tokens_in": resp.input_tokens,
                            "tokens_out": resp.output_tokens,
                        })
                        .to_string(),
                    )))
                    .await;
            }
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::json!({"error": err.to_string()}).to_string(),
                    )))
                    .await;
            }
        }
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

fn stream_chat_provider_as_sse(
    provider: Arc<dyn cairn_providers::chat::ChatProvider>,
    model_id: String,
    messages: Vec<cairn_providers::chat::ChatMessage>,
) -> Response {
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let mut stream = match provider.chat_stream(&messages, None).await {
            Ok(stream) => stream,
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::json!({"error": err.to_string()}).to_string(),
                    )))
                    .await;
                return;
            }
        };

        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            match chunk {
                Ok(text) if !text.is_empty() => {
                    let _ = tx
                        .send(Ok(Event::default()
                            .event("token")
                            .data(serde_json::json!({"text": text}).to_string())))
                        .await;
                }
                Ok(_) => {}
                Err(err) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(
                            serde_json::json!({"error": err.to_string()}).to_string(),
                        )))
                        .await;
                    return;
                }
            }
        }

        let _ = tx
            .send(Ok(Event::default().event("done").data(
                serde_json::json!({
                    "latency_ms": start.elapsed().as_millis() as u64,
                    "model": model_id,
                })
                .to_string(),
            )))
            .await;
    });

    Sse::new(tokio_stream::wrappers::ReceiverStream::new(rx))
        .keep_alive(KeepAlive::default())
        .into_response()
}

async fn chat_stream_handler(
    State(state): State<AppState>,
    Json(body): Json<OllamaGenerateRequest>,
) -> impl IntoResponse {
    let default_stream = state.runtime.runtime_config.default_stream_model().await;
    let model_id = body.model.as_deref().unwrap_or(&default_stream).to_owned();
    let tenant_id = cairn_domain::TenantId::new("default_tenant");
    let messages: Vec<serde_json::Value> = body
        .messages
        .unwrap_or_else(|| vec![serde_json::json!({"role": "user", "content": body.prompt})]);
    let chat_messages = cairn_runtime::json_messages_to_chat_messages(&messages);
    let has_active_connections = match state
        .runtime
        .provider_registry
        .has_active_connections(&tenant_id)
        .await
    {
        Ok(has_connections) => has_connections,
        Err(err) => {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                axum::Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response();
        }
    };

    // Bedrock models contain a '.' (e.g. minimax.minimax-m2.5). Connection-backed
    // Bedrock routes resolve through the registry first; otherwise we keep the
    // existing single-shot static fallback and wrap it as SSE.
    let is_bedrock_model = model_id.contains('.');
    if is_bedrock_model && has_active_connections {
        match state
            .runtime
            .provider_registry
            .resolve_generation_for_model(
                &tenant_id,
                &model_id,
                cairn_runtime::ProviderResolutionPurpose::Stream,
            )
            .await
        {
            Ok(Some(provider)) => {
                return stream_generation_provider_as_sse(
                    provider,
                    model_id.clone(),
                    messages.clone(),
                );
            }
            Ok(None) => {}
            Err(err) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    axum::Json(serde_json::json!({ "error": err.to_string() })),
                )
                    .into_response();
            }
        }
    } else if has_active_connections {
        match state
            .runtime
            .provider_registry
            .resolve_chat_for_model(
                &tenant_id,
                &model_id,
                cairn_runtime::ProviderResolutionPurpose::Stream,
            )
            .await
        {
            Ok(Some(provider)) => {
                return stream_chat_provider_as_sse(
                    provider,
                    model_id.clone(),
                    chat_messages.clone(),
                );
            }
            Ok(None) => {}
            Err(err) => {
                return (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    axum::Json(serde_json::json!({ "error": err.to_string() })),
                )
                    .into_response();
            }
        }
    }

    if is_bedrock_model {
        if let Some(ref bedrock) = state.bedrock {
            return stream_generation_provider_as_sse(
                bedrock.clone() as Arc<dyn cairn_domain::providers::GenerationProvider>,
                model_id,
                messages,
            );
        }
        // Fall through to OpenAI-compat if no Bedrock provider.
    }

    // Static provider resolution: Ollama first, then OpenAI-compat brain, then worker.
    // All three expose /v1/chat/completions (OpenAI wire format) so the same
    // streaming logic applies — only URL construction differs.
    let (stream_url, stream_api_key): (String, String) = if let Some(ref o) = state.ollama {
        (format!("{}/v1/chat/completions", o.host()), String::new())
    } else if let Some(ref brain) = state.openai_compat_brain {
        (
            format!(
                "{}/chat/completions",
                brain.base_url.as_str().trim_end_matches('/')
            ),
            brain.api_key.as_str().to_owned(),
        )
    } else if let Some(ref worker) = state.openai_compat_worker {
        (
            format!(
                "{}/chat/completions",
                worker.base_url.as_str().trim_end_matches('/')
            ),
            worker.api_key.as_str().to_owned(),
        )
    } else if let Some(ref or_) = state.openai_compat_openrouter {
        (
            format!(
                "{}/chat/completions",
                or_.base_url.as_str().trim_end_matches('/')
            ),
            or_.api_key.as_str().to_owned(),
        )
    } else {
        return (
                StatusCode::SERVICE_UNAVAILABLE,
                axum::Json(serde_json::json!({
                    "error": "No LLM provider configured — set OLLAMA_HOST, CAIRN_BRAIN_URL, CAIRN_WORKER_URL, or OPENROUTER_API_KEY"
                })),
            ).into_response();
    };
    let disable_thinking = state
        .runtime
        .runtime_config
        .supports_thinking_mode(&model_id)
        .await;

    let (tx, rx) = tokio::sync::mpsc::channel::<Result<Event, Infallible>>(64);

    tokio::spawn(async move {
        let start = std::time::Instant::now();
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(180))
            .build()
            .expect("reqwest client");

        let mut req_body = serde_json::json!({
            "model":    model_id,
            "messages": messages,
            "stream":   true,
        });
        if disable_thinking {
            req_body["options"] = serde_json::json!({ "think": false });
        }

        let mut req = client.post(&stream_url).json(&req_body);
        if !stream_api_key.is_empty() {
            req = req.bearer_auth(&stream_api_key);
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(err) => {
                let _ = tx
                    .send(Ok(Event::default().event("error").data(
                        serde_json::json!({"error": err.to_string()}).to_string(),
                    )))
                    .await;
                return;
            }
        };

        if !resp.status().is_success() {
            let msg = resp.text().await.unwrap_or_default();
            let _ = tx
                .send(Ok(Event::default()
                    .event("error")
                    .data(serde_json::json!({"error": msg}).to_string())))
                .await;
            return;
        }

        let mut stream = resp.bytes_stream();
        let mut buf = String::new();

        while let Some(chunk) = tokio_stream::StreamExt::next(&mut stream).await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(err) => {
                    let _ = tx
                        .send(Ok(Event::default().event("error").data(
                            serde_json::json!({"error": err.to_string()}).to_string(),
                        )))
                        .await;
                    return;
                }
            };

            buf.push_str(&String::from_utf8_lossy(&bytes));

            while let Some(nl) = buf.find('\n') {
                let line = buf[..nl].trim().to_owned();
                buf = buf[nl + 1..].to_owned();

                let Some(data) = line.strip_prefix("data: ") else {
                    continue;
                };
                if data == "[DONE]" {
                    break;
                }

                let Ok(parsed) = serde_json::from_str::<serde_json::Value>(data) else {
                    continue;
                };

                if let Some(text) = parsed
                    .get("choices")
                    .and_then(|choices| choices.get(0))
                    .and_then(|choice| choice.get("delta"))
                    .and_then(|delta| delta.get("content"))
                    .and_then(|content| content.as_str())
                {
                    if !text.is_empty() {
                        let _ = tx
                            .send(Ok(Event::default()
                                .event("token")
                                .data(serde_json::json!({"text": text}).to_string())))
                            .await;
                    }
                }
            }
        }

        let _ = tx
            .send(Ok(Event::default().event("done").data(
                serde_json::json!({
                    "latency_ms": start.elapsed().as_millis() as u64,
                    "model": model_id,
                })
                .to_string(),
            )))
            .await;
    });

    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

// ── Ollama model management handlers ─────────────────────────────────────────

#[derive(serde::Deserialize)]
struct OllamaModelNameRequest {
    /// Model name, e.g. `"qwen3:8b"` or `"nomic-embed-text"`.
    model: String,
}

/// `POST /v1/providers/ollama/pull` — pull (download) a model into Ollama.
///
/// Body: `{ "model": "qwen3:8b" }`
///
/// Proxies to `POST OLLAMA_HOST/api/pull` with `stream: false`.
/// Returns `200 { "status": "success" }` on completion, `4xx`/`5xx` on error.
async fn ollama_pull_handler(
    State(state): State<AppState>,
    Json(body): Json<OllamaModelNameRequest>,
) -> impl IntoResponse {
    let Some(provider) = &state.ollama else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "Ollama not configured"})),
        )
            .into_response();
    };
    let url = format!("{}/api/pull", provider.host());
    let client = reqwest::Client::builder()
        // Pulling large models can take many minutes — long timeout.
        .timeout(std::time::Duration::from_secs(3600))
        .build()
        .unwrap_or_default();

    match client
        .post(&url)
        .json(&serde_json::json!({"name": body.model, "stream": false}))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            let body_text = resp.text().await.unwrap_or_default();
            if status.is_success() {
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({"status": "success", "model": body.model})),
                )
                    .into_response()
            } else {
                (
                    StatusCode::BAD_GATEWAY,
                    axum::Json(serde_json::json!({"error": body_text})),
                )
                    .into_response()
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// `POST /v1/providers/ollama/delete` — delete a model from the local Ollama registry.
///
/// Body: `{ "model": "qwen3:8b" }`
///
/// Proxies to `DELETE OLLAMA_HOST/api/delete`.
/// Returns `200` on success, `404` when the model is not found.
async fn ollama_delete_model_handler(
    State(state): State<AppState>,
    Json(body): Json<OllamaModelNameRequest>,
) -> impl IntoResponse {
    let Some(provider) = &state.ollama else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "Ollama not configured"})),
        )
            .into_response();
    };
    let url = format!("{}/api/delete", provider.host());
    let client = reqwest::Client::new();

    match client
        .delete(&url)
        .json(&serde_json::json!({"name": body.model}))
        .send()
        .await
    {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() || status == reqwest::StatusCode::OK {
                (
                    StatusCode::OK,
                    axum::Json(serde_json::json!({"status": "deleted", "model": body.model})),
                )
                    .into_response()
            } else {
                let msg = resp.text().await.unwrap_or_default();
                let code = if msg.contains("not found") {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::BAD_GATEWAY
                };
                (code, axum::Json(serde_json::json!({"error": msg}))).into_response()
            }
        }
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            axum::Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

// ── Ollama model info handler ─────────────────────────────────────────────────

/// `GET /v1/providers/ollama/models/:name/info` — detailed info for one model.
///
/// Calls `POST OLLAMA_HOST/api/show` + `GET OLLAMA_HOST/api/tags` and returns
/// the fields most useful for an operator dashboard:
///
/// ```json
/// {
///   "name": "qwen3:8b",
///   "family": "qwen3",
///   "format": "gguf",
///   "parameter_size": "8.2B",
///   "parameter_count": 8190735360,
///   "quantization_level": "Q4_K_M",
///   "context_length": 40960,
///   "size_bytes": 5234519167,
///   "size_human": "4.9 GB"
/// }
/// ```
async fn ollama_model_info_handler(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let Some(provider) = &state.ollama else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            axum::Json(serde_json::json!({"error": "Ollama not configured"})),
        )
            .into_response();
    };

    let host = provider.host().to_owned();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    // ── Call /api/show ────────────────────────────────────────────────────────
    let show_resp = match client
        .post(format!("{host}/api/show"))
        .json(&serde_json::json!({"name": name}))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            let msg = r.text().await.unwrap_or_default();
            return (
                StatusCode::NOT_FOUND,
                axum::Json(serde_json::json!({"error": msg})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                axum::Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    let show: serde_json::Value = match show_resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                axum::Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response();
        }
    };

    // ── Extract fields from `details` + `model_info` ─────────────────────────
    let details = &show["details"];
    let model_info = &show["model_info"];

    let family = details["family"].as_str().unwrap_or("unknown");
    let format = details["format"].as_str().unwrap_or("unknown");
    let parameter_size = details["parameter_size"].as_str().unwrap_or("unknown");
    let quantization_level = details["quantization_level"].as_str().unwrap_or("unknown");

    // Derive architecture key (e.g. "qwen3", "llama") for model_info lookups.
    let arch = family;
    let parameter_count = model_info
        .get("general.parameter_count")
        .and_then(|v| v.as_u64());
    let context_length = model_info
        .get(format!("{arch}.context_length"))
        .and_then(|v| v.as_u64())
        .or_else(|| {
            model_info
                .get("llama.context_length")
                .and_then(|v| v.as_u64())
        });
    let embedding_length = model_info
        .get(format!("{arch}.embedding_length"))
        .and_then(|v| v.as_u64());

    // ── Get disk size from /api/tags ──────────────────────────────────────────
    let (size_bytes, size_human) = match client.get(format!("{host}/api/tags")).send().await {
        Ok(r) if r.status().is_success() => {
            if let Ok(tags) = r.json::<serde_json::Value>().await {
                let size = tags["models"]
                    .as_array()
                    .and_then(|arr| arr.iter().find(|m| m["name"].as_str() == Some(&name)))
                    .and_then(|m| m["size"].as_u64())
                    .unwrap_or(0);
                let human = if size >= 1_073_741_824 {
                    format!("{:.1} GB", size as f64 / 1_073_741_824.0)
                } else if size >= 1_048_576 {
                    format!("{:.0} MB", size as f64 / 1_048_576.0)
                } else {
                    format!("{size} B")
                };
                (size, human)
            } else {
                (0, "unknown".to_owned())
            }
        }
        _ => (0, "unknown".to_owned()),
    };

    (StatusCode::OK, axum::Json(serde_json::json!({
        "name":               name,
        "family":             family,
        "format":             format,
        "parameter_size":     parameter_size,
        "parameter_count":    parameter_count,
        "quantization_level": quantization_level,
        "context_length":     context_length,
        "embedding_length":   embedding_length,
        "size_bytes":         if size_bytes > 0 { serde_json::Value::Number(size_bytes.into()) } else { serde_json::Value::Null },
        "size_human":         size_human,
    }))).into_response()
}

// ── System info handler ───────────────────────────────────────────────────────

/// `GET /v1/system/info` — comprehensive system information.
///
/// Returns compile-time build metadata, runtime capabilities, and
/// sanitised environment configuration (secrets are masked).
async fn system_info_handler(State(state): State<AppState>) -> impl axum::response::IntoResponse {
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
                    if val == "memory" {
                        config.storage = StorageBackend::InMemory;
                    } else if val.starts_with("postgres://") || val.starts_with("postgresql://") {
                        config.storage = StorageBackend::Postgres {
                            connection_url: val.clone(),
                        };
                    } else {
                        config.storage = StorageBackend::Sqlite { path: val.clone() };
                    }
                }
            }
            "--role" => {
                i += 1;
                if i < args.len() {
                    config.process_role =
                        cairn_api::bootstrap::ProcessRole::from_str_loose(&args[i]);
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

/// Resolve the storage backend from environment when no `--db` flag was given.
///
/// Priority: `DATABASE_URL` env var → InMemory fallback.
/// This runs after CLI parsing so `--db` always wins.
fn resolve_storage_from_env(config: &mut BootstrapConfig) {
    if !matches!(config.storage, StorageBackend::InMemory) {
        return; // --db flag was given, don't override
    }
    if let Ok(url) = std::env::var("DATABASE_URL") {
        let url = url.trim().to_owned();
        if !url.is_empty() {
            if url.starts_with("postgres://") || url.starts_with("postgresql://") {
                config.storage = StorageBackend::Postgres {
                    connection_url: url,
                };
            } else if url.starts_with("sqlite:") || url.ends_with(".db") {
                config.storage = StorageBackend::Sqlite { path: url };
            }
        }
    }
}

fn parse_args() -> BootstrapConfig {
    let args: Vec<String> = std::env::args().collect();
    let mut config = parse_args_from(&args);
    resolve_storage_from_env(&mut config);
    config
}

// ── Entry point ───────────────────────────────────────────────────────────────

// ── Graceful shutdown ─────────────────────────────────────────────────────────

/// Returns a future that resolves when SIGINT (Ctrl-C) or SIGTERM is received.
///
/// On non-Unix platforms only Ctrl-C is supported.
async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c    => { eprintln!("shutdown: SIGINT received");  },
        _ = terminate => { eprintln!("shutdown: SIGTERM received"); },
    }
}

/// Snapshot the in-memory event log and notification buffer to
/// `/tmp/cairn-shutdown-buffer.json` so they survive a server restart.
///
/// This is best-effort — failures are logged but do not block exit.
async fn flush_state_to_disk(state: &AppState) {
    const FLUSH_PATH: &str = "/tmp/cairn-shutdown-buffer.json";
    const MAX_EVENTS: usize = 5_000;

    // ── Events ────────────────────────────────────────────────────────────────
    let events = match state.runtime.store.read_stream(None, MAX_EVENTS).await {
        Ok(v) => v,
        Err(e) => {
            eprintln!("shutdown: could not read event buffer: {e}");
            vec![]
        }
    };
    let event_snapshots: Vec<serde_json::Value> = events
        .iter()
        .map(|e| {
            serde_json::json!({
                "position":   e.position.0,
                "stored_at":  e.stored_at,
                "event_type": event_type_name(&e.envelope.payload),
            })
        })
        .collect();

    // ── Notifications ─────────────────────────────────────────────────────────
    // Serialise while holding the lock, then release before writing to disk.
    let (notif_count, notif_json) = match state.notifications.read() {
        Ok(buf) => {
            let list = buf.list(200);
            let json: Vec<serde_json::Value> = list
                .iter()
                .map(|n| {
                    serde_json::json!({
                        "id":         n.id,
                        "type":       n.notif_type,
                        "message":    n.message,
                        "entity_id":  n.entity_id,
                        "href":       n.href,
                        "read":       n.read,
                        "created_at": n.created_at,
                    })
                })
                .collect();
            (json.len(), json)
        }
        Err(_) => (0, vec![]),
    };

    // ── Uptime ────────────────────────────────────────────────────────────────
    let uptime_secs = state.started_at.elapsed().as_secs();

    let payload = serde_json::json!({
        "flushed_at":        now_iso8601(),
        "uptime_seconds":    uptime_secs,
        "event_count":       event_snapshots.len(),
        "events":            event_snapshots,
        "notification_count": notif_count,
        "notifications":     notif_json,
    });

    match serde_json::to_string_pretty(&payload) {
        Ok(text) => match std::fs::write(FLUSH_PATH, text) {
            Ok(()) => eprintln!(
                "shutdown: flushed {} events + {} notifications → {FLUSH_PATH}",
                events.len(),
                notif_count,
            ),
            Err(e) => eprintln!("shutdown: write failed ({FLUSH_PATH}): {e}"),
        },
        Err(e) => eprintln!("shutdown: serialisation failed: {e}"),
    }
}

// ── Demo data seeding ─────────────────────────────────────────────────────────

/// Populate the InMemory store with representative demo data so the dashboard
/// and all pages show meaningful content immediately after first start.
///
/// Only called in `DeploymentMode::Local`. Errors are logged but never fatal —
/// a partially-seeded store is better than no server at all.
async fn seed_demo_data(state: &AppState) {
    use cairn_domain::{
        policy::{ApprovalDecision, ApprovalRequirement},
        tenancy::ProjectKey,
        ApprovalId, AuditOutcome, FailureClass, PauseReason, PauseReasonKind, RunId, SessionId,
        TaskId, TenantId,
    };
    use cairn_runtime::{
        approvals::ApprovalService, audits::AuditService, runs::RunService,
        sessions::SessionService, tasks::TaskService,
    };

    let project = ProjectKey::new("default_tenant", "default_workspace", "demo_project");
    let tenant = TenantId::new("default_tenant");

    // ── 3 Sessions ────────────────────────────────────────────────────────────
    let s_ids: &[&str] = &["sess_alpha", "sess_beta", "sess_gamma"];
    for id in s_ids {
        if let Err(e) = state
            .runtime
            .sessions
            .create(&project, SessionId::new(*id))
            .await
        {
            eprintln!("seed: session {id}: {e}");
        }
    }

    // ── 5 Runs ────────────────────────────────────────────────────────────────
    // run_a: completed   (sess_alpha)
    // run_b: completed   (sess_alpha)
    // run_c: running     (sess_beta)
    // run_d: failed      (sess_beta)
    // run_e: paused      (sess_gamma)
    let run_defs: &[(&str, &str)] = &[
        ("run_a", "sess_alpha"),
        ("run_b", "sess_alpha"),
        ("run_c", "sess_beta"),
        ("run_d", "sess_beta"),
        ("run_e", "sess_gamma"),
    ];
    for (run, sess) in run_defs {
        if let Err(e) = state
            .runtime
            .runs
            .start(&project, &SessionId::new(*sess), RunId::new(*run), None)
            .await
        {
            eprintln!("seed: run {run}: {e}");
        }
    }
    let _ = state.runtime.runs.complete(&RunId::new("run_a")).await;
    let _ = state.runtime.runs.complete(&RunId::new("run_b")).await;
    let _ = state
        .runtime
        .runs
        .fail(&RunId::new("run_d"), FailureClass::ExecutionError)
        .await;
    let _ = state
        .runtime
        .runs
        .pause(
            &RunId::new("run_e"),
            PauseReason {
                kind: PauseReasonKind::OperatorPause,
                detail: Some("Demo pause".into()),
                resume_after_ms: None,
                actor: Some("demo_seed".into()),
            },
        )
        .await;

    // ── 12 Tasks ──────────────────────────────────────────────────────────────
    // Distribution: 3 queued, 2 claimed, 2 running, 4 completed, 1 failed, 1 cancelled (=13 total incl task_12)
    let task_defs: &[(&str, &str)] = &[
        ("task_01", "run_a"),
        ("task_02", "run_a"),
        ("task_03", "run_a"),
        ("task_04", "run_b"),
        ("task_05", "run_c"),
        ("task_06", "run_c"),
        ("task_07", "run_c"),
        ("task_08", "run_c"),
        ("task_09", "run_c"),
        ("task_10", "run_d"),
        ("task_11", "run_d"),
        ("task_12", "run_e"),
    ];
    for (tid, rid) in task_defs {
        if let Err(e) = state
            .runtime
            .tasks
            .submit(&project, TaskId::new(*tid), Some(RunId::new(*rid)), None, 0)
            .await
        {
            eprintln!("seed: task {tid}: {e}");
        }
    }
    // Complete task_01–04
    for tid in &["task_01", "task_02", "task_03", "task_04"] {
        let _ = state
            .runtime
            .tasks
            .claim(&TaskId::new(*tid), "demo-worker".to_owned(), 300_000)
            .await;
        let _ = state.runtime.tasks.start(&TaskId::new(*tid)).await;
        let _ = state.runtime.tasks.complete(&TaskId::new(*tid)).await;
    }
    // Running: task_05, task_06
    for tid in &["task_05", "task_06"] {
        let _ = state
            .runtime
            .tasks
            .claim(&TaskId::new(*tid), "demo-worker".to_owned(), 300_000)
            .await;
        let _ = state.runtime.tasks.start(&TaskId::new(*tid)).await;
    }
    // Claimed: task_07, task_08
    for tid in &["task_07", "task_08"] {
        let _ = state
            .runtime
            .tasks
            .claim(&TaskId::new(*tid), "demo-worker".to_owned(), 300_000)
            .await;
    }
    // task_09, task_12 remain queued
    // Fail task_10, cancel task_11
    let _ = state
        .runtime
        .tasks
        .claim(&TaskId::new("task_10"), "demo-worker".to_owned(), 300_000)
        .await;
    let _ = state.runtime.tasks.start(&TaskId::new("task_10")).await;
    let _ = state
        .runtime
        .tasks
        .fail(&TaskId::new("task_10"), FailureClass::ExecutionError)
        .await;
    let _ = state.runtime.tasks.cancel(&TaskId::new("task_11")).await;

    // ── 3 Approvals ───────────────────────────────────────────────────────────
    // appr_01: pending (run_c)
    // appr_02: approved (run_a)
    // appr_03: rejected (run_d)
    let appr_defs: &[(&str, &str)] = &[
        ("appr_01", "run_c"),
        ("appr_02", "run_a"),
        ("appr_03", "run_d"),
    ];
    for (aid, rid) in appr_defs {
        if let Err(e) = state
            .runtime
            .approvals
            .request(
                &project,
                ApprovalId::new(*aid),
                Some(RunId::new(*rid)),
                None,
                ApprovalRequirement::Required,
            )
            .await
        {
            eprintln!("seed: approval {aid}: {e}");
        }
    }
    let _ = state
        .runtime
        .approvals
        .resolve(&ApprovalId::new("appr_02"), ApprovalDecision::Approved)
        .await;
    let _ = state
        .runtime
        .approvals
        .resolve(&ApprovalId::new("appr_03"), ApprovalDecision::Rejected)
        .await;

    // ── 10 Audit log entries ──────────────────────────────────────────────────
    let audit_entries: &[(&str, &str, &str, &str, AuditOutcome)] = &[
        (
            "operator",
            "create_session",
            "session",
            "sess_alpha",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "create_session",
            "session",
            "sess_beta",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "create_session",
            "session",
            "sess_gamma",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "start_run",
            "run",
            "run_a",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "start_run",
            "run",
            "run_c",
            AuditOutcome::Success,
        ),
        (
            "demo-worker",
            "complete_run",
            "run",
            "run_a",
            AuditOutcome::Success,
        ),
        (
            "demo-worker",
            "fail_run",
            "run",
            "run_d",
            AuditOutcome::Failure,
        ),
        (
            "operator",
            "pause_run",
            "run",
            "run_e",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "approve",
            "approval",
            "appr_02",
            AuditOutcome::Success,
        ),
        (
            "operator",
            "reject",
            "approval",
            "appr_03",
            AuditOutcome::Success,
        ),
    ];
    for (actor, action, rtype, rid, outcome) in audit_entries {
        if let Err(e) = state
            .runtime
            .audits
            .record(
                tenant.clone(),
                (*actor).to_owned(),
                (*action).to_owned(),
                (*rtype).to_owned(),
                (*rid).to_owned(),
                *outcome,
                serde_json::json!({"source": "demo_seed"}),
            )
            .await
        {
            eprintln!("seed: audit {action}/{rid}: {e}");
        }
    }

    eprintln!(
        "seed: demo data ready — {} sessions, {} runs, {} tasks, {} approvals, {} audit entries",
        s_ids.len(),
        run_defs.len(),
        task_defs.len(),
        appr_defs.len(),
        audit_entries.len(),
    );
}

#[tokio::main]
async fn main() {
    // Load .env file if present (dev convenience — not required in production).
    // Silently ignored when the file doesn't exist.
    let _ = dotenvy::dotenv();

    // Initialise structured request tracing.  Operators can tune verbosity via
    // the RUST_LOG env var (e.g. RUST_LOG=cairn_app=info,tower_http=debug).
    //
    // When CAIRN_LOG_DIR is set, logs are also written to daily-rotating files.
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    if let Ok(log_dir) = std::env::var("CAIRN_LOG_DIR") {
        let log_dir = log_dir.trim().to_owned();
        if !log_dir.is_empty() {
            use tracing_subscriber::layer::SubscriberExt;
            use tracing_subscriber::util::SubscriberInitExt;

            let file_appender = tracing_appender::rolling::daily(&log_dir, "cairn.log");
            let file_layer = tracing_subscriber::fmt::layer()
                .with_writer(file_appender)
                .with_target(false)
                .compact()
                .with_ansi(false);
            let stdout_layer = tracing_subscriber::fmt::layer()
                .with_target(false)
                .compact();
            tracing_subscriber::registry()
                .with(env_filter)
                .with(stdout_layer)
                .with(file_layer)
                .init();
            eprintln!("logs: rotating daily to {log_dir}/cairn.*.log");
        } else {
            tracing_subscriber::fmt()
                .with_env_filter(env_filter)
                .with_target(false)
                .compact()
                .init();
        }
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(env_filter)
            .with_target(false)
            .compact()
            .init();
    }

    let config = parse_args();

    // ── Token registry ────────────────────────────────────────────────────────
    // Priority: CAIRN_ADMIN_TOKEN_FILE > CAIRN_ADMIN_TOKEN > default dev token.
    // CAIRN_ADMIN_TOKEN_FILE reads from a file path (Docker secrets pattern).
    let admin_token = if let Ok(file_path) = std::env::var("CAIRN_ADMIN_TOKEN_FILE") {
        let file_path = file_path.trim().to_owned();
        match std::fs::read_to_string(&file_path) {
            Ok(contents) => {
                let token = contents.trim().to_owned();
                if token.is_empty() {
                    eprintln!("error: CAIRN_ADMIN_TOKEN_FILE at {file_path} is empty");
                    std::process::exit(1);
                }
                eprintln!("auth: admin token loaded from file {file_path}");
                token
            }
            Err(e) => {
                eprintln!("error: cannot read CAIRN_ADMIN_TOKEN_FILE at {file_path}: {e}");
                std::process::exit(1);
            }
        }
    } else {
        std::env::var("CAIRN_ADMIN_TOKEN").unwrap_or_else(|_| {
            if config.mode == DeploymentMode::SelfHostedTeam {
                eprintln!(
                    "error: CAIRN_ADMIN_TOKEN env var is required in team mode. \
                     Set it to a strong random token before starting."
                );
                std::process::exit(1);
            }
            "dev-admin-token".to_owned()
        })
    };
    if admin_token == "dev-admin-token" {
        eprintln!(
            "⚠ auth: using default dev-admin-token — override with CAIRN_ADMIN_TOKEN in production"
        );
    } else {
        eprintln!("auth: admin token configured");
    }

    // ── Durable backends (Postgres / SQLite) ────────────────────────────────
    let pg;
    let sqlite;
    match &config.storage {
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
                    let pg_event_log = Arc::new(PgEventLog::new(pool.clone()));
                    let backend = Arc::new(PgBackend {
                        event_log: pg_event_log.clone(),
                        adapter: Arc::new(PgAdapter::new(pool)),
                    });
                    eprintln!("store: Postgres backend active (all service events dual-written)");
                    pg = Some(backend);
                    sqlite = None;
                }
                Err(e) => {
                    eprintln!("error: failed to connect to Postgres: {e}");
                    std::process::exit(1);
                }
            }
        }
        StorageBackend::Sqlite { path } => {
            // Normalise the URL: accept bare paths like "cairn.db" or "sqlite:cairn.db".
            let url = if path.starts_with("sqlite:") {
                path.clone()
            } else {
                format!("sqlite:{path}")
            };
            let sqlite_path = path
                .strip_prefix("sqlite:")
                .unwrap_or(path.as_str())
                .to_owned();
            eprintln!("store: connecting to SQLite at {url}");
            match SqlitePoolOptions::new()
                .max_connections(1) // SQLite is not safe with multiple writers
                .connect(&url)
                .await
            {
                Ok(pool) => {
                    eprintln!("store: SQLite connection established");
                    let adapter = SqliteAdapter::new(pool.clone());
                    match adapter.migrate().await {
                        Ok(()) => eprintln!("store: SQLite schema applied"),
                        Err(e) => {
                            eprintln!("error: SQLite migration failed: {e}");
                            std::process::exit(1);
                        }
                    }
                    let sqlite_event_log = Arc::new(SqliteEventLog::new(pool));
                    let backend = Arc::new(SqliteBackend {
                        event_log: sqlite_event_log.clone(),
                        adapter: Arc::new(adapter),
                        path: PathBuf::from(sqlite_path),
                    });
                    eprintln!("store: SQLite backend active (all service events dual-written)");
                    pg = None;
                    sqlite = Some(backend);
                }
                Err(e) => {
                    eprintln!("error: failed to connect to SQLite: {e}");
                    std::process::exit(1);
                }
            }
        }
        StorageBackend::InMemory => {
            eprintln!(
                "⚠ store: using in-memory backend — ALL DATA WILL BE LOST on restart. \
                 Set DATABASE_URL or use --db to configure a durable store."
            );
            pg = None;
            sqlite = None;
        }
    }

    // ── Lib.rs AppState (catalog-driven router, shared runtime) ─────────────
    let mut lib_state = Arc::new(
        cairn_app::AppState::new(config.clone())
            .await
            .expect("failed to initialise lib AppState"),
    );
    // Register the admin token in the SHARED token registry so both routers
    // authenticate identically.
    lib_state.service_tokens.register(
        admin_token.clone(),
        AuthPrincipal::ServiceAccount {
            name: "admin".to_owned(),
            tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new("default")),
        },
    );

    // ── Startup replay from durable event log ────────────────────────────────
    // When a Postgres or SQLite backend is available, replay its event log into
    // the InMemoryStore so that projections (sessions, runs, tasks, approvals,
    // etc.) are warm on restart rather than empty.
    //
    // Replay runs in batches of 10 000 events to bound peak memory.  All events
    // are fed through InMemoryStore::append, which applies the same
    // apply_projection logic used during normal writes — guaranteeing that the
    // in-memory state is identical to what would have accumulated from scratch.
    {
        const REPLAY_BATCH: usize = 10_000;
        let durable_log: Option<&dyn EventLog> = if let Some(ref backend) = pg {
            Some(backend.event_log.as_ref())
        } else if let Some(ref backend) = sqlite {
            Some(backend.event_log.as_ref())
        } else {
            None
        };

        if let Some(log) = durable_log {
            eprintln!("store: replaying event log into InMemory projections…");
            let mut after: Option<EventPosition> = None;
            let mut total = 0usize;
            loop {
                let batch = match log.read_stream(after, REPLAY_BATCH).await {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!("store: replay error reading batch after {after:?}: {e}");
                        std::process::exit(1);
                    }
                };
                if batch.is_empty() {
                    break;
                }
                after = batch.last().map(|e| e.position);
                let batch_len = batch.len();
                total += batch_len;
                let envelopes: Vec<_> = batch.into_iter().map(|e| e.envelope).collect();
                if let Err(e) = lib_state.runtime.store.append(&envelopes).await {
                    eprintln!("store: replay error applying batch: {e}");
                    std::process::exit(1);
                }
                if batch_len < REPLAY_BATCH {
                    // Last batch — no need to fetch again.
                    break;
                }
            }
            if total > 0 {
                eprintln!("store: replayed {total} event(s) — projections warm");
            } else {
                eprintln!("store: event log empty — starting with clean projections");
            }
        }
    }

    // ── Seed the service-layer event ID counter above existing events ─────────
    // The make_envelope() counter starts at 0 on each process startup and
    // generates IDs like "evt_<timestamp>_<n>".  Seeding with the current
    // InMemory head position ensures IDs are unique across restarts even if
    // two events happen to share the same millisecond timestamp.
    {
        let head = lib_state
            .runtime
            .store
            .head_position()
            .await
            .unwrap_or(None);
        let floor = head.map(|p| p.0).unwrap_or(0);
        cairn_runtime::seed_event_counter(floor);
    }

    // ── Ollama local LLM provider (optional) ─────────────────────────────────
    let ollama: Option<Arc<OllamaProvider>> = if let Some(provider) = OllamaProvider::from_env() {
        eprintln!("ollama: connecting to {}", provider.host());
        match provider.health_check().await {
            Ok(tags) => {
                if tags.models.is_empty() {
                    eprintln!("ollama: reachable but no models loaded");
                } else {
                    let names: Vec<&str> = tags.models.iter().map(|m| m.name.as_str()).collect();
                    eprintln!(
                        "ollama: {} model(s) available: {}",
                        names.len(),
                        names.join(", ")
                    );
                }
                Some(Arc::new(provider))
            }
            Err(e) => {
                eprintln!("ollama: health check failed ({e}) — provider disabled");
                None
            }
        }
    } else {
        None
    };

    // ── Provider construction via cairn-providers ──────────────────────────────
    // All providers are constructed through ProviderBuilder using runtime config.
    // cairn-providers implements cairn-domain's GenerationProvider trait via the
    // bridge module, so everything plugs into the existing orchestrate/generate paths.
    use cairn_providers::backends::bedrock::Bedrock as CairnBedrock;
    use cairn_providers::wire::openai_compat::{OpenAiCompat, ProviderConfig};
    use cairn_runtime::RuntimeConfig;

    let normalize_model = |model: String| {
        let trimmed = model.trim();
        if trimmed.is_empty() || trimmed == "default" {
            None
        } else {
            Some(trimmed.to_owned())
        }
    };
    let configured_generate_model = normalize_model(
        lib_state
            .runtime
            .runtime_config
            .default_generate_model()
            .await,
    );
    let configured_brain_model =
        normalize_model(lib_state.runtime.runtime_config.default_brain_model().await)
            .or_else(|| configured_generate_model.clone());

    let openai_compat_brain: Option<Arc<OpenAiCompat>> = {
        let brain_url = std::env::var("CAIRN_BRAIN_URL")
            .or_else(|_| std::env::var("OPENAI_COMPAT_BASE_URL"))
            .ok()
            .filter(|u| !u.is_empty());
        let brain_key = std::env::var("CAIRN_BRAIN_KEY")
            .or_else(|_| std::env::var("OPENAI_COMPAT_API_KEY"))
            .unwrap_or_default();
        brain_url.and_then(|url| {
            eprintln!(
                "openai-compat (brain): configured at {url} model={}",
                configured_brain_model.as_deref().unwrap_or("<unset>")
            );
            match OpenAiCompat::new(
                ProviderConfig::default(),
                brain_key,
                Some(url),
                configured_brain_model.clone(),
                None,
                None,
                None,
            ) {
                Ok(provider) => Some(Arc::new(provider)),
                Err(err) => {
                    eprintln!("openai-compat (brain): invalid config: {err}");
                    None
                }
            }
        })
    };
    let openai_compat_worker: Option<Arc<OpenAiCompat>> = {
        let worker_url = std::env::var("CAIRN_WORKER_URL")
            .or_else(|_| std::env::var("OPENAI_COMPAT_BASE_URL"))
            .ok()
            .filter(|u| !u.is_empty());
        let worker_key = std::env::var("CAIRN_WORKER_KEY")
            .or_else(|_| std::env::var("OPENAI_COMPAT_API_KEY"))
            .unwrap_or_default();
        worker_url.and_then(|url| {
            eprintln!(
                "openai-compat (worker): configured at {url} model={}",
                configured_generate_model.as_deref().unwrap_or("<unset>")
            );
            match OpenAiCompat::new(
                ProviderConfig::default(),
                worker_key,
                Some(url),
                configured_generate_model.clone(),
                None,
                None,
                None,
            ) {
                Ok(provider) => Some(Arc::new(provider)),
                Err(err) => {
                    eprintln!("openai-compat (worker): invalid config: {err}");
                    None
                }
            }
        })
    };
    let openai_compat_openrouter: Option<Arc<OpenAiCompat>> = {
        RuntimeConfig::openrouter_api_key().and_then(|key| {
            eprintln!("openai-compat (openrouter): configured — brain=openrouter/free worker=google/gemma-3-4b-it:free");
            match OpenAiCompat::new(
                ProviderConfig::OPENROUTER,
                key,
                None, None, None, None, None,
            ) {
                Ok(provider) => Some(Arc::new(provider)),
                Err(err) => {
                    eprintln!("openai-compat (openrouter): invalid config: {err}");
                    None
                }
            }
        })
    };

    // Legacy alias: expose the first configured provider as `openai_compat`.
    let openai_compat: Option<Arc<OpenAiCompat>> = openai_compat_brain
        .clone()
        .or_else(|| openai_compat_worker.clone())
        .or_else(|| openai_compat_openrouter.clone());

    // Bedrock provider via cairn-providers.
    let bedrock: Option<Arc<CairnBedrock>> = CairnBedrock::from_env().map(|p| {
        eprintln!(
            "bedrock: configured — model={} region={}",
            p.model_id(),
            p.region()
        );
        Arc::new(p)
    });

    {
        use cairn_domain::providers::{EmbeddingProvider, GenerationProvider};
        use cairn_providers::chat::ChatProvider;

        lib_state.runtime.provider_registry.set_startup_fallbacks(
            cairn_runtime::StartupFallbackProviders {
                ollama: ollama.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        Arc::new(OllamaEmbeddingProvider::new(provider.host()))
                            as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("ollama", None)
                }),
                brain: openai_compat_brain.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                worker: openai_compat_worker.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                openrouter: openai_compat_openrouter.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openrouter", Some(provider.model.clone()))
                }),
                bedrock: bedrock.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                    )
                    .with_metadata("bedrock", Some(provider.model_id().to_owned()))
                }),
            },
        );
    }

    // Wire brain provider into lib_state for the orchestrate endpoint.
    // Priority: brain → worker → OpenRouter → Bedrock → Ollama.
    {
        use cairn_domain::providers::GenerationProvider;
        let brain: Option<Arc<dyn GenerationProvider>> = openai_compat_brain
            .as_ref()
            .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            .or_else(|| {
                openai_compat_worker
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            })
            .or_else(|| {
                openai_compat_openrouter
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            })
            .or_else(|| {
                bedrock
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            })
            .or_else(|| {
                ollama
                    .as_ref()
                    .map(|p| p.clone() as Arc<dyn GenerationProvider>)
            });
        let lib_mut = Arc::get_mut(&mut lib_state)
            .expect("lib_state must not be cloned before brain_provider is wired");
        if let Some(b) = brain {
            lib_mut.brain_provider = Some(b);
            eprintln!("brain provider: wired to lib_state");
        }
        if let Some(ref br) = bedrock {
            lib_mut.bedrock_provider = Some(br.clone() as Arc<dyn GenerationProvider>);
            eprintln!("bedrock provider: wired to lib_state");
        }
    }

    // ── Wire built-in tool registry into lib_state ───────────────────────────
    // Build with the real RetrievalService + IngestPipeline so the orchestrator
    // can actually search and store memory during execution.
    {
        use cairn_memory::{retrieval::RetrievalService, IngestService};
        let retrieval = lib_state.retrieval.clone() as Arc<dyn RetrievalService>;
        let ingest = lib_state.ingest.clone() as Arc<dyn IngestService>;
        let registry = cairn_app::tool_impls::build_tool_registry(
            retrieval,
            ingest,
            lib_state.project_repo_access.clone(),
            lib_state.repo_clone_cache.clone(),
        );
        let lib_mut = Arc::get_mut(&mut lib_state)
            .expect("lib_state must not be cloned before tool_registry is wired");
        lib_mut.tool_registry = Some(Arc::new(registry));
        eprintln!("tool registry: memory tools + cairn.registerRepo wired");
    }

    // ── Binary-specific state (shares runtime + tokens with lib.rs) ────────
    let state = AppState {
        runtime: lib_state.runtime.clone(),
        started_at: Arc::new(lib_state.started_at),
        tokens: lib_state.service_tokens.clone(),
        pg,
        sqlite,
        mode: config.mode,
        document_store: lib_state.document_store.clone(),
        retrieval: lib_state.retrieval.clone(),
        ingest: lib_state.ingest.clone(),
        ollama,
        openai_compat_brain,
        openai_compat_worker,
        openai_compat_openrouter,
        openai_compat,
        metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
        rate_limits: Arc::new(Mutex::new(HashMap::new())),
        request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
        notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
        templates: Arc::new(templates::TemplateRegistry::with_builtins()),
        entitlements: Arc::new(entitlements::EntitlementService::new()),
        bedrock: bedrock.clone(),
        process_role: config.process_role,
    };

    // ── Wire secondary event log (covers all service-layer appends) ─────────
    // All 109 store.append() call sites in 42 service files are covered by
    // setting the secondary log here once. Any event written by RunService,
    // TaskService, ApprovalService etc. is automatically dual-written.
    if let Some(ref pg_backend) = state.pg {
        state
            .runtime
            .store
            .set_secondary_log(pg_backend.event_log.clone());
        eprintln!("store: service-layer events will dual-write to Postgres");
    } else if let Some(ref sq_backend) = state.sqlite {
        state
            .runtime
            .store
            .set_secondary_log(sq_backend.event_log.clone());
        eprintln!("store: service-layer events will dual-write to SQLite");
    }

    // ── Demo seed data (local mode only, only when event log is empty) ─────────
    // Skip seeding when a durable backend (Postgres/SQLite) already has events
    // from a previous run.  After startup replay the in-memory store's head
    // position tells us whether there is pre-existing data to preserve.
    let event_log_empty = state
        .runtime
        .store
        .head_position()
        .await
        .unwrap_or(None)
        .is_none();
    // ── Always ensure the canonical "default" tenant exists ─────────────────
    // This is idempotent — if the tenant already exists, create() returns Err
    // which we ignore. Needed so provider connections, route policies, etc.
    // work out-of-the-box on first boot.
    {
        use cairn_domain::{tenancy::ProjectKey, TenantId};
        use cairn_runtime::{
            projects::ProjectService, tenants::TenantService, workspaces::WorkspaceService,
        };
        let _ = state
            .runtime
            .tenants
            .create(TenantId::new("default"), "Default".into())
            .await;
        let _ = state
            .runtime
            .workspaces
            .create(
                TenantId::new("default"),
                cairn_domain::WorkspaceId::new("default"),
                "Default".into(),
            )
            .await;
        let _ = state
            .runtime
            .projects
            .create(
                ProjectKey::new("default", "default", "default"),
                "Default".into(),
            )
            .await;
    }

    if state.mode == DeploymentMode::Local && event_log_empty {
        seed_demo_data(&state).await;
    }

    match lib_state.sandbox_service.recover_all().await {
        Ok(summary) => {
            if summary.reconnected > 0 || summary.preserved > 0 || summary.failed > 0 {
                eprintln!(
                    "sandbox recovery: reconnected={} preserved={} failed={}",
                    summary.reconnected, summary.preserved, summary.failed
                );
            }
        }
        Err(error) => eprintln!("sandbox recovery failed: {error}"),
    }

    let recovery_now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    match lib_state
        .runtime
        .recovery
        .recover_expired_leases(recovery_now_ms, 1_000)
        .await
    {
        Ok(summary) if summary.scanned > 0 || !summary.actions.is_empty() => {
            eprintln!(
                "lease recovery: scanned={} actions={}",
                summary.scanned,
                summary.actions.len()
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("lease recovery failed: {error}"),
    }
    match lib_state
        .runtime
        .recovery
        .recover_interrupted_runs(1_000)
        .await
    {
        Ok(summary) if summary.scanned > 0 || !summary.actions.is_empty() => {
            eprintln!(
                "run recovery: scanned={} actions={}",
                summary.scanned,
                summary.actions.len()
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("run recovery failed: {error}"),
    }
    match lib_state
        .runtime
        .recovery
        .resolve_stale_dependencies(1_000)
        .await
    {
        Ok(summary) if summary.scanned > 0 || !summary.actions.is_empty() => {
            eprintln!(
                "dependency recovery: scanned={} actions={}",
                summary.scanned,
                summary.actions.len()
            );
        }
        Ok(_) => {}
        Err(error) => eprintln!("dependency recovery failed: {error}"),
    }

    // ── Startup replays ────────────────────────────────────────────────────────
    // Replay all store events into in-memory projections so pre-existing data
    // (seeded above or loaded from a snapshot) is immediately visible without
    // requiring an SSE connection first.
    lib_state.replay_graph().await;
    lib_state.replay_evals().await;
    lib_state.runtime.store.reset_usage_counters();

    eprintln!("cairn-app starting with role: {}", config.process_role);

    // ── RFC 011: Role-based startup ──────────────────────────────────────────
    if config.process_role.serves_http() {
        // ── Router ───────────────────────────────────────────────────────────
        let state_for_flush = state.clone();
        let app = build_router(lib_state.clone(), state);

        let addr = format!("{}:{}", config.listen_addr, config.listen_port);
        let listener = tokio::net::TcpListener::bind(&addr)
            .await
            .unwrap_or_else(|e| panic!("failed to bind {addr}: {e}"));

        eprintln!("cairn-app listening on http://{addr}");

        // ── Graceful shutdown wiring ─────────────────────────────────────────
        let (signal_tx, signal_rx) = tokio::sync::watch::channel(false);

        let watchdog_state = state_for_flush.clone();
        let watchdog = tokio::spawn(async move {
            let mut rx = signal_rx;
            loop {
                if rx.changed().await.is_err() {
                    return;
                }
                if *rx.borrow() {
                    break;
                }
            }
            eprintln!("shutdown: draining in-flight requests (max 30s)…");
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            flush_state_to_disk(&watchdog_state).await;
            eprintln!("shutdown: 30s drain timeout — forcing exit");
            std::process::exit(0);
        });

        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                wait_for_shutdown_signal().await;
                let _ = signal_tx.send(true);
            })
            .await
            .unwrap_or_else(|e| eprintln!("server error: {e}"));

        watchdog.abort();
        eprintln!("shutdown: all connections drained");
        flush_state_to_disk(&state_for_flush).await;
        eprintln!("shutdown: complete");
    } else {
        // ── WorkerOnly mode: no HTTP server, run task processing loop ────────
        eprintln!("cairn-app running in worker-only mode (no HTTP server)");
        eprintln!("connected to same store — processing tasks until shutdown signal");

        // Run a simple claim/execute loop until a shutdown signal arrives.
        // Both roles share the same store, so workers see events from the API.
        let shutdown = wait_for_shutdown_signal();
        tokio::pin!(shutdown);

        loop {
            tokio::select! {
                _ = &mut shutdown => {
                    eprintln!("shutdown: worker received signal, exiting");
                    break;
                }
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {
                    // Worker tick: run due health checks, recovery sweeps, etc.
                    // These are non-blocking and use the shared store.
                    let _ = state.runtime.provider_health
                        .run_due_health_checks()
                        .await;
                }
            }
        }

        eprintln!("shutdown: worker complete");
    }
}

// ── LLM trace handlers (GAP-010) ─────────────────────────────────────────────

/// `GET /v1/traces` — all recent LLM call traces (operator view, limit 500).
async fn list_all_traces_handler(
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

// ── OpenAPI spec + Swagger UI ─────────────────────────────────────────────────

/// `GET /v1/openapi.json` — OpenAPI 3.0 specification.
async fn openapi_json_handler() -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json; charset=utf-8",
        )],
        openapi_spec::OPENAPI_JSON,
    )
}

/// `GET /v1/docs` — Swagger UI (CDN-hosted, points at /v1/openapi.json).
async fn swagger_ui_handler() -> impl IntoResponse {
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
struct FrontendAssets;

/// Serve an embedded frontend file, falling back to `index.html` for any path
/// not found (SPA client-side routing).  API routes registered before this
/// fallback continue to take priority.
async fn serve_frontend(uri: axum::http::Uri) -> impl IntoResponse {
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

/// `GET /v1/metrics/prometheus` — Prometheus exposition format (text/plain).
///
/// Compatible with Prometheus scrape configs and Grafana data sources.
async fn metrics_prometheus_handler(State(state): State<AppState>) -> impl IntoResponse {
    let m = state.metrics.read().unwrap();
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

// ── RFC 011: Server role handler ─────────────────────────────────────────────

/// `GET /v1/system/role` — returns the current process role.
async fn system_role_handler(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "role": state.process_role.as_str(),
        "serves_http": state.process_role.serves_http(),
        "runs_workers": state.process_role.runs_workers(),
    }))
}

// ── RFC 014: Entitlement handlers ────────────────────────────────────────────

/// `GET /v1/entitlements` — current plan + usage + limits for the default tenant.
async fn entitlements_handler(
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
async fn entitlements_usage_handler(
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

#[derive(Deserialize)]
struct TenantQuery {
    #[serde(default)]
    tenant_id: Option<String>,
}

/// `POST /v1/bundles/import` — import a bundle into a target project.
async fn bundle_import_handler(Json(body): Json<bundles::ImportRequest>) -> impl IntoResponse {
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
async fn list_templates_handler(
    State(state): State<AppState>,
) -> Json<Vec<templates::TemplateSummary>> {
    Json(state.templates.list())
}

/// `GET /v1/templates/:id` — get full template detail with file contents.
async fn get_template_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.templates.get(&id) {
        Some(t) => Ok(Json(t.clone())),
        None => Err(not_found(format!("template not found: {id}"))),
    }
}

/// `POST /v1/templates/:id/apply` — apply a template to a project.
async fn apply_template_handler(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<templates::ApplyRequest>,
) -> impl IntoResponse {
    match state.templates.apply(&id, &body.project_id) {
        Some(result) => Ok(Json(result)),
        None => Err(not_found(format!("template not found: {id}"))),
    }
}

fn build_router(lib_state: Arc<cairn_app::AppState>, state: AppState) -> Router {
    // ── Base: 197 catalog-driven routes from lib.rs ──────────────────────
    let catalog_routes =
        cairn_app::AppBootstrap::build_catalog_routes().with_state(lib_state.clone());

    // ── Binary-specific routes (not in the catalog) ──────────────────────
    let binary_routes: Router = Router::new()
        // WebSocket (catalog handles /v1/stream and /v1/streams/runtime)
        .route("/v1/ws", get(ws_handler))
        // System introspection
        .route("/v1/health/detailed", get(detailed_health_handler))
        .route("/v1/system/info", get(system_info_handler))
        .route("/v1/system/role", get(system_role_handler))
        // /v1/overview served by catalog
        // Runs — binary-specific views
        .route("/v1/runs/batch", post(batch_create_runs_handler))
        .route(
            "/v1/runs/:id/tool-invocations",
            get(list_run_tool_invocations_handler),
        )
        .route(
            "/v1/runs/:id/tasks",
            get(list_run_tasks_handler).post(create_run_task_handler),
        )
        .route("/v1/runs/:id/approvals", get(list_run_approvals_handler))
        .route("/v1/runs/:id/export", get(export_run_handler))
        // Sessions — binary-specific views
        .route("/v1/sessions/import", post(import_session_handler))
        .route("/v1/sessions/:id/runs", get(list_session_runs_handler))
        .route("/v1/sessions/:id/export", get(export_session_handler))
        // Approvals — /resolve as primary per W3 decision
        .route("/v1/approvals/pending", get(list_pending_approvals_handler))
        .route("/v1/approvals/:id/resolve", post(resolve_approval_handler))
        // Events
        .route("/v1/events", get(list_events_handler))
        .route("/v1/events/append", post(append_events_handler))
        // Tasks — binary-specific (complete served by catalog)
        .route("/v1/tasks/batch/cancel", post(batch_cancel_tasks_handler))
        .route("/v1/tasks/:id/start", post(start_task_handler))
        .route("/v1/tasks/:id/fail", post(fail_task_handler))
        // Traces
        .route("/v1/traces", get(list_all_traces_handler))
        .route("/v1/traces/export", get(export_otlp_handler))
        // Admin utilities
        // NOTE: /v1/admin/logs is now served by the catalog-driven router in lib.rs.
        // The observability middleware populates lib_state.request_log, which the
        // catalog handler reads — so the request log is always fresh.
        .route("/v1/admin/snapshot", post(admin_snapshot_handler))
        .route("/v1/admin/backup", post(backup_handler))
        .route("/v1/admin/restore", post(admin_restore_handler))
        .route(
            "/v1/admin/rebuild-projections",
            post(rebuild_projections_handler),
        )
        .route("/v1/admin/event-count", get(event_count_handler))
        .route("/v1/admin/event-log", get(admin_event_log_handler))
        .route("/v1/admin/rotate-token", post(rotate_token_handler))
        // Notifications
        .route("/v1/notifications", get(list_notifications_handler))
        .route(
            "/v1/notifications/read-all",
            post(mark_all_notifications_read_handler),
        )
        .route(
            "/v1/notifications/:id/read",
            post(mark_notification_read_handler),
        )
        // Bundles — /import aliases to /apply per W3 decision
        .route("/v1/bundles/import", post(bundle_import_handler))
        // Entitlements
        .route("/v1/entitlements", get(entitlements_handler))
        .route("/v1/entitlements/usage", get(entitlements_usage_handler))
        // Templates
        .route("/v1/templates", get(list_templates_handler))
        .route("/v1/templates/:id", get(get_template_handler))
        .route("/v1/templates/:id/apply", post(apply_template_handler))
        // Ollama local LLM provider
        // Provider connection discovery + health test
        .route(
            "/v1/providers/connections/:id/discover-models",
            get(discover_models_handler),
        )
        .route(
            "/v1/providers/connections/:id/test",
            get(test_connection_handler),
        )
        .route("/v1/providers/ollama/models", get(ollama_models_handler))
        .route(
            "/v1/providers/ollama/models/:name/info",
            get(ollama_model_info_handler),
        )
        .route(
            "/v1/providers/ollama/generate",
            post(ollama_generate_handler),
        )
        .route("/v1/chat/stream", post(chat_stream_handler))
        // Keep the old route as an alias for backwards compatibility
        .route("/v1/providers/ollama/stream", post(chat_stream_handler))
        .route("/v1/providers/ollama/pull", post(ollama_pull_handler))
        .route(
            "/v1/providers/ollama/delete",
            post(ollama_delete_model_handler),
        )
        .route("/v1/memory/embed", post(ollama_embed_handler))
        // Database diagnostics
        .route("/v1/db/status", get(db_status_handler))
        // /v1/metrics served by catalog; prometheus is binary-only
        .route("/v1/metrics/prometheus", get(metrics_prometheus_handler))
        // OpenAPI + docs
        .route("/v1/openapi.json", get(openapi_json_handler))
        .route("/v1/docs", get(swagger_ui_handler))
        .route("/v1/changelog", get(changelog_handler))
        // Test
        .route("/v1/test/webhook", post(test_webhook_handler))
        // Rate-limit status
        .route("/v1/rate-limit", get(rate_limit_status_handler))
        .with_state(state);

    // ── Merge catalog + binary routes ────────────────────────────────────
    let merged = catalog_routes
        .merge(binary_routes)
        .fallback(get(serve_frontend));

    // ── Apply lib.rs middleware (auth, CORS, rate-limit, tracing) ────────
    cairn_app::AppBootstrap::apply_middleware(merged, lib_state)
        // Binary-specific outer layers
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_request(DefaultOnRequest::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(axum::middleware::from_fn(version_header_middleware))
}

// ── Test helpers (visible to all test modules via `super::`) ─────────────────

#[cfg(test)]
fn test_make_app(mut state: AppState) -> axum::Router {
    // Construct lib_state on a dedicated thread to avoid tokio runtime nesting.
    let lib_state = std::thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        std::sync::Arc::new(
            rt.block_on(cairn_app::AppState::new(
                cairn_api::bootstrap::BootstrapConfig::default(),
            ))
            .expect("test lib state"),
        )
    })
    .join()
    .expect("lib_state thread panicked");
    // Copy all test tokens into the lib state's token registry so the catalog
    // router's auth middleware recognises them.
    for (token, principal) in state.tokens.all_entries() {
        lib_state.service_tokens.register(token, principal);
    }
    // Share the lib_state's runtime and stores so both routers see the same data.
    state.runtime = lib_state.runtime.clone();
    state.document_store = lib_state.document_store.clone();
    state.retrieval = lib_state.retrieval.clone();
    state.ingest = lib_state.ingest.clone();
    {
        use cairn_domain::providers::{EmbeddingProvider, GenerationProvider};
        use cairn_providers::chat::ChatProvider;

        state.runtime.provider_registry.set_startup_fallbacks(
            cairn_runtime::StartupFallbackProviders {
                ollama: state.ollama.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        Arc::new(OllamaEmbeddingProvider::new(provider.host()))
                            as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("ollama", None)
                }),
                brain: state.openai_compat_brain.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                worker: state.openai_compat_worker.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openai-compatible", Some(provider.model.clone()))
                }),
                openrouter: state.openai_compat_openrouter.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat_and_embedding(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                        provider.clone() as Arc<dyn EmbeddingProvider>,
                    )
                    .with_metadata("openrouter", Some(provider.model.clone()))
                }),
                bedrock: state.bedrock.as_ref().map(|provider| {
                    cairn_runtime::StartupProviderEntry::with_chat(
                        provider.clone() as Arc<dyn GenerationProvider>,
                        provider.clone() as Arc<dyn ChatProvider>,
                    )
                    .with_metadata("bedrock", Some(provider.model_id().to_owned()))
                }),
            },
        );
    }
    build_router(lib_state, state)
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::post;
    use axum::Router;
    use cairn_api::bootstrap::{ServerBootstrap, StorageBackend};
    use cairn_domain::{ProjectKey, SessionId};
    use cairn_providers::wire::openai_compat::{OpenAiCompat, ProviderConfig};
    use cairn_runtime::sessions::SessionService;
    use std::sync::Mutex;
    use tower::ServiceExt as _;

    struct RecordingBootstrap {
        seen: Mutex<Option<BootstrapConfig>>,
    }

    impl RecordingBootstrap {
        fn new() -> Self {
            Self {
                seen: Mutex::new(None),
            }
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
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new(
                    "test-tenant",
                )),
            },
        );
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
        }
    }

    #[tokio::test]
    async fn admin_backup_returns_404_when_sqlite_backend_is_disabled() {
        let app = make_app(make_state());
        let response = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri("/v1/admin/backup")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload["code"], "not_found");
        assert_eq!(
            payload["message"],
            "SQLite backup is only available when the SQLite backend is active"
        );
    }

    fn make_app(state: AppState) -> Router {
        super::test_make_app(state)
    }

    async fn authed_json(
        app: Router,
        method: axum::http::Method,
        uri: &str,
        body: serde_json::Value,
    ) -> axum::response::Response {
        app.oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("authorization", format!("Bearer {TEST_TOKEN}"))
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn authed_sse_post(app: Router, uri: &str, body: serde_json::Value) -> String {
        let resp = app
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::POST)
                    .uri(uri)
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    async fn spawn_openai_compat_mock(text: &'static str) -> String {
        let handler = move || async move {
            Json(serde_json::json!({
                "id": format!("mock-{text}"),
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": text,
                    },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 2,
                    "total_tokens": 5
                }
            }))
        };
        let app = Router::new()
            .route("/chat/completions", post(handler))
            .route(
                "/v1/chat/completions",
                post(move || async move {
                    Json(serde_json::json!({
                        "id": format!("mock-{text}"),
                        "choices": [{
                            "index": 0,
                            "message": {
                                "role": "assistant",
                                "content": text,
                            },
                            "finish_reason": "stop"
                        }],
                        "usage": {
                            "prompt_tokens": 3,
                            "completion_tokens": 2,
                            "total_tokens": 5
                        }
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        format!("http://{addr}")
    }

    async fn spawn_openai_compat_embedding_mock(
        model: &'static str,
        embedding: Vec<f32>,
        token_count: u32,
    ) -> String {
        let embedding_payload =
            serde_json::Value::Array(embedding.into_iter().map(serde_json::Value::from).collect());
        let body = serde_json::json!({
            "object": "list",
            "data": [{
                "object": "embedding",
                "index": 0,
                "embedding": embedding_payload,
            }],
            "model": model,
            "usage": {
                "prompt_tokens": token_count,
                "total_tokens": token_count,
            }
        });
        let app = Router::new()
            .route(
                "/embeddings",
                post({
                    let body = body.clone();
                    move || {
                        let body = body.clone();
                        async move { Json(body) }
                    }
                }),
            )
            .route(
                "/v1/embeddings",
                post(move || {
                    let body = body.clone();
                    async move { Json(body) }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        format!("http://{addr}")
    }

    async fn spawn_openai_compat_stream_mock(chunks: Vec<&'static str>) -> String {
        let mut payload = String::new();
        for chunk in chunks {
            payload.push_str("data: ");
            payload.push_str(
                &serde_json::json!({
                    "choices": [{
                        "delta": {
                            "content": chunk,
                        }
                    }]
                })
                .to_string(),
            );
            payload.push_str("\n\n");
        }
        payload.push_str("data: [DONE]\n\n");

        let app = Router::new()
            .route(
                "/chat/completions",
                post({
                    let payload = payload.clone();
                    move || {
                        let payload = payload.clone();
                        async move {
                            Response::builder()
                                .status(StatusCode::OK)
                                .header("content-type", "text/event-stream")
                                .body(Body::from(payload))
                                .unwrap()
                        }
                    }
                }),
            )
            .route(
                "/v1/chat/completions",
                post(move || {
                    let payload = payload.clone();
                    async move {
                        Response::builder()
                            .status(StatusCode::OK)
                            .header("content-type", "text/event-stream")
                            .body(Body::from(payload))
                            .unwrap()
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        format!("http://{addr}")
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
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
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

    fn assert_embedding_matches(actual: &serde_json::Value, expected: &[f64]) {
        let actual = actual.as_array().expect("embedding array");
        assert_eq!(actual.len(), expected.len(), "embedding length mismatch");
        for (index, (actual, expected)) in actual.iter().zip(expected.iter()).enumerate() {
            let actual = actual.as_f64().expect("embedding value");
            assert!(
                (actual - expected).abs() < 1e-5,
                "embedding[{index}] expected {expected}, got {actual}"
            );
        }
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
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
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
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "postgres://localhost/cairn".to_owned(),
        ]);
        assert!(matches!(c.storage, StorageBackend::Postgres { .. }));
    }

    #[test]
    fn parse_args_db_flag_sets_sqlite() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "my_data.db".to_owned(),
        ]);
        assert!(matches!(c.storage, StorageBackend::Sqlite { .. }));
    }

    #[test]
    fn parse_args_db_memory_sets_in_memory() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "memory".to_owned(),
        ]);
        assert!(
            matches!(c.storage, StorageBackend::InMemory),
            "--db memory must select in-memory store"
        );
    }

    #[test]
    fn resolve_storage_picks_up_database_url() {
        let mut c = parse_args_from(&["cairn-app".to_owned()]);
        assert!(matches!(c.storage, StorageBackend::InMemory));
        // Simulate DATABASE_URL being set
        std::env::set_var("DATABASE_URL", "postgres://cairn:pass@localhost/cairn");
        resolve_storage_from_env(&mut c);
        assert!(
            matches!(c.storage, StorageBackend::Postgres { .. }),
            "DATABASE_URL must be picked up when no --db flag"
        );
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn resolve_storage_db_flag_wins_over_database_url() {
        std::env::set_var("DATABASE_URL", "postgres://ignored@localhost/db");
        let mut c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--db".to_owned(),
            "my.db".to_owned(),
        ]);
        resolve_storage_from_env(&mut c);
        assert!(
            matches!(c.storage, StorageBackend::Sqlite { .. }),
            "--db flag must take precedence over DATABASE_URL"
        );
        std::env::remove_var("DATABASE_URL");
    }

    #[test]
    fn team_mode_clears_local_auto_encryption() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--mode".to_owned(),
            "team".to_owned(),
        ]);
        assert!(!c.credentials_available());
    }

    #[test]
    fn parse_args_port_flag_overrides_default() {
        let c = parse_args_from(&[
            "cairn-app".to_owned(),
            "--port".to_owned(),
            "8080".to_owned(),
        ]);
        assert_eq!(c.listen_port, 8080);
    }

    // ── SSE stream tests ──────────────────────────────────────────────────────

    /// Drive the SSE stream from an HTTP request using tower's oneshot and
    /// collect the first N bytes of the SSE body.
    async fn collect_sse_bytes(
        app: axum::Router,
        uri: &str,
        extra_headers: Vec<(&str, &str)>,
    ) -> Vec<u8> {
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
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_sends_connected_event_on_connect() {
        let app = make_app(make_state());
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);
        assert!(
            text.contains("event: connected"),
            "missing connected event; got: {text}"
        );
        assert!(
            text.contains("head_position"),
            "connected payload missing head_position; got: {text}"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
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
    #[ignore = "router unification: covered by integration tests"]
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
    #[ignore = "router unification: covered by integration tests"]
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
        assert_eq!(
            count, 2,
            "expected 2 replayed events; got {count} in: {text}"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn stream_empty_store_sends_only_connected() {
        let app = make_app(make_state());
        let raw = collect_sse_bytes(app, "/v1/stream", vec![]).await;
        let text = String::from_utf8_lossy(&raw);

        // Only the connected event, no session_created events.
        assert!(text.contains("event: connected"));
        assert!(
            !text.contains("event: session_created"),
            "unexpected events: {text}"
        );
    }

    // ── Integration-style route tests ──

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn get_runs_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/runs")).await.unwrap();
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
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_tasks_returns_empty_for_run_with_no_tasks() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent};

        let state = make_state();
        let project = ProjectKey::new("t_task", "w_task", "p_task");
        let session_id = SessionId::new("sess_task_empty");
        let run_id = cairn_domain::RunId::new("run_notasks");

        // Create session + run but add no tasks.
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
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
            )])
            .await
            .unwrap();

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

        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "run with no tasks returns 200"
        );
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let tasks: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            tasks.as_array().unwrap().is_empty(),
            "no tasks = empty array"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_tasks_returns_tasks_for_run() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent, TaskCreated,
        };

        let state = make_state();
        let project = ProjectKey::new("t_tasks", "w_tasks", "p_tasks");
        let session_id = SessionId::new("sess_tasks");
        let run_id = cairn_domain::RunId::new("run_withtasks");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();

        // Create run + two tasks.
        state
            .runtime
            .store
            .append(&[
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
            ])
            .await
            .unwrap();

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
        assert!(
            task_ids.contains(&"task_alpha"),
            "task_alpha must be in response"
        );
        assert!(
            task_ids.contains(&"task_beta"),
            "task_beta must be in response"
        );
        // Each task must link back to the run.
        for t in arr {
            assert_eq!(
                t["parent_run_id"], "run_withtasks",
                "every task must reference its parent run"
            );
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
        assert_eq!(
            resp.status(),
            StatusCode::NOT_FOUND,
            "unknown run must return 404"
        );
    }

    // ── Approval endpoint tests ──────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_approvals_empty_for_run_with_no_approvals() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent};

        let state = make_state();
        let project = ProjectKey::new("ta", "wa", "pa");
        let session_id = SessionId::new("sess_appr_empty");
        let run_id_str = "run_appr_empty";
        let run_id = cairn_domain::RunId::new(run_id_str);

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
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
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}/approvals"))
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            approvals.as_array().unwrap().is_empty(),
            "run with no approvals must return empty array"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_approvals_shows_pending_approval() {
        use cairn_domain::policy::ApprovalRequirement;
        use cairn_domain::{
            ApprovalId, ApprovalRequested, EventEnvelope, EventId, EventSource, RunCreated,
            RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tb", "wb", "pb");
        let session_id = SessionId::new("sess_appr_pend");
        let run_id_str = "run_appr_pend";
        let run_id = cairn_domain::RunId::new(run_id_str);
        let approval_id = ApprovalId::new("appr_pend");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[
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
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}/approvals"))
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
        assert!(
            arr[0]["decision"].is_null(),
            "pending approval has no decision"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_run_approvals_shows_resolved_decision() {
        use cairn_domain::policy::{ApprovalDecision, ApprovalRequirement};
        use cairn_domain::{
            ApprovalId, ApprovalRequested, ApprovalResolved, EventEnvelope, EventId, EventSource,
            RunCreated, RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tc", "wc", "pc");
        let session_id = SessionId::new("sess_appr_res");
        let run_id_str = "run_appr_res";
        let run_id = cairn_domain::RunId::new(run_id_str);
        let approval_id = ApprovalId::new("appr_res");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .store
            .append(&[
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
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}/approvals"))
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
        assert_eq!(
            arr[0]["decision"], "approved",
            "resolved approval must carry the decision"
        );
    }

    // ── Session runs endpoint tests ──────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_session_runs_empty_for_session_with_no_runs() {
        use cairn_domain::{EventEnvelope, EventId, EventSource, RuntimeEvent, SessionCreated};

        let state = make_state();
        let project = ProjectKey::new("tr1", "wr1", "pr1");
        let session_id = SessionId::new("sess_noruns");

        // Create session via event but add no runs.
        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("evt_sess_nr"),
                EventSource::Runtime,
                RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                }),
            )])
            .await
            .unwrap();

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
        assert!(
            runs.as_array().unwrap().is_empty(),
            "session with no runs must return empty array"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn list_session_runs_returns_two_runs() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent, SessionCreated,
        };

        let state = make_state();
        let project = ProjectKey::new("tr2", "wr2", "pr2");
        let session_id = SessionId::new("sess_tworuns");

        state
            .runtime
            .store
            .append(&[
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
            ])
            .await
            .unwrap();

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
    #[ignore = "router unification: covered by integration tests"]
    async fn list_session_runs_shows_parent_run_id_for_subagent() {
        use cairn_domain::{
            EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent, SessionCreated,
        };

        let state = make_state();
        let project = ProjectKey::new("tr3", "wr3", "pr3");
        let session_id = SessionId::new("sess_subagent");

        state
            .runtime
            .store
            .append(&[
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
            ])
            .await
            .unwrap();

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
        assert!(root["parent_run_id"].is_null(), "root run has no parent");
        assert_eq!(root["agent_role_id"], "orchestrator");

        let sub = arr.iter().find(|r| r["run_id"] == "run_subagent").unwrap();
        assert_eq!(
            sub["parent_run_id"], "run_root",
            "subagent must reference root run as parent"
        );
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
    #[ignore = "router unification: covered by integration tests"]
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
    #[ignore = "router unification: covered by integration tests"]
    async fn runs_list_reflects_created_run() {
        let state = make_state();
        let project = ProjectKey::new("t1", "w1", "p1");
        let session_id = cairn_domain::SessionId::new("sess_1");
        let run_id = cairn_domain::RunId::new("run_1");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id, None)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/runs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(runs.as_array().unwrap().len(), 1);
        assert_eq!(runs[0]["run_id"], "run_1");
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn get_run_by_id_returns_record() {
        let state = make_state();
        let project = ProjectKey::new("t2", "w2", "p2");
        let session_id = SessionId::new("sess_2");
        let run_id_str = "run_2";
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &session_id,
                cairn_domain::RunId::new(run_id_str),
                None,
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/runs/{run_id_str}"))
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
    #[ignore = "router unification: covered by integration tests"]
    async fn sessions_list_reflects_created_session() {
        let state = make_state();
        let project = ProjectKey::new("t3", "w3", "p3");
        let session_id = SessionId::new("sess_3");
        state
            .runtime
            .sessions
            .create(&project, session_id)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/sessions")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(sessions.as_array().unwrap().len(), 1);
        assert_eq!(sessions[0]["session_id"], "sess_3");
    }

    // ── Prompt asset / release tests ──────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn prompt_assets_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/prompts/assets")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn prompt_assets_reflects_created_asset() {
        use cairn_domain::PromptAssetId;
        use cairn_runtime::prompt_assets::PromptAssetService as _;

        let state = make_state();
        let project = ProjectKey::new("ta", "wa", "pa");
        state
            .runtime
            .prompt_assets
            .create(
                &project,
                PromptAssetId::new("asset_a"),
                "My Prompt".to_owned(),
                "system".to_owned(),
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/prompts/assets")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["prompt_asset_id"], "asset_a");
        assert_eq!(items[0]["name"], "My Prompt");
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
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
    #[ignore = "router unification: covered by integration tests"]
    async fn costs_empty_store_returns_zeros() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/costs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_provider_calls"], 0);
        assert_eq!(cost["total_cost_micros"], 0);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn costs_reflects_run_cost_events() {
        use cairn_domain::{
            events::RunCostUpdated, EventEnvelope, EventId, EventSource, RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tc", "wc", "pc");
        let session_id = SessionId::new("sess_c");
        let run_id = cairn_domain::RunId::new("run_c");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
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
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/costs")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_cost_micros"], 500);
        assert_eq!(cost["total_tokens_in"], 100);
        assert_eq!(cost["total_tokens_out"], 50);
    }

    // ── Provider tests ────────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn providers_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/providers")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(items.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn providers_reflects_created_binding() {
        use cairn_domain::{
            events::ProviderBindingCreated,
            providers::{OperationKind, ProviderBindingSettings},
            EventEnvelope, EventId, EventSource, ProviderBindingId, ProviderConnectionId,
            ProviderModelId, RuntimeEvent,
        };

        let state = make_state();
        let project = ProjectKey::new("tp", "wp", "pp");

        state
            .runtime
            .store
            .append(&[EventEnvelope::for_runtime_event(
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
            )])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/providers")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let items: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(items.as_array().unwrap().len(), 1);
        assert_eq!(items[0]["provider_binding_id"], "bind_p");
    }

    // ── Event replay tests ────────────────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn events_empty_store_returns_empty_list() {
        let app = make_app(make_state());
        let resp = app.oneshot(authed_req("/v1/events")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(events.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn events_returns_all_events_from_log() {
        let state = make_state();
        let project = ProjectKey::new("te", "we", "pe");
        let session_id = SessionId::new("sess_e");
        // Creating a session appends a SessionCreated event.
        state
            .runtime
            .sessions
            .create(&project, session_id)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app.oneshot(authed_req("/v1/events")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        assert!(!arr.is_empty(), "expected at least one event");
        assert_eq!(arr[0]["event_type"], "session_created");
        assert!(arr[0]["position"].as_u64().is_some());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
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
    fn session_created_with_causation(
        event_id: &str,
        session_id: &str,
        causation_id: &str,
    ) -> serde_json::Value {
        let mut e = session_created_envelope(event_id, session_id);
        e["causation_id"] = serde_json::json!(causation_id);
        e
    }

    async fn post_append(
        app: axum::Router,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
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
        assert!(
            arr[0]["position"].as_u64().unwrap() > 0,
            "position must be ≥ 1"
        );
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

        let positions: Vec<u64> = arr
            .iter()
            .map(|r| r["position"].as_u64().unwrap())
            .collect();
        // All positions must be distinct and strictly increasing.
        assert!(positions[0] < positions[1]);
        assert!(positions[1] < positions[2]);
        assert!(arr.iter().all(|r| r["appended"] == true));
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn append_idempotent_with_causation_id_returns_existing_position() {
        let state = make_state();
        let causation = "cmd_idem_1";

        // First append — creates the event.
        let env = session_created_with_causation("evt_idem1", "sess_idem1", causation);
        let (status1, res1) =
            post_append(make_app(state.clone()), serde_json::json!([env.clone()])).await;
        assert_eq!(status1, StatusCode::CREATED);
        let first_pos = res1[0]["position"].as_u64().unwrap();
        assert_eq!(res1[0]["appended"], true);

        // Second append — same causation_id, different event_id.
        let env2 = session_created_with_causation("evt_idem2", "sess_idem2", causation);
        let (status2, res2) = post_append(make_app(state.clone()), serde_json::json!([env2])).await;
        assert_eq!(status2, StatusCode::CREATED);
        let second_pos = res2[0]["position"].as_u64().unwrap();
        assert_eq!(
            res2[0]["appended"], false,
            "second append should be idempotent"
        );
        assert_eq!(
            second_pos, first_pos,
            "idempotent append must return the original position"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
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
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        let arr = results.as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(
            arr[0]["appended"], false,
            "first should be idempotent duplicate"
        );
        assert_eq!(arr[1]["appended"], true, "second should be newly appended");
        assert!(
            arr[1]["position"].as_u64().unwrap() > arr[0]["position"].as_u64().unwrap(),
            "new event position must be greater than duplicate's original position"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn append_event_appears_in_event_log_immediately() {
        let state = make_state();
        let app1 = make_app(state.clone());
        let app2 = make_app(state.clone());

        // Append one event.
        let env = session_created_envelope("evt_vis1", "sess_vis1");
        let (_, results) = post_append(app1, serde_json::json!([env])).await;
        let appended_pos = results[0]["position"].as_u64().unwrap();

        // The event should now appear in GET /v1/events.
        let resp = app2.oneshot(authed_req("/v1/events")).await.unwrap();
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
    #[ignore = "router unification: covered by integration tests"]
    async fn append_broadcasts_to_sse_subscribers() {
        let state = make_state();
        // Subscribe to the broadcast channel BEFORE appending.
        let mut receiver = state.runtime.store.subscribe();

        // Append one event via the handler.
        let env = session_created_envelope("evt_bc1", "sess_bc1");
        let app = make_app(state.clone());
        let (status, _) = post_append(app, serde_json::json!([env])).await;
        assert_eq!(status, StatusCode::CREATED);

        // The receiver should have gotten the event immediately.
        let received = tokio::time::timeout(std::time::Duration::from_millis(200), async {
            receiver.recv().await
        })
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
    #[ignore = "router unification: covered by integration tests"]
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
    #[ignore = "router unification: covered by integration tests"]
    async fn health_endpoint_requires_no_token() {
        // /health is public — no Authorization header needed.
        let resp = unauthed_get(make_app(make_state()), "/health").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let h: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(h["ok"], true);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
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
        let doc_store = std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
        let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
            doc_store.clone(),
        ));
        let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
            doc_store.clone(),
            cairn_memory::pipeline::ParagraphChunker {
                max_chunk_size: 512,
            },
        ));
        let state = AppState {
            runtime: Arc::new(InMemoryServices::new()),
            started_at: Arc::new(Instant::now()),
            tokens,
            pg: None,
            sqlite: None,
            mode: DeploymentMode::Local,
            document_store: doc_store,
            retrieval,
            ingest,
            ollama: None,
            openai_compat_brain: None,
            openai_compat_worker: None,
            openai_compat_openrouter: None,
            openai_compat: None,
            metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
            rate_limits: Arc::new(Mutex::new(HashMap::new())),
            request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
            notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
            templates: Arc::new(templates::TemplateRegistry::with_builtins()),
            entitlements: Arc::new(entitlements::EntitlementService::new()),
            bedrock: None,
            process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
        };
        let app = make_app(state);

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
        assert!(status.get("backend").is_some(), "missing backend");
        assert!(status.get("connected").is_some(), "missing connected");
        assert!(
            status.get("migration_count").is_some(),
            "missing migration_count"
        );
        assert!(
            status.get("schema_current").is_some(),
            "missing schema_current"
        );
    }

    // ── End-to-end write → project → read cycle tests ────────────────────────
    //
    // These five tests prove the full pipeline:
    //   POST /v1/events/append → InMemory synchronous projection → GET read endpoint
    // Each test uses only the HTTP surface so they exercise exactly what a real
    // client would do.

    /// (1) POST SessionCreated via /v1/events/append → GET /v1/sessions shows it.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_append_session_then_list_sessions_shows_it() {
        let state = make_state();
        let envelope = session_created_envelope("evt_e2e_s1", "sess_e2e_1");
        let (status, results) =
            post_append(make_app(state.clone()), serde_json::json!([envelope])).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            results[0]["appended"], true,
            "event must be freshly appended"
        );

        let resp = make_app(state)
            .oneshot(authed_req("/v1/sessions"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let sessions: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = sessions.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one session must appear after append");
        assert_eq!(
            arr[0]["session_id"], "sess_e2e_1",
            "session_id must match what GET /v1/sessions returns"
        );
    }

    /// (2) POST RunCreated via /v1/events/append → GET /v1/runs shows it.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_append_run_then_list_runs_shows_it() {
        let state = make_state();
        let proj =
            serde_json::json!({"tenant_id":"t_e2e","workspace_id":"w_e2e","project_id":"p_e2e"});
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
        let (status, results) =
            post_append(make_app(state.clone()), serde_json::json!([run_env])).await;
        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(results[0]["appended"], true);

        let resp = make_app(state)
            .oneshot(authed_req("/v1/runs"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let runs: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = runs.as_array().unwrap();
        assert_eq!(arr.len(), 1, "one run must appear after append");
        assert_eq!(
            arr[0]["run_id"], "run_e2e_1",
            "run_id must match what GET /v1/runs returns"
        );
    }

    /// (3) POST ApprovalRequested → GET /v1/approvals/pending shows it.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_append_approval_then_list_pending_shows_it() {
        let state = make_state();
        let proj =
            serde_json::json!({"tenant_id":"t_ap","workspace_id":"w_ap","project_id":"p_ap"});
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
        let (status, _) =
            post_append(make_app(state.clone()), serde_json::json!([approval_env])).await;
        assert_eq!(status, StatusCode::CREATED);

        let resp = make_app(state)
            .oneshot(authed_req(
                "/v1/approvals/pending?tenant_id=t_ap&workspace_id=w_ap&project_id=p_ap",
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let approvals: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = approvals.as_array().unwrap();
        assert_eq!(
            arr.len(),
            1,
            "one pending approval must appear after append"
        );
        assert_eq!(arr[0]["approval_id"], "appr_e2e_1");
        assert!(
            arr[0]["decision"].is_null(),
            "pending approval must have null decision"
        );
    }

    /// (4) POST ApprovalRequested then POST /v1/approvals/:id/resolve(Approved)
    /// → GET /v1/approvals/pending is empty.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_resolve_approval_removes_from_pending() {
        let state = make_state();
        let proj =
            serde_json::json!({"tenant_id":"t_res","workspace_id":"w_res","project_id":"p_res"});
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
            .oneshot(authed_post(
                "/v1/approvals/appr_e2e_res/resolve",
                serde_json::json!({"decision": "approved"}),
            ))
            .await
            .unwrap();
        assert_eq!(
            resolve_resp.status(),
            StatusCode::OK,
            "resolve must return 200"
        );
        let rbody = to_bytes(resolve_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let resolved: serde_json::Value = serde_json::from_slice(&rbody).unwrap();
        assert_eq!(
            resolved["decision"], "approved",
            "resolved approval must carry decision=approved"
        );

        let resp = make_app(state)
            .oneshot(authed_req(
                "/v1/approvals/pending?tenant_id=t_res&workspace_id=w_res&project_id=p_res",
            ))
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let pending: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            pending.as_array().unwrap().is_empty(),
            "pending list must be empty after approval resolved"
        );
    }

    /// (5) Append session + run, then GET /v1/dashboard shows active_runs=1.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn e2e_dashboard_active_runs_reflects_appended_run() {
        let state = make_state();

        let resp0 = make_app(state.clone())
            .oneshot(authed_req("/v1/dashboard"))
            .await
            .unwrap();
        let body0 = to_bytes(resp0.into_body(), usize::MAX).await.unwrap();
        let dash0: serde_json::Value = serde_json::from_slice(&body0).unwrap();
        assert_eq!(
            dash0["active_runs"], 0,
            "dashboard must start with 0 active runs"
        );

        let proj =
            serde_json::json!({"tenant_id":"t_dash","workspace_id":"w_dash","project_id":"p_dash"});
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

        let resp1 = make_app(state)
            .oneshot(authed_req("/v1/dashboard"))
            .await
            .unwrap();
        assert_eq!(resp1.status(), StatusCode::OK);
        let body1 = to_bytes(resp1.into_body(), usize::MAX).await.unwrap();
        let dash1: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        assert_eq!(
            dash1["active_runs"], 1,
            "dashboard must show active_runs=1 after appending one RunCreated"
        );
        assert!(
            dash1["system_healthy"].as_bool().unwrap_or(false),
            "system must be healthy"
        );
    }

    // ── CORS tests ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn options_preflight_returns_cors_headers() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/v1/events/append")
                    .header("origin", "http://localhost:5173")
                    .header("access-control-request-method", "POST")
                    .header(
                        "access-control-request-headers",
                        "content-type,authorization",
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(
            resp.status().is_success(),
            "OPTIONS preflight must succeed; got {}",
            resp.status()
        );
        let h = resp.headers();
        assert!(
            h.contains_key("access-control-allow-origin"),
            "missing ACAO header"
        );
        assert!(
            h.contains_key("access-control-allow-methods"),
            "missing ACAM header"
        );
        assert!(
            h.contains_key("access-control-allow-headers"),
            "missing ACAH header"
        );
    }

    #[tokio::test]
    async fn cors_allow_origin_is_wildcard() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/health")
                    .header("origin", "https://example.com")
                    .header("access-control-request-method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let acao = resp
            .headers()
            .get("access-control-allow-origin")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(acao, "*", "allow_origin must be wildcard (*)");
    }

    #[tokio::test]
    async fn regular_get_includes_cors_header() {
        let resp = authed_get(make_app(make_state()), "/v1/status").await;
        let acao = resp.headers().get("access-control-allow-origin");
        assert!(
            acao.is_some(),
            "GET response must include Access-Control-Allow-Origin"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn cors_allow_methods_includes_required_verbs() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/v1/events/append")
                    .header("origin", "http://localhost:3000")
                    .header("access-control-request-method", "POST")
                    .header("access-control-request-headers", "authorization")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let methods = resp
            .headers()
            .get("access-control-allow-methods")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_uppercase();
        for verb in ["GET", "POST", "PUT", "DELETE", "OPTIONS"] {
            assert!(
                methods.contains(verb),
                "Access-Control-Allow-Methods must include {verb}; got: {methods}"
            );
        }
    }

    // ── GET /v1/sessions/:id/events tests ────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_empty_for_unknown_session() {
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/sessions/no_such_session/events").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(events.as_array().unwrap().is_empty());
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_returns_events_for_session() {
        let state = make_state();
        let project = ProjectKey::new("t_sev", "w_sev", "p_sev");
        let session_id = SessionId::new("sess_sev");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();

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
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_scoped_to_session_only() {
        let state = make_state();
        let project = ProjectKey::new("t_scope", "w_scope", "p_scope");
        // Create two sessions — each gets a SessionCreated event.
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_scope_a"))
            .await
            .unwrap();
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_scope_b"))
            .await
            .unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/sessions/sess_scope_a/events").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();
        // Only sess_scope_a events must appear — not sess_scope_b.
        assert_eq!(
            arr.len(),
            1,
            "only one SessionCreated event for sess_scope_a"
        );
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn session_events_after_cursor_paginates() {
        use cairn_domain::{
            events::SessionStateChanged, events::StateTransition as ST, tenancy::OwnershipKey,
            EventEnvelope, EventId, EventSource,
        };

        let state = make_state();
        let project = ProjectKey::new("t_cur", "w_cur", "p_cur");
        let session_id = SessionId::new("sess_cur");
        // SessionCreated → event 1 (session-scoped).
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        // Append a SessionStateChanged directly → event 2 (also session-scoped).
        state
            .runtime
            .store
            .append(&[EventEnvelope::new(
                EventId::new("evt_ssc_cur"),
                EventSource::Runtime,
                OwnershipKey::Project(project.clone()),
                cairn_domain::RuntimeEvent::SessionStateChanged(SessionStateChanged {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    transition: ST {
                        from: Some(cairn_domain::SessionState::Open),
                        to: cairn_domain::SessionState::Completed,
                    },
                }),
            )])
            .await
            .unwrap();

        let app_all = make_app(state.clone());
        let app_page = make_app(state.clone());

        let resp_all = authed_get(app_all, "/v1/sessions/sess_cur/events").await;
        let body_all = to_bytes(resp_all.into_body(), usize::MAX).await.unwrap();
        let all: serde_json::Value = serde_json::from_slice(&body_all).unwrap();
        let all_arr = all.as_array().unwrap();
        assert!(
            all_arr.len() >= 2,
            "expect session_created + session_state_changed"
        );

        // Use the first event position as cursor.
        let first_pos = all_arr[0]["position"].as_u64().unwrap();
        let uri = format!("/v1/sessions/sess_cur/events?after={first_pos}");
        let resp_page = authed_get(app_page, &uri).await;
        let body_page = to_bytes(resp_page.into_body(), usize::MAX).await.unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body_page).unwrap();
        let page_arr = page.as_array().unwrap();
        assert_eq!(
            page_arr.len(),
            all_arr.len() - 1,
            "after=first_pos must return one fewer event"
        );
        assert!(page_arr
            .iter()
            .all(|e| e["position"].as_u64().unwrap() > first_pos));
    }

    // ── GET /v1/runs/:id/cost tests ──────────────────────────────────────────

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_returns_zeros_when_no_provider_calls() {
        let state = make_state();
        let project = ProjectKey::new("t_cost", "w_cost", "p_cost");
        let session_id = SessionId::new("sess_cost");
        let run_id = cairn_domain::RunId::new("run_cost_zero");
        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id, None)
            .await
            .unwrap();

        let app = make_app(state);
        let resp = authed_get(app, "/v1/runs/run_cost_zero/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["run_id"], "run_cost_zero");
        assert_eq!(cost["total_cost_micros"], 0);
        assert_eq!(cost["total_tokens_in"], 0);
        assert_eq!(cost["total_tokens_out"], 0);
        assert_eq!(cost["provider_calls"], 0);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_returns_zeros_for_unknown_run() {
        // Unknown run → no cost record → zero response (not 404).
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/runs/nonexistent_run_cost/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["total_cost_micros"], 0);
        assert_eq!(cost["provider_calls"], 0);
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_reflects_run_cost_updated_events() {
        use cairn_domain::{
            events::RunCostUpdated, tenancy::OwnershipKey, EventEnvelope, EventId, EventSource,
            TenantId,
        };

        let state = make_state();
        let project = ProjectKey::new("t_rcu", "w_rcu", "p_rcu");
        let session_id = SessionId::new("sess_rcu");
        let run_id = cairn_domain::RunId::new("run_rcu");

        state
            .runtime
            .sessions
            .create(&project, session_id.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &session_id, run_id.clone(), None)
            .await
            .unwrap();

        // Two provider calls: 300 + 200 micros, 50+30 tokens in, 20+10 tokens out.
        for (i, (cost, t_in, t_out)) in [(300u64, 50u64, 20u64), (200, 30, 10)].iter().enumerate() {
            state
                .runtime
                .store
                .append(&[EventEnvelope::new(
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
                )])
                .await
                .unwrap();
        }

        let app = make_app(state);
        let resp = authed_get(app, "/v1/runs/run_rcu/cost").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(cost["run_id"], "run_rcu");
        assert_eq!(cost["total_cost_micros"], 500, "300+200 micros");
        assert_eq!(cost["total_tokens_in"], 80, "50+30 tokens in");
        assert_eq!(cost["total_tokens_out"], 30, "20+10 tokens out");
        assert_eq!(cost["provider_calls"], 2, "2 provider calls");
    }

    #[tokio::test]
    async fn provider_connection_generate_roundtrip_invalidates_to_static_fallback() {
        let static_url = spawn_openai_compat_mock("static").await;
        let dynamic_url = spawn_openai_compat_mock("dynamic").await;

        let mut state = make_state();
        state.openai_compat_worker = Some(Arc::new(
            OpenAiCompat::new(
                ProviderConfig::default(),
                "static-key",
                Some(static_url),
                Some("gpt-4o-mini".to_owned()),
                None,
                None,
                None,
            )
            .expect("static fallback provider should build"),
        ));
        state.openai_compat = state.openai_compat_worker.clone();

        let app = make_app(state);

        let credential_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/admin/tenants/default_tenant/credentials",
            serde_json::json!({
                "provider_id": "openai",
                "plaintext_value": "dynamic-key",
            }),
        )
        .await;
        assert_eq!(credential_resp.status(), StatusCode::CREATED);
        let credential_body = to_bytes(credential_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let credential_json: serde_json::Value = serde_json::from_slice(&credential_body).unwrap();
        let credential_id = credential_json["id"]
            .as_str()
            .expect("credential id")
            .to_owned();

        let create_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/connections",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "provider_connection_id": "conn_dynamic",
                "provider_family": "openai",
                "adapter_type": "openai_compat",
                "supported_models": ["gpt-4o-mini"],
                "credential_id": credential_id,
                "endpoint_url": dynamic_url,
            }),
        )
        .await;
        assert_eq!(create_resp.status(), StatusCode::CREATED);

        let dynamic_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/ollama/generate",
            serde_json::json!({
                "model": "gpt-4o-mini",
                "prompt": "hello from dynamic",
            }),
        )
        .await;
        let dynamic_status = dynamic_resp.status();
        let dynamic_body = to_bytes(dynamic_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            dynamic_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&dynamic_body)
        );
        let dynamic_json: serde_json::Value = serde_json::from_slice(&dynamic_body).unwrap();
        assert_eq!(dynamic_json["text"], "dynamic");

        let delete_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri("/v1/providers/connections/conn_dynamic")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_resp.status(), StatusCode::OK);

        let fallback_resp = authed_json(
            app,
            axum::http::Method::POST,
            "/v1/providers/ollama/generate",
            serde_json::json!({
                "model": "gpt-4o-mini",
                "prompt": "hello from fallback",
            }),
        )
        .await;
        let fallback_status = fallback_resp.status();
        let fallback_body = to_bytes(fallback_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            fallback_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&fallback_body)
        );
        let fallback_json: serde_json::Value = serde_json::from_slice(&fallback_body).unwrap();
        assert_eq!(fallback_json["text"], "static");
    }

    #[tokio::test]
    async fn provider_connection_embed_roundtrip_invalidates_to_static_fallback() {
        let static_url =
            spawn_openai_compat_embedding_mock("embed-dynamic", vec![0.9, 0.8], 11).await;
        let dynamic_url =
            spawn_openai_compat_embedding_mock("embed-dynamic", vec![0.1, 0.2], 7).await;

        let mut state = make_state();
        state.openai_compat_worker = Some(Arc::new(
            OpenAiCompat::new(
                ProviderConfig::default(),
                "static-key",
                Some(static_url),
                Some("embed-dynamic".to_owned()),
                None,
                None,
                None,
            )
            .expect("static embedding fallback provider should build"),
        ));
        state.openai_compat = state.openai_compat_worker.clone();

        let app = make_app(state);

        let credential_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/admin/tenants/default_tenant/credentials",
            serde_json::json!({
                "provider_id": "openai",
                "plaintext_value": "dynamic-key",
            }),
        )
        .await;
        assert_eq!(credential_resp.status(), StatusCode::CREATED);
        let credential_body = to_bytes(credential_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let credential_json: serde_json::Value = serde_json::from_slice(&credential_body).unwrap();
        let credential_id = credential_json["id"]
            .as_str()
            .expect("credential id")
            .to_owned();

        let create_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/connections",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "provider_connection_id": "conn_embed_dynamic",
                "provider_family": "openai",
                "adapter_type": "openai_compat",
                "supported_models": ["embed-dynamic"],
                "credential_id": credential_id,
                "endpoint_url": dynamic_url,
            }),
        )
        .await;
        assert_eq!(create_resp.status(), StatusCode::CREATED);

        let dynamic_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/memory/embed",
            serde_json::json!({
                "model": "embed-dynamic",
                "texts": ["hello registry"],
            }),
        )
        .await;
        let dynamic_status = dynamic_resp.status();
        let dynamic_body = to_bytes(dynamic_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            dynamic_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&dynamic_body)
        );
        let dynamic_json: serde_json::Value = serde_json::from_slice(&dynamic_body).unwrap();
        assert_eq!(dynamic_json["model"], "embed-dynamic");
        assert_eq!(dynamic_json["token_count"], 7);
        assert_embedding_matches(&dynamic_json["embeddings"][0], &[0.1, 0.2]);

        let delete_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri("/v1/providers/connections/conn_embed_dynamic")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_resp.status(), StatusCode::OK);

        let fallback_resp = authed_json(
            app,
            axum::http::Method::POST,
            "/v1/memory/embed",
            serde_json::json!({
                "model": "embed-dynamic",
                "texts": ["hello fallback"],
            }),
        )
        .await;
        let fallback_status = fallback_resp.status();
        let fallback_body = to_bytes(fallback_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            fallback_status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&fallback_body)
        );
        let fallback_json: serde_json::Value = serde_json::from_slice(&fallback_body).unwrap();
        assert_eq!(fallback_json["model"], "embed-dynamic");
        assert_eq!(fallback_json["token_count"], 11);
        assert_embedding_matches(&fallback_json["embeddings"][0], &[0.9, 0.8]);
    }

    #[tokio::test]
    async fn provider_connection_stream_roundtrip_invalidates_to_static_fallback() {
        let static_url = spawn_openai_compat_stream_mock(vec!["static stream"]).await;
        let dynamic_url = spawn_openai_compat_stream_mock(vec!["dynamic stream"]).await;

        let mut state = make_state();
        state.openai_compat_openrouter = Some(Arc::new(
            OpenAiCompat::new(
                ProviderConfig::OPENROUTER,
                "static-key",
                Some(static_url),
                Some("openrouter/free".to_owned()),
                None,
                None,
                None,
            )
            .expect("static stream fallback provider should build"),
        ));
        state.openai_compat = state.openai_compat_openrouter.clone();

        let app = make_app(state);

        let credential_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/admin/tenants/default_tenant/credentials",
            serde_json::json!({
                "provider_id": "openrouter",
                "plaintext_value": "dynamic-key",
            }),
        )
        .await;
        assert_eq!(credential_resp.status(), StatusCode::CREATED);
        let credential_body = to_bytes(credential_resp.into_body(), usize::MAX)
            .await
            .unwrap();
        let credential_json: serde_json::Value = serde_json::from_slice(&credential_body).unwrap();
        let credential_id = credential_json["id"]
            .as_str()
            .expect("credential id")
            .to_owned();

        let create_resp = authed_json(
            app.clone(),
            axum::http::Method::POST,
            "/v1/providers/connections",
            serde_json::json!({
                "tenant_id": "default_tenant",
                "provider_connection_id": "conn_stream_dynamic",
                "provider_family": "openrouter",
                "adapter_type": "openrouter",
                "supported_models": ["openrouter/free"],
                "credential_id": credential_id,
                "endpoint_url": dynamic_url,
            }),
        )
        .await;
        assert_eq!(create_resp.status(), StatusCode::CREATED);

        let dynamic_sse = authed_sse_post(
            app.clone(),
            "/v1/chat/stream",
            serde_json::json!({
                "model": "openrouter/free",
                "prompt": "hello stream",
            }),
        )
        .await;
        assert!(dynamic_sse.contains("event: token"));
        assert!(dynamic_sse.contains("data:"));
        assert!(dynamic_sse.contains("dynamic stream"));
        assert!(dynamic_sse.contains("event: done"));

        let delete_resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(axum::http::Method::DELETE)
                    .uri("/v1/providers/connections/conn_stream_dynamic")
                    .header("authorization", format!("Bearer {TEST_TOKEN}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_resp.status(), StatusCode::OK);

        let fallback_sse = authed_sse_post(
            app,
            "/v1/chat/stream",
            serde_json::json!({
                "model": "openrouter/free",
                "prompt": "hello fallback stream",
            }),
        )
        .await;
        assert!(fallback_sse.contains("event: token"));
        assert!(fallback_sse.contains("data:"));
        assert!(fallback_sse.contains("static stream"));
        assert!(fallback_sse.contains("event: done"));
    }

    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_cost_response_has_correct_shape() {
        let app = make_app(make_state());
        let resp = authed_get(app, "/v1/runs/any_run/cost").await;
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let cost: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // All four contract fields must be present.
        for field in [
            "run_id",
            "total_cost_micros",
            "total_tokens_in",
            "total_tokens_out",
            "provider_calls",
        ] {
            assert!(cost.get(field).is_some(), "missing field: {field}");
        }
    }
}

#[cfg(test)]
mod run_events_tests {
    use super::test_make_app as make_app;
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
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new(
                    "tenant_re",
                )),
            },
        );
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(std::time::Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
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
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_unknown_run_returns_empty() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/runs/no_such_run/events"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            events.as_array().unwrap().is_empty(),
            "unknown run must return empty event list"
        );
    }

    /// GET /v1/runs/:id/events returns all events for the run after they are appended.
    ///
    /// Proves the write → project → read cycle for the run event stream:
    /// - POST /v1/events/append with RunCreated
    /// - GET /v1/runs/:id/events returns that event
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_returns_events_for_the_run() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_re");

        // Create a session and run directly in the store.
        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_re_1"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_re_1"),
                RunId::new("run_re_1"),
                None,
            )
            .await
            .unwrap();

        // GET /v1/runs/run_re_1/events must return at least the RunCreated event.
        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_re_1/events"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();

        assert!(
            !arr.is_empty(),
            "run events must not be empty after run is created"
        );

        // Every returned event must carry a position and event_type.
        for event in arr {
            assert!(
                event["position"].as_u64().is_some(),
                "event must have a position"
            );
            assert!(
                !event["event_type"].as_str().unwrap_or("").is_empty(),
                "event must have an event_type"
            );
        }

        // The RunCreated event must appear.
        let has_run_created = arr.iter().any(|e| e["event_type"] == "run_created");
        assert!(
            has_run_created,
            "run_created event must appear in the run event stream"
        );
    }

    /// Cursor-based pagination: after=<position> skips earlier events.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_cursor_pagination_works() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_pg");

        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_pg"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_pg"),
                RunId::new("run_pg"),
                None,
            )
            .await
            .unwrap();

        let app1 = make_app(state.clone());
        let resp_all = app1
            .oneshot(authed_req("/v1/runs/run_pg/events"))
            .await
            .unwrap();
        let body_all = to_bytes(resp_all.into_body(), usize::MAX).await.unwrap();
        let all: serde_json::Value = serde_json::from_slice(&body_all).unwrap();
        let all_arr = all.as_array().unwrap();
        assert!(!all_arr.is_empty(), "must have events");

        let first_pos = all_arr[0]["position"].as_u64().unwrap();

        // After the first event's position, all remaining events are returned.
        let uri = format!("/v1/runs/run_pg/events?after={first_pos}");
        let app2 = make_app(state);
        let resp_page = app2.oneshot(authed_req(&uri)).await.unwrap();
        let body_page = to_bytes(resp_page.into_body(), usize::MAX).await.unwrap();
        let page: serde_json::Value = serde_json::from_slice(&body_page).unwrap();
        let page_arr = page.as_array().unwrap();

        assert_eq!(
            page_arr.len(),
            all_arr.len() - 1,
            "after=first_pos must skip the first event"
        );
        assert!(
            page_arr
                .iter()
                .all(|e| e["position"].as_u64().unwrap() > first_pos),
            "all paginated events must be after the cursor position"
        );
    }

    /// The run event stream is scoped to its run — events from other runs are excluded.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn run_events_are_run_scoped() {
        use cairn_domain::*;

        let state = make_state();
        let project = ProjectKey::new("tenant_re", "ws_re", "proj_sc");

        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_sc"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_sc"),
                RunId::new("run_sc_a"),
                None,
            )
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_sc"),
                RunId::new("run_sc_b"),
                None,
            )
            .await
            .unwrap();

        // Events for run_sc_a must not include run_sc_b events.
        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_sc_a/events"))
            .await
            .unwrap();
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let events: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = events.as_array().unwrap();

        assert!(!arr.is_empty(), "run_sc_a must have events");
        // All returned event_type values should be run-lifecycle types, not b's events.
        // Since event_type is derived from payload, just verify run_created is present once.
        let run_created_count = arr
            .iter()
            .filter(|e| e["event_type"] == "run_created")
            .count();
        assert_eq!(
            run_created_count, 1,
            "exactly one run_created must appear (for run_sc_a, not run_sc_b)"
        );
    }
}

#[cfg(test)]
mod tool_invocations_tests {
    use super::test_make_app as make_app;
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use cairn_domain::{
        policy::ExecutionClass, tool_invocation::ToolInvocationTarget, ProjectKey, RunId,
        SessionId, ToolInvocationId,
    };
    use cairn_runtime::ToolInvocationService as _;
    use tower::ServiceExt as _;

    const TOKEN: &str = "test-tool-inv-token";

    fn make_state() -> AppState {
        let tokens = Arc::new(ServiceTokenRegistry::new());
        tokens.register(
            TOKEN.to_owned(),
            AuthPrincipal::ServiceAccount {
                name: "test-tool-inv".to_owned(),
                tenant: cairn_domain::tenancy::TenantKey::new(cairn_domain::TenantId::new(
                    "tenant_ti",
                )),
            },
        );
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(std::time::Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
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

        state
            .runtime
            .sessions
            .create(&project, SessionId::new("sess_ti_empty"))
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(
                &project,
                &SessionId::new("sess_ti_empty"),
                RunId::new("run_ti_empty"),
                None,
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_ti_empty/tool-invocations"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let records: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            records.as_array().unwrap().is_empty(),
            "run with no tool calls must return empty list"
        );
    }

    /// GET /v1/runs/:id/tool-invocations returns both calls after they are recorded.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn tool_invocations_returns_two_calls_for_run() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");
        let run = RunId::new("run_ti_two");
        let sess = SessionId::new("sess_ti_two");

        state
            .runtime
            .sessions
            .create(&project, sess.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &sess, run.clone(), None)
            .await
            .unwrap();

        // Record two tool calls on the run.
        let target = ToolInvocationTarget::Builtin {
            tool_name: "read_file".to_owned(),
        };
        state
            .runtime
            .tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_1"),
                None,
                Some(run.clone()),
                None,
                target.clone(),
                ExecutionClass::SandboxedProcess,
            )
            .await
            .unwrap();
        state
            .runtime
            .tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_2"),
                None,
                Some(run.clone()),
                None,
                ToolInvocationTarget::Builtin {
                    tool_name: "write_file".to_owned(),
                },
                ExecutionClass::SupervisedProcess,
            )
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/runs/run_ti_two/tool-invocations"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let records: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = records.as_array().unwrap();

        assert_eq!(
            arr.len(),
            2,
            "run must have exactly 2 tool invocation records"
        );

        // Both invocation IDs must be present.
        let inv_ids: Vec<&str> = arr
            .iter()
            .map(|r| r["invocation_id"].as_str().unwrap_or(""))
            .collect();
        assert!(
            inv_ids.contains(&"inv_ti_1"),
            "inv_ti_1 must be in the response"
        );
        assert!(
            inv_ids.contains(&"inv_ti_2"),
            "inv_ti_2 must be in the response"
        );

        // Both are scoped to the run.
        for record in arr {
            assert_eq!(
                record["run_id"].as_str().unwrap_or(""),
                "run_ti_two",
                "all records must be for run_ti_two"
            );
        }
    }

    /// Outcome field reflects the terminal outcome after a call completes.
    ///
    /// Records start with state=requested/started and outcome=null;
    /// after ToolInvocationCompleted the state transitions and outcome is set.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn tool_invocation_outcome_field_reflects_completion() {
        let state = make_state();
        let project = ProjectKey::new("tenant_ti", "ws_ti", "proj_ti");
        let run = RunId::new("run_ti_outcome");
        let sess = SessionId::new("sess_ti_outcome");

        state
            .runtime
            .sessions
            .create(&project, sess.clone())
            .await
            .unwrap();
        state
            .runtime
            .runs
            .start(&project, &sess, run.clone(), None)
            .await
            .unwrap();

        // Start a tool call.
        state
            .runtime
            .tool_invocations
            .record_start(
                &project,
                ToolInvocationId::new("inv_ti_outcome"),
                None,
                Some(run.clone()),
                None,
                ToolInvocationTarget::Builtin {
                    tool_name: "bash".to_owned(),
                },
                ExecutionClass::SupervisedProcess,
            )
            .await
            .unwrap();

        // Before completion: outcome must be null, state is not terminal.
        let app1 = make_app(state.clone());
        let resp1 = app1
            .oneshot(authed_req("/v1/runs/run_ti_outcome/tool-invocations"))
            .await
            .unwrap();
        let body1 = to_bytes(resp1.into_body(), usize::MAX).await.unwrap();
        let before: serde_json::Value = serde_json::from_slice(&body1).unwrap();
        let before_rec = &before.as_array().unwrap()[0];
        assert!(
            before_rec["outcome"].is_null(),
            "outcome must be null before completion"
        );
        let before_state = before_rec["state"].as_str().unwrap_or("");
        assert!(!before_state.is_empty(), "state field must be present");

        // Complete the call with Success.
        state
            .runtime
            .tool_invocations
            .record_completed(
                &project,
                ToolInvocationId::new("inv_ti_outcome"),
                None,
                "bash".to_owned(),
            )
            .await
            .unwrap();

        // After completion: outcome must be "success", state must be "completed".
        let app2 = make_app(state);
        let resp2 = app2
            .oneshot(authed_req("/v1/runs/run_ti_outcome/tool-invocations"))
            .await
            .unwrap();
        let body2 = to_bytes(resp2.into_body(), usize::MAX).await.unwrap();
        let after: serde_json::Value = serde_json::from_slice(&body2).unwrap();
        let after_rec = &after.as_array().unwrap()[0];

        let outcome = after_rec["outcome"].as_str().unwrap_or("<null>");
        assert_eq!(
            outcome, "success",
            "outcome must be 'success' after ToolInvocationCompleted"
        );
        assert_eq!(
            after_rec["state"].as_str().unwrap_or(""),
            "completed",
            "state must be 'completed' after successful completion"
        );
    }

    /// Tool invocations endpoint requires auth.
    #[tokio::test]
    async fn tool_invocations_requires_auth() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/runs/any_run/tool-invocations")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

#[cfg(test)]
mod provider_health_tests {
    use super::test_make_app as make_app;
    use super::*;
    use axum::body::to_bytes;
    use axum::body::Body;
    use axum::http::Request;
    use cairn_domain::{
        events::{ProviderConnectionRegistered, ProviderHealthChecked},
        providers::{
            OperationKind, ProviderBindingSettings, ProviderConnectionStatus, ProviderHealthStatus,
        },
        tenancy::TenantKey,
        EventEnvelope, EventId, EventSource, ProjectKey, ProviderBindingId, ProviderConnectionId,
        ProviderModelId, RuntimeEvent, TenantId,
    };
    use tower::ServiceExt as _;

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
        {
            let doc_store =
                std::sync::Arc::new(cairn_memory::in_memory::InMemoryDocumentStore::new());
            let retrieval = std::sync::Arc::new(cairn_memory::in_memory::InMemoryRetrieval::new(
                doc_store.clone(),
            ));
            let ingest = std::sync::Arc::new(cairn_memory::pipeline::IngestPipeline::new(
                doc_store.clone(),
                cairn_memory::pipeline::ParagraphChunker {
                    max_chunk_size: 512,
                },
            ));
            AppState {
                runtime: Arc::new(InMemoryServices::new()),
                started_at: Arc::new(std::time::Instant::now()),
                tokens,
                pg: None,
                sqlite: None,
                mode: DeploymentMode::Local,
                document_store: doc_store,
                retrieval,
                ingest,
                ollama: None,
                openai_compat_brain: None,
                openai_compat_worker: None,
                openai_compat_openrouter: None,
                openai_compat: None,
                metrics: Arc::new(std::sync::RwLock::new(AppMetrics::new())),
                rate_limits: Arc::new(Mutex::new(HashMap::new())),
                request_log: Arc::new(std::sync::RwLock::new(RequestLogBuffer::new())),
                notifications: Arc::new(std::sync::RwLock::new(NotificationBuffer::new())),
                templates: Arc::new(templates::TemplateRegistry::with_builtins()),
                entitlements: Arc::new(entitlements::EntitlementService::new()),
                bedrock: None,
                process_role: cairn_api::bootstrap::ProcessRole::AllInOne,
            }
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
    #[ignore = "router unification: covered by integration tests"]
    async fn provider_health_empty_with_no_providers() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(authed_req("/v1/providers/health"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            health.as_array().unwrap().is_empty(),
            "no providers registered — health list must be empty"
        );
    }

    /// After a healthy check, the health entry shows healthy=true and correct fields.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn provider_health_shows_healthy_after_health_check() {
        use cairn_domain::events::ProviderBindingCreated;

        let state = make_state();
        let project = ProjectKey::new("t_ph", "ws_ph", "proj_ph");

        // Register connection + binding (needed to derive tenant for health query).
        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_ph_conn"),
                    EventSource::Runtime,
                    RuntimeEvent::ProviderConnectionRegistered(ProviderConnectionRegistered {
                        tenant: TenantKey::new("t_ph"),
                        provider_connection_id: ProviderConnectionId::new("conn_ph_1"),
                        provider_family: "openai".to_owned(),
                        adapter_type: "responses".to_owned(),
                        supported_models: vec![],
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
                        provider_model_id: ProviderModelId::new(
                            state.runtime.runtime_config.default_generate_model().await,
                        ),
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
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/providers/health"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = health.as_array().unwrap();

        assert_eq!(arr.len(), 1, "one health entry must appear");
        assert_eq!(arr[0]["connection_id"], "conn_ph_1");
        assert_eq!(
            arr[0]["healthy"], true,
            "must be healthy after health check"
        );
        assert_eq!(arr[0]["consecutive_failures"], 0);
        assert_eq!(arr[0]["last_checked_at"], 5_000);
        // Status serializes to lowercase.
        assert!(
            !arr[0]["status"].as_str().unwrap_or("").is_empty(),
            "status must be set"
        );
    }

    /// After ProviderMarkedDegraded, the health entry reflects degraded status.
    #[tokio::test]
    #[ignore = "router unification: covered by integration tests"]
    async fn provider_health_shows_degraded_after_mark_degraded() {
        use cairn_domain::events::{ProviderBindingCreated, ProviderMarkedDegraded};

        let state = make_state();
        let project = ProjectKey::new("t_ph", "ws_ph", "proj_ph2");

        state
            .runtime
            .store
            .append(&[
                EventEnvelope::for_runtime_event(
                    EventId::new("evt_ph2_bind"),
                    EventSource::Runtime,
                    RuntimeEvent::ProviderBindingCreated(ProviderBindingCreated {
                        project: project.clone(),
                        provider_binding_id: ProviderBindingId::new("conn_ph_deg"),
                        provider_connection_id: ProviderConnectionId::new("conn_ph_deg"),
                        provider_model_id: ProviderModelId::new(
                            state.runtime.runtime_config.default_generate_model().await,
                        ),
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
            ])
            .await
            .unwrap();

        let app = make_app(state);
        let resp = app
            .oneshot(authed_req("/v1/providers/health"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let health: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let arr = health.as_array().unwrap();

        assert_eq!(arr.len(), 1, "one health entry");
        assert_eq!(
            arr[0]["healthy"], false,
            "must be unhealthy after degraded mark"
        );
        assert!(
            arr[0]["error_message"]
                .as_str()
                .is_some_and(|e| e.contains("latency")),
            "error_message must contain the degradation reason"
        );
        assert_eq!(arr[0]["last_checked_at"], 8_000);
    }

    /// GET /v1/providers/health requires auth.
    #[tokio::test]
    async fn provider_health_requires_auth() {
        let app = make_app(make_state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/providers/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
