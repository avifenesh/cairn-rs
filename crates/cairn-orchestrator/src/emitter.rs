//! OrchestratorEventEmitter — real-time progress notifications for the dashboard.
//!
//! Every significant phase boundary in the orchestrator loop calls the emitter.
//! The default implementation is a no-op; the concrete implementation in
//! `cairn-app` writes structured events to the SSE broadcast channel so the
//! dashboard can show live progress without polling.
//!
//! # Method call order within a single iteration
//!
//! ```text
//! on_started          (once, before the first iteration)
//! loop {
//!   on_gather_completed  (gather phase done)
//!   on_decide_completed  (decide phase done — proposals known)
//!   for each tool call:
//!     on_tool_called   (before dispatch)
//!     on_tool_result   (after dispatch, success or failure)
//!   on_step_completed  (iteration done — loop signal known)
//! }
//! on_finished          (once, after the final LoopTermination)
//! ```

use async_trait::async_trait;
use cairn_domain::{RunId, TaskId};

use crate::context::{
    DecideOutput, ExecuteOutcome, GatherOutput, LoopTermination, OrchestrationContext,
};

// ── OrchestratorEvent ────────────────────────────────────────────────────────

/// A structured event emitted at each phase boundary.
///
/// Each variant carries the minimum data the dashboard needs to render a live
/// progress update.  All variants include the `run_id` so the SSE handler can
/// route events to the correct subscriber.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum OrchestratorEvent {
    /// The orchestrator loop has started for a run.
    Started {
        run_id:     RunId,
        goal:       String,
        agent_type: String,
        iteration:  u32,
    },
    /// The GATHER phase completed successfully.
    GatherCompleted {
        run_id:         RunId,
        iteration:      u32,
        memory_chunks:  usize,
        recent_events:  usize,
    },
    /// The DECIDE phase completed — proposals are known.
    DecideCompleted {
        run_id:           RunId,
        iteration:        u32,
        proposal_count:   usize,
        first_action:     String,
        confidence:       f64,
        latency_ms:       u64,
    },
    /// A tool call is about to be dispatched.
    ToolCalled {
        run_id:    RunId,
        iteration: u32,
        tool_name: String,
        args:      Option<serde_json::Value>,
    },
    /// A tool call returned (success or failure).
    ToolResult {
        run_id:    RunId,
        iteration: u32,
        tool_name: String,
        succeeded: bool,
        output:    Option<serde_json::Value>,
        error:     Option<String>,
    },
    /// One full iteration (gather + decide + execute) completed.
    StepCompleted {
        run_id:     RunId,
        iteration:  u32,
        /// Signal from the execute phase: "continue", "done", "wait_approval", …
        signal:     String,
        succeeded:  usize,
        failed:     usize,
    },
    /// The loop has finished (terminal or suspended).
    Finished {
        run_id:      RunId,
        termination: String,
        /// Human-readable summary (from `LoopTermination::Completed` or error reason).
        detail:      Option<String>,
    },
}

// ── OrchestratorEventEmitter trait ───────────────────────────────────────────

/// Receives structured events as the orchestrator loop progresses.
///
/// All methods have default no-op implementations so concrete emitters only
/// need to override the events they care about.
#[async_trait]
pub trait OrchestratorEventEmitter: Send + Sync {
    /// Called once before the first iteration starts.
    async fn on_started(&self, _ctx: &OrchestrationContext) {}

    /// Called after the GATHER phase completes each iteration.
    async fn on_gather_completed(&self, _ctx: &OrchestrationContext, _gather: &GatherOutput) {}

    /// Called after the DECIDE phase completes each iteration.
    async fn on_decide_completed(&self, _ctx: &OrchestrationContext, _decide: &DecideOutput) {}

    /// Called immediately before each tool is dispatched.
    ///
    /// `tool_name` and `args` are taken from the `ActionProposal`.
    async fn on_tool_called(
        &self,
        _ctx:       &OrchestrationContext,
        _tool_name: &str,
        _args:      Option<&serde_json::Value>,
    ) {}

    /// Called after a tool invocation returns (success or failure).
    async fn on_tool_result(
        &self,
        _ctx:       &OrchestrationContext,
        _tool_name: &str,
        _succeeded: bool,
        _output:    Option<&serde_json::Value>,
        _error:     Option<&str>,
    ) {}

    /// Called after EXECUTE completes each iteration (loop signal known).
    async fn on_step_completed(
        &self,
        _ctx:     &OrchestrationContext,
        _decide:  &DecideOutput,
        _execute: &ExecuteOutcome,
    ) {}

    /// Called once after the loop terminates (terminal or suspended).
    async fn on_finished(&self, _ctx: &OrchestrationContext, _termination: &LoopTermination) {}
}

// ── NoOpEmitter ───────────────────────────────────────────────────────────────

/// Default no-op emitter.  Used in tests and local mode where SSE streaming
/// is not required.
pub struct NoOpEmitter;

#[async_trait]
impl OrchestratorEventEmitter for NoOpEmitter {}

// ── ChannelEmitter ────────────────────────────────────────────────────────────

/// Emitter that serialises events to JSON and sends them down a
/// `tokio::sync::broadcast::Sender<String>`.
///
/// Drop-in for the SSE broadcast channel in `cairn-app`:
///
/// ```rust,ignore
/// use tokio::sync::broadcast;
/// let (tx, _) = broadcast::channel(256);
/// let emitter = ChannelEmitter::new(tx);
/// loop.with_emitter(Arc::new(emitter)).run(ctx).await?;
/// ```
pub struct ChannelEmitter {
    tx: tokio::sync::broadcast::Sender<String>,
}

impl ChannelEmitter {
    pub fn new(tx: tokio::sync::broadcast::Sender<String>) -> Self {
        Self { tx }
    }

    fn send(&self, event: OrchestratorEvent) {
        if let Ok(json) = serde_json::to_string(&event) {
            let _ = self.tx.send(json); // ignore "no receivers" errors
        }
    }
}

#[async_trait]
impl OrchestratorEventEmitter for ChannelEmitter {
    async fn on_started(&self, ctx: &OrchestrationContext) {
        self.send(OrchestratorEvent::Started {
            run_id:     ctx.run_id.clone(),
            goal:       ctx.goal.clone(),
            agent_type: ctx.agent_type.clone(),
            iteration:  ctx.iteration,
        });
    }

    async fn on_gather_completed(&self, ctx: &OrchestrationContext, gather: &GatherOutput) {
        self.send(OrchestratorEvent::GatherCompleted {
            run_id:        ctx.run_id.clone(),
            iteration:     ctx.iteration,
            memory_chunks: gather.memory_chunks.len(),
            recent_events: gather.recent_events.len(),
        });
    }

    async fn on_decide_completed(&self, ctx: &OrchestrationContext, decide: &DecideOutput) {
        self.send(OrchestratorEvent::DecideCompleted {
            run_id:         ctx.run_id.clone(),
            iteration:      ctx.iteration,
            proposal_count: decide.proposals.len(),
            first_action:   decide.proposals.first()
                .map(|p| format!("{:?}", p.action_type).to_lowercase())
                .unwrap_or_else(|| "none".to_owned()),
            confidence:     decide.calibrated_confidence,
            latency_ms:     decide.latency_ms,
        });
    }

    async fn on_tool_called(
        &self,
        ctx:       &OrchestrationContext,
        tool_name: &str,
        args:      Option<&serde_json::Value>,
    ) {
        self.send(OrchestratorEvent::ToolCalled {
            run_id:    ctx.run_id.clone(),
            iteration: ctx.iteration,
            tool_name: tool_name.to_owned(),
            args:      args.cloned(),
        });
    }

    async fn on_tool_result(
        &self,
        ctx:       &OrchestrationContext,
        tool_name: &str,
        succeeded: bool,
        output:    Option<&serde_json::Value>,
        error:     Option<&str>,
    ) {
        self.send(OrchestratorEvent::ToolResult {
            run_id:    ctx.run_id.clone(),
            iteration: ctx.iteration,
            tool_name: tool_name.to_owned(),
            succeeded,
            output:    output.cloned(),
            error:     error.map(str::to_owned),
        });
    }

    async fn on_step_completed(
        &self,
        ctx:     &OrchestrationContext,
        _decide: &DecideOutput,
        execute: &ExecuteOutcome,
    ) {
        use crate::context::ActionStatus;
        let succeeded = execute.results.iter()
            .filter(|r| r.status == ActionStatus::Succeeded)
            .count();
        let failed = execute.results.iter()
            .filter(|r| matches!(r.status, ActionStatus::Failed { .. }))
            .count();
        let signal = format!("{:?}", execute.loop_signal).to_lowercase();
        self.send(OrchestratorEvent::StepCompleted {
            run_id:    ctx.run_id.clone(),
            iteration: ctx.iteration,
            signal,
            succeeded,
            failed,
        });
    }

    async fn on_finished(&self, ctx: &OrchestrationContext, termination: &LoopTermination) {
        let (term_str, detail) = match termination {
            LoopTermination::Completed { summary } =>
                ("completed".to_owned(), Some(summary.clone())),
            LoopTermination::Failed { reason } =>
                ("failed".to_owned(), Some(reason.clone())),
            LoopTermination::MaxIterationsReached =>
                ("max_iterations_reached".to_owned(), None),
            LoopTermination::TimedOut =>
                ("timed_out".to_owned(), None),
            LoopTermination::WaitingApproval { approval_id } =>
                ("waiting_approval".to_owned(), Some(approval_id.to_string())),
            LoopTermination::WaitingSubagent { child_task_id } =>
                ("waiting_subagent".to_owned(), Some(child_task_id.to_string())),
        };
        self.send(OrchestratorEvent::Finished {
            run_id:      ctx.run_id.clone(),
            termination: term_str,
            detail,
        });
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use cairn_domain::{ProjectKey, RunId, SessionId};
    use crate::context::{
        DecideOutput, ExecuteOutcome, GatherOutput, LoopSignal, LoopTermination,
        OrchestrationContext,
    };

    fn ctx() -> OrchestrationContext {
        OrchestrationContext {
            project:               ProjectKey::new("t", "w", "p"),
            session_id:            SessionId::new("sess"),
            run_id:                RunId::new("run"),
            task_id:               None,
            iteration:             0,
            goal:                  "test".to_owned(),
            agent_type:            "test_agent".to_owned(),
            run_started_at_ms:     0,
            discovered_tool_names: vec![],
        }
    }

    fn empty_decide() -> DecideOutput {
        DecideOutput {
            raw_response:          "{}".into(),
            proposals:             vec![],
            calibrated_confidence: 0.9,
            requires_approval:     false,
            model_id:              "stub".into(),
            latency_ms:            0,
            input_tokens:          None,
            output_tokens:         None,
        }
    }

    fn empty_execute() -> ExecuteOutcome {
        ExecuteOutcome { results: vec![], loop_signal: LoopSignal::Continue }
    }

    // ── NoOpEmitter compiles and does nothing ─────────────────────────────────

    #[tokio::test]
    async fn noop_emitter_all_methods_compile() {
        let e = NoOpEmitter;
        let ctx = ctx();
        let g = GatherOutput::default();
        let d = empty_decide();
        let x = empty_execute();
        e.on_started(&ctx).await;
        e.on_gather_completed(&ctx, &g).await;
        e.on_decide_completed(&ctx, &d).await;
        e.on_tool_called(&ctx, "memory_search", None).await;
        e.on_tool_result(&ctx, "memory_search", true, None, None).await;
        e.on_step_completed(&ctx, &d, &x).await;
        e.on_finished(&ctx, &LoopTermination::Completed { summary: "done".into() }).await;
    }

    // ── ChannelEmitter serialises events correctly ────────────────────────────

    #[tokio::test]
    async fn channel_emitter_sends_started_event() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);
        let emitter = ChannelEmitter::new(tx);

        emitter.on_started(&ctx()).await;

        let msg = rx.try_recv().unwrap();
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["event"], "started");
        assert_eq!(v["run_id"], "run");
        assert_eq!(v["goal"], "test");
    }

    #[tokio::test]
    async fn channel_emitter_sends_decide_event() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);
        let emitter = ChannelEmitter::new(tx);

        emitter.on_decide_completed(&ctx(), &empty_decide()).await;

        let msg = rx.try_recv().unwrap();
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["event"], "decide_completed");
        assert_eq!(v["proposal_count"], 0);
    }

    #[tokio::test]
    async fn channel_emitter_sends_finished_event() {
        let (tx, mut rx) = tokio::sync::broadcast::channel::<String>(16);
        let emitter = ChannelEmitter::new(tx);

        emitter.on_finished(
            &ctx(),
            &LoopTermination::Completed { summary: "all done".into() },
        ).await;

        let msg = rx.try_recv().unwrap();
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["event"], "finished");
        assert_eq!(v["termination"], "completed");
        assert_eq!(v["detail"], "all done");
    }

    #[tokio::test]
    async fn channel_emitter_tolerates_no_receivers() {
        let (tx, _rx) = tokio::sync::broadcast::channel::<String>(16);
        drop(_rx); // drop receiver — send must not panic
        let emitter = ChannelEmitter::new(tx);
        emitter.on_started(&ctx()).await; // must not panic
    }

    // ── OrchestratorEventEmitter is object-safe ───────────────────────────────

    #[tokio::test]
    async fn emitter_is_object_safe() {
        let _: Arc<dyn OrchestratorEventEmitter> = Arc::new(NoOpEmitter);
    }

    // ── Collecting emitter for integration tests ──────────────────────────────

    struct CollectingEmitter(Mutex<Vec<String>>);

    impl CollectingEmitter {
        fn new() -> Arc<Self> { Arc::new(Self(Mutex::new(vec![]))) }
        fn events(&self) -> Vec<String> { self.0.lock().unwrap().clone() }
    }

    #[async_trait]
    impl OrchestratorEventEmitter for CollectingEmitter {
        async fn on_started(&self, _: &OrchestrationContext) {
            self.0.lock().unwrap().push("started".into());
        }
        async fn on_gather_completed(&self, _: &OrchestrationContext, _: &GatherOutput) {
            self.0.lock().unwrap().push("gather_completed".into());
        }
        async fn on_decide_completed(&self, _: &OrchestrationContext, _: &DecideOutput) {
            self.0.lock().unwrap().push("decide_completed".into());
        }
        async fn on_step_completed(&self, _: &OrchestrationContext, _: &DecideOutput, _: &ExecuteOutcome) {
            self.0.lock().unwrap().push("step_completed".into());
        }
        async fn on_finished(&self, _: &OrchestrationContext, _: &LoopTermination) {
            self.0.lock().unwrap().push("finished".into());
        }
    }

    #[tokio::test]
    async fn collecting_emitter_is_object_safe_and_collects() {
        let e = CollectingEmitter::new();
        let dyn_e: Arc<dyn OrchestratorEventEmitter> = e.clone();
        let ctx = ctx();
        dyn_e.on_started(&ctx).await;
        dyn_e.on_gather_completed(&ctx, &GatherOutput::default()).await;
        dyn_e.on_decide_completed(&ctx, &empty_decide()).await;
        dyn_e.on_step_completed(&ctx, &empty_decide(), &empty_execute()).await;
        dyn_e.on_finished(&ctx, &LoopTermination::Completed { summary: "done".into() }).await;
        let events = e.events();
        assert_eq!(events, vec!["started", "gather_completed", "decide_completed", "step_completed", "finished"]);
    }
}
