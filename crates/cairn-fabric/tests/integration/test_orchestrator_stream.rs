//! End-to-end proof that `OrchestratorLoop::run` populates FF's
//! attempt-scoped stream in the exact order the loop emits.
//!
//! This test drives the REAL loop (not direct sink calls) through one
//! iteration with scripted `Gather` / `Decide` / `Execute` phases,
//! then reads the stream via `restore_frames()` and asserts frames
//! appear in the order `OrchestratorLoop::run_inner` writes them:
//!
//! ```text
//!   §3b'  log_llm_response  (post-decide)
//!   §5a   log_tool_call     (pre-execute)
//!   §5b   log_tool_result   (post-execute)
//!   §6b   save_checkpoint   (post-CheckpointHook::save)
//! ```
//!
//! XRANGE preserves insertion order in Valkey streams, so matching the
//! asserted order means the loop wrote the frames in the correct order
//! — not that Valkey sorted them. Driving the sink directly would only
//! prove the Valkey invariant, not the cairn-side emission order.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{decisions::RunMode, ActionProposal, ActionType, ProjectKey, RunId, TaskId};
use cairn_fabric::{CairnWorker, FabricConfig};
use cairn_orchestrator::{
    context::{ActionResult, ActionStatus, DecideOutput, ExecuteOutcome, GatherOutput, LoopSignal},
    DecidePhase, ExecutePhase, GatherPhase, LoopConfig, OrchestrationContext, OrchestratorError,
    OrchestratorLoop, TaskFrameSink,
};
use ff_core::contracts::StreamFrame;

use crate::TestHarness;

// ── Mock phases ─────────────────────────────────────────────────────────────
//
// Minimal stubs that let the real OrchestratorLoop::run drive a single
// iteration end-to-end. They mirror the `ScriptedDecide` / `ScriptedExecute`
// pattern used by the loop_runner's inline unit tests, extracted here
// because those are `pub(crate)` in the test module.

struct EmptyGather;
#[async_trait]
impl GatherPhase for EmptyGather {
    async fn gather(&self, _ctx: &OrchestrationContext) -> Result<GatherOutput, OrchestratorError> {
        Ok(GatherOutput::default())
    }
}

struct FixedDecide {
    output: DecideOutput,
}
#[async_trait]
impl DecidePhase for FixedDecide {
    async fn decide(
        &self,
        _ctx: &OrchestrationContext,
        _gather: &GatherOutput,
    ) -> Result<DecideOutput, OrchestratorError> {
        Ok(self.output.clone())
    }
}

/// ExecutePhase that marks every proposal as Succeeded and returns
/// `LoopSignal::Done`, so the loop exits cleanly after one iteration.
/// The `tool_output` is populated so the loop's `log_tool_result` call
/// writes a real payload instead of the null-fallback branch.
struct OneShotExecute;
#[async_trait]
impl ExecutePhase for OneShotExecute {
    async fn execute(
        &self,
        _ctx: &OrchestrationContext,
        decide: &DecideOutput,
    ) -> Result<ExecuteOutcome, OrchestratorError> {
        let results = decide
            .proposals
            .iter()
            .map(|p| ActionResult {
                proposal: p.clone(),
                status: ActionStatus::Succeeded,
                tool_output: Some(serde_json::json!({ "ok": true })),
                invocation_id: None,
                duration_ms: 0,
            })
            .collect();
        Ok(ExecuteOutcome {
            results,
            loop_signal: LoopSignal::Done,
        })
    }
}

// ── Test ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn orchestrator_loop_emits_four_frames_in_per_iteration_order() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    // 1. Submit a task so CairnWorker::claim_next finds it eligible.
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

    // 2. Ask the in-process scheduler for a claim grant against the
    //    harness lane, then materialize it into a live CairnTask.
    //    Scheduler polls a rolling 32-partition window per call, so
    //    loop with a short backoff until a grant comes up.
    let worker_config = worker_config_from(&h);
    let worker = CairnWorker::connect(&worker_config, h.fabric.bridge.clone())
        .await
        .expect("CairnWorker::connect failed");
    let claimed = grant_and_claim_with_retry(&h, &worker_config, &worker).await;

    // 3. Build the OrchestrationContext + scripted phases that emit
    //    exactly one tool proposal (so log_tool_call + log_tool_result
    //    each fire exactly once) and terminate on the first iteration.
    let run_id = RunId::new(format!("integ_run_{}", uuid::Uuid::new_v4()));
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock before epoch")
        .as_millis() as u64;
    let ctx = OrchestrationContext {
        project: h.project.clone(),
        session_id: session_id.clone(),
        run_id: run_id.clone(),
        task_id: Some(task_id.clone()),
        iteration: 0,
        goal: "orchestrator-stream round-trip".to_owned(),
        agent_type: "test".to_owned(),
        // Use the real wall-clock so the loop's deadline_ms =
        // run_started_at_ms + timeout_ms is in the future. 0 would
        // trip the §1 timeout check before iteration 0 ran.
        run_started_at_ms: now_ms,
        working_dir: PathBuf::from("/tmp"),
        run_mode: RunMode::Direct,
        discovered_tool_names: Vec::new(),
        step_history: Vec::new(),
    };

    let decide_output = DecideOutput {
        raw_response: "invoke fs.read".to_owned(),
        proposals: vec![ActionProposal {
            action_type: ActionType::InvokeTool,
            description: "read the smoke file".to_owned(),
            confidence: 0.95,
            tool_name: Some("fs.read".to_owned()),
            tool_args: Some(serde_json::json!({ "path": "/tmp/x" })),
            requires_approval: false,
        }],
        calibrated_confidence: 0.95,
        requires_approval: false,
        model_id: "claude-3-opus".to_owned(),
        latency_ms: 1_200,
        input_tokens: Some(500),
        output_tokens: Some(200),
    };

    let sink: Arc<dyn TaskFrameSink> = Arc::new(claimed);
    let orchestrator = OrchestratorLoop::new(
        EmptyGather,
        FixedDecide {
            output: decide_output,
        },
        OneShotExecute,
        LoopConfig::default(),
    )
    .with_task_sink(sink);

    // 4. Run the real loop. Completes after exactly one iteration (Done).
    let termination = orchestrator
        .run(ctx)
        .await
        .expect("OrchestratorLoop::run returned an infrastructure error");
    assert!(
        matches!(
            termination,
            cairn_orchestrator::LoopTermination::Completed { .. }
        ),
        "loop did not reach Completed termination, got {termination:?}",
    );

    // 5. Read the FF stream and assert frame order matches the loop's
    //    per-iteration emission order: llm_response → tool_call →
    //    tool_result → checkpoint. XRANGE preserves insertion order;
    //    an out-of-order result proves the loop emitted in the wrong
    //    sequence (or the sink dropped one frame).
    let eid = cairn_fabric::id_map::session_task_to_execution_id(
        &h.project,
        &session_id,
        &task_id,
        h.partition_config(),
    );
    let frames: Vec<StreamFrame> = cairn_fabric::stream::restore_frames(
        &h.fabric.runtime.client,
        h.partition_config(),
        &eid,
        ff_core::types::AttemptIndex::new(0),
        100,
    )
    .await
    .expect("restore_frames failed");

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
        vec!["llm_response", "tool_call", "tool_result", "checkpoint"],
        "frames emitted in wrong order by OrchestratorLoop::run_inner; \
         expected llm_response → tool_call → tool_result → checkpoint \
         (see loop_runner.rs §3b' → §5a → §5b → §6b)",
    );

    // 6. Payload round-trips — spot-check each frame so a silent
    //    field-drop in any of the four sink methods surfaces here,
    //    not in downstream resume reconstruction.
    let parse = |frame: &StreamFrame| -> serde_json::Value {
        let payload = frame
            .fields
            .get("payload")
            .expect("frame missing payload field");
        serde_json::from_str(payload).expect("payload not valid JSON")
    };

    // llm_response frame — `stream.rs::log_llm_response` serializes model +
    // token counts + latency + timestamp. Assert EVERY field is present
    // AND typed so a silent field-drop or type coercion in the sink
    // (e.g. u64→String) surfaces here, not in downstream cost
    // reconciliation.
    let llm = parse(&frames[0]);
    assert_eq!(llm["model"], "claude-3-opus");
    assert_eq!(
        llm["tokens_in"].as_u64(),
        Some(500),
        "tokens_in must be a u64 number, got {:?}",
        llm["tokens_in"]
    );
    assert_eq!(
        llm["tokens_out"].as_u64(),
        Some(200),
        "tokens_out must be a u64 number, got {:?}",
        llm["tokens_out"]
    );
    assert_eq!(
        llm["latency_ms"].as_u64(),
        Some(1_200),
        "latency_ms must be a u64 number, got {:?}",
        llm["latency_ms"]
    );
    assert!(
        llm["timestamp_ms"].as_u64().is_some(),
        "llm_response must carry a numeric timestamp_ms, got {:?}",
        llm["timestamp_ms"]
    );

    // tool_call frame — `stream.rs::log_tool_call` serializes tool_name +
    // args + timestamp. Args is a nested JSON object round-tripped verbatim.
    let tool_call = parse(&frames[1]);
    assert_eq!(tool_call["tool_name"], "fs.read");
    assert_eq!(tool_call["args"]["path"], "/tmp/x");
    assert!(
        tool_call["timestamp_ms"].as_u64().is_some(),
        "tool_call must carry a numeric timestamp_ms, got {:?}",
        tool_call["timestamp_ms"]
    );

    // tool_result frame — `stream.rs::log_tool_result` serializes
    // tool_name + output + success + duration_ms + timestamp. duration_ms
    // is the per-result averaged value the loop computes
    // (loop_runner.rs:485-489) — present even when it's 0 from a
    // sub-result-count wall-clock truncation (see issue #33). Assert
    // presence + type, not a specific value, so this test doesn't flake
    // on fast runs.
    let tool_result = parse(&frames[2]);
    assert_eq!(tool_result["tool_name"], "fs.read");
    assert_eq!(tool_result["success"], true);
    assert_eq!(tool_result["output"]["ok"], true);
    assert!(
        tool_result["duration_ms"].as_u64().is_some(),
        "tool_result must carry a numeric duration_ms (0 = unknown, \
         not missing), got {:?}",
        tool_result["duration_ms"]
    );
    assert!(
        tool_result["timestamp_ms"].as_u64().is_some(),
        "tool_result must carry a numeric timestamp_ms, got {:?}",
        tool_result["timestamp_ms"]
    );

    // checkpoint frame — `save_checkpoint` takes raw bytes from the loop
    // runner's per-iteration JSON snapshot. Assert the payload bytes are
    // non-empty (FF would reject empty) AND that they decode back as
    // JSON carrying iteration + run_id + session_id. If `save_checkpoint`
    // ever drops the bytes or writes zero-length, this test fails here
    // instead of silently in resume reconstruction.
    let checkpoint_bytes = frames[3]
        .fields
        .get("payload")
        .expect("checkpoint frame missing payload field");
    assert!(
        !checkpoint_bytes.is_empty(),
        "checkpoint payload must be non-empty bytes (FF rejects empty \
         frames); got {} bytes",
        checkpoint_bytes.len()
    );
    let checkpoint = parse(&frames[3]);
    assert_eq!(checkpoint["iteration"], 0);
    assert_eq!(checkpoint["run_id"], run_id.as_str());
    assert_eq!(checkpoint["session_id"], session_id.as_str());

    h.teardown().await;
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn worker_config_from(h: &TestHarness) -> FabricConfig {
    FabricConfig {
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
    }
}

/// The scheduler's `claim_for_worker` scans a rolling 32-partition
/// window per call (`PARTITION_SCAN_CHUNK` in ff-sdk). With 256
/// partitions up to 8 polls are needed to cover the full keyspace.
/// Loop with a short backoff until a grant comes up; deadline-bounded
/// to surface real failures.
async fn grant_and_claim_with_retry(
    h: &TestHarness,
    worker_config: &FabricConfig,
    worker: &CairnWorker,
) -> cairn_fabric::CairnTask {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let grant = match h
            .fabric
            .scheduler
            .claim_for_worker(
                &worker_config.lane_id,
                &worker_config.worker_id,
                &worker_config.worker_instance_id,
                worker_config.grant_ttl_ms,
            )
            .await
        {
            Ok(Some(g)) => g,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    panic!(
                        "no eligible task returned within 10s — lane/capability/partition \
                         mismatch between harness submit path and scheduler claim path"
                    );
                }
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            }
            Err(e) => panic!("scheduler claim_for_worker failed: {e}"),
        };
        match worker
            .claim_from_grant(worker_config.lane_id.clone(), grant)
            .await
        {
            Ok(task) => return task,
            Err(e) => panic!("claim_from_grant failed: {e}"),
        }
    }
}

#[allow(dead_code)]
fn _task_id_is_imported(_: TaskId) {}

// Same for ProjectKey — named here so future maintainers find the
// concrete scope-triple the harness derives.
#[allow(dead_code)]
fn _project_key_is_imported(_: ProjectKey) {}
