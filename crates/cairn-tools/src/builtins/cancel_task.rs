//! `cancel_task` built-in tool — cancel a running/queued task (Sensitive).
//!
//! `ExecutionClass::SupervisedProcess` means the orchestrator gates this
//! through `ApprovalService` before executing.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey, TaskId};
use cairn_runtime::{tasks::TaskService, services::TaskServiceImpl};
use serde_json::Value;

use super::{ToolError, ToolHandler, ToolResult, ToolTier};

/// Cancel a task — Sensitive, requires approval.
pub struct CancelTaskTool {
    tasks: Arc<dyn TaskService>,
}

impl CancelTaskTool {
    pub fn new(tasks: Arc<dyn TaskService>) -> Self { Self { tasks } }
}

#[async_trait]
impl ToolHandler for CancelTaskTool {
    fn name(&self) -> &str { "cancel_task" }
    fn tier(&self) -> ToolTier { ToolTier::Registered }
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
    fn execution_class(&self) -> ExecutionClass { ExecutionClass::SupervisedProcess }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let task_id_str = args.get("task_id").and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "task_id".into(), message: "required string".into(),
            })?;

        let reason = args.get("reason").and_then(|v| v.as_str())
            .unwrap_or("cancelled by agent")
            .to_owned();

        match self.tasks.cancel(&TaskId::new(task_id_str)).await {
            Ok(task) => Ok(ToolResult::ok(serde_json::json!({
                "task_id": task.task_id.as_str(),
                "state":   format!("{:?}", task.state).to_lowercase(),
                "reason":  reason,
            }))),
            Err(cairn_runtime::error::RuntimeError::NotFound { .. }) =>
                Err(ToolError::Permanent(format!("task not found: {task_id_str}"))),
            Err(cairn_runtime::error::RuntimeError::InvalidTransition { .. }) =>
                Err(ToolError::Permanent(format!(
                    "task {task_id_str} cannot be cancelled in its current state"
                ))),
            Err(e) =>
                Err(ToolError::Transient(format!("cancel failed: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use cairn_domain::{ProjectKey, TaskId, lifecycle::TaskState};
    use cairn_runtime::{InMemoryServices, tasks::TaskService, services::TaskServiceImpl};
    use cairn_store::InMemoryStore;

    fn project() -> ProjectKey { ProjectKey::new("t", "w", "p") }

    async fn svc() -> Arc<InMemoryServices> { Arc::new(InMemoryServices::new()) }

    fn task_svc(store: Arc<InMemoryStore>) -> Arc<dyn TaskService> {
        Arc::new(TaskServiceImpl::new(store))
    }

    #[tokio::test]
    async fn cancels_queued_task() {
        let svc = svc().await;
        svc.tasks.submit(&project(), TaskId::new("task_ct"), None, None, 0).await.unwrap();

        let tool = CancelTaskTool::new(task_svc(svc.store.clone()));
        let res = tool.execute(&project(), serde_json::json!({
            "task_id": "task_ct",
            "reason": "test cancel"
        })).await.unwrap();
        assert_eq!(res.output["state"], "canceled");

        let record = svc.tasks.get(&TaskId::new("task_ct")).await.unwrap().unwrap();
        assert_eq!(record.state, TaskState::Canceled);
    }

    #[tokio::test]
    async fn missing_task_id_is_invalid() {
        let svc = svc().await;
        let tool = CancelTaskTool::new(task_svc(svc.store.clone()));
        let err = tool.execute(&project(), serde_json::json!({})).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[test]
    fn tier_is_registered() {
        let store = Arc::new(InMemoryStore::new());
        let tool = CancelTaskTool::new(Arc::new(TaskServiceImpl::new(store)));
        assert_eq!(tool.tier(), ToolTier::Registered);
        assert!(matches!(tool.execution_class(), ExecutionClass::SupervisedProcess));
    }
}
