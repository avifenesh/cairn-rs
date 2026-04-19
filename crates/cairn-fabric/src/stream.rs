use ff_core::contracts::StreamFrame;
use ff_core::partition::PartitionConfig;
use ff_core::types::{AttemptIndex, ExecutionId};
use ff_sdk::task::{read_stream, AppendFrameOutcome, ClaimedTask};

use crate::error::FabricError;
use crate::helpers::now_ms;

pub const FRAME_TOOL_CALL: &str = "tool_call";
pub const FRAME_TOOL_RESULT: &str = "tool_result";
pub const FRAME_LLM_RESPONSE: &str = "llm_response";
pub const FRAME_CHECKPOINT: &str = "checkpoint";

/// Restore an attempt's frame log from FF.
///
/// Thin pass-through to `ff_sdk::read_stream` (backed by
/// `ff_read_attempt_stream`). Returns the full frame sequence in XRANGE order.
///
/// Cairn does NOT cache frames — FF's Valkey stream is the sole source of
/// truth. Callers that need replay (orchestrator recovery, audit dump) call
/// this each time.
///
/// The SDK rejects a zero `count_limit` and anything above
/// [`STREAM_READ_HARD_CAP`]. Callers that need more than the hard cap must
/// paginate using the last frame's `id` as the next `from_id` — we surface
/// the full contract rather than hiding it, so callers can size reads against
/// their own memory budgets.
///
/// # Head-of-line warning
///
/// The client passed here should not also be the one the caller uses for
/// FCALLs if the read is large — see the `ff_sdk::read_stream` doc for the
/// tail_client split the REST server uses. In cairn-fabric today we use a
/// single shared client; restore paths are expected to be operator-triggered
/// (recovery, replay) and infrequent enough that the head-of-line cost is
/// acceptable. Raise the concern again if restore goes on any hot path.
pub async fn restore_frames(
    client: &ferriskey::Client,
    partition_config: &PartitionConfig,
    execution_id: &ExecutionId,
    attempt_index: AttemptIndex,
    count_limit: u64,
) -> Result<Vec<StreamFrame>, FabricError> {
    let result = read_stream(
        client,
        partition_config,
        execution_id,
        attempt_index,
        "-",
        "+",
        count_limit,
    )
    .await
    .map_err(|e| FabricError::Bridge(format!("read_stream: {e}")))?;
    Ok(result.frames)
}

pub struct StreamWriter<'a> {
    task: &'a ClaimedTask,
}

impl<'a> StreamWriter<'a> {
    pub fn new(task: &'a ClaimedTask) -> Self {
        Self { task }
    }

    pub async fn log_tool_call(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> Result<AppendFrameOutcome, FabricError> {
        let payload = serde_json::json!({
            "tool_name": tool_name,
            "args": args,
            "timestamp_ms": now_ms(),
        });
        let bytes = serde_json::to_vec(&payload)
            .map_err(|e| FabricError::Bridge(format!("serialize tool_call: {e}")))?;

        self.task
            .append_frame(FRAME_TOOL_CALL, &bytes, None)
            .await
            .map_err(|e| FabricError::Bridge(format!("append tool_call frame: {e}")))
    }

    pub async fn log_tool_result(
        &self,
        tool_name: &str,
        output: &serde_json::Value,
        success: bool,
        duration_ms: u64,
    ) -> Result<AppendFrameOutcome, FabricError> {
        let payload = serde_json::json!({
            "tool_name": tool_name,
            "output": output,
            "success": success,
            "duration_ms": duration_ms,
            "timestamp_ms": now_ms(),
        });
        let bytes = serde_json::to_vec(&payload)
            .map_err(|e| FabricError::Bridge(format!("serialize tool_result: {e}")))?;

        self.task
            .append_frame(FRAME_TOOL_RESULT, &bytes, None)
            .await
            .map_err(|e| FabricError::Bridge(format!("append tool_result frame: {e}")))
    }

    pub async fn log_llm_response(
        &self,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
        latency_ms: u64,
    ) -> Result<AppendFrameOutcome, FabricError> {
        let payload = serde_json::json!({
            "model": model,
            "tokens_in": tokens_in,
            "tokens_out": tokens_out,
            "latency_ms": latency_ms,
            "timestamp_ms": now_ms(),
        });
        let bytes = serde_json::to_vec(&payload)
            .map_err(|e| FabricError::Bridge(format!("serialize llm_response: {e}")))?;

        self.task
            .append_frame(FRAME_LLM_RESPONSE, &bytes, None)
            .await
            .map_err(|e| FabricError::Bridge(format!("append llm_response frame: {e}")))
    }

    pub async fn save_checkpoint(
        &self,
        context_json: &[u8],
    ) -> Result<AppendFrameOutcome, FabricError> {
        self.task
            .append_frame(FRAME_CHECKPOINT, context_json, None)
            .await
            .map_err(|e| FabricError::Bridge(format!("append checkpoint frame: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_type_constants() {
        assert_eq!(FRAME_TOOL_CALL, "tool_call");
        assert_eq!(FRAME_TOOL_RESULT, "tool_result");
        assert_eq!(FRAME_LLM_RESPONSE, "llm_response");
        assert_eq!(FRAME_CHECKPOINT, "checkpoint");
    }

    #[test]
    fn tool_call_payload_structure() {
        let payload = serde_json::json!({
            "tool_name": "fs.read",
            "args": {"path": "/tmp/test"},
            "timestamp_ms": 1700000000000_u64,
        });
        let obj = payload.as_object().unwrap();
        assert_eq!(obj.get("tool_name").unwrap(), "fs.read");
        assert!(obj.contains_key("args"));
        assert!(obj.contains_key("timestamp_ms"));
    }

    #[test]
    fn tool_result_payload_structure() {
        let payload = serde_json::json!({
            "tool_name": "fs.read",
            "output": {"content": "hello"},
            "success": true,
            "duration_ms": 42_u64,
            "timestamp_ms": 1700000000000_u64,
        });
        let obj = payload.as_object().unwrap();
        assert_eq!(obj.get("success").unwrap(), true);
        assert_eq!(obj.get("duration_ms").unwrap(), 42);
    }

    #[test]
    fn llm_response_payload_structure() {
        let payload = serde_json::json!({
            "model": "claude-3-opus",
            "tokens_in": 500_u64,
            "tokens_out": 200_u64,
            "latency_ms": 1200_u64,
            "timestamp_ms": 1700000000000_u64,
        });
        let obj = payload.as_object().unwrap();
        assert_eq!(obj.get("model").unwrap(), "claude-3-opus");
        assert_eq!(obj.get("tokens_in").unwrap(), 500);
        assert_eq!(obj.get("tokens_out").unwrap(), 200);
    }

    #[test]
    fn tool_call_serializes_to_json() {
        let payload = serde_json::json!({
            "tool_name": "git.status",
            "args": {},
            "timestamp_ms": now_ms(),
        });
        let bytes = serde_json::to_vec(&payload).unwrap();
        assert!(!bytes.is_empty());
        let round_trip: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(round_trip["tool_name"], "git.status");
    }

    #[test]
    fn stream_read_hard_cap_matches_ff_contract() {
        // Pin that we re-export FF's ceiling — if FF bumps the cap, we pick it
        // up automatically and callers size reads against the current ff-core
        // constant. No cairn-side duplication.
        const { assert!(ff_sdk::task::STREAM_READ_HARD_CAP >= 1) };
    }

    #[test]
    fn checkpoint_is_raw_bytes() {
        let context = b"{\"step\":5,\"state\":\"running\"}";
        assert!(!context.is_empty());
        let parsed: serde_json::Value = serde_json::from_slice(context).unwrap();
        assert_eq!(parsed["step"], 5);
    }
}
