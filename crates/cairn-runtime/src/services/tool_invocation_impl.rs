//! Runtime seam for tool invocations.
//!
//! Wires assistant_tool_call through the runtime event model so that
//! every tool invocation is durable, replayable, and permissioned.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::tool_invocation::{ToolInvocationOutcomeKind, ToolInvocationTarget};
use cairn_domain::*;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;

/// Runtime-facing tool invocation service.
///
/// Workers 5 (tools) and 8 (API) use this seam to record tool calls
/// through the canonical event model without writing events directly.
#[async_trait]
pub trait ToolInvocationService: Send + Sync {
    /// Record that a tool invocation has been requested and started.
    async fn record_start(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        session_id: Option<SessionId>,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        target: ToolInvocationTarget,
        execution_class: ExecutionClass,
    ) -> Result<(), RuntimeError>;

    /// Record that a tool invocation completed successfully.
    ///
    /// RFC 020 Track 3 (invariant #11): `additional_events` lets callers batch
    /// tool-buffered side-effect events (e.g. `IngestJobStarted`,
    /// domain mutations emitted via `ToolContext::buffer_event`) into the
    /// SAME `EventLog::append` call as the completion marker. Either ALL
    /// events land or NONE do — no partial state where a projection saw the
    /// side-effect but the cache did not.
    ///
    /// Callers with no buffered events pass `&[]`.
    ///
    /// RFC 020 Track 3: `tool_call_id` + `result_json` are threaded into
    /// the `ToolInvocationCompleted` event so a restart can rebuild
    /// `ToolCallResultCache` purely from the event log. Legacy / non-
    /// orchestrator callers pass `None` for both.
    async fn record_completed(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        task_id: Option<TaskId>,
        tool_name: String,
        additional_events: &[cairn_domain::RuntimeEvent],
        tool_call_id: Option<String>,
        result_json: Option<serde_json::Value>,
    ) -> Result<(), RuntimeError>;

    /// Record that a tool invocation failed.
    async fn record_failed(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        task_id: Option<TaskId>,
        tool_name: String,
        outcome: ToolInvocationOutcomeKind,
        error_message: Option<String>,
    ) -> Result<(), RuntimeError>;

    /// RFC 020 Track 3: append a raw audit event (e.g. `ToolRecoveryPaused`)
    /// through the tool-invocation service seam. Default routes through
    /// the underlying event log. Orchestrator uses this for cache / pause
    /// audit trails that don't fit `record_completed` / `record_failed`.
    async fn append_audit_events(
        &self,
        events: &[cairn_domain::RuntimeEvent],
    ) -> Result<(), RuntimeError>;
}

pub struct ToolInvocationServiceImpl<S> {
    store: Arc<S>,
}

impl<S> ToolInvocationServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_ms() -> u64 {
    // T3-M6: `unwrap_or_default()` matches the rest of cairn-runtime's
    // now_ms helpers. Panicking here on clock skew (pre-1970 system
    // clock) would crash the tool-invocation service for every caller.
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[async_trait]
impl<S> ToolInvocationService for ToolInvocationServiceImpl<S>
where
    S: EventLog + 'static,
{
    async fn record_start(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        session_id: Option<SessionId>,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        target: ToolInvocationTarget,
        execution_class: ExecutionClass,
    ) -> Result<(), RuntimeError> {
        let now = now_ms();
        let event = make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
            project: project.clone(),
            invocation_id,
            session_id,
            run_id,
            task_id,
            target,
            execution_class,
            prompt_release_id: None,
            requested_at_ms: now,
            started_at_ms: now,
        }));
        self.store.append(&[event]).await?;
        Ok(())
    }

    async fn record_completed(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        task_id: Option<TaskId>,
        tool_name: String,
        additional_events: &[RuntimeEvent],
        tool_call_id: Option<String>,
        result_json: Option<serde_json::Value>,
    ) -> Result<(), RuntimeError> {
        // RFC 020 invariant #11: buffered side-effect events and the completion
        // marker append in ONE call so durability is all-or-nothing.
        let mut batch = Vec::with_capacity(additional_events.len() + 1);
        for ev in additional_events {
            batch.push(make_envelope(ev.clone()));
        }
        batch.push(make_envelope(RuntimeEvent::ToolInvocationCompleted(
            ToolInvocationCompleted {
                project: project.clone(),
                invocation_id,
                task_id,
                tool_name,
                finished_at_ms: now_ms(),
                outcome: ToolInvocationOutcomeKind::Success,
                tool_call_id,
                result_json,
            },
        )));
        self.store.append(&batch).await?;
        Ok(())
    }

    async fn record_failed(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        task_id: Option<TaskId>,
        tool_name: String,
        outcome: ToolInvocationOutcomeKind,
        error_message: Option<String>,
    ) -> Result<(), RuntimeError> {
        let event = make_envelope(RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
            project: project.clone(),
            invocation_id,
            task_id,
            tool_name,
            finished_at_ms: now_ms(),
            outcome,
            error_message,
        }));
        self.store.append(&[event]).await?;
        Ok(())
    }

    async fn append_audit_events(
        &self,
        events: &[RuntimeEvent],
    ) -> Result<(), RuntimeError> {
        if events.is_empty() {
            return Ok(());
        }
        let batch: Vec<_> = events.iter().cloned().map(make_envelope).collect();
        self.store.append(&batch).await?;
        Ok(())
    }
}
