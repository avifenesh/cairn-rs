//! End-to-end proof that the orchestrator's `TaskFrameSink` wiring
//! reaches FF's attempt-scoped stream.
//!
//! The orchestrator loop calls `task_sink.log_tool_call`,
//! `log_tool_result`, `log_llm_response`, and `save_checkpoint` at the
//! points wired in commits 2–4 of this stack. The blanket
//! `impl TaskFrameSink for CairnTask` delegates to `StreamWriter`,
//! which appends frames via `ff_append_frame`. This test claims a
//! real `CairnTask` against a testcontainers-managed Valkey, drives
//! one synthetic iteration through the sink trait, then reads the
//! stream back via `restore_frames()` and asserts:
//!
//! 1. All four frame types landed.
//! 2. They appear in the order the orchestrator emits them:
//!    `tool_call` → `tool_result` → `llm_response` → `checkpoint`.
//! 3. The payload round-trips (the `tool_name` on frame 0 matches
//!    what we logged).
//!
//! # Why `insecure-direct-claim`?
//!
//! The only public path to a live `CairnTask` today is
//! `CairnWorker::claim_next`, which is gated behind the
//! `insecure-direct-claim` feature (`ClaimedTask::new` is `pub(crate)`
//! on ff-sdk — see `crates/cairn-fabric/src/worker_sdk.rs` module
//! docstring for the full architectural note). The test is
//! cfg-gated so default CI runs skip it; the pre-push + fabric
//! integration CI job enables the feature.

#![cfg(feature = "insecure-direct-claim")]

use std::collections::BTreeSet;
use std::sync::Arc;

use cairn_fabric::{CairnWorker, FabricConfig};
use cairn_orchestrator::task_sink::TaskFrameSink;
use ff_core::contracts::StreamFrame;

use crate::TestHarness;

/// Claim a task via `CairnWorker::claim_next` and drive the orchestrator
/// sink surface through it. Reads stream via `restore_frames` and
/// asserts four frames in emission order.
#[tokio::test]
async fn orchestrator_sink_emits_four_frames_in_xrange_order() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    // 1. Submit a task via FabricTaskService so it's eligible for claim.
    h.fabric
        .tasks
        .submit(
            &h.project,
            task_id.clone(),
            None,
            None,
            0,
            Some(&session_id),
        )
        .await
        .expect("submit failed");

    // 2. Stand up a worker that shares the harness's lane/config. The
    //    `bridge` clone lets CairnTask emit the ExecutionCompleted
    //    bridge event if we ever consume it; this test drops the task
    //    without a terminal call and relies on FF's lease-expiry
    //    scanner (tests don't assert terminal projection state).
    let worker_config = FabricConfig {
        valkey_host: h.fabric.runtime.config.valkey_host.clone(),
        valkey_port: h.fabric.runtime.config.valkey_port,
        tls: h.fabric.runtime.config.tls,
        cluster: h.fabric.runtime.config.cluster,
        lane_id: h.fabric.runtime.config.lane_id.clone(),
        worker_id: ff_core::types::WorkerId::new("orchestrator-stream-worker"),
        worker_instance_id: ff_core::types::WorkerInstanceId::new(uuid::Uuid::new_v4().to_string()),
        namespace: h.fabric.runtime.config.namespace.clone(),
        lease_ttl_ms: h.fabric.runtime.config.lease_ttl_ms,
        grant_ttl_ms: h.fabric.runtime.config.grant_ttl_ms,
        max_concurrent_tasks: h.fabric.runtime.config.max_concurrent_tasks,
        signal_dedup_ttl_ms: h.fabric.runtime.config.signal_dedup_ttl_ms,
        fcall_timeout_ms: h.fabric.runtime.config.fcall_timeout_ms,
        worker_capabilities: BTreeSet::new(),
        waitpoint_hmac_secret: h.fabric.runtime.config.waitpoint_hmac_secret.clone(),
        waitpoint_hmac_kid: h.fabric.runtime.config.waitpoint_hmac_kid.clone(),
    };
    let worker = CairnWorker::connect(&worker_config, h.fabric.bridge.clone())
        .await
        .expect("CairnWorker::connect failed");

    // `CairnWorker::claim_next` scans a rolling 32-partition window per
    // call (PARTITION_SCAN_CHUNK in ff-sdk). With 256 partitions, up to
    // 8 polls are needed to cover the full keyspace. Loop with a short
    // backoff until a task comes up OR we hit the deadline.
    let claim_deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let claimed = loop {
        match worker.claim_next().await.expect("claim_next rpc failed") {
            Some(task) => break task,
            None => {
                if std::time::Instant::now() >= claim_deadline {
                    panic!(
                        "no eligible task returned within 10s — lane/capability/partition \
                         mismatch between harness submit path and worker claim path"
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    };

    // 3. Call the orchestrator's sink trait on the live CairnTask.
    //    `Arc<dyn TaskFrameSink>` is how OrchestratorLoop holds it; we
    //    recreate that shape to be honest about which path is exercised.
    //
    //    Ordering here mirrors `OrchestratorLoop::run_inner`:
    //      §5a pre-execute:         log_tool_call
    //      §5b post-execute:        log_tool_result
    //      §3b' post-decide:        log_llm_response
    //      §6b post-checkpoint hook: save_checkpoint
    //
    //    The loop emits tool_call before decide (§5a sits after §3b' in
    //    the code flow but before the next iteration's decide); this
    //    test mirrors a single iteration's on-wire order precisely.
    let sink: Arc<dyn TaskFrameSink> = Arc::new(claimed);

    sink.log_tool_call("fs.read", &serde_json::json!({ "path": "/tmp/x" }))
        .await
        .expect("log_tool_call");
    sink.log_tool_result(
        "fs.read",
        &serde_json::json!({ "content": "hello" }),
        true,
        42,
    )
    .await
    .expect("log_tool_result");
    sink.log_llm_response("claude-3-opus", 500, 200, 1_200)
        .await
        .expect("log_llm_response");
    sink.save_checkpoint(b"{\"iteration\":0,\"state\":\"checkpoint\"}")
        .await
        .expect("save_checkpoint");

    // 4. Read the stream back via restore_frames. The execution id for
    //    the task matches the id_map derivation cairn-fabric uses;
    //    attempt index 0 is the first attempt (this task hasn't been
    //    re-claimed).
    //
    //    `restore_frames` is the same API orchestrator-resumption
    //    paths will use; calling it here proves the write half lands
    //    on the key the read half will query.
    let eid = cairn_fabric::id_map::task_to_execution_id(&h.project, &task_id);
    let frames: Vec<StreamFrame> = cairn_fabric::stream::restore_frames(
        &h.fabric.runtime.client,
        &h.fabric.runtime.partition_config,
        &eid,
        ff_core::types::AttemptIndex::new(0),
        100,
    )
    .await
    .expect("restore_frames failed");

    // 5. Assertions.
    assert_eq!(
        frames.len(),
        4,
        "expected 4 frames (tool_call, tool_result, llm_response, checkpoint), got {}: {frames:?}",
        frames.len(),
    );

    let frame_types: Vec<&str> = frames
        .iter()
        .map(|f| {
            f.fields
                .get("frame_type")
                .map(String::as_str)
                .unwrap_or("<missing>")
        })
        .collect();
    assert_eq!(
        frame_types,
        vec!["tool_call", "tool_result", "llm_response", "checkpoint"],
        "frames out of order or wrong types",
    );

    // 6. Spot-check the first frame's payload — confirms the tool_name
    //    we logged round-trips through ff_append_frame + XRANGE.
    let tool_call_payload = frames[0]
        .fields
        .get("payload")
        .expect("tool_call frame missing payload field");
    let parsed: serde_json::Value =
        serde_json::from_str(tool_call_payload).expect("tool_call payload not valid JSON");
    assert_eq!(
        parsed["tool_name"], "fs.read",
        "tool_call payload lost its tool_name on round-trip: {parsed}"
    );

    h.teardown().await;
}
