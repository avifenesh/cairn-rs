//! `TaskFrameSink` — the orchestrator's non-consuming handle to a live
//! `cairn_fabric::CairnTask`.
//!
//! # Why a trait?
//!
//! `CairnTask`'s terminal methods (`complete_with_result`,
//! `fail_with_retry`, `suspend_for_approval`, …) all take `self` by value
//! so the ff-sdk lease contract is consumed atomically. That shape is
//! correct at the handler/worker layer but leaves the orchestrator loop
//! unable to hold a `CairnTask` across iterations.
//!
//! `TaskFrameSink` carves out the **non-consuming** surface the loop
//! actually needs: FF stream frames (tool-call, tool-result,
//! llm-response, checkpoint) and the lease-health poll. Terminal and
//! suspension operations stay at the caller — the loop reports its
//! outcome via `LoopTermination` and the caller runs the consuming
//! method on the `CairnTask` it holds.
//!
//! # Relationship to `EventLog` bridge events
//!
//! Stream frames are **additive**, not a replacement. The existing
//! `OrchestratorEventEmitter` telemetry (which drives cairn-store
//! projections + SSE) continues to fire; stream frames give ff-sdk's
//! `restore_frames()` a durable replay source for cross-process
//! resumption. Both reach Valkey via different FF keys.
//!
//! # Error handling contract
//!
//! Implementations return `Result<(), OrchestratorError>`, but the loop
//! runner treats a frame-write failure the same way it treats a
//! `CheckpointHook::save` failure: log a WARN and continue. FF stream
//! writes are best-effort telemetry; we never fail a run because a
//! frame append didn't land.

use async_trait::async_trait;

use crate::error::OrchestratorError;

/// Non-consuming handle to a live FF task.
///
/// Impls live outside this crate — [`cairn_fabric::CairnTask`] provides
/// the production impl. Unit tests use [`NoOpTaskSink`] or a
/// test-specific recording impl.
#[async_trait]
pub trait TaskFrameSink: Send + Sync {
    /// Append a `tool_call` frame to FF's attempt-scoped stream.
    ///
    /// Called by the loop runner BEFORE dispatching each tool
    /// invocation so a restart between `before` and `after` leaves an
    /// in-flight marker for `restore_frames()` to observe.
    async fn log_tool_call(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), OrchestratorError>;

    /// Append a `tool_result` frame. Called AFTER the tool dispatch
    /// completes (or errors); `success` discriminates so the replay
    /// side can reconcile partial/failed invocations.
    async fn log_tool_result(
        &self,
        name: &str,
        output: &serde_json::Value,
        success: bool,
        duration_ms: u64,
    ) -> Result<(), OrchestratorError>;

    /// Append an `llm_response` frame capturing the decide call's
    /// model + token counts + latency. The frame backs audit and
    /// cost-reconciliation without cairn-store having to parse the raw
    /// LLM body.
    async fn log_llm_response(
        &self,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
        latency_ms: u64,
    ) -> Result<(), OrchestratorError>;

    /// Append a `checkpoint` frame with a serialized context blob.
    /// Paired with `restore_frames()` for cross-process resume.
    async fn save_checkpoint(&self, context_json: &[u8]) -> Result<(), OrchestratorError>;

    /// Return `false` when the underlying FF task has failed 3
    /// consecutive lease renewals. The loop polls this before each
    /// expensive side effect so it can abort cleanly instead of
    /// committing work FF will reject at the next fcall.
    fn is_lease_healthy(&self) -> bool;
}

/// Default impl — no-op for everything, lease always healthy.
///
/// Used when the caller has no `CairnTask` (local tinkering, unit
/// tests). The orchestrator still runs correctly through the
/// `EventLog` bridge; only the FF-stream telemetry is absent.
pub struct NoOpTaskSink;

#[async_trait]
impl TaskFrameSink for NoOpTaskSink {
    async fn log_tool_call(
        &self,
        _name: &str,
        _args: &serde_json::Value,
    ) -> Result<(), OrchestratorError> {
        Ok(())
    }

    async fn log_tool_result(
        &self,
        _name: &str,
        _output: &serde_json::Value,
        _success: bool,
        _duration_ms: u64,
    ) -> Result<(), OrchestratorError> {
        Ok(())
    }

    async fn log_llm_response(
        &self,
        _model: &str,
        _tokens_in: u64,
        _tokens_out: u64,
        _latency_ms: u64,
    ) -> Result<(), OrchestratorError> {
        Ok(())
    }

    async fn save_checkpoint(&self, _context_json: &[u8]) -> Result<(), OrchestratorError> {
        Ok(())
    }

    fn is_lease_healthy(&self) -> bool {
        true
    }
}

// ── Blanket impl for cairn_fabric::CairnTask ────────────────────────────────
//
// Keeping the impl in this crate (not cairn-fabric) lets cairn-fabric
// stay unaware of the orchestrator. The sink methods delegate 1:1 to
// `CairnTask`'s inherent methods; failures map via `From<FabricError>`
// which we define alongside.

#[async_trait]
impl TaskFrameSink for cairn_fabric::CairnTask {
    async fn log_tool_call(
        &self,
        name: &str,
        args: &serde_json::Value,
    ) -> Result<(), OrchestratorError> {
        cairn_fabric::CairnTask::log_tool_call(self, name, args)
            .await
            .map_err(fabric_to_orchestrator)
    }

    async fn log_tool_result(
        &self,
        name: &str,
        output: &serde_json::Value,
        success: bool,
        duration_ms: u64,
    ) -> Result<(), OrchestratorError> {
        cairn_fabric::CairnTask::log_tool_result(self, name, output, success, duration_ms)
            .await
            .map_err(fabric_to_orchestrator)
    }

    async fn log_llm_response(
        &self,
        model: &str,
        tokens_in: u64,
        tokens_out: u64,
        latency_ms: u64,
    ) -> Result<(), OrchestratorError> {
        cairn_fabric::CairnTask::log_llm_response(self, model, tokens_in, tokens_out, latency_ms)
            .await
            .map_err(fabric_to_orchestrator)
    }

    async fn save_checkpoint(&self, context_json: &[u8]) -> Result<(), OrchestratorError> {
        cairn_fabric::CairnTask::save_checkpoint(self, context_json)
            .await
            .map_err(fabric_to_orchestrator)
    }

    fn is_lease_healthy(&self) -> bool {
        cairn_fabric::CairnTask::is_lease_healthy(self)
    }
}

/// Convert FF bridge failures into the orchestrator's error shape.
/// Frame writes are best-effort, so the loop runner logs and swallows
/// whatever we return here; this keeps the type plumbing honest
/// without introducing a separate FF-facing error variant in
/// [`OrchestratorError`].
fn fabric_to_orchestrator(err: cairn_fabric::FabricError) -> OrchestratorError {
    OrchestratorError::Execute(format!("fabric frame sink: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn noop_sink_accepts_all_writes_and_reports_healthy() {
        let sink = NoOpTaskSink;
        assert!(sink.is_lease_healthy());
        sink.log_tool_call("t", &serde_json::json!({}))
            .await
            .unwrap();
        sink.log_tool_result("t", &serde_json::json!(null), true, 0)
            .await
            .unwrap();
        sink.log_llm_response("m", 0, 0, 0).await.unwrap();
        sink.save_checkpoint(b"{}").await.unwrap();
    }
}
