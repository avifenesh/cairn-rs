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
    pub(crate) metrics: Arc<RwLock<AppMetrics>>,
    pub(crate) rate_limits: RateLimitTable,
    /// Binary-local request log for OTLP export. Note: the observability
    /// middleware writes to lib_state.request_log (different type/instance).
    /// TODO: unify lib and binary RequestLogBuffer types so OTLP export
    /// reads from the middleware-populated buffer.
    pub(crate) request_log: Arc<RwLock<RequestLogBuffer>>,
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

// ── Request metrics ──────────────────────────────────────────────────────────

/// Rolling-window request metrics.  No external crates required.
///
/// Latency samples are stored in a fixed-size ring buffer; percentiles are
/// computed on-demand from a sorted copy of the buffer (cheap for N=1000).
pub(crate) const LATENCY_RING_SIZE: usize = 1_000;

pub(crate) struct AppMetrics {
    pub(crate) total_requests: u64,
    pub(crate) requests_by_path: HashMap<String, u64>,
    pub(crate) errors_by_status: HashMap<u16, u64>,
    /// Rolling window — LATENCY_RING_SIZE most-recent latencies in ms.
    pub(crate) latency_ring: VecDeque<u64>,
}

impl AppMetrics {
    pub(crate) fn new() -> Self {
        Self {
            total_requests: 0,
            requests_by_path: HashMap::new(),
            errors_by_status: HashMap::new(),
            latency_ring: VecDeque::with_capacity(LATENCY_RING_SIZE),
        }
    }
    pub(crate) fn percentile(&self, p: f64) -> u64 {
        if self.latency_ring.is_empty() {
            return 0;
        }
        let mut sorted: Vec<u64> = self.latency_ring.iter().copied().collect();
        sorted.sort_unstable();
        let idx = ((p / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    pub(crate) fn avg_latency_ms(&self) -> u64 {
        if self.latency_ring.is_empty() {
            return 0;
        }
        self.latency_ring.iter().sum::<u64>() / self.latency_ring.len() as u64
    }

    pub(crate) fn error_rate(&self) -> f64 {
        if self.total_requests == 0 {
            return 0.0;
        }
        let errors: u64 = self.errors_by_status.values().sum();
        errors as f64 / self.total_requests as f64
    }
}

// ── Request log ring buffer ───────────────────────────────────────────────────

/// Maximum number of structured log entries retained in memory.
pub(crate) const LOG_RING_SIZE: usize = 2_000;

/// One structured request log entry.
#[derive(Clone, Serialize)]
pub(crate) struct LogEntry {
    pub(crate) timestamp: String,
    pub(crate) level: &'static str,
    pub(crate) message: String,
    pub(crate) request_id: String,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: Option<String>,
    pub(crate) status: u16,
    pub(crate) latency_ms: u64,
    /// Wall-clock start time in Unix nanoseconds.  Used for OTLP span export.
    pub(crate) start_time_unix_ns: u64,
}

/// Fixed-capacity FIFO ring buffer of structured log entries.
pub(crate) struct RequestLogBuffer {
    pub(crate) entries: VecDeque<LogEntry>,
}

impl RequestLogBuffer {
    pub(crate) fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(LOG_RING_SIZE),
        }
    }
    /// Return the last `n` entries whose level matches the filter (empty = all).
    pub(crate) fn tail(&self, n: usize, level_filter: &[&str]) -> Vec<&LogEntry> {
        let mut result: Vec<&LogEntry> = self
            .entries
            .iter()
            .rev()
            .filter(|e| level_filter.is_empty() || level_filter.contains(&e.level))
            .take(n)
            .collect();
        result.reverse();
        result
    }
}
