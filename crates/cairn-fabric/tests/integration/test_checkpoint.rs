// Stream read (checkpoint restore) integration test.
//
// Proves `cairn_fabric::stream::restore_frames` round-trips against live FF:
// append 3 frames, then restore and assert we get all 3 back in XRANGE order
// with the expected frame_type per entry. FF is the sole source of truth for
// frames — no cairn-side cache touched.
//
// Runs against a testcontainers-provisioned Valkey (see tests/integration.rs).
// No #[ignore], no manual setup — `cargo test -p cairn-fabric --test
// integration` boots a disposable container and loads the FF Lua library
// automatically.

use cairn_fabric::id_map;
use cairn_fabric::stream::{restore_frames, FRAME_CHECKPOINT, FRAME_TOOL_CALL, FRAME_TOOL_RESULT};
use ff_core::contracts::{AppendFrameArgs, AppendFrameResult, STREAM_READ_HARD_CAP};
use ff_core::keys::ExecKeyContext;
use ff_core::partition::execution_partition;
use ff_core::types::{AttemptId, AttemptIndex, ExecutionId, LeaseEpoch, LeaseId, TimestampMs};
use ff_script::functions::stream::{ff_append_frame, StreamOpKeys};

use crate::TestHarness;

/// Thin helper: read (lease_id, lease_epoch, attempt_id, attempt_index) from
/// the execution core hash. task_service::claim writes these directly; we
/// reuse them to forge ff_append_frame calls as if we were the lease holder.
///
/// Reading exec_core (HGETALL) keeps the test lean-bridge honest — we do not
/// thread lease state through a cairn-side struct just to exercise append.
async fn lease_context(
    client: &ferriskey::Client,
    ctx: &ExecKeyContext,
) -> (LeaseId, LeaseEpoch, AttemptId, AttemptIndex) {
    let fields: std::collections::HashMap<String, String> = client
        .hgetall(&ctx.core())
        .await
        .expect("HGETALL exec_core failed");

    let lease_id = LeaseId::parse(
        fields
            .get("current_lease_id")
            .expect("lease_id missing — did claim() succeed?")
            .trim(),
    )
    .expect("bad lease_id");
    let lease_epoch = LeaseEpoch::new(
        fields
            .get("current_lease_epoch")
            .and_then(|s| s.parse().ok())
            .unwrap_or(1),
    );
    let attempt_id = AttemptId::parse(
        fields
            .get("current_attempt_id")
            .expect("attempt_id missing")
            .trim(),
    )
    .expect("bad attempt_id");
    let attempt_index = AttemptIndex::new(
        fields
            .get("current_attempt_index")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0),
    );
    (lease_id, lease_epoch, attempt_id, attempt_index)
}

#[tokio::test]
async fn test_checkpoint_restore_reads_frames() {
    let h = TestHarness::setup().await;
    let session_id = h.unique_session_id();
    let task_id = h.unique_task_id();

    // Get an Active execution via task_service so lease + attempt fields are
    // populated on exec_core. Terminal operations need a lease holder; so
    // does ff_append_frame (the Lua validates lease_id/epoch).
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

    h.fabric
        .tasks
        .claim(&h.project, Some(&session_id), &task_id, "test-worker".into(), 30_000)
        .await
        .expect("claim failed");

    let eid: ExecutionId = id_map::session_task_to_execution_id(&h.project, &session_id, &task_id, h.partition_config());
    let partition = execution_partition(&eid, h.partition_config());
    let ctx = ExecKeyContext::new(&partition, &eid);
    let (lease_id, lease_epoch, attempt_id, attempt_index) =
        lease_context(&h.fabric.runtime.client, &ctx).await;
    let keys = StreamOpKeys { ctx: &ctx };

    // Append 3 frames in known order. Stream is created lazily on first
    // append — no setup call needed.
    for (idx, ft) in [FRAME_TOOL_CALL, FRAME_TOOL_RESULT, FRAME_CHECKPOINT]
        .iter()
        .enumerate()
    {
        let args = AppendFrameArgs {
            execution_id: eid.clone(),
            attempt_index,
            lease_id: lease_id.clone(),
            lease_epoch,
            attempt_id: attempt_id.clone(),
            frame_type: (*ft).to_owned(),
            timestamp: TimestampMs::now(),
            payload: format!(r#"{{"seq":{idx}}}"#).into_bytes(),
            encoding: Some("utf8".into()),
            metadata_json: None,
            correlation_id: None,
            source: Some("test-worker".into()),
            retention_maxlen: None,
            max_payload_bytes: None,
        };
        let result = ff_append_frame(&h.fabric.runtime.client, &keys, &args)
            .await
            .unwrap_or_else(|e| panic!("append frame {idx} failed: {e}"));
        assert!(
            matches!(result, AppendFrameResult::Appended { .. }),
            "frame {idx} did not append cleanly: {result:?}"
        );
    }

    // Restore — this is the API under test.
    let frames = restore_frames(
        &h.fabric.runtime.client,
        &h.fabric.runtime.partition_config,
        &eid,
        attempt_index,
        STREAM_READ_HARD_CAP,
    )
    .await
    .expect("restore_frames failed");

    assert_eq!(
        frames.len(),
        3,
        "expected 3 frames, got {}: {frames:?}",
        frames.len()
    );

    // XRANGE returns entries in ID-ascending order → our append order.
    let types: Vec<&str> = frames
        .iter()
        .map(|f| f.fields.get("frame_type").map(String::as_str).unwrap_or(""))
        .collect();
    assert_eq!(
        types,
        vec![FRAME_TOOL_CALL, FRAME_TOOL_RESULT, FRAME_CHECKPOINT],
        "frame_type sequence did not match append order",
    );

    // Spot-check that the payload survived the round trip. `ff_append_frame`
    // writes under the `payload` field — see lua/stream.lua.
    let first_payload = frames[0].fields.get("payload").cloned().unwrap_or_default();
    assert!(
        first_payload.contains("\"seq\":0"),
        "frame 0 payload missing seq marker: {first_payload}"
    );

    h.teardown().await;
}
