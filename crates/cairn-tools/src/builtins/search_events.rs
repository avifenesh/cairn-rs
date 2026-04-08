//! search_events — query the EventLog for recent or entity-scoped events.
//!
//! ## Parameters
//! ```json
//! {
//!   "entity_id":    "run_abc123",  // optional; scopes to one entity
//!   "entity_type":  "run",         // required when entity_id is set
//!   "after":        42,            // optional; cursor (event position)
//!   "limit":        20             // default 20, max 100
//! }
//! ```
//!
//! ## Output
//! ```json
//! {
//!   "events": [{ "position": 1, "event_type": "run_created", "stored_at": 0 }],
//!   "total":  1
//! }
//! ```

use super::{ToolError, ToolHandler, ToolResult, ToolTier};
use async_trait::async_trait;
use cairn_domain::{
    policy::ExecutionClass, ApprovalId, CheckpointId, EvalRunId, IngestJobId, MailboxMessageId,
    ProjectKey, PromptAssetId, PromptReleaseId, PromptVersionId, RunId, SessionId, SignalId,
    TaskId, ToolInvocationId,
};
use cairn_store::{EntityRef, EventLog, EventPosition};
use serde_json::Value;
use std::sync::Arc;

const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;

pub struct SearchEventsTool {
    event_log: Arc<dyn EventLog + Send + Sync>,
}

impl SearchEventsTool {
    pub fn new(event_log: Arc<dyn EventLog + Send + Sync>) -> Self {
        Self { event_log }
    }
}

fn parse_entity_ref(entity_type: &str, entity_id: &str) -> Option<EntityRef> {
    match entity_type {
        "session" => Some(EntityRef::Session(SessionId::new(entity_id))),
        "run" => Some(EntityRef::Run(RunId::new(entity_id))),
        "task" => Some(EntityRef::Task(TaskId::new(entity_id))),
        "approval" => Some(EntityRef::Approval(ApprovalId::new(entity_id))),
        "checkpoint" => Some(EntityRef::Checkpoint(CheckpointId::new(entity_id))),
        "mailbox" => Some(EntityRef::Mailbox(MailboxMessageId::new(entity_id))),
        "tool_invocation" => Some(EntityRef::ToolInvocation(ToolInvocationId::new(entity_id))),
        "signal" => Some(EntityRef::Signal(SignalId::new(entity_id))),
        "ingest_job" => Some(EntityRef::IngestJob(IngestJobId::new(entity_id))),
        "eval_run" => Some(EntityRef::EvalRun(EvalRunId::new(entity_id))),
        "prompt_asset" => Some(EntityRef::PromptAsset(PromptAssetId::new(entity_id))),
        "prompt_version" => Some(EntityRef::PromptVersion(PromptVersionId::new(entity_id))),
        "prompt_release" => Some(EntityRef::PromptRelease(PromptReleaseId::new(entity_id))),
        _ => None,
    }
}

#[async_trait]
impl ToolHandler for SearchEventsTool {
    fn name(&self) -> &str {
        "search_events"
    }
    fn tier(&self) -> ToolTier {
        ToolTier::Registered
    }
    fn description(&self) -> &str {
        "Query the event log for recent events, optionally scoped to one entity."
    }
    fn execution_class(&self) -> ExecutionClass {
        ExecutionClass::SupervisedProcess
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "entity_id":   { "type": "string", "description": "Scope to events for this entity." },
                "entity_type": {
                    "type": "string",
                    "enum": ["session","run","task","approval","checkpoint","mailbox",
                             "tool_invocation","signal","ingest_job","eval_run",
                             "prompt_asset","prompt_version","prompt_release"],
                    "description": "Required when entity_id is provided."
                },
                "after":  { "type": "integer", "description": "Return events after this position (cursor)." },
                "limit":  { "type": "integer", "default": 20, "description": "Max events (max 100)." }
            }
        })
    }

    async fn execute(&self, _project: &ProjectKey, args: Value) -> Result<ToolResult, ToolError> {
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_LIMIT as u64)
            .min(MAX_LIMIT as u64) as usize;
        let after = args
            .get("after")
            .and_then(|v| v.as_u64())
            .map(EventPosition);

        let events = if let Some(entity_id) = args.get("entity_id").and_then(|v| v.as_str()) {
            let entity_type = args
                .get("entity_type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::InvalidArgs {
                    field: "entity_type".into(),
                    message: "required when entity_id is provided".into(),
                })?;
            let entity_ref =
                parse_entity_ref(entity_type, entity_id).ok_or_else(|| ToolError::InvalidArgs {
                    field: "entity_type".into(),
                    message: format!("unknown entity type: '{entity_type}'"),
                })?;
            self.event_log
                .read_by_entity(&entity_ref, after, limit)
                .await
                .map_err(|e| ToolError::Transient(e.to_string()))?
        } else {
            self.event_log
                .read_stream(after, limit)
                .await
                .map_err(|e| ToolError::Transient(e.to_string()))?
        };

        let event_list: Vec<Value> = events
            .iter()
            .map(|e| {
                serde_json::json!({
                    "position":   e.position.0,
                    "event_type": event_type_name(&e.envelope.payload),
                    "event_id":   e.envelope.event_id.as_str(),
                    "stored_at":  e.stored_at,
                })
            })
            .collect();

        let total = event_list.len();
        Ok(ToolResult::ok(
            serde_json::json!({ "events": event_list, "total": total }),
        ))
    }
}

fn event_type_name(payload: &cairn_domain::RuntimeEvent) -> &'static str {
    use cairn_domain::RuntimeEvent::*;
    match payload {
        SessionCreated(_) => "session_created",
        SessionStateChanged(_) => "session_state_changed",
        RunCreated(_) => "run_created",
        RunStateChanged(_) => "run_state_changed",
        TaskCreated(_) => "task_created",
        TaskStateChanged(_) => "task_state_changed",
        TaskLeaseClaimed(_) => "task_lease_claimed",
        TaskLeaseHeartbeated(_) => "task_lease_heartbeated",
        ApprovalRequested(_) => "approval_requested",
        ApprovalResolved(_) => "approval_resolved",
        CheckpointRecorded(_) => "checkpoint_recorded",
        CheckpointRestored(_) => "checkpoint_restored",
        ToolInvocationStarted(_) => "tool_invocation_started",
        ToolInvocationCompleted(_) => "tool_invocation_completed",
        ToolInvocationFailed(_) => "tool_invocation_failed",
        OutcomeRecorded(_) => "outcome_recorded",
        EvalRunStarted(_) => "eval_run_started",
        EvalRunCompleted(_) => "eval_run_completed",
        ProviderCallCompleted(_) => "provider_call_completed",
        RouteDecisionMade(_) => "route_decision_made",
        DefaultSettingSet(_) => "default_setting_set",
        DefaultSettingCleared(_) => "default_setting_cleared",
        ScheduledTaskCreated(_) => "scheduled_task_created",
        _ => "other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent};
    use cairn_store::InMemoryStore;

    fn project() -> ProjectKey {
        ProjectKey::new("t", "w", "p")
    }
    fn make_tool() -> SearchEventsTool {
        SearchEventsTool::new(Arc::new(InMemoryStore::new()))
    }

    #[test]
    fn name_tier() {
        assert_eq!(make_tool().name(), "search_events");
    }

    #[tokio::test]
    async fn empty_log_returns_empty_list() {
        let result = make_tool()
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 0);
    }

    #[tokio::test]
    async fn entity_id_without_type_is_invalid() {
        let err = make_tool()
            .execute(&project(), serde_json::json!({"entity_id": "run_1"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { field, .. } if field == "entity_type"));
    }

    #[tokio::test]
    async fn unknown_entity_type_is_invalid() {
        let err = make_tool()
            .execute(
                &project(),
                serde_json::json!({"entity_id":"r","entity_type":"widget"}),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::InvalidArgs { .. }));
    }

    #[tokio::test]
    async fn returns_events_from_stream() {
        let store = Arc::new(InMemoryStore::new());
        use cairn_store::EventLog;
        store
            .append(&[EventEnvelope::for_runtime_event(
                EventId::new("e1"),
                EventSource::Runtime,
                RuntimeEvent::RunCreated(RunCreated {
                    project: project(),
                    run_id: RunId::new("run_1"),
                    session_id: cairn_domain::SessionId::new("s1"),
                    parent_run_id: None,
                    prompt_release_id: None,
                    agent_role_id: None,
                }),
            )])
            .await
            .unwrap();
        let tool = SearchEventsTool::new(store);
        let result = tool
            .execute(&project(), serde_json::json!({}))
            .await
            .unwrap();
        assert_eq!(result.output["total"], 1);
        assert_eq!(result.output["events"][0]["event_type"], "run_created");
    }
}
