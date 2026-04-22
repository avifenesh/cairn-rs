//! Pure-type unit tests for RFC 020 recovery primitives — **not compliance proof.**
//!
//! # Footgun
//!
//! Tests in this file exercise `cairn_runtime::startup` types
//! (`ToolCallId`, `ReadinessState`) and the pure
//! `recovery_dispatch_decision()` function **in isolation**. They never
//! boot `cairn-app`, never crash it, and never replay an event log.
//!
//! Do NOT cite a passing test from this file as evidence that an RFC
//! 020 durability invariant holds. Post-Track-4, all mock-based
//! integration tests that previously lived here have been migrated to
//! LiveHarness-based tests in `crates/cairn-app/tests/`. The
//! authoritative compliance evidence for RFC 020 now lives in:
//!
//! - `crates/cairn-app/tests/test_rfc020_dual_checkpoint.rs` — Invariants
//!   #5/#11/#14 (dual checkpoints + RecoverySummary emission).
//! - `crates/cairn-app/tests/test_rfc020_tool_idempotency.rs` —
//!   Invariants #6 and #11 (ToolCallId determinism + cache re-hydration).
//! - `crates/cairn-app/tests/test_rfc020_recovery.rs` — end-to-end
//!   crash-restart coverage via LiveHarness.
//!
//! This file now holds only pure-type unit tests that act as fast
//! regression guards for the underlying type invariants (seconds to run;
//! catch obvious breakage before the multi-minute LiveHarness suite is
//! worth spinning up).

use cairn_domain::recovery::RetrySafety;
use cairn_runtime::startup::{
    recovery_dispatch_decision, BranchState, ReadinessState, RecoveryDispatchDecision, ToolCallId,
    ToolCallResultCache,
};

// ── RFC 020 Invariant 6: ToolCallId determinism on resume ───────────────────

#[test]
fn rfc020_invariant6_same_tool_call_id_on_resume() {
    // The key insight: ToolCallId is derived from position, not time/PID
    let original = ToolCallId::derive("run-200", 1, 0, "shell_exec", r#"{"cmd":"make test"}"#);
    let resumed = ToolCallId::derive("run-200", 1, 0, "shell_exec", r#"{"cmd":"make test"}"#);
    assert_eq!(original, resumed, "resumed run gets same ToolCallId");
}

// ── RFC 020: RetrySafety classification (pure decision function) ────────────

#[test]
fn rfc020_idempotent_safe_retries_silently() {
    let cache = ToolCallResultCache::new();
    let tcid = ToolCallId::derive("run-300", 0, 0, "memory_search", r#"{"query":"q"}"#);

    let decision = recovery_dispatch_decision(
        &cache,
        &tcid,
        "memory_search",
        RetrySafety::IdempotentSafe,
        true,
    );
    assert_eq!(decision, RecoveryDispatchDecision::Dispatch);
}

// ── RFC 020: ToolCallId determinism for parallel calls ──────────────────────

#[test]
fn rfc020_parallel_calls_get_distinct_ids_via_call_index() {
    // Two parallel calls to the same tool with the same args
    let id0 = ToolCallId::derive("run-400", 2, 0, "memory_search", r#"{"query":"foo"}"#);
    let id1 = ToolCallId::derive("run-400", 2, 1, "memory_search", r#"{"query":"foo"}"#);
    assert_ne!(id0, id1, "call_index distinguishes parallel calls");

    // Recovery recomputes the same IDs (sorted by tool_name, args)
    let id0_resumed = ToolCallId::derive("run-400", 2, 0, "memory_search", r#"{"query":"foo"}"#);
    let id1_resumed = ToolCallId::derive("run-400", 2, 1, "memory_search", r#"{"query":"foo"}"#);
    assert_eq!(id0, id0_resumed);
    assert_eq!(id1, id1_resumed);
}

// ── RFC 020: Health endpoint readiness split ────────────────────────────────

#[test]
fn rfc020_health_liveness_immediate_readiness_deferred() {
    let state = ReadinessState::new();

    // Liveness: immediately available (health returns 200)
    // Readiness: starts as false (health/ready returns 503)
    assert!(!state.is_ready());

    let progress = state.progress();
    assert_eq!(progress.status, "recovering");
    assert_eq!(progress.branches.event_log.state, BranchState::Pending);
    assert_eq!(progress.branches.runs.state, BranchState::Pending);
}

#[test]
fn rfc020_readiness_flips_after_all_branches_complete() {
    let state = ReadinessState::new();

    // Simulate step 2: event log replay
    state.update_branch("2", |b| {
        b.event_log = branch_complete(15234);
        b.tool_result_cache = branch_complete(42);
        b.decision_cache = branch_complete(87);
        b.memory = branch_complete(3401);
        b.graph = branch_complete(892);
        b.evals = branch_complete(14);
        b.webhook_dedup = branch_complete(156);
        b.triggers = branch_complete(5);
    });

    // Step 3: parallel branches
    state.update_branch("3", |b| {
        b.repo_store = branch_complete(3);
        b.plugin_host = branch_complete(1);
        b.providers = branch_complete(2);
    });

    // Step 4: sequential recovery
    state.update_branch("4b", |b| {
        b.sandboxes = branch_complete(4);
        b.runs = branch_complete(7);
    });

    // Flip ready
    state.mark_ready();
    assert!(state.is_ready());

    let progress = state.progress();
    assert_eq!(progress.status, "ready");
}

// ── Helper ──────────────────────────────────────────────────────────────────

use cairn_runtime::startup::BranchStatus;

fn branch_complete(count: u64) -> BranchStatus {
    BranchStatus::complete(count)
}
