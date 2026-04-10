//! schedule_task — create a recurring ScheduledTask via event log.
//!
//! ## Parameters
//! ```json
//! { "name": "weekly_review", "cron_expression": "0 9 * * 1", "enabled": true }
//! ```
//!
//! ## Output
//! ```json
//! { "scheduled_task_id": "sched_...", "name": "weekly_review", "cron_expression": "0 9 * * 1" }
//! ```

use super::{ToolEffect, ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::{
    events::ScheduledTaskCreated, policy::ExecutionClass, recovery::RetrySafety, EventEnvelope,
    EventId, EventSource, ProjectKey, RuntimeEvent, ScheduledTaskId,
};
use cairn_store::EventLog;
use serde_json::Value;
use std::sync::Arc;

pub struct ScheduleTaskTool {
    event_log: Arc<dyn EventLog + Send + Sync>,
}

impl ScheduleTaskTool {
    pub fn new(event_log: Arc<dyn EventLog + Send + Sync>) -> Self {
        Self { event_log }
    }
}

#[async_trait]
impl ToolHandler for ScheduleTaskTool {
    fn name(&self) -> &str {
        "schedule_task"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn tool_effect(&self) -> ToolEffect {
        ToolEffect::External
    }
    fn retry_safety(&self) -> RetrySafety {
        RetrySafety::DangerousPause
    }
    fn description(&self) -> &str {
        "Create a recurring scheduled task with a cron expression."
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "required": ["name", "cron_expression"],
            "properties": {
                "name":            { "type": "string", "description": "Human-readable task label." },
                "cron_expression": { "type": "string", "description": "Cron schedule (e.g. '0 9 * * 1')." },
                "enabled":         { "type": "boolean", "default": true }
            }
        })
    }

    async fn execute(&self, project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "name".into(),
                message: "required".into(),
            })?
            .to_owned();
        let cron = args
            .get("cron_expression")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError::InvalidArgs {
                field: "cron_expression".into(),
                message: "required".into(),
            })?
            .to_owned();
        if name.trim().is_empty() {
            return Err(ToolError::InvalidArgs {
                field: "name".into(),
                message: "must not be empty".into(),
            });
        }
        let now_ms = now_millis();
        let task_id = ScheduledTaskId::new(format!("sched_{}", now_ms));
        let event = EventEnvelope::for_runtime_event(
            EventId::new(format!("evt_sched_{}", now_ms)),
            EventSource::Runtime,
            RuntimeEvent::ScheduledTaskCreated(ScheduledTaskCreated {
                tenant_id: project.tenant_id.clone(),
                scheduled_task_id: task_id.clone(),
                name: name.clone(),
                cron_expression: cron.clone(),
                next_run_at: None,
                created_at: now_ms,
            }),
        );
        self.event_log
            .append(&[event])
            .await
            .map_err(|e| ToolError::Transient(e.to_string()))?;
        Ok(ToolResult::ok(serde_json::json!({
            "scheduled_task_id": task_id.as_str(),
            "name":              name,
            "cron_expression":   cron,
        })))
    }
}

fn now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_store::InMemoryStore;
    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }
    fn make_tool() -> ScheduleTaskTool {
        ScheduleTaskTool::new(Arc::new(InMemoryStore::new()))
    }

    #[test]
    fn name_tier() {
        assert_eq!(make_tool().name(), "schedule_task");
    }

    #[tokio::test]
    async fn missing_name_is_invalid() {
        let err = make_tool()
            .execute(
                &project(),
                serde_json::json!({"cron_expression":"0 9 * * 1"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn creates_scheduled_task() {
        let store = Arc::new(InMemoryStore::new());
        let tool = ScheduleTaskTool::new(store.clone());
        let result = tool
            .execute(
                &project(),
                serde_json::json!({
                    "name": "weekly_review", "cron_expression": "0 9 * * 1"
                }),
            )
            .await
            .unwrap();
        let task_id = result.output["scheduled_task_id"].as_str().unwrap();
        assert!(!task_id.is_empty());
        assert_eq!(result.output["name"], "weekly_review");
        use cairn_store::EventLog;
        let events = store.read_stream(None, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].envelope.payload,
            RuntimeEvent::ScheduledTaskCreated(_)
        ));
    }
}
