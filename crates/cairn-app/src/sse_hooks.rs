//! SSE publisher hooks wiring cairn-memory proposal events to cairn-api SSE frames.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use cairn_api::memory_api::MemoryItem;
use cairn_api::sse::SseFrame;
use cairn_api::sse_payloads::build_memory_proposed_frame;
use cairn_memory::api_impl::MemoryProposalHook;
use tokio::sync::broadcast;

const SSE_BUFFER_CAPACITY: usize = 10_000;

/// SSE publisher hook that emits `memory_proposed` frames when
/// cairn-memory creates a Proposed memory.
///
/// Wire into MemoryApiImpl:
/// ```ignore
/// let hook = SseMemoryProposalHook::with_sse_channel(sse_tx, buffer, seq);
/// let api = MemoryApiImpl::new(retrieval, store).with_proposal_hook(Box::new(hook));
/// ```
pub struct SseMemoryProposalHook {
    /// Collected frames for test assertions.
    frames: std::sync::Mutex<Vec<SseFrame>>,
    /// Live SSE broadcast channel sender.
    sse_tx: Option<broadcast::Sender<SseFrame>>,
    /// Replay buffer shared with AppState for Last-Event-ID reconnect.
    #[allow(clippy::type_complexity)]
    sse_buffer: Option<Arc<RwLock<VecDeque<(u64, SseFrame)>>>>,
    /// Monotonic sequence counter shared with AppState.
    sse_seq: Option<Arc<AtomicU64>>,
}

impl Default for SseMemoryProposalHook {
    fn default() -> Self {
        Self::new()
    }
}

impl SseMemoryProposalHook {
    /// Test-only constructor: collects frames in a Vec without broadcasting.
    pub fn new() -> Self {
        Self {
            frames: std::sync::Mutex::new(Vec::new()),
            sse_tx: None,
            sse_buffer: None,
            sse_seq: None,
        }
    }

    /// Production constructor: broadcasts frames to the SSE channel and replay buffer.
    pub fn with_sse_channel(
        sse_tx: broadcast::Sender<SseFrame>,
        sse_buffer: Arc<RwLock<VecDeque<(u64, SseFrame)>>>,
        sse_seq: Arc<AtomicU64>,
    ) -> Self {
        Self {
            frames: std::sync::Mutex::new(Vec::new()),
            sse_tx: Some(sse_tx),
            sse_buffer: Some(sse_buffer),
            sse_seq: Some(sse_seq),
        }
    }

    pub fn collected_frames(&self) -> Vec<SseFrame> {
        self.frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
}

pub struct SharedMemoryProposalHook(pub Arc<SseMemoryProposalHook>);

impl MemoryProposalHook for SseMemoryProposalHook {
    fn on_proposed(&self, item: &MemoryItem) {
        let mut frame = match build_memory_proposed_frame(item.clone(), None) {
            Some(f) => f,
            None => return, // warning already logged by build_memory_proposed_frame
        };

        // Assign sequence ID and broadcast if wired to the SSE channel.
        if let (Some(tx), Some(buffer), Some(seq)) = (&self.sse_tx, &self.sse_buffer, &self.sse_seq)
        {
            let id = seq.fetch_add(1, Ordering::SeqCst);
            frame.id = Some(id.to_string());

            // Push to replay buffer.
            if let Ok(mut buf) = buffer.write() {
                if buf.len() >= SSE_BUFFER_CAPACITY {
                    buf.pop_front();
                }
                buf.push_back((id, frame.clone()));
            }

            let _ = tx.send(frame.clone());
        }

        self.frames
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(frame);
    }
}

impl MemoryProposalHook for SharedMemoryProposalHook {
    fn on_proposed(&self, item: &MemoryItem) {
        self.0.on_proposed(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_api::memory_api::{MemoryItem, MemoryStatus};

    #[test]
    fn hook_captures_memory_proposed_frame() {
        let hook = SseMemoryProposalHook::new();

        let item = MemoryItem {
            id: "mem_1".to_owned(),
            content: "Important fact".to_owned(),
            category: Some("facts".to_owned()),
            status: MemoryStatus::Proposed,
            source: Some("assistant".to_owned()),
            confidence: None,
            created_at: "2026-04-03T10:00:00Z".to_owned(),
        };

        hook.on_proposed(&item);

        let frames = hook.collected_frames();
        assert_eq!(frames.len(), 1);
        assert_eq!(
            frames[0].event,
            cairn_api::sse::SseEventName::MemoryProposed
        );
        assert_eq!(frames[0].data["memory"]["content"], "Important fact");
        assert_eq!(frames[0].data["memory"]["status"], "proposed");
    }

    #[test]
    fn hook_broadcasts_to_sse_channel() {
        let (tx, mut rx) = broadcast::channel(16);
        let buffer = Arc::new(RwLock::new(VecDeque::new()));
        let seq = Arc::new(AtomicU64::new(100));

        let hook = SseMemoryProposalHook::with_sse_channel(tx, buffer.clone(), seq.clone());

        let item = MemoryItem {
            id: "mem_2".to_owned(),
            content: "Broadcast test".to_owned(),
            category: Some("facts".to_owned()),
            status: MemoryStatus::Proposed,
            source: Some("assistant".to_owned()),
            confidence: None,
            created_at: "2026-04-03T10:00:00Z".to_owned(),
        };

        hook.on_proposed(&item);

        // Frame was broadcast.
        let received = rx.try_recv().unwrap();
        assert_eq!(received.event, cairn_api::sse::SseEventName::MemoryProposed);
        assert_eq!(received.id, Some("100".to_owned()));

        // Frame was buffered for replay.
        let buf = buffer.read().unwrap();
        assert_eq!(buf.len(), 1);
        assert_eq!(buf[0].0, 100);

        // Sequence counter advanced.
        assert_eq!(seq.load(Ordering::SeqCst), 101);

        // Frame also collected locally.
        assert_eq!(hook.collected_frames().len(), 1);
    }
}

// ── SseOrchestratorEmitter ────────────────────────────────────────────────────

/// Implements `OrchestratorEventEmitter` by serialising `OrchestratorEvent`
/// structs into `AgentProgress` SSE frames and broadcasting them on the shared
/// runtime SSE channel.
///
/// Wire into `OrchestratorLoop::with_emitter(Arc::new(emitter))` inside
/// `orchestrate_run_handler`.
pub struct SseOrchestratorEmitter {
    sse_tx: broadcast::Sender<SseFrame>,
    sse_buffer: Arc<RwLock<VecDeque<(u64, SseFrame)>>>,
    sse_seq: Arc<AtomicU64>,
}

impl SseOrchestratorEmitter {
    pub fn new(
        sse_tx: broadcast::Sender<SseFrame>,
        sse_buffer: Arc<RwLock<VecDeque<(u64, SseFrame)>>>,
        sse_seq: Arc<AtomicU64>,
    ) -> Self {
        Self {
            sse_tx,
            sse_buffer,
            sse_seq,
        }
    }

    fn emit(&self, data: serde_json::Value) {
        let id = self.sse_seq.fetch_add(1, Ordering::SeqCst);
        let frame = SseFrame {
            event: cairn_api::sse::SseEventName::AgentProgress,
            data,
            id: Some(id.to_string()),
            tenant_id: None,
        };
        if let Ok(mut buf) = self.sse_buffer.write() {
            if buf.len() >= SSE_BUFFER_CAPACITY {
                buf.pop_front();
            }
            buf.push_back((id, frame.clone()));
        }
        let _ = self.sse_tx.send(frame);
    }
}

#[async_trait::async_trait]
impl cairn_orchestrator::OrchestratorEventEmitter for SseOrchestratorEmitter {
    async fn on_started(&self, ctx: &cairn_orchestrator::OrchestrationContext) {
        self.emit(serde_json::json!({
            "event":      "orchestrate_started",
            "run_id":     ctx.run_id,
            "goal":       ctx.goal,
            "agent_type": ctx.agent_type,
            "iteration":  ctx.iteration,
        }));
    }

    async fn on_gather_completed(
        &self,
        ctx: &cairn_orchestrator::OrchestrationContext,
        gather: &cairn_orchestrator::GatherOutput,
    ) {
        self.emit(serde_json::json!({
            "event":         "gather_completed",
            "run_id":        ctx.run_id,
            "iteration":     ctx.iteration,
            "memory_chunks": gather.memory_chunks.len(),
            "recent_events": gather.recent_events.len(),
        }));
    }

    async fn on_decide_completed(
        &self,
        ctx: &cairn_orchestrator::OrchestrationContext,
        decide: &cairn_orchestrator::DecideOutput,
    ) {
        self.emit(serde_json::json!({
            "event":          "decide_completed",
            "run_id":         ctx.run_id,
            "iteration":      ctx.iteration,
            "proposal_count": decide.proposals.len(),
            "confidence":     decide.calibrated_confidence,
            "latency_ms":     decide.latency_ms,
        }));
    }

    async fn on_tool_called(
        &self,
        ctx: &cairn_orchestrator::OrchestrationContext,
        tool_name: &str,
        args: Option<&serde_json::Value>,
    ) {
        self.emit(serde_json::json!({
            "event":     "tool_called",
            "run_id":    ctx.run_id,
            "iteration": ctx.iteration,
            "tool_name": tool_name,
            "args":      args,
        }));
    }

    async fn on_tool_result(
        &self,
        ctx: &cairn_orchestrator::OrchestrationContext,
        tool_name: &str,
        succeeded: bool,
        output: Option<&serde_json::Value>,
        error: Option<&str>,
        duration_ms: u64,
    ) {
        self.emit(serde_json::json!({
            "event":       "tool_result",
            "run_id":      ctx.run_id,
            "iteration":   ctx.iteration,
            "tool_name":   tool_name,
            "succeeded":   succeeded,
            "output":      output,
            "error":       error,
            "duration_ms": duration_ms,
        }));
    }

    async fn on_step_completed(
        &self,
        ctx: &cairn_orchestrator::OrchestrationContext,
        decide: &cairn_orchestrator::DecideOutput,
        execute: &cairn_orchestrator::ExecuteOutcome,
    ) {
        use cairn_orchestrator::ActionStatus;
        let succeeded = execute
            .results
            .iter()
            .filter(|r| r.status == ActionStatus::Succeeded)
            .count();
        let failed = execute
            .results
            .iter()
            .filter(|r| matches!(r.status, ActionStatus::Failed { .. }))
            .count();
        self.emit(serde_json::json!({
            "event":          "step_completed",
            "run_id":         ctx.run_id,
            "iteration":      ctx.iteration,
            "proposal_count": decide.proposals.len(),
            "succeeded":      succeeded,
            "failed":         failed,
        }));
    }

    async fn on_finished(
        &self,
        ctx: &cairn_orchestrator::OrchestrationContext,
        termination: &cairn_orchestrator::LoopTermination,
    ) {
        let (term_str, detail) = match termination {
            cairn_orchestrator::LoopTermination::Completed { summary } => {
                ("completed", Some(summary.as_str()))
            }
            cairn_orchestrator::LoopTermination::Failed { reason } => {
                ("failed", Some(reason.as_str()))
            }
            cairn_orchestrator::LoopTermination::MaxIterationsReached => {
                ("max_iterations_reached", None)
            }
            cairn_orchestrator::LoopTermination::TimedOut => ("timed_out", None),
            cairn_orchestrator::LoopTermination::WaitingApproval { .. } => {
                ("waiting_approval", None)
            }
            cairn_orchestrator::LoopTermination::WaitingSubagent { .. } => {
                ("waiting_subagent", None)
            }
            cairn_orchestrator::LoopTermination::PlanProposed { .. } => ("plan_proposed", None),
        };
        self.emit(serde_json::json!({
            "event":       "orchestrate_finished",
            "run_id":      ctx.run_id,
            "termination": term_str,
            "detail":      detail,
        }));
    }
}

#[cfg(test)]
mod sse_orchestrator_tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicU64;

    fn make_emitter() -> (SseOrchestratorEmitter, broadcast::Receiver<SseFrame>) {
        let (tx, rx) = broadcast::channel(64);
        let buf = Arc::new(RwLock::new(VecDeque::new()));
        let seq = Arc::new(AtomicU64::new(0));
        (SseOrchestratorEmitter::new(tx, buf, seq), rx)
    }

    fn ctx() -> cairn_orchestrator::OrchestrationContext {
        use cairn_domain::{ProjectKey, RunId, SessionId};
        cairn_orchestrator::OrchestrationContext {
            project: ProjectKey::new("t", "w", "p"),
            session_id: SessionId::new("s1"),
            run_id: RunId::new("run_1"),
            task_id: None,
            iteration: 0,
            goal: "do the thing".to_owned(),
            agent_type: "orchestrator".to_owned(),
            run_started_at_ms: 0,
            working_dir: PathBuf::from("."),
            run_mode: cairn_domain::decisions::RunMode::Direct,
            discovered_tool_names: vec![],
            step_history: vec![],
            is_recovery: false,
            approval_timeout: None,
        }
    }

    #[tokio::test]
    async fn emits_orchestrate_started_as_agent_progress() {
        use cairn_orchestrator::OrchestratorEventEmitter;
        let (emitter, mut rx) = make_emitter();
        emitter.on_started(&ctx()).await;
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.event, cairn_api::sse::SseEventName::AgentProgress);
        assert_eq!(frame.data["event"], "orchestrate_started");
        assert_eq!(frame.data["run_id"], "run_1");
        assert_eq!(frame.data["goal"], "do the thing");
    }

    #[tokio::test]
    async fn emits_tool_called_and_result() {
        use cairn_orchestrator::OrchestratorEventEmitter;
        let (emitter, mut rx) = make_emitter();
        let c = ctx();
        emitter
            .on_tool_called(
                &c,
                "web_fetch",
                Some(&serde_json::json!({"url":"https://example.com"})),
            )
            .await;
        emitter
            .on_tool_result(
                &c,
                "web_fetch",
                true,
                Some(&serde_json::json!({"body":"ok"})),
                None,
                42,
            )
            .await;

        let f1 = rx.try_recv().unwrap();
        assert_eq!(f1.data["event"], "tool_called");
        assert_eq!(f1.data["tool_name"], "web_fetch");

        let f2 = rx.try_recv().unwrap();
        assert_eq!(f2.data["event"], "tool_result");
        assert_eq!(f2.data["succeeded"], true);
        // duration_ms lands on the SSE frame so dashboards can surface
        // per-tool latency without re-reading FF's attempt stream.
        assert_eq!(f2.data["duration_ms"], 42);
    }

    #[tokio::test]
    async fn emits_orchestrate_finished_on_completion() {
        use cairn_orchestrator::{LoopTermination, OrchestratorEventEmitter};
        let (emitter, mut rx) = make_emitter();
        emitter
            .on_finished(
                &ctx(),
                &LoopTermination::Completed {
                    summary: "all done".to_owned(),
                },
            )
            .await;
        let frame = rx.try_recv().unwrap();
        assert_eq!(frame.data["event"], "orchestrate_finished");
        assert_eq!(frame.data["termination"], "completed");
        assert_eq!(frame.data["detail"], "all done");
    }

    #[tokio::test]
    async fn sequence_ids_are_monotonic() {
        use cairn_orchestrator::OrchestratorEventEmitter;
        let (emitter, mut rx) = make_emitter();
        let c = ctx();
        emitter.on_started(&c).await;
        emitter.on_tool_called(&c, "t1", None).await;
        let f0 = rx.try_recv().unwrap();
        let f1 = rx.try_recv().unwrap();
        let id0: u64 = f0.id.unwrap().parse().unwrap();
        let id1: u64 = f1.id.unwrap().parse().unwrap();
        assert!(id1 > id0);
    }
}
