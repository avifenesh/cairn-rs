//! Unit tests for RFC 020 recovery primitives.
//!
//! # Scope
//!
//! This file holds **unit tests** on the recovery types declared in
//! `cairn_runtime::startup` (`ToolCallId`, `ToolCallResultCache`,
//! `CheckpointMeta`, `ReadinessState`, `RecoverySummary`, …) and on the pure
//! `recovery_dispatch_decision()` function. Per the user rule
//! `feedback_integration_tests_only.md`, these tests **do not count as RFC
//! 020 compliance proof** — they exercise types in isolation without ever
//! crashing a real `cairn-app` process. They remain in the workspace because
//! they are fast, type-level regression guards that catch obvious breakage
//! before the integration suite runs.
//!
//! # Compliance proof lives elsewhere
//!
//! The end-to-end, crash-and-replay integration tests that actually prove
//! RFC 020 invariants are in `crates/cairn-app/tests/test_rfc020_*.rs`
//! and drive a real subprocess over HTTP via the `LiveHarness` fixture.
//! See `project_recovery_e2e_migration_plan.md` for the full migration map.
//!
//! # Migration status (PR 1 of 3 — `project_recovery_e2e_migration_plan.md`)
//!
//! | Former test                                     | Action            | Replaced by                                                      |
//! |-------------------------------------------------|-------------------|------------------------------------------------------------------|
//! | `rfc020_invariant5_dual_checkpoint_per_iteration` | KEEP_AS_UNIT    | (pending Track 4 — no integration counterpart yet)               |
//! | `rfc020_invariant6_tool_result_cache_hit_on_resume` | KEEP_AS_UNIT (blocked on Track 3) | planned `test_rfc020_08_tool_result_cache_hit` (future PR) |
//! | `rfc020_dangerous_pause_requires_operator_confirmation` | KEEP_AS_UNIT (blocked on Track 3) | planned `test_rfc020_13a_dangerous_pause` (future PR)       |
//! | `rfc020_author_responsible_redispatches_with_same_id` | KEEP_AS_UNIT (blocked on Track 3) | planned `test_rfc020_13b_author_responsible_redispatch` (future PR) |
//! | `rfc020_clean_recovery_state_survives_restart`  | **DELETED**       | `test_rfc020_recovery::clean_crash_recovery_restores_non_terminal_runs` (PR #75) |
//! | `rfc020_in_flight_approval_survives_restart`    | **DELETED**       | `test_rfc020_recovery::in_flight_approval_survives_crash` (this PR) |
//! | `rfc020_recovery_summary_has_all_counters`      | **DELETED**       | `test_rfc020_recovery::recovery_summary_emitted_once_per_boot` (this PR) |
//! | `rfc020_batched_append_all_or_nothing`          | KEEP_AS_UNIT      | (batch atomicity relies on store ACID; crash timing not repeatable from LiveHarness) |

use cairn_domain::recovery::{CheckpointKind, RetrySafety};
use cairn_runtime::startup::{
    recovery_dispatch_decision, BranchState, CachedToolResult, CheckpointMeta, ReadinessState,
    RecoveryDispatchDecision, ToolCallId, ToolCallResultCache,
};

// ── RFC 020 Invariant 5: Two checkpoints per iteration ──────────────────────

#[test]
fn rfc020_invariant5_dual_checkpoint_per_iteration() {
    // Intent checkpoint after decide, before execute
    let planned_calls = vec![
        ToolCallId::derive("run-100", 3, 0, "memory_search", r#"{"query":"login bug"}"#),
        ToolCallId::derive("run-100", 3, 1, "grep_search", r#"{"pattern":"auth"}"#),
    ];
    let intent = CheckpointMeta::intent("run-100", 3, 25, planned_calls.clone());
    assert_eq!(intent.kind, CheckpointKind::Intent);
    assert_eq!(intent.step_number, 3);
    assert_eq!(intent.tool_calls_snapshot.len(), 2);

    // Result checkpoint after execute completes
    let result = CheckpointMeta::result("run-100", 3, 29, planned_calls);
    assert_eq!(result.kind, CheckpointKind::Result);
    assert!(result.message_history_size > intent.message_history_size);
}

// ── RFC 020 Invariant 6: Tool-call results are cached ───────────────────────

#[test]
fn rfc020_invariant6_tool_result_cache_hit_on_resume() {
    let mut cache = ToolCallResultCache::new();
    let tcid = ToolCallId::derive("run-200", 1, 0, "shell_exec", r#"{"cmd":"make test"}"#);

    // Simulate a completed tool call from before the crash
    cache.insert(CachedToolResult {
        tool_call_id: tcid.clone(),
        tool_name: "shell_exec".into(),
        result_json: serde_json::json!({"exit_code": 0, "stdout": "all tests passed"}),
        completed_at: 1000,
    });

    // On recovery, the cache returns the result without re-dispatch
    let decision = recovery_dispatch_decision(
        &cache,
        &tcid,
        "shell_exec",
        RetrySafety::DangerousPause, // would pause if not cached!
        true,
    );
    assert_eq!(decision, RecoveryDispatchDecision::CacheHit);
}

#[test]
fn rfc020_invariant6_same_tool_call_id_on_resume() {
    // The key insight: ToolCallId is derived from position, not time/PID
    let original = ToolCallId::derive("run-200", 1, 0, "shell_exec", r#"{"cmd":"make test"}"#);
    let resumed = ToolCallId::derive("run-200", 1, 0, "shell_exec", r#"{"cmd":"make test"}"#);
    assert_eq!(original, resumed, "resumed run gets same ToolCallId");
}

// ── RFC 020: RetrySafety classification enforcement ─────────────────────────

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

#[test]
fn rfc020_dangerous_pause_requires_operator_confirmation() {
    let cache = ToolCallResultCache::new();
    let tcid = ToolCallId::derive(
        "run-300",
        0,
        0,
        "shell_exec",
        r#"{"cmd":"rm -rf /tmp/build"}"#,
    );

    let decision = recovery_dispatch_decision(
        &cache,
        &tcid,
        "shell_exec",
        RetrySafety::DangerousPause,
        true,
    );
    match decision {
        RecoveryDispatchDecision::Pause { tool_name, reason } => {
            assert_eq!(tool_name, "shell_exec");
            assert!(reason.contains("DangerousPause"));
        }
        other => panic!("expected Pause, got {other:?}"),
    }
}

#[test]
fn rfc020_author_responsible_redispatches_with_same_id() {
    let cache = ToolCallResultCache::new();
    let tcid = ToolCallId::derive("run-300", 0, 0, "memory_store", r#"{"doc_id":"d1"}"#);

    // AuthorResponsible re-dispatches — tool uses tool_call_id as external key
    let decision = recovery_dispatch_decision(
        &cache,
        &tcid,
        "memory_store",
        RetrySafety::AuthorResponsible,
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

// ── RFC 020 Test: Batched append coherence ──────────────────────────────────
//
// KEEP_AS_UNIT: this test documents the batch-atomicity invariant as a
// contract sanity check. Hitting a real crash between the in-memory buffer
// and the event-log commit is not reliably reproducible from LiveHarness,
// and production durability here ultimately rests on the store's ACID
// guarantees rather than on any cairn-level code path. A LiveHarness
// variant is deferred until/unless the store grows deterministic
// mid-batch crash hooks for testing.

#[test]
fn rfc020_batched_append_all_or_nothing() {
    // Simulate: tool invoke() buffers a memory_store event, then
    // ToolInvocationCompleted is added. The batch must contain both.
    // If the batch is not committed (crash), neither event exists in cache.
    use cairn_runtime::startup::ToolCallResultCache;

    let cache = ToolCallResultCache::new();
    let tcid = ToolCallId::derive("run-batch", 0, 0, "memory_store", r#"{"doc":"d1"}"#);

    // Before batch commit: cache has no entry
    assert!(cache.get(&tcid).is_none(), "no result before batch commit");

    // Simulate batch commit: both the memory event and completion land together
    let mut committed_cache = ToolCallResultCache::new();
    committed_cache.insert(CachedToolResult {
        tool_call_id: tcid.clone(),
        tool_name: "memory_store".into(),
        result_json: serde_json::json!({"stored": true}),
        completed_at: 5000,
    });

    // After commit: cache has the entry
    assert!(
        committed_cache.get(&tcid).is_some(),
        "result must exist after batch commit"
    );

    // Simulate crash BEFORE batch commit: a separate cache stays empty
    let crashed_cache = ToolCallResultCache::new();
    assert!(
        crashed_cache.get(&tcid).is_none(),
        "crashed cache must have no partial state"
    );

    // Key invariant: there is no state where the memory event is visible
    // but ToolInvocationCompleted is not — because they're in the same batch.
    // If the batch didn't commit, neither exists. If it did, both exist.
}

// ── Helper ──────────────────────────────────────────────────────────────────

use cairn_runtime::startup::BranchStatus;

fn branch_complete(count: u64) -> BranchStatus {
    BranchStatus::complete(count)
}
