//! Binary-specific application state, backend handles, and in-memory buffers.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use cairn_api::auth::ServiceTokenRegistry;
use cairn_api::bootstrap::{DeploymentMode, ProcessRole};
use cairn_memory::in_memory::{InMemoryDocumentStore, InMemoryRetrieval};
use cairn_memory::pipeline::{IngestPipeline, ParagraphChunker};
use cairn_runtime::{InMemoryServices, OllamaProvider};
use cairn_store::pg::{PgAdapter, PgEventLog};
use cairn_store::sqlite::{SqliteAdapter, SqliteEventLog};
use serde::Serialize;

use crate::entitlements;
use crate::templates;

// ── Postgres backend ──────────────────────────────────────────────────────────

/// Bundled Postgres connection handles.
///
/// Created at startup when `--db postgres://...` is supplied.
/// Appends go to both Postgres (durable) and InMemory (read models + SSE);
/// event log replays (GET /v1/events) are served from Postgres when present.
#[derive(Clone)]
pub(crate) struct PgBackend {
    pub(crate) event_log: Arc<PgEventLog>,
    pub(crate) adapter: Arc<PgAdapter>,
}

/// Bundled SQLite connection handles.
///
/// Created at startup when `--db sqlite:path` or a bare `.db` path is supplied.
/// Appends go to both SQLite (durable) and InMemory (read models + SSE).
#[derive(Clone)]
pub(crate) struct SqliteBackend {
    pub(crate) event_log: Arc<SqliteEventLog>,
    pub(crate) adapter: Arc<SqliteAdapter>,
    pub(crate) path: PathBuf,
}

// ── Rate limiting ─────────────────────────────────────────────────────────────

/// One sliding-window bucket per identity key (token or IP).
#[derive(Clone)]
pub(crate) struct RateBucket {
    /// Number of requests in the current 60-second window.
    pub(crate) count: u32,
    /// When the current window started (used to decide when to reset).
    pub(crate) window_start: Instant,
}
/// Shared rate-limit table.  Keyed by token (preferred) or IP address.
pub(crate) type RateLimitTable = Arc<Mutex<HashMap<String, RateBucket>>>;

/// Per-token limit: requests per minute.
pub(crate) const RATE_LIMIT_TOKEN: u32 = 1_000;
/// Per-IP limit when no token is present: requests per minute.
pub(crate) const RATE_LIMIT_IP: u32 = 100;
/// Window duration.
pub(crate) const RATE_WINDOW: Duration = Duration::from_secs(60);

// ── App state ────────────────────────────────────────────────────────────────

#[derive(Clone)]
/// Binary-specific state for routes not covered by the lib.rs catalog.
///
/// Why the split: `cairn_app::AppState` (lib.rs) owns the catalog of 197+
/// provider-agnostic routes. This struct adds binary-specific routes that
/// depend on concrete store backends (Postgres/SQLite), WebSocket, or system
/// introspection. Collapsing them would leak backend deps into the lib.
///
/// Shares `runtime` and `tokens` with `cairn_app::AppState` (same Arc).
/// Fields like `document_store`, `retrieval`, and `ingest` are served
/// exclusively by the catalog router and are NOT duplicated here.
pub(crate) struct AppState {
    pub(crate) runtime: Arc<InMemoryServices>,
    pub(crate) started_at: Arc<Instant>,
    pub(crate) tokens: Arc<ServiceTokenRegistry>,
    pub(crate) pg: Option<Arc<PgBackend>>,
    pub(crate) sqlite: Option<Arc<SqliteBackend>>,
    pub(crate) mode: DeploymentMode,
    /// Shared with lib.rs AppState — kept for seed_demo_data and dead handlers
    /// pending cleanup.
    #[allow(dead_code)]
    pub(crate) document_store: Arc<InMemoryDocumentStore>,
    #[allow(dead_code)]
    pub(crate) retrieval: Arc<InMemoryRetrieval>,
    #[allow(dead_code)]
    pub(crate) ingest: Arc<IngestPipeline<Arc<InMemoryDocumentStore>, ParagraphChunker>>,
    pub(crate) ollama: Option<Arc<OllamaProvider>>,
    /// Heavy/generate provider: CAIRN_BRAIN_URL (gemma4 31B etc.)
    pub(crate) openai_compat_brain: Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    /// Light/embed+worker provider: CAIRN_WORKER_URL (qwen3.5, qwen3-embedding)
    pub(crate) openai_compat_worker:
        Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    /// OpenRouter provider: OPENROUTER_API_KEY activates https://openrouter.ai/api/v1
    pub(crate) openai_compat_openrouter:
        Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    /// Backward-compat alias: first of brain/worker/openrouter that is configured.
    pub(crate) openai_compat: Option<Arc<cairn_providers::wire::openai_compat::OpenAiCompat>>,
    /// Shared with lib.rs AppState — the observability middleware populates
    /// this struct on every request so the binary Prometheus handler can
    /// expose live `cairn_http_*` counters/gauges without duplicating the
    /// recording path. See issue #243 for the bug that motivated the split.
    pub(crate) lib_metrics: Arc<cairn_app::metrics::AppMetrics>,
    pub(crate) rate_limits: RateLimitTable,
    /// Shared with lib_state — the observability middleware populates this
    /// buffer and the OTLP export handler reads from it.
    pub(crate) request_log: Arc<RwLock<cairn_app::tokens::RequestLogBuffer>>,
    pub(crate) notifications: Arc<RwLock<NotificationBuffer>>,
    pub(crate) templates: Arc<templates::TemplateRegistry>,
    pub(crate) entitlements: Arc<entitlements::EntitlementService>,
    pub(crate) bedrock: Option<Arc<cairn_providers::backends::bedrock::Bedrock>>,
    pub(crate) process_role: ProcessRole,
}

// ── Notification buffer ───────────────────────────────────────────────────────

pub(crate) const NOTIF_RING_SIZE: usize = 200;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum NotifType {
    ApprovalRequested,
    ApprovalResolved,
    RunCompleted,
    RunFailed,
    TaskStuck,
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct Notification {
    pub(crate) id: String,
    #[serde(rename = "type")]
    pub(crate) notif_type: NotifType,
    pub(crate) message: String,
    /// Entity ID the notification links to (run_id, approval_id, task_id, …).
    pub(crate) entity_id: Option<String>,
    /// Hash navigation target for the UI (e.g. "runs", "approvals").
    pub(crate) href: String,
    pub(crate) read: bool,
    pub(crate) created_at: u64,
}

pub(crate) struct NotificationBuffer {
    pub(crate) entries: VecDeque<Notification>,
}

impl NotificationBuffer {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(NOTIF_RING_SIZE),
        }
    }

    pub(crate) fn push(&mut self, n: Notification) {
        if self.entries.len() == NOTIF_RING_SIZE {
            self.entries.pop_front();
        }
        self.entries.push_back(n);
    }

    pub(crate) fn list(&self, limit: usize) -> Vec<&Notification> {
        let len = self.entries.len();
        let start = len.saturating_sub(limit);
        self.entries.iter().skip(start).collect()
    }

    pub(crate) fn mark_read(&mut self, id: &str) -> bool {
        if let Some(n) = self.entries.iter_mut().find(|n| n.id == id) {
            n.read = true;
            true
        } else {
            false
        }
    }

    pub(crate) fn mark_all_read(&mut self) {
        for n in &mut self.entries {
            n.read = true;
        }
    }

    pub(crate) fn unread_count(&self) -> usize {
        self.entries.iter().filter(|n| !n.read).count()
    }
}

// LogEntry, RequestLogBuffer, and the binary-side `AppMetrics` struct were
// removed: the lib-side `cairn_app::metrics::AppMetrics` is now the single
// source of truth for HTTP counters/gauges, populated by the observability
// middleware on every request (see issue #243 for the orphaning story).
// (cairn_app::tokens::RequestLogEntry / cairn_app::tokens::RequestLogBuffer)
