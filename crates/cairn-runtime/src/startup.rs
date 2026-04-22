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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

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
        let mut p = self
            .inner
            .progress
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
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

    /// RFC 020 Track 3: reconstruct a `ToolCallId` from its persisted string
    /// form (as stored on `ToolInvocationCompleted.tool_call_id`). Used by
    /// the startup replay to rebuild the cache without access to the original
    /// args that went into `derive`.
    pub fn from_raw(raw: String) -> Self {
        Self(raw)
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
        self.entries.insert(result.tool_call_id.0.clone(), result);
    }

    /// RFC 020 Track 3: insert by raw string key (used by startup replay
    /// when rebuilding from `ToolInvocationCompleted` events).
    pub fn insert_raw(&mut self, key: String, result: CachedToolResult) {
        self.entries.insert(key, result);
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

// ── Dispatch Recovery (RFC 020 §"Tool-Call Idempotency") ────────────────────

use cairn_domain::recovery::RetrySafety;

/// Result of checking whether a tool call should be dispatched on recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryDispatchDecision {
    /// Tool result found in cache — return cached result, do not re-dispatch.
    CacheHit,
    /// Safe to re-dispatch (IdempotentSafe or AuthorResponsible).
    Dispatch,
    /// Must pause for operator confirmation (DangerousPause).
    Pause { tool_name: String, reason: String },
}

/// Decide whether to dispatch a tool call during recovery.
///
/// On recovery with no cached result, the decision depends on RetrySafety:
/// - IdempotentSafe: re-dispatch silently
/// - AuthorResponsible: re-dispatch with same tool_call_id (tool handles dedup)
/// - DangerousPause: pause the run and ask the operator
pub fn recovery_dispatch_decision(
    cache: &ToolCallResultCache,
    tool_call_id: &ToolCallId,
    tool_name: &str,
    retry_safety: RetrySafety,
    is_recovery: bool,
) -> RecoveryDispatchDecision {
    // Always check cache first
    if cache.get(tool_call_id).is_some() {
        return RecoveryDispatchDecision::CacheHit;
    }

    // If not recovery, always dispatch fresh
    if !is_recovery {
        return RecoveryDispatchDecision::Dispatch;
    }

    // Recovery with no cached result — branch on RetrySafety
    match retry_safety {
        RetrySafety::IdempotentSafe | RetrySafety::AuthorResponsible => {
            RecoveryDispatchDecision::Dispatch
        }
        RetrySafety::DangerousPause => RecoveryDispatchDecision::Pause {
            tool_name: tool_name.to_string(),
            reason: "DangerousPause tool with no cached result on recovery".into(),
        },
    }
}

// ── Dual Checkpoint (RFC 020 §"Checkpoint recovery rules") ──────────────────

use cairn_domain::recovery::CheckpointKind;

/// Metadata for a dual checkpoint (Intent or Result).
#[derive(Clone, Debug, Serialize)]
pub struct CheckpointMeta {
    pub run_id: String,
    pub step_number: u32,
    pub kind: CheckpointKind,
    pub message_history_size: u32,
    pub tool_calls_snapshot: Vec<ToolCallId>,
    pub saved_at: u64,
}

impl CheckpointMeta {
    /// Create an Intent checkpoint (after decide, before execute).
    pub fn intent(
        run_id: impl Into<String>,
        step_number: u32,
        message_history_size: u32,
        planned_calls: Vec<ToolCallId>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            step_number,
            kind: CheckpointKind::Intent,
            message_history_size,
            tool_calls_snapshot: planned_calls,
            saved_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    /// Create a Result checkpoint (after execute completes).
    pub fn result(
        run_id: impl Into<String>,
        step_number: u32,
        message_history_size: u32,
        completed_calls: Vec<ToolCallId>,
    ) -> Self {
        Self {
            run_id: run_id.into(),
            step_number,
            kind: CheckpointKind::Result,
            message_history_size,
            tool_calls_snapshot: completed_calls,
            saved_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }
}

// ── Projection Warmup Enumeration (RFC 020 §"Startup order" step 2) ─────────

/// All projections that MUST be warmed before readiness flips to 200.
///
/// The startup graph (step 2) replays the event log into each of these.
/// They are listed here so the startup runner can enumerate them,
/// report per-projection progress, and verify completeness.
pub static REQUIRED_PROJECTIONS: &[&str] = &[
    // Core runtime projections
    "run",
    "task",
    "approval",
    "session",
    "mailbox",
    // Knowledge-layer projections
    "memory_index",
    "graph",
    "eval_scorecard",
    // Decision layer (RFC 019)
    "decision_cache",
    // Tool idempotency (RFC 020)
    "tool_result_cache",
    // Webhook dedup (sealed RFC 017)
    "webhook_dedup",
];

/// Progress tracker for projection warmup.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ProjectionWarmupProgress {
    pub projections_complete: u32,
    pub projections_total: u32,
    pub events_replayed: u64,
    pub per_projection: std::collections::HashMap<String, ProjectionStatus>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectionStatus {
    pub state: BranchState,
    pub entries: u64,
}

impl ProjectionWarmupProgress {
    pub fn new() -> Self {
        let mut per = std::collections::HashMap::new();
        for name in REQUIRED_PROJECTIONS {
            per.insert(
                name.to_string(),
                ProjectionStatus {
                    state: BranchState::Pending,
                    entries: 0,
                },
            );
        }
        Self {
            projections_total: REQUIRED_PROJECTIONS.len() as u32,
            per_projection: per,
            ..Default::default()
        }
    }

    pub fn mark_complete(&mut self, name: &str, entries: u64) {
        self.projections_complete += 1;
        if let Some(status) = self.per_projection.get_mut(name) {
            status.state = BranchState::Complete;
            status.entries = entries;
        }
    }

    pub fn is_complete(&self) -> bool {
        self.projections_complete >= self.projections_total
    }
}

// ── Batched Append Helper (RFC 020 §"Tool-Call Idempotency" invariant 11) ──

use cairn_domain::{EventEnvelope, RuntimeEvent};

/// Collects events buffered by a tool during invocation and the final
/// `ToolInvocationCompleted` event into a single `EventLog::append` batch.
///
/// This enforces RFC 020 invariant 11: either ALL events (tool side-effects
/// and the completion marker) are durable, or NONE are. No partial state
/// where the projection saw the side-effect but the cache did not.
pub struct ToolDispatchBatch {
    events: Vec<EventEnvelope<RuntimeEvent>>,
}

impl ToolDispatchBatch {
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }

    /// Add a buffered tool event (e.g. IngestEvent, RepoAllowlistExpanded).
    pub fn push(&mut self, event: EventEnvelope<RuntimeEvent>) {
        self.events.push(event);
    }

    /// Add the final ToolInvocationCompleted event.
    pub fn push_completion(&mut self, event: EventEnvelope<RuntimeEvent>) {
        self.events.push(event);
    }

    /// Consume the batch into a Vec for `EventLog::append`.
    pub fn into_batch(self) -> Vec<EventEnvelope<RuntimeEvent>> {
        self.events
    }

    /// Number of events in the batch.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
}

impl Default for ToolDispatchBatch {
    fn default() -> Self {
        Self::new()
    }
}

// ── Cache replay (RFC 020 Track 3) ──────────────────────────────────────────

/// Replay `ToolInvocationCompleted` events from the event log into a
/// `ToolCallResultCache`. Called once during cairn-app startup so a
/// restart serves cached results for tools that completed before the
/// previous process crashed.
///
/// Events without `tool_call_id` (legacy / pre-Track-3 log entries) are
/// skipped — they carry no deterministic key to hash into, so no cache
/// hit is possible on replay regardless.
///
/// Returns the number of cache entries populated.
pub async fn replay_tool_result_cache<S: cairn_store::EventLog>(
    store: &S,
    cache: &std::sync::Mutex<ToolCallResultCache>,
) -> Result<usize, cairn_store::StoreError> {
    use cairn_domain::RuntimeEvent;
    use cairn_store::EventPosition;

    let mut populated = 0usize;
    let mut cursor: Option<EventPosition> = None;
    const PAGE: usize = 500;
    loop {
        let page = store.read_stream(cursor, PAGE).await?;
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|e| e.position);
        for stored in &page {
            if let RuntimeEvent::ToolInvocationCompleted(ev) = &stored.envelope.payload {
                if let (Some(tcid_str), Some(result)) = (&ev.tool_call_id, &ev.result_json) {
                    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
                    // ToolCallId is a newtype around String; rebuild by hashing
                    // is not possible since we don't have the args, but we can
                    // reconstruct via a private constructor? Use pub as_str.
                    // We stored the string form, so build a proxy:
                    guard.insert_raw(tcid_str.clone(), CachedToolResult {
                        tool_call_id: ToolCallId::from_raw(tcid_str.clone()),
                        tool_name: ev.tool_name.clone(),
                        result_json: result.clone(),
                        completed_at: ev.finished_at_ms,
                    });
                    populated += 1;
                }
            }
        }
        match last {
            Some(pos) => cursor = Some(pos),
            None => break,
        }
        if page.len() < PAGE {
            break;
        }
    }
    Ok(populated)
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

    // ── Recovery Dispatch Tests ──────────────────────────────────────

    #[test]
    fn recovery_dispatch_cache_hit_returns_cached() {
        let mut cache = ToolCallResultCache::new();
        let tcid = ToolCallId::derive("run-1", 0, 0, "memory_search", "{}");
        cache.insert(CachedToolResult {
            tool_call_id: tcid.clone(),
            tool_name: "memory_search".into(),
            result_json: serde_json::json!({}),
            completed_at: 0,
        });

        let decision = recovery_dispatch_decision(
            &cache,
            &tcid,
            "memory_search",
            RetrySafety::DangerousPause,
            true,
        );
        assert_eq!(decision, RecoveryDispatchDecision::CacheHit);
    }

    #[test]
    fn recovery_dispatch_idempotent_safe_redispatches() {
        let cache = ToolCallResultCache::new();
        let tcid = ToolCallId::derive("run-1", 0, 0, "memory_search", "{}");

        let decision = recovery_dispatch_decision(
            &cache,
            &tcid,
            "memory_search",
            RetrySafety::IdempotentSafe,
            true,
        );
        assert_eq!(decision, RecoveryDispatchDecision::Dispatch);
    }

    #[test]
    fn recovery_dispatch_author_responsible_redispatches() {
        let cache = ToolCallResultCache::new();
        let tcid = ToolCallId::derive("run-1", 0, 0, "memory_store", "{}");

        let decision = recovery_dispatch_decision(
            &cache,
            &tcid,
            "memory_store",
            RetrySafety::AuthorResponsible,
            true,
        );
        assert_eq!(decision, RecoveryDispatchDecision::Dispatch);
    }

    #[test]
    fn recovery_dispatch_dangerous_pause_pauses() {
        let cache = ToolCallResultCache::new();
        let tcid = ToolCallId::derive("run-1", 0, 0, "shell_exec", "{}");

        let decision = recovery_dispatch_decision(
            &cache,
            &tcid,
            "shell_exec",
            RetrySafety::DangerousPause,
            true,
        );
        assert!(matches!(decision, RecoveryDispatchDecision::Pause { .. }));
    }

    #[test]
    fn non_recovery_always_dispatches() {
        let cache = ToolCallResultCache::new();
        let tcid = ToolCallId::derive("run-1", 0, 0, "shell_exec", "{}");

        // Even DangerousPause dispatches when not in recovery
        let decision = recovery_dispatch_decision(
            &cache,
            &tcid,
            "shell_exec",
            RetrySafety::DangerousPause,
            false,
        );
        assert_eq!(decision, RecoveryDispatchDecision::Dispatch);
    }

    // ── Dual Checkpoint Tests ───────────────────────────────────────

    #[test]
    fn checkpoint_intent_has_correct_kind() {
        let calls = vec![ToolCallId::derive("run-1", 0, 0, "memory_search", "{}")];
        let cp = CheckpointMeta::intent("run-1", 0, 10, calls.clone());
        assert_eq!(cp.kind, CheckpointKind::Intent);
        assert_eq!(cp.step_number, 0);
        assert_eq!(cp.message_history_size, 10);
        assert_eq!(cp.tool_calls_snapshot.len(), 1);
    }

    #[test]
    fn checkpoint_result_has_correct_kind() {
        let calls = vec![ToolCallId::derive("run-1", 0, 0, "memory_search", "{}")];
        let cp = CheckpointMeta::result("run-1", 0, 12, calls);
        assert_eq!(cp.kind, CheckpointKind::Result);
        assert_eq!(cp.message_history_size, 12);
    }

    #[test]
    fn dual_checkpoints_per_iteration() {
        // Simulate one full iteration with Intent then Result checkpoint
        let planned = vec![
            ToolCallId::derive("run-1", 0, 0, "memory_search", r#"{"query":"foo"}"#),
            ToolCallId::derive("run-1", 0, 1, "grep_search", r#"{"pattern":"bar"}"#),
        ];
        let intent = CheckpointMeta::intent("run-1", 0, 10, planned.clone());

        // After execute, same tools are in the completed snapshot
        let completed = planned;
        let result = CheckpointMeta::result("run-1", 0, 12, completed);

        // Intent checkpoint is the safe rollback point
        assert_eq!(intent.kind, CheckpointKind::Intent);
        // Result checkpoint is the progress commit point
        assert_eq!(result.kind, CheckpointKind::Result);
        // Message history grew after tool results were appended
        assert!(result.message_history_size >= intent.message_history_size);
    }

    // ── Projection warmup tests ─────────────────────────────────────────

    #[test]
    fn required_projections_includes_core_set() {
        assert!(REQUIRED_PROJECTIONS.contains(&"run"));
        assert!(REQUIRED_PROJECTIONS.contains(&"task"));
        assert!(REQUIRED_PROJECTIONS.contains(&"decision_cache"));
        assert!(REQUIRED_PROJECTIONS.contains(&"tool_result_cache"));
        assert!(REQUIRED_PROJECTIONS.contains(&"graph"));
    }

    #[test]
    fn warmup_progress_tracks_completion() {
        let mut progress = ProjectionWarmupProgress::new();
        assert!(!progress.is_complete());
        assert_eq!(
            progress.projections_total,
            REQUIRED_PROJECTIONS.len() as u32
        );

        for name in REQUIRED_PROJECTIONS {
            progress.mark_complete(name, 100);
        }
        assert!(progress.is_complete());
    }

    // ── Batched append tests ────────────────────────────────────────────

    #[test]
    fn tool_dispatch_batch_collects_events() {
        use cairn_domain::{EventEnvelope, EventId, EventSource};
        let mut batch = ToolDispatchBatch::new();
        assert!(batch.is_empty());

        // Simulate two buffered events + completion.
        let make_env = |payload: RuntimeEvent| -> EventEnvelope<RuntimeEvent> {
            EventEnvelope::for_runtime_event(
                EventId::new("evt_test"),
                EventSource::Runtime,
                payload,
            )
        };

        batch.push(make_env(RuntimeEvent::SessionCreated(
            cairn_domain::events::SessionCreated {
                project: cairn_domain::ProjectKey::new("t", "w", "p"),
                session_id: cairn_domain::SessionId::new("s1"),
            },
        )));
        batch.push_completion(make_env(RuntimeEvent::SessionCreated(
            cairn_domain::events::SessionCreated {
                project: cairn_domain::ProjectKey::new("t", "w", "p"),
                session_id: cairn_domain::SessionId::new("s2"),
            },
        )));

        assert_eq!(batch.len(), 2);
        let events = batch.into_batch();
        assert_eq!(events.len(), 2);
    }
}
