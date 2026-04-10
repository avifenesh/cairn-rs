//! Integration tests for RFC 020: Durable Recovery and Tool-Call Idempotency.

use cairn_domain::recovery::{CheckpointKind, RetrySafety};
use cairn_runtime::startup::{
    recovery_dispatch_decision, BranchState, CachedToolResult, CheckpointMeta, ReadinessState,
    RecoveryDispatchDecision, RecoverySummary, ToolCallId, ToolCallResultCache,
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

// ── RFC 020: Recovery summary ───────────────────────────────────────────────

#[test]
fn rfc020_recovery_summary_has_all_counters() {
    let summary = RecoverySummary {
        recovered_runs: 5,
        recovered_tasks: 12,
        recovered_sandboxes: 3,
        preserved_sandboxes: 1,
        orphaned_sandboxes_cleaned: 2,
        decision_cache_entries: 87,
        stale_pending_cleared: 2,
        tool_result_cache_entries: 42,
        memory_projection_entries: 3401,
        graph_nodes_recovered: 892,
        graph_edges_recovered: 2104,
        webhook_dedup_entries: 156,
        trigger_projections: 5,
        boot_id: "boot-1234".into(),
        startup_ms: 2340,
    };

    assert_eq!(summary.recovered_runs, 5);
    assert_eq!(summary.preserved_sandboxes, 1);
    assert_eq!(summary.startup_ms, 2340);
    assert_eq!(summary.boot_id, "boot-1234");
}

// ── Helper ──────────────────────────────────────────────────────────────────

fn branch_complete(count: u64) -> cairn_runtime::startup::BranchStatus {
    cairn_runtime::startup::BranchStatus::complete(count)
}
