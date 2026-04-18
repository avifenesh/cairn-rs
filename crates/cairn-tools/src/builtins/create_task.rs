//! create_task — spawn a sub-task via TaskService.
//!
//! Gives the agent a delegation primitive: it can break the current work
//! into sub-tasks that run under the same parent run, then either continue
//! immediately or signal the orchestrator to wait for completion.
//!
//! ## Parameters
//! ```json
//! {
//!   "run_id":      "run_abc123",   // required — parent run
//!   "description": "Research RFCs", // required — human-readable task goal
//!   "agent_type":  "researcher",   // optional — role for the sub-agent
//!   "blocking":    true            // optional; default false
//! }
//! ```
//!
//! ## Output
//! ```json
//! {
//!   "task_id": "task_...",
//!   "run_id":  "run_abc123",
//!   "state":   "queued",
//!   "blocking": true
//! }
//! ```
//!
//! When `blocking = true` the caller should treat `loop_signal = WaitSubagent`.
//! The task_id is returned so the orchestrator can track it.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{policy::ExecutionClass, ProjectKey, RunId, TaskId};
use cairn_runtime::tasks::TaskService;
use serde_json::Value;

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use cairn_domain::recovery::RetrySafety;

/// Sub-task creation tool.
pub struct CreateTaskTool {
    task_service: Arc<dyn TaskService>,
}

impl CreateTaskTool {
    pub fn new(task_service: Arc<dyn TaskService>) -> Self {
        Self { task_service }
    }
}

#[async_trait]
impl ToolHandler for CreateTaskTool {
    fn name(&self) -> &str {
        "create_task"
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
        "Spawn a sub-task under the current run. Use blocking=true to wait for completion."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["run_id", "description"],
            "properties": {
                "run_id": {
                    "type": "string",
                    "description": "Parent run ID (the orchestrator's own run_id)."
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of the sub-task goal."
                },
                "agent_type": {
                    "type": "string",
                    "description": "Role for the sub-agent (e.g. 'researcher', 'executor')."
                },
                "blocking": {
                    "type": "boolean",
                    "default": false,
                    "description": "If true, the orchestrator should wait for this task to complete."
                }
            }
        })
    }

    // Task creation is a structural change to the run — monitored but no approval.
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        // ── Validate ──────────────────────────────────────────────────────────
        let run_id_str =
            args.get("run_id")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "run_id".into(),
                    message: "required".into(),
                })?;
        if run_id_str.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "run_id".into(),
                message: "must not be empty".into(),
            });
        }

        let description = args
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "description".into(),
                message: "required".into(),
            })?
            .to_owned();
        if description.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "description".into(),
                message: "must not be empty".into(),
            });
        }

        let blocking = args
            .get("blocking")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // ── Generate a unique task ID ─────────────────────────────────────────
        let now_ms = now_millis();
        let task_id = TaskId::new(format!("task_{}_{}", run_id_str, now_ms));
        let parent_run_id = RunId::new(run_id_str);

        // ── Submit via TaskService ────────────────────────────────────────────
        let record = self
            .task_service
            .submit(project, task_id.clone(), Some(parent_run_id), None, 0)
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;

        Ok(ToolResult::ok(serde_json::json!({
            "task_id":    record.task_id.as_str(),
            "run_id":     run_id_str,
            "state":      format!("{:?}", record.state).to_lowercase(),
            "blocking":   blocking,
            "description": description,
        })))
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "in-memory-runtime"))]
mod tests {
    use super::*;
    use cairn_runtime::services::TaskServiceImpl;
    use cairn_store::InMemoryStore;
    use std::sync::Arc;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }

    fn make_tool() -> CreateTaskTool {
        let store = Arc::new(InMemoryStore::new());
        CreateTaskTool::new(Arc::new(TaskServiceImpl::new(store)))
    }

    // ── Metadata ──────────────────────────────────────────────────────────────

    #[test]
    fn name_tier_class() {
        let t = make_tool();
        assert_eq!(t.name(), "create_task");
        assert_eq!(t.tier(), ToolTier::Registered);
        assert_eq!(t.execution_class(), ExecutionClass::SupervisedProcess);
    }

    #[test]
    fn schema_requires_run_id_and_description() {
        let req = make_tool().parameters_schema()["required"]
            .as_array()
            .unwrap()
            .clone();
        assert!(req.iter().any(|v| v.as_str() == Some("run_id")));
        assert!(req.iter().any(|v| v.as_str() == Some("description")));
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn missing_run_id_is_invalid() {
        let err = make_tool()
            .execute(&project(), serde_json::json!({"description": "do a thing"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "run_id"));
    }

    #[tokio::test]
    async fn missing_description_is_invalid() {
        let err = make_tool()
            .execute(&project(), serde_json::json!({"run_id": "r1"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "description"));
    }

    #[tokio::test]
    async fn empty_description_is_invalid() {
        let err = make_tool()
            .execute(
                &project(),
                serde_json::json!({"run_id": "r1", "description": "  "}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "description"));
    }

    // ── Happy paths ───────────────────────────────────────────────────────────

    #[tokio::test]
    async fn creates_task_successfully() {
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({
                    "run_id": "run_parent", "description": "Research cairn-rs architecture"
                }),
            )
            .await
            .unwrap();

        let task_id = result.output["task_id"].as_str().unwrap();
        assert!(!task_id.is_empty(), "task_id must be populated");
        assert_eq!(result.output["run_id"], "run_parent");
        assert_eq!(result.output["state"], "queued", "new task must be queued");
        assert_eq!(result.output["blocking"], false, "default is non-blocking");
        assert_eq!(
            result.output["description"],
            "Research cairn-rs architecture"
        );
    }

    #[tokio::test]
    async fn blocking_flag_is_preserved() {
        let result = make_tool()
            .execute(
                &project(),
                serde_json::json!({
                    "run_id": "run_1", "description": "delegate subtask", "blocking": true
                }),
            )
            .await
            .unwrap();
        assert_eq!(result.output["blocking"], true);
    }

    #[tokio::test]
    async fn two_tasks_get_distinct_ids() {
        let tool = make_tool();
        let r1 = tool
            .execute(
                &project(),
                serde_json::json!({"run_id":"r","description":"a"}),
            )
            .await
            .unwrap();
        // small delay to ensure distinct timestamps
        tokio::time::sleep(tokio::time::Duration::from_millis(2)).await;
        let r2 = tool
            .execute(
                &project(),
                serde_json::json!({"run_id":"r","description":"b"}),
            )
            .await
            .unwrap();
        assert_ne!(
            r1.output["task_id"].as_str().unwrap(),
            r2.output["task_id"].as_str().unwrap(),
            "consecutive task IDs must be distinct"
        );
    }
}
