//! Startup dependency graph — RFC 020: Durable Recovery.
//!
//! Implements the parallel-where-independent startup DAG:
//!
//! ```text
//! Step 1: Load config, open HTTP for health only
//! Step 2: Replay event log into ALL projections (serial)
//! Step 3: Parallel recovery branches (A: repo clones, B: plugin host, C: providers)
//! Step 4: Sequential recovery (4a: sandbox, 4b: runs/tasks)
//! Step 5: Emit RecoverySummary
//! Step 6: Flip /health/ready to 200
//! ```

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use serde::Serialize;

// ── Readiness State ─────────────────────────────────────────────────────────

/// Tracks the startup progress for the /health/ready endpoint.
/// Shared across the startup graph and the health handler.
#[derive(Clone)]
pub struct ReadinessState {
    inner: Arc<ReadinessInner>,
}

struct ReadinessInner {
    ready: AtomicBool,
    started_at: u64,
    progress: RwLock<StartupProgress>,
}

/// JSON body returned by /health/ready during startup.
#[derive(Clone, Debug, Serialize)]
pub struct StartupProgress {
    pub status: String,
    pub step: String,
    pub branches: StartupBranches,
    pub started_at: u64,
    pub elapsed_ms: u64,
}

/// Per-branch status in the startup DAG.
#[derive(Clone, Debug, Serialize)]
pub struct StartupBranches {
    pub event_log: BranchStatus,
    pub tool_result_cache: BranchStatus,
    pub decision_cache: BranchStatus,
    pub memory: BranchStatus,
    pub graph: BranchStatus,
    pub evals: BranchStatus,
    pub repo_store: BranchStatus,
    pub plugin_host: BranchStatus,
    pub providers: BranchStatus,
    pub sandboxes: BranchStatus,
    pub webhook_dedup: BranchStatus,
    pub triggers: BranchStatus,
    pub runs: BranchStatus,
}

/// Status of a single startup branch.
#[derive(Clone, Debug, Serialize)]
pub struct BranchStatus {
    pub state: BranchState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchState {
    Pending,
    InProgress,
    Complete,
    Failed,
}

impl BranchStatus {
    pub fn pending() -> Self {
        Self {
            state: BranchState::Pending,
            count: None,
            detail: None,
        }
    }

    pub fn in_progress() -> Self {
        Self {
            state: BranchState::InProgress,
            count: None,
            detail: None,
        }
    }

    pub fn complete(count: u64) -> Self {
        Self {
            state: BranchState::Complete,
            count: Some(count),
            detail: None,
        }
    }

    pub fn complete_with_detail(count: u64, detail: impl Into<String>) -> Self {
        Self {
            state: BranchState::Complete,
            count: Some(count),
            detail: Some(detail.into()),
        }
    }

    pub fn failed(detail: impl Into<String>) -> Self {
        Self {
            state: BranchState::Failed,
            count: None,
            detail: Some(detail.into()),
        }
    }
}

impl Default for StartupBranches {
    fn default() -> Self {
        Self {
            event_log: BranchStatus::pending(),
            tool_result_cache: BranchStatus::pending(),
            decision_cache: BranchStatus::pending(),
            memory: BranchStatus::pending(),
            graph: BranchStatus::pending(),
            evals: BranchStatus::pending(),
            repo_store: BranchStatus::pending(),
            plugin_host: BranchStatus::pending(),
            providers: BranchStatus::pending(),
            sandboxes: BranchStatus::pending(),
            webhook_dedup: BranchStatus::pending(),
            triggers: BranchStatus::pending(),
            runs: BranchStatus::pending(),
        }
    }
}

impl ReadinessState {
    pub fn new() -> Self {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        Self {
            inner: Arc::new(ReadinessInner {
                ready: AtomicBool::new(false),
                started_at: now,
                progress: RwLock::new(StartupProgress {
                    status: "recovering".into(),
                    step: "1".into(),
                    branches: StartupBranches::default(),
                    started_at: now,
                    elapsed_ms: 0,
                }),
            }),
        }
    }

    /// Check if the system is ready to serve requests.
    pub fn is_ready(&self) -> bool {
        self.inner.ready.load(Ordering::SeqCst)
    }

    /// Flip readiness to true (step 6).
    pub fn mark_ready(&self) {
        self.inner.ready.store(true, Ordering::SeqCst);
        if let Ok(mut p) = self.inner.progress.write() {
            p.status = "ready".into();
            p.step = "6".into();
            self.update_elapsed(&mut p);
        }
    }

    /// Get the current progress snapshot for /health/ready.
    pub fn progress(&self) -> StartupProgress {
        let mut p = self.inner.progress.read().unwrap().clone();
        self.update_elapsed(&mut p);
        p
    }

    /// Update a specific branch status.
    pub fn update_branch(&self, step: &str, updater: impl FnOnce(&mut StartupBranches)) {
        if let Ok(mut p) = self.inner.progress.write() {
            p.step = step.into();
            updater(&mut p.branches);
            self.update_elapsed(&mut p);
        }
    }

    fn update_elapsed(&self, p: &mut StartupProgress) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        p.elapsed_ms = now.saturating_sub(self.inner.started_at);
    }
}

impl Default for ReadinessState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Recovery Summary (RFC 020 §"Events") ────────────────────────────────────

/// Emitted once per boot with recovery statistics.
#[derive(Clone, Debug, Default, Serialize)]
pub struct RecoverySummary {
    pub recovered_runs: u32,
    pub recovered_tasks: u32,
    pub recovered_sandboxes: u32,
    pub preserved_sandboxes: u32,
    pub orphaned_sandboxes_cleaned: u32,
    pub decision_cache_entries: u32,
    pub stale_pending_cleared: u32,
    pub tool_result_cache_entries: u32,
    pub memory_projection_entries: u32,
    pub graph_nodes_recovered: u32,
    pub graph_edges_recovered: u32,
    pub webhook_dedup_entries: u32,
    pub trigger_projections: u32,
    pub boot_id: String,
    pub startup_ms: u64,
}

// ── ToolCallId (RFC 020 §"Tool-Call Idempotency") ───────────────────────────

/// Deterministic tool call identifier derived from position in the run.
///
/// A resumed run computing the same tool call at the same step gets the
/// same ToolCallId, enabling the result cache to serve cached results
/// instead of re-dispatching.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize)]
pub struct ToolCallId(String);

impl ToolCallId {
    /// Derive a deterministic tool call ID from the run position.
    ///
    /// `call_index` is a per-step monotonic counter starting at 0.
    /// Parallel calls to the same tool with the same args get different
    /// call_index values (0, 1, ...) → distinct IDs.
    ///
    /// The orchestrator must sort parallel dispatch entries by
    /// (tool_name, normalized_args) before assigning indices so recovery
    /// recomputes the same IDs.
    pub fn derive(
        run_id: &str,
        step_number: u32,
        call_index: u32,
        tool_name: &str,
        normalized_args: &str,
    ) -> Self {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        run_id.hash(&mut hasher);
        step_number.hash(&mut hasher);
        call_index.hash(&mut hasher);
        tool_name.hash(&mut hasher);
        normalized_args.hash(&mut hasher);
        let hash = hasher.finish();
        Self(format!("tc_{hash:016x}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ToolCallId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── ToolCallResultCache (RFC 020) ───────────────────────────────────────────

/// Cached result of a completed tool call.
#[derive(Clone, Debug)]
pub struct CachedToolResult {
    pub tool_call_id: ToolCallId,
    pub tool_name: String,
    pub result_json: serde_json::Value,
    pub completed_at: u64,
}

/// In-memory cache of tool call results, keyed by ToolCallId.
/// Scoped per-run. Populated from ToolInvocationCompleted events on replay.
pub struct ToolCallResultCache {
    entries: std::collections::HashMap<String, CachedToolResult>,
}

impl ToolCallResultCache {
    pub fn new() -> Self {
        Self {
            entries: std::collections::HashMap::new(),
        }
    }

    /// Insert a completed tool result.
    pub fn insert(&mut self, result: CachedToolResult) {
        self.entries
            .insert(result.tool_call_id.0.clone(), result);
    }

    /// Look up a cached result by tool call ID.
    pub fn get(&self, tool_call_id: &ToolCallId) -> Option<&CachedToolResult> {
        self.entries.get(&tool_call_id.0)
    }

    /// Total entries in the cache.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for ToolCallResultCache {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_id_is_deterministic() {
        let id1 = ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"foo"}"#);
        let id2 = ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"foo"}"#);
        assert_eq!(id1, id2, "same inputs must produce same ID");
    }

    #[test]
    fn tool_call_id_differs_on_call_index() {
        let id1 = ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"foo"}"#);
        let id2 = ToolCallId::derive("run-1", 0, 1, "memory_search", r#"{"query":"foo"}"#);
        assert_ne!(id1, id2, "different call_index must produce different IDs");
    }

    #[test]
    fn tool_call_id_differs_on_step() {
        let id1 = ToolCallId::derive("run-1", 0, 0, "shell_exec", r#"{"cmd":"ls"}"#);
        let id2 = ToolCallId::derive("run-1", 1, 0, "shell_exec", r#"{"cmd":"ls"}"#);
        assert_ne!(id1, id2);
    }

    #[test]
    fn tool_call_id_differs_on_run() {
        let id1 = ToolCallId::derive("run-1", 0, 0, "shell_exec", r#"{"cmd":"ls"}"#);
        let id2 = ToolCallId::derive("run-2", 0, 0, "shell_exec", r#"{"cmd":"ls"}"#);
        assert_ne!(id1, id2);
    }

    #[test]
    fn tool_call_id_differs_on_args() {
        let id1 = ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"foo"}"#);
        let id2 = ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"bar"}"#);
        assert_ne!(id1, id2);
    }

    #[test]
    fn tool_result_cache_stores_and_retrieves() {
        let mut cache = ToolCallResultCache::new();
        let tcid = ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"foo"}"#);

        cache.insert(CachedToolResult {
            tool_call_id: tcid.clone(),
            tool_name: "memory_search".into(),
            result_json: serde_json::json!({"results": []}),
            completed_at: 12345,
        });

        assert_eq!(cache.len(), 1);
        let hit = cache.get(&tcid).unwrap();
        assert_eq!(hit.tool_name, "memory_search");
        assert_eq!(hit.completed_at, 12345);
    }

    #[test]
    fn tool_result_cache_miss_returns_none() {
        let cache = ToolCallResultCache::new();
        let tcid = ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"foo"}"#);
        assert!(cache.get(&tcid).is_none());
    }

    #[test]
    fn readiness_state_starts_not_ready() {
        let state = ReadinessState::new();
        assert!(!state.is_ready());
        let progress = state.progress();
        assert_eq!(progress.status, "recovering");
    }

    #[test]
    fn readiness_state_flip_to_ready() {
        let state = ReadinessState::new();
        state.mark_ready();
        assert!(state.is_ready());
        let progress = state.progress();
        assert_eq!(progress.status, "ready");
        assert_eq!(progress.step, "6");
    }

    #[test]
    fn readiness_state_branch_updates() {
        let state = ReadinessState::new();
        state.update_branch("2", |b| {
            b.event_log = BranchStatus::complete(15234);
        });

        let progress = state.progress();
        assert_eq!(progress.step, "2");
        assert_eq!(progress.branches.event_log.state, BranchState::Complete);
        assert_eq!(progress.branches.event_log.count, Some(15234));
        // Other branches still pending
        assert_eq!(progress.branches.runs.state, BranchState::Pending);
    }

    #[test]
    fn recovery_summary_default_is_zeroed() {
        let summary = RecoverySummary::default();
        assert_eq!(summary.recovered_runs, 0);
        assert_eq!(summary.recovered_tasks, 0);
        assert_eq!(summary.startup_ms, 0);
    }
}
