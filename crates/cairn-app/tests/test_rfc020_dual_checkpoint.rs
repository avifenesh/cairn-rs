//! RFC 020 Track 4 — dual-checkpoint + audit-event integration tests.
//!
//! Compliance evidence for the durability invariants Track 4 closes:
//!
//! | RFC 020 # | Test fn                                                    |
//! |-----------|------------------------------------------------------------|
//! | #14       | `dual_checkpoint_intent_and_result_per_iteration`          |
//! | #11       | `recovery_summary_event_emitted_on_restart` (extends #11)  |
//! | #15       | `checkpoint_compression_v1_skipped_by_design` (ignored)    |
//!
//! Test #14 drives `CheckpointServiceImpl::save_dual` end-to-end against
//! an in-process `InMemoryStore` and verifies the emitted
//! `CheckpointRecorded` events carry the new Track 4 fields (`kind`,
//! `message_history_size`, `tool_call_ids`). Test #11 proves the new
//! `RecoverySummaryEmitted` event fires exactly once per
//! `RecoveryService::recover_all` invocation, with distinct `boot_id`s
//! across boots. Test #15 is intentionally `#[ignore]` per Gap 3 — v1
//! ships full snapshots, not diffs.
//!
//! End-to-end proof that the orchestrator actually calls `save_dual` is
//! covered by the wiring in `cairn-app/src/handlers/runs.rs`
//! (`DualCheckpointHook::new` attached to `OrchestratorLoop` before
//! `run()`), plus the orchestrator-level unit test
//! `dual_checkpoints_per_iteration` in `cairn_runtime::startup` which
//! asserts the `CheckpointMeta::intent` / `::result` constructors.

use cairn_domain::{
    CheckpointDisposition, CheckpointId, CheckpointKind, ProjectKey, RunId, RuntimeEvent,
};
use cairn_store::{EventLog, InMemoryStore};
use serde_json::json;

// ─────────────────────────────────────────────────────────────────────────────
// RFC 020 Integration Test #14 — dual checkpoint per iteration.
// ─────────────────────────────────────────────────────────────────────────────

/// RFC 020 Invariant #5: each orchestrator iteration emits *two* checkpoint
/// records — an `Intent` checkpoint after decide (capturing the planned
/// tool calls before dispatch) and a `Result` checkpoint after execute
/// settles. This test proves:
///
/// 1. `CheckpointServiceImpl::save_dual` emits `CheckpointRecorded` with
///    the `kind`, `message_history_size`, and `tool_call_ids` fields
///    populated (new Track 4 fields).
/// 2. Two checkpoints can coexist on the same run with distinct
///    `CheckpointId`s and distinct `CheckpointKind`s.
/// 3. The event ordering is Intent-before-Result, matching the RFC 020
///    §"Checkpoint recovery rules" resume-semantics table.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dual_checkpoint_intent_and_result_per_iteration() {
    use cairn_runtime::{CheckpointService, CheckpointServiceImpl};
    use std::sync::Arc;

    let store = Arc::new(InMemoryStore::new());
    let svc = CheckpointServiceImpl::new(store.clone());
    let project = ProjectKey::new("tenant_test", "ws_test", "proj_test");
    let run_id = RunId::new("run_dual_ckpt");

    // ── Intent: planned ToolCallIds + pre-execute message history ──────────
    let intent_body = json!({
        "iteration": 0,
        "decide": {"proposal_count": 2},
        "history_so_far": ["user: Summarise the repo"],
    });
    let planned = vec![
        "planned:grep_search:{\"pattern\":\"fn main\"}".to_owned(),
        "planned:file_read:{\"path\":\"Cargo.toml\"}".to_owned(),
    ];
    let intent_cp_id = CheckpointId::new("cp_intent_dual_1");
    let intent = svc
        .save_dual(
            &project,
            &run_id,
            intent_cp_id.clone(),
            CheckpointKind::Intent,
            intent_body.clone(),
            planned.clone(),
        )
        .await
        .expect("save_dual Intent must succeed");
    assert_eq!(intent.checkpoint_id, intent_cp_id);
    assert_eq!(intent.disposition, CheckpointDisposition::Latest);

    // ── Result: post-execute history, empty tool_call_ids per Q4 ───────────
    let result_body = json!({
        "iteration": 0,
        "decide": {"proposal_count": 2},
        "execute": {"result_count": 2, "loop_signal": "Continue"},
        "history_after_iter": [
            "user: Summarise the repo",
            "assistant: (tool output + summary)",
        ],
    });
    let result_cp_id = CheckpointId::new("cp_result_dual_1");
    let result = svc
        .save_dual(
            &project,
            &run_id,
            result_cp_id.clone(),
            CheckpointKind::Result,
            result_body.clone(),
            // RFC 020 §6 Q4 — Result carries empty tool_call_ids.
            Vec::new(),
        )
        .await
        .expect("save_dual Result must succeed");
    assert_eq!(result.checkpoint_id, result_cp_id);
    assert_eq!(
        result.disposition,
        CheckpointDisposition::Latest,
        "Result checkpoint supersedes Intent as the newest",
    );

    // ── Verify both checkpoints exist on the run with distinct IDs ─────────
    let by_run = svc.list_by_run(&run_id, 10).await.unwrap();
    assert_eq!(by_run.len(), 2, "dual checkpoint: expected exactly two");
    let ids: std::collections::HashSet<_> =
        by_run.iter().map(|c| c.checkpoint_id.clone()).collect();
    assert!(ids.contains(&intent_cp_id), "intent cp missing");
    assert!(ids.contains(&result_cp_id), "result cp missing");
    assert_ne!(intent_cp_id, result_cp_id, "distinct IDs (§6 Q3)");

    // ── Verify the emitted CheckpointRecorded events carry the Track 4 fields ──
    let recorded = read_checkpoint_events(store.as_ref(), &run_id).await;
    assert_eq!(
        recorded.len(),
        2,
        "expected 2 CheckpointRecorded events, got {recorded:#?}"
    );
    // Order of emission: Intent first (append order preserved by
    // `EventLog::append` + boot-time replay semantics).
    assert_eq!(recorded[0].kind, Some(CheckpointKind::Intent));
    assert_eq!(recorded[1].kind, Some(CheckpointKind::Result));
    // Intent carries the planned tool calls.
    assert_eq!(
        recorded[0].tool_call_ids, planned,
        "Intent checkpoint must surface planned ToolCallIds for recovery replay"
    );
    // Result is empty per Q4.
    assert!(
        recorded[1].tool_call_ids.is_empty(),
        "Result checkpoint tool_call_ids must be empty (Intent owns the list)"
    );
    // message_history_size is populated on both and non-zero (full snapshot).
    let intent_size = recorded[0]
        .message_history_size
        .expect("Intent must carry message_history_size");
    let result_size = recorded[1]
        .message_history_size
        .expect("Result must carry message_history_size");
    assert!(
        intent_size > 0,
        "Intent checkpoint body must be non-empty (full snapshot, Gap 3)"
    );
    assert!(result_size > 0, "Result checkpoint body must be non-empty");
    // Result grew (extra execute-side content appended).
    assert!(
        result_size >= intent_size,
        "Result size ({result_size}) must be >= Intent size ({intent_size}) — \
         message history only grows within an iteration"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// RFC 020 Integration Test #11 extension — RecoverySummary event per sweep.
// ─────────────────────────────────────────────────────────────────────────────

/// RFC 020 Track 4 extends Test #11 (`recovery_summary_emitted_once_per_boot`
/// in `test_rfc020_recovery.rs`). That test proves the `RecoveryAttempted`
/// / `RecoveryCompleted` pair fires once per boot via the LiveHarness
/// subprocess contract; this one proves the additional richer
/// `RecoverySummaryEmitted` audit event fires exactly once per
/// `RecoveryService::recover_all` invocation and carries the expected
/// `boot_id` payload.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_summary_event_emitted_on_restart() {
    use cairn_domain::BootId;
    use cairn_runtime::{RecoveryService, RecoveryServiceImpl};
    use std::sync::Arc;

    let store = Arc::new(InMemoryStore::new());
    let svc = RecoveryServiceImpl::new(store.clone());

    // Two invocations with distinct boot_ids (simulates two process boots).
    let boot1 = BootId::new("boot_track4_test_1");
    let boot2 = BootId::new("boot_track4_test_2");

    // Sweep 1 → one RecoverySummary event.
    let _ = svc
        .recover_all(&boot1, &[])
        .await
        .expect("sweep 1 must succeed");
    let summaries_after_sweep1 = count_recovery_summaries(store.as_ref()).await;
    assert_eq!(
        summaries_after_sweep1, 1,
        "first sweep must emit exactly one recovery_summary"
    );

    // Sweep 2 → exactly one more RecoverySummary event.
    let _ = svc
        .recover_all(&boot2, &[])
        .await
        .expect("sweep 2 must succeed");
    let summaries_after_sweep2 = count_recovery_summaries(store.as_ref()).await;
    assert_eq!(
        summaries_after_sweep2, 2,
        "second sweep must emit exactly one additional recovery_summary \
         (total=2, got={summaries_after_sweep2})"
    );

    // Boot ids on the two summaries must differ and preserve order.
    let ids = recovery_summary_boot_ids(store.as_ref()).await;
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], "boot_track4_test_1");
    assert_eq!(ids[1], "boot_track4_test_2");
}

// ─────────────────────────────────────────────────────────────────────────────
// RFC 020 Integration Test #15 — checkpoint body compression (SKIPPED BY DESIGN).
// ─────────────────────────────────────────────────────────────────────────────

/// RFC 020 Integration Test #15: checkpoint body compression / diff-based
/// checkpoints.
///
/// **Skipped by design.** Per the Gap 3 resolution documented in
/// `project_rfc020_delta_and_gaps.md` Part B, v1 ships full snapshots per
/// checkpoint — not diffs. The `CheckpointRecorded.message_history_size`
/// field is populated so operators can monitor checkpoint cost and, if
/// average size exceeds their policy, a future Track 4b can add diffing.
///
/// Un-ignore this test when a diff-based Track 4b lands; the concrete
/// assertions will be: (a) checkpoint body is smaller than the full
/// message history, (b) replay reconstructs history from the base +
/// diffs, (c) `message_history_size` reports the compacted size.
#[ignore = "Gap 3 resolution: v1 ships full snapshots, not diffs. \
            Un-ignore when a diff-based Track 4b is adopted."]
#[tokio::test]
async fn checkpoint_compression_v1_skipped_by_design() {
    unreachable!(
        "see rustdoc above — Gap 3 resolution defers diff-based checkpoints \
         to a future Track 4b"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

async fn read_checkpoint_events(
    store: &InMemoryStore,
    run_id: &RunId,
) -> Vec<cairn_domain::CheckpointRecorded> {
    let mut out = Vec::new();
    let mut cursor: Option<cairn_store::EventPosition> = None;
    loop {
        let page = store.read_stream(cursor, 500).await.unwrap();
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|e| e.position);
        for stored in &page {
            if let RuntimeEvent::CheckpointRecorded(ev) = &stored.envelope.payload {
                if ev.run_id == *run_id {
                    out.push(ev.clone());
                }
            }
        }
        if page.len() < 500 {
            break;
        }
        cursor = last;
    }
    out
}

async fn count_recovery_summaries(store: &InMemoryStore) -> usize {
    let mut count = 0usize;
    let mut cursor: Option<cairn_store::EventPosition> = None;
    loop {
        let page = store.read_stream(cursor, 500).await.unwrap();
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|e| e.position);
        for stored in &page {
            if matches!(
                stored.envelope.payload,
                RuntimeEvent::RecoverySummaryEmitted(_)
            ) {
                count += 1;
            }
        }
        if page.len() < 500 {
            break;
        }
        cursor = last;
    }
    count
}

async fn recovery_summary_boot_ids(store: &InMemoryStore) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    let mut cursor: Option<cairn_store::EventPosition> = None;
    loop {
        let page = store.read_stream(cursor, 500).await.unwrap();
        if page.is_empty() {
            break;
        }
        let last = page.last().map(|e| e.position);
        for stored in &page {
            if let RuntimeEvent::RecoverySummaryEmitted(ev) = &stored.envelope.payload {
                ids.push(ev.boot_id.clone());
            }
        }
        if page.len() < 500 {
            break;
        }
        cursor = last;
    }
    ids
}
