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
    async fn record_completed(
        &self,
        project: &ProjectKey,
        invocation_id: ToolInvocationId,
        task_id: Option<TaskId>,
        tool_name: String,
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
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
    ) -> Result<(), RuntimeError> {
        let event = make_envelope(RuntimeEvent::ToolInvocationCompleted(
            ToolInvocationCompleted {
                project: project.clone(),
                invocation_id,
                task_id,
                tool_name,
                finished_at_ms: now_ms(),
                outcome: ToolInvocationOutcomeKind::Success,
            },
        ));
        self.store.append(&[event]).await?;
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
}
