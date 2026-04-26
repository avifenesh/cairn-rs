//! Runtime seam for tool invocations.
//!
//! Wires assistant_tool_call through the runtime event model so that
//! every tool invocation is durable, replayable, and permissioned.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::tool_invocation::{
    truncate_output_preview, ToolInvocationOutcomeKind, ToolInvocationTarget,
};
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
    ///
    /// F55: `args_json` carries the structured tool arguments so the
    /// `tool_invocations` projection can surface "what cairn ran" to
    /// operators. Callers that cannot produce args (legacy record paths,
    /// synthetic cache-hit paths) pass `None`.
    #[allow(clippy::too_many_arguments)]
    async fn record_start(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        session_id: Option<SessionId>,
        run_id: Option<RunId>,
        task_id: Option<TaskId>,
        target: ToolInvocationTarget,
        execution_class: ExecutionClass,
        args_json: Option<serde_json::Value>,
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
    #[allow(clippy::too_many_arguments)]
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
    ///
    /// F55: `output_preview` carries whatever stdout/stderr tail the
    /// runtime captured before the failure, truncated to
    /// `TOOL_OUTPUT_PREVIEW_MAX_BYTES`. `None` when the runtime had no
    /// output to surface.
    #[allow(clippy::too_many_arguments)]
    async fn record_failed(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        task_id: Option<TaskId>,
        tool_name: String,
        outcome: ToolInvocationOutcomeKind,
        error_message: Option<String>,
        output_preview: Option<String>,
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
        args_json: Option<serde_json::Value>,
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
            args_json,
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
        // F55: derive the truncated preview from `result_json` at the
        // seam so every completion path — orchestrator, tool-context,
        // recovery — persists a consistent shape without forcing each
        // caller to re-implement UTF-8 safe truncation.
        let output_preview = result_json.as_ref().map(tool_result_preview_for_projection);
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
                output_preview,
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
        output_preview: Option<String>,
    ) -> Result<(), RuntimeError> {
        // F55: truncate any caller-supplied preview at the seam so the
        // projection never stores a multi-MB blob even if a runtime
        // caller passed raw output by accident.
        let output_preview = output_preview.as_deref().map(truncate_output_preview);
        let event = make_envelope(RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
            project: project.clone(),
            invocation_id,
            task_id,
            tool_name,
            finished_at_ms: now_ms(),
            outcome,
            error_message,
            output_preview,
        }));
        self.store.append(&[event]).await?;
        Ok(())
    }

    async fn append_audit_events(&self, events: &[RuntimeEvent]) -> Result<(), RuntimeError> {
        if events.is_empty() {
            return Ok(());
        }
        let batch: Vec<_> = events.iter().cloned().map(make_envelope).collect();
        self.store.append(&batch).await?;
        Ok(())
    }
}

/// F55: derive a UTF-8 safe, length-capped preview string from a tool's
/// structured result payload for operator observability.
///
/// - Plain-string results (the common case for bash/read/grep) use the
///   string directly.
/// - Anything else is serialized back to pretty JSON so operators can
///   eyeball structured tool returns in the run-detail view.
///
/// The returned string is always truncated via `truncate_output_preview`
/// so the projection never holds more than
/// `TOOL_OUTPUT_PREVIEW_MAX_BYTES` + the truncation marker.
pub(crate) fn tool_result_preview_for_projection(value: &serde_json::Value) -> String {
    let raw = match value {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    truncate_output_preview(&raw)
}
