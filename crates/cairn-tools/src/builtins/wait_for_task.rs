//! `wait_for_task` built-in tool — poll until a task reaches a terminal state.
//!
//! Polls `get_task` at a configurable interval until the task reaches a
//! terminal state (`completed`, `failed`, `canceled`, `dead_lettered`) or the
//! wall-clock timeout expires.
//!
//! Returns the final task state when terminal or a timeout error.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey, TaskId};
use cairn_store::projections::TaskReadModel;
use serde_json::Value;

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

/// Maximum wait time (hard cap regardless of what caller requests).
const MAX_WAIT_SECS: u64 = 300; // 5 minutes
/// Minimum poll interval.
const MIN_POLL_MS: u64 = 500;

/// Poll until a task is terminal.
pub struct WaitForTaskTool {
    store: Arc<dyn TaskReadModel>,
}

impl WaitForTaskTool {
    pub fn new(store: Arc<dyn TaskReadModel>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ToolHandler for WaitForTaskTool {
    fn name(&self) -> &str {
        "wait_for_task"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::Observational
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::IdempotentSafe
    }
    fn description(&self) -> &str {
        "Wait until a task reaches a terminal state (completed/failed/canceled). \
         Polls at the given interval and returns the final task state, \
         or times out if the task doesn't complete within the deadline."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": {
                    "type": "string",
                    "description": "Task ID to wait for"
                },
                "timeout_secs": {
                    "type": "integer",
                    "description": "Maximum seconds to wait (default 60, max 300)",
                    "default": 60,
                    "minimum": 1,
                    "maximum": 300
                },
                "poll_interval_ms": {
                    "type": "integer",
                    "description": "Polling interval in milliseconds (default 1000, min 500)",
                    "default": 1000,
                    "minimum": 500,
                    "maximum": 10000
                }
            }
        })
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SandboxedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let task_id_str = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "task_id".into(),
                message: "required string".into(),
            })?;
        let task_id = TaskId::new(task_id_str);

        let timeout_secs = args
            .get("timeout_secs")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(MAX_WAIT_SECS))
            .unwrap_or(60);

        let poll_ms = args
            .get("poll_interval_ms")
            .and_then(|v| v.as_u64())
            .map(|n| n.max(MIN_POLL_MS))
            .unwrap_or(1_000);

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);

        loop {
            match TaskReadModel::get(self.store.as_ref(), &task_id).await {
                Ok(Some(task)) => {
                    if task.state.is_terminal() {
                        return Ok(ToolResult::ok(serde_json::json!({
                            "task_id":  task.task_id.as_str(),
                            "state":    format!("{:?}", task.state).to_lowercase(),
                            "terminal": true,
                            "waited":   true,
                        })));
                    }
                }
                Ok(None) => {
                    return Err(ToolError::Permanent(format!(
                        "task not found: {task_id_str}"
                    )));
                }
                Err(e) => {
                    return Err(ToolError::Transient(format!("store error: {e}")));
                }
            }

            if std::time::Instant::now() >= deadline {
                return Err(ToolError::TimedOut);
            }

            tokio::time::sleep(std::time::Duration::from_millis(poll_ms)).await;
        }
    }
}

#[cfg(all(test, feature = "in-memory-runtime"))]
mod tests {
    use super::*;
    use cairn_domain::{ProjectKey, TaskId};
    use cairn_runtime::InMemoryServices;
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    async fn svc() -> Arc<InMemoryServices> {
        Arc::new(InMemoryServices::new())
    }

    #[tokio::test]
    async fn returns_immediately_when_already_terminal() {
        let svc = svc().await;
        svc.tasks
            .submit(&project(), None, TaskId::new("task_wt"), None, None, 0)
            .await
            .unwrap();
        svc.tasks
            .claim(None, &TaskId::new("task_wt"), "worker".into(), 30_000)
            .await
            .unwrap();
        svc.tasks
            .start(None, &TaskId::new("task_wt"))
            .await
            .unwrap();
        svc.tasks
            .complete(None, &TaskId::new("task_wt"))
            .await
            .unwrap();

        let tool = WaitForTaskTool::new(svc.store.clone());
        let res = tool
            .execute(
                &project(),
                serde_json::json!({
                    "task_id": "task_wt",
                    "timeout_secs": 5
                }),
            )
            .await
            .unwrap();
        assert_eq!(res.output["state"], "completed");
        assert_eq!(res.output["terminal"], true);
    }

    #[tokio::test]
    async fn times_out_when_task_not_terminal() {
        let svc = svc().await;
        svc.tasks
            .submit(&project(), None, TaskId::new("task_wt2"), None, None, 0)
            .await
            .unwrap();
        // Task stays in queued — will never be terminal before timeout.

        let tool = WaitForTaskTool::new(svc.store.clone());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "task_id": "task_wt2",
                    "timeout_secs": 1,
                    "poll_interval_ms": 500
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::TimedOut));
    }

    #[tokio::test]
    async fn not_found_is_permanent_error() {
        let svc = svc().await;
        let tool = WaitForTaskTool::new(svc.store.clone());
        let err = tool
            .execute(
                &project(),
                serde_json::json!({
                    "task_id": "nope",
                    "timeout_secs": 1
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
    }

    #[test]
    fn tier_is_registered() {
        assert_eq!(
            WaitForTaskTool::new(Arc::new(cairn_store::InMemoryStore::new())).tier(),
            ToolTier::Registered
        );
    }
}
