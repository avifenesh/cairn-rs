//! `get_task` built-in tool — inspect a specific task by ID.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey, TaskId};
use cairn_store::projections::TaskReadModel;
use serde_json::Value;

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

/// Read the current state of a task.
pub struct GetTaskTool {
    store: Arc<dyn TaskReadModel>,
}

impl GetTaskTool {
    pub fn new(store: Arc<dyn TaskReadModel>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl ToolHandler for GetTaskTool {
    fn name(&self) -> &str {
        "get_task"
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
        "Inspect a specific task by its ID. \
         Returns task state, lease info, parent run/task linkage, and timestamps."
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["task_id"],
            "properties": {
                "task_id": { "type": "string", "description": "The task ID to look up" }
            }
        })
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SandboxedProcess
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let task_id = args
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "task_id".into(),
                message: "required string".into(),
            })?;

        match TaskReadModel::get(self.store.as_ref(), &TaskId::new(task_id)).await {
            Ok(Some(task)) => Ok(ToolResult::ok(serde_json::json!({
                "task_id":         task.task_id.as_str(),
                "state":           format!("{:?}", task.state).to_lowercase(),
                "is_terminal":     task.state.is_terminal(),
                "parent_run_id":   task.parent_run_id.as_ref().map(|r| r.as_str()),
                "parent_task_id":  task.parent_task_id.as_ref().map(|t| t.as_str()),
                "lease_owner":     task.lease_owner,
                "lease_expires_at": task.lease_expires_at,
                "created_at":       task.created_at,
                "updated_at":      task.updated_at,
                "project": {
                    "tenant_id":    task.project.tenant_id.as_str(),
                    "workspace_id": task.project.workspace_id.as_str(),
                    "project_id":   task.project.project_id.as_str(),
                },
            }))),
            Ok(None) => Err(ToolError::Permanent(format!("task not found: {task_id}"))),
            Err(e) => Err(ToolError::Transient(format!("store error: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{ProjectKey, TaskId};
    use cairn_runtime::{tasks::TaskService, InMemoryServices};
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    async fn svc() -> Arc<InMemoryServices> {
        Arc::new(InMemoryServices::new())
    }

    #[tokio::test]
    async fn returns_task_state() {
        let svc = svc().await;
        svc.tasks
            .submit(&project(), TaskId::new("task_gt"), None, None, 0)
            .await
            .unwrap();

        let tool = GetTaskTool::new(svc.store.clone());
        let res = tool
            .execute(&project(), serde_json::json!({ "task_id": "task_gt" }))
            .await
            .unwrap();
        assert_eq!(res.output["task_id"], "task_gt");
        assert_eq!(res.output["state"], "queued");
        assert_eq!(res.output["is_terminal"], false);
    }

    #[tokio::test]
    async fn not_found_is_permanent_error() {
        let svc = svc().await;
        let tool = GetTaskTool::new(svc.store.clone());
        let err = tool
            .execute(&project(), serde_json::json!({ "task_id": "nope" }))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Permanent(_)));
    }

    #[test]
    fn tier_is_registered() {
        assert_eq!(
            GetTaskTool::new(Arc::new(cairn_store::InMemoryStore::new())).tier(),
            ToolTier::Registered
        );
    }
}
