//! `cancel_task` built-in tool — cancel a running/queued task (Sensitive).
//!
//! `ExecutionClass::SupervisedProcess` means the orchestrator gates this
//! through `ApprovalService` before executing.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey, TaskId};
use cairn_runtime::tasks::TaskService;
use serde_json::Value;

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

/// Cancel a task — Sensitive, requires approval.
pub struct CancelTaskTool {
    tasks: Arc<dyn TaskService>,
}

impl CancelTaskTool {
    pub fn new(tasks: Arc<dyn TaskService>) -> Self {
        Self { tasks }
    }
}

#[async_trait]
impl ToolHandler for CancelTaskTool {
    fn name(&self) -> &str {
        "cancel_task"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Internal
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::AuthorResponsible
    }
    fn description(&self) -> &str {
        "Cancel a task that is queued, leased, or running. \
         This is an irreversible action and requires operator approval. \
         Returns the final task state after cancellation."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "ID of the task to cancel"
                },
                "reason": {
                    "type": "string",
                    "description": "Optional human-readable reason for cancellation"
                }
            }
        })
    }

    /// Sensitive — requires approval before execution.
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let task_id_str = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "task_id".into(),
                message: "required string".into(),
            })?;

        let reason = args
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("cancelled by agent")
            .to_owned();

        // Pass `None` for session_id; the adapter resolves the task's
        // session from the projection (TaskRecord.parent_run_id →
        // RunRecord.session_id). Tools don't carry session context
        // directly.
        match self.tasks.cancel(None, &TaskId::new(task_id_str)).await {
            Ok(task) => Ok(ToolResult::ok(serde_json::json!({
                "task_id": task.task_id.as_str(),
                "state":   format!("{:?}", task.state).to_lowercase(),
                "reason":  reason,
            }))),
            Err(cairn_runtime::error::RuntimeError::NotFound { .. }) => Err(ToolError::Permanent(
                format!("task not found: {task_id_str}"),
            )),
            Err(cairn_runtime::error::RuntimeError::InvalidTransition { .. }) => {
                Err(ToolError::Permanent(format!(
                    "task {task_id_str} cannot be cancelled in its current state"
                )))
            }
            Err(e) => Err(ToolError::Transient(format!("cancel failed: {e}"))),
        }
    }
}
