//! Operator token storage and request log buffer.

// ── OperatorTokenStore ────────────────────────────────────────────────────────

/// Metadata for one operator API token.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct OperatorTokenRecord {
    /// Opaque token identifier (e.g. `tok_<uuid>`). Used as the delete key.
    pub token_id: String,
    pub operator_id: String,
    pub tenant_id: String,
    /// Human-readable label.
    pub name: String,
    /// Unix-ms creation timestamp.
    pub created_at: u64,
    /// Optional expiry (Unix ms). `None` = never expires.
    pub expires_at: Option<u64>,
}

/// Per-operator API token store — metadata + raw-token lookup for revocation.
#[derive(Debug, Default)]
pub struct OperatorTokenStore {
    inner: std::sync::RwLock<std::collections::HashMap<String, (String, OperatorTokenRecord)>>,
}

impl OperatorTokenStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&self, raw_token: String, record: OperatorTokenRecord) {
        self.inner
            .write()
            .unwrap()
            .insert(record.token_id.clone(), (raw_token, record));
    }

    /// Raw token string for revocation — not exposed via API.
    pub fn raw_token(&self, token_id: &str) -> Option<String> {
        self.inner
            .read()
            .unwrap()
            .get(token_id)
            .map(|(t, _)| t.clone())
    }

    pub fn remove(&self, token_id: &str) -> bool {
        self.inner.write().unwrap().remove(token_id).is_some()
    }

    pub fn list(&self) -> Vec<OperatorTokenRecord> {
        self.inner
            .read()
            .unwrap()
            .values()
            .map(|(_, r)| r.clone())
            .collect()
    }
}

// ── Request log ring buffer ──────────────────────────────────────────────────

pub(crate) const REQUEST_LOG_RING_SIZE: usize = 2_000;

/// Structured log entry written by the observability middleware for every request.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RequestLogEntry {
    pub timestamp: String,
    pub level: &'static str,
    pub message: String,
    pub request_id: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub status: u16,
    pub latency_ms: u64,
}

/// Fixed-capacity FIFO ring buffer of structured request log entries.
#[derive(Clone)]
pub struct RequestLogBuffer {
    entries: std::collections::VecDeque<RequestLogEntry>,
}

impl Default for RequestLogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestLogBuffer {
    pub fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::with_capacity(REQUEST_LOG_RING_SIZE),
        }
    }

    pub fn push(&mut self, entry: RequestLogEntry) {
        if self.entries.len() == REQUEST_LOG_RING_SIZE {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    /// Return the last `n` entries whose level matches the filter (empty = all).
    pub fn tail(&self, n: usize, level_filter: &[&str]) -> Vec<&RequestLogEntry> {
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
