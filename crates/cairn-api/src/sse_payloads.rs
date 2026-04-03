//! Preserved SSE payload shapes matching the frontend contract.
//!
//! The frontend expects specific JSON shapes per event type, NOT raw
//! serialized RuntimeEvent enums. This module maps domain events to
//! the exact shapes the UI consumes.

use cairn_domain::events::*;
use cairn_domain::tool_invocation::ToolInvocationTarget;
use serde::Serialize;

use crate::feed::FeedItem;

/// `task_update` payload: `{ task }` per fixture `task_update__running_task.json`.
#[derive(Clone, Debug, Serialize)]
pub struct TaskUpdatePayload {
    pub task: TaskUpdateInner,
}

/// Fields match fixture: id, type, status, title, description, progress, createdAt, updatedAt.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskUpdateInner {
    pub id: String,
    #[serde(rename = "type")]
    pub task_type: Option<String>,
    pub status: Option<String>,
    pub title: Option<String>,
    pub description: Option<String>,
    pub progress: Option<u32>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

/// `approval_required` payload: `{ approval }` per fixture `approval_required__pending.json`.
#[derive(Clone, Debug, Serialize)]
pub struct ApprovalRequiredPayload {
    pub approval: ApprovalInner,
}

/// Fields match fixture: id, type, status, title, description, context, createdAt.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalInner {
    pub id: String,
    #[serde(rename = "type")]
    pub approval_type: Option<String>,
    pub status: String,
    pub title: Option<String>,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
    pub created_at: Option<String>,
}

/// `assistant_tool_call` payload per fixture: `{ taskId, toolName, phase, args? }`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantToolCallPayload {
    pub task_id: Option<String>,
    pub tool_name: String,
    pub phase: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<serde_json::Value>,
}

/// `agent_progress` payload: `{ agentId, message }`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProgressPayload {
    pub agent_id: String,
    pub message: String,
}

// -- Non-runtime SSE families (signal/feed publisher, not RuntimeEvent) --

/// `feed_update` payload: `{ item }` wrapping a FeedItem.
#[derive(Clone, Debug, Serialize)]
pub struct FeedUpdatePayload {
    pub item: FeedItem,
}

/// `poll_completed` payload: `{ source, newCount }`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PollCompletedPayload {
    pub source: String,
    pub new_count: u32,
}

/// `digest_ready` payload (no fields consumed by UI, but presence matters).
#[derive(Clone, Debug, Serialize)]
pub struct DigestReadyPayload {}

/// `memory_proposed` payload: `{ memory: { ... } }` wrapping a full MemoryItem.
#[derive(Clone, Debug, Serialize)]
pub struct MemoryProposedPayload {
    pub memory: crate::memory_api::MemoryItem,
}

/// `memory_accepted` payload (no fields consumed by UI).
#[derive(Clone, Debug, Serialize)]
pub struct MemoryAcceptedPayload {}

fn task_update_payload_from_record(
    record: &cairn_store::projections::TaskRecord,
) -> TaskUpdatePayload {
    TaskUpdatePayload {
        task: TaskUpdateInner {
            id: record.task_id.to_string(),
            task_type: None,
            status: Some(format!("{:?}", record.state).to_lowercase()),
            title: record.title.clone(),
            description: record.description.clone(),
            progress: None,
            created_at: Some(record.created_at.to_string()),
            updated_at: Some(record.updated_at.to_string()),
        },
    }
}

fn approval_required_payload_from_record(
    record: &cairn_store::projections::ApprovalRecord,
) -> ApprovalRequiredPayload {
    let status = match record.decision {
        Some(cairn_domain::policy::ApprovalDecision::Approved) => "approved",
        Some(cairn_domain::policy::ApprovalDecision::Rejected) => "rejected",
        None => "pending",
    };

    ApprovalRequiredPayload {
        approval: ApprovalInner {
            id: record.approval_id.to_string(),
            approval_type: None,
            status: status.to_owned(),
            title: record.title.clone(),
            description: record.description.clone(),
            context: None,
            created_at: Some(record.created_at.to_string()),
        },
    }
}

/// Builds a `memory_proposed` SSE frame from a MemoryItem.
pub fn build_memory_proposed_frame(
    item: crate::memory_api::MemoryItem,
    event_id: Option<String>,
) -> crate::sse::SseFrame {
    let data = serde_json::to_value(&MemoryProposedPayload { memory: item }).unwrap_or_default();
    crate::sse::SseFrame {
        event: crate::sse::SseEventName::MemoryProposed,
        data,
        id: event_id,
    }
}

// -- Assistant streaming SSE families (from cairn-agent StreamingOutput) --

/// `assistant_delta` payload: `{ taskId, deltaText }`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDeltaPayload {
    pub task_id: String,
    pub delta_text: String,
}

/// `assistant_end` payload: `{ taskId, messageText }`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEndPayload {
    pub task_id: String,
    pub message_text: String,
}

/// `assistant_reasoning` payload: `{ taskId, round, thought }`.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantReasoningPayload {
    pub task_id: String,
    pub round: u32,
    pub thought: String,
}

/// Builds a higher-fidelity `assistant_tool_call` SSE frame using
/// `ToolLifecycleOutput` from cairn-tools for args/result enrichment.
pub fn build_enriched_tool_call_frame(
    lifecycle: &cairn_tools::runtime_service::ToolLifecycleOutput,
    task_id: Option<&str>,
    event_id: Option<String>,
) -> crate::sse::SseFrame {
    let payload = AssistantToolCallPayload {
        task_id: task_id.map(|s| s.to_owned()),
        tool_name: lifecycle.tool_name.clone(),
        phase: match lifecycle.phase.as_str() {
            "start" => "start",
            "completed" => "completed",
            "failed" => "failed",
            _ => "unknown",
        },
        args: lifecycle.args.clone(),
    };
    let data = serde_json::to_value(&payload).unwrap_or_default();
    crate::sse::SseFrame {
        event: crate::sse::SseEventName::AssistantToolCall,
        data,
        id: event_id,
    }
}

/// Builds a higher-fidelity `task_update` SSE frame using a `TaskRecord`
/// from the store, filling title/description/timestamps from real data.
pub fn build_enriched_task_update_frame(
    record: &cairn_store::projections::TaskRecord,
    event_id: Option<String>,
) -> crate::sse::SseFrame {
    let payload = task_update_payload_from_record(record);
    let data = serde_json::to_value(&payload).unwrap_or_default();
    crate::sse::SseFrame {
        event: crate::sse::SseEventName::TaskUpdate,
        data,
        id: event_id,
    }
}

/// Builds a higher-fidelity `approval_required` SSE frame using an `ApprovalRecord`.
pub fn build_enriched_approval_frame(
    record: &cairn_store::projections::ApprovalRecord,
    event_id: Option<String>,
) -> crate::sse::SseFrame {
    let payload = approval_required_payload_from_record(record);
    let data = serde_json::to_value(&payload).unwrap_or_default();
    crate::sse::SseFrame {
        event: crate::sse::SseEventName::ApprovalRequired,
        data,
        id: event_id,
    }
}

/// Builds a higher-fidelity `assistant_end` SSE frame with the fully
/// assembled message text. The caller accumulates deltas during streaming
/// and passes the final text here.
pub fn build_enriched_assistant_end_frame(
    task_id: &str,
    message_text: &str,
    event_id: Option<String>,
) -> crate::sse::SseFrame {
    let payload = AssistantEndPayload {
        task_id: task_id.to_owned(),
        message_text: message_text.to_owned(),
    };
    let data = serde_json::to_value(&payload).unwrap_or_default();
    crate::sse::SseFrame {
        event: crate::sse::SseEventName::AssistantEnd,
        data,
        id: event_id,
    }
}

/// Maps a `StreamingOutput` from cairn-agent to an SSE frame with
/// the preserved event name and frontend-compatible payload shape.
pub fn build_streaming_sse_frame(
    output: &cairn_agent::streaming::StreamingOutput,
    task_id: &str,
    event_id: Option<String>,
) -> Option<crate::sse::SseFrame> {
    use cairn_agent::streaming::StreamingOutput;

    let (event_name, data) = match output {
        StreamingOutput::AssistantDelta(d) => {
            let payload = AssistantDeltaPayload {
                task_id: task_id.to_owned(),
                delta_text: d.content.clone(),
            };
            (
                crate::sse::SseEventName::AssistantDelta,
                serde_json::to_value(&payload).ok()?,
            )
        }
        StreamingOutput::AssistantReasoning(r) => {
            let payload = AssistantReasoningPayload {
                task_id: task_id.to_owned(),
                round: r.index + 1,
                thought: r.content.clone(),
            };
            (
                crate::sse::SseEventName::AssistantReasoning,
                serde_json::to_value(&payload).ok()?,
            )
        }
        StreamingOutput::AssistantEnd(_) => {
            // Caller must use build_enriched_assistant_end_frame with
            // the assembled message text from accumulated deltas.
            // Returning None forces the caller to handle this explicitly.
            return None;
        }
        StreamingOutput::ToolCallRequested(t) => {
            let payload = AssistantToolCallPayload {
                task_id: Some(task_id.to_owned()),
                tool_name: t.tool_name.clone(),
                phase: "start",
                args: Some(t.arguments.clone()),
            };
            (
                crate::sse::SseEventName::AssistantToolCall,
                serde_json::to_value(&payload).ok()?,
            )
        }
        StreamingOutput::ToolResult(_) => {
            // tool_result is not in the preserved SSE catalog
            return None;
        }
    };

    Some(crate::sse::SseFrame {
        event: event_name,
        data,
        id: event_id,
    })
}

/// Builds a `feed_update` SSE frame.
pub fn build_feed_update_frame(item: FeedItem, event_id: Option<String>) -> crate::sse::SseFrame {
    let data = serde_json::to_value(&FeedUpdatePayload { item }).unwrap_or_default();
    crate::sse::SseFrame {
        event: crate::sse::SseEventName::FeedUpdate,
        data,
        id: event_id,
    }
}

/// Builds a `poll_completed` SSE frame.
pub fn build_poll_completed_frame(
    source: &str,
    new_count: u32,
    event_id: Option<String>,
) -> crate::sse::SseFrame {
    let data = serde_json::to_value(&PollCompletedPayload {
        source: source.to_owned(),
        new_count,
    })
    .unwrap_or_default();
    crate::sse::SseFrame {
        event: crate::sse::SseEventName::PollCompleted,
        data,
        id: event_id,
    }
}

// -- Runtime event mapping --

/// Maps a `RuntimeEvent` to its preserved frontend-compatible JSON payload.
pub fn shape_event_payload(event: &RuntimeEvent) -> Option<serde_json::Value> {
    match event {
        RuntimeEvent::TaskCreated(e) => {
            let payload = TaskUpdatePayload {
                task: TaskUpdateInner {
                    id: e.task_id.to_string(),
                    task_type: None,
                    status: Some("queued".to_owned()),
                    title: None,
                    description: None,
                    progress: None,
                    created_at: None,
                    updated_at: None,
                },
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::TaskStateChanged(e) => {
            let payload = TaskUpdatePayload {
                task: TaskUpdateInner {
                    id: e.task_id.to_string(),
                    task_type: None,
                    status: Some(format!("{:?}", e.transition.to).to_lowercase()),
                    title: None,
                    description: None,
                    progress: None,
                    created_at: None,
                    updated_at: None,
                },
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::TaskLeaseClaimed(e) => {
            let payload = TaskUpdatePayload {
                task: TaskUpdateInner {
                    id: e.task_id.to_string(),
                    task_type: None,
                    status: Some("leased".to_owned()),
                    title: None,
                    description: None,
                    progress: None,
                    created_at: None,
                    updated_at: None,
                },
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::TaskLeaseHeartbeated(e) => {
            let payload = TaskUpdatePayload {
                task: TaskUpdateInner {
                    id: e.task_id.to_string(),
                    task_type: None,
                    status: None,
                    title: None,
                    description: None,
                    progress: None,
                    created_at: None,
                    updated_at: None,
                },
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::ApprovalRequested(e) => {
            let payload = ApprovalRequiredPayload {
                approval: ApprovalInner {
                    id: e.approval_id.to_string(),
                    approval_type: None,
                    status: "pending".to_owned(),
                    title: None,
                    description: None,
                    context: None,
                    created_at: None,
                },
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::ToolInvocationStarted(e) => {
            let tool_name = match &e.target {
                ToolInvocationTarget::Builtin { tool_name } => tool_name.clone(),
                ToolInvocationTarget::Plugin { tool_name, .. } => tool_name.clone(),
            };
            let payload = AssistantToolCallPayload {
                task_id: e.task_id.as_ref().map(|id| id.to_string()),
                tool_name,
                phase: "start",
                args: None,
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::ToolInvocationCompleted(e) => {
            let payload = AssistantToolCallPayload {
                task_id: e.task_id.as_ref().map(|id| id.to_string()),
                tool_name: e.tool_name.clone(),
                phase: "completed",
                args: None,
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::ToolInvocationFailed(e) => {
            let payload = AssistantToolCallPayload {
                task_id: e.task_id.as_ref().map(|id| id.to_string()),
                tool_name: e.tool_name.clone(),
                phase: "failed",
                args: None,
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::ExternalWorkerReported(e) => {
            let message = e
                .report
                .progress
                .as_ref()
                .and_then(|p| p.message.clone())
                .unwrap_or_default();
            let payload = AgentProgressPayload {
                agent_id: e.report.worker_id.to_string(),
                message,
            };
            serde_json::to_value(&payload).ok()
        }
        RuntimeEvent::SubagentSpawned(e) => {
            let payload = AgentProgressPayload {
                agent_id: e.child_task_id.to_string(),
                message: "subagent spawned".to_owned(),
            };
            serde_json::to_value(&payload).ok()
        }
        _ => None,
    }
}

/// Maps a `RuntimeEvent` to the preserved SSE payload, preferring store-backed
/// current-state records when they are available for task and approval events.
///
/// This keeps the runtime-event fallback explicit while giving the API surface
/// a single helper that can promote task_update / approval_required to the
/// richer read-model-backed path when the caller has already joined current
/// state.
pub fn shape_event_payload_with_records(
    event: &RuntimeEvent,
    task_record: Option<&cairn_store::projections::TaskRecord>,
    approval_record: Option<&cairn_store::projections::ApprovalRecord>,
) -> Option<serde_json::Value> {
    match event {
        RuntimeEvent::TaskCreated(_)
        | RuntimeEvent::TaskStateChanged(_)
        | RuntimeEvent::TaskLeaseClaimed(_)
        | RuntimeEvent::TaskLeaseHeartbeated(_) => task_record
            .and_then(|record| serde_json::to_value(&task_update_payload_from_record(record)).ok())
            .or_else(|| shape_event_payload(event)),
        RuntimeEvent::ApprovalRequested(_) => approval_record
            .and_then(|record| serde_json::to_value(&approval_required_payload_from_record(record)).ok())
            .or_else(|| shape_event_payload(event)),
        _ => shape_event_payload(event),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::events::{
        ApprovalRequested, RuntimeEvent, TaskCreated, TaskStateChanged, ToolInvocationStarted,
    };
    use cairn_domain::ids::{ApprovalId, TaskId, ToolInvocationId};
    use cairn_domain::lifecycle::TaskState;
    use cairn_domain::policy::{ApprovalRequirement, ExecutionClass};
    use cairn_domain::tenancy::ProjectKey;
    use cairn_domain::tool_invocation::ToolInvocationTarget;

    #[test]
    fn task_update_has_task_wrapper() {
        let event = RuntimeEvent::TaskCreated(TaskCreated {
            project: ProjectKey::new("t", "w", "p"),
            task_id: TaskId::new("task_1"),
            parent_run_id: None,
            parent_task_id: None,
        });
        let payload = shape_event_payload(&event).unwrap();
        assert!(payload.get("task").is_some());
        assert_eq!(payload["task"]["id"], "task_1");
        assert_eq!(payload["task"]["status"], "queued");
    }

    #[test]
    fn task_state_changed_matches_fixture_shape() {
        let event = RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: ProjectKey::new("t", "w", "p"),
            task_id: TaskId::new("task_1"),
            transition: cairn_domain::events::StateTransition {
                from: Some(TaskState::Running),
                to: TaskState::Completed,
            },
            failure_class: None,
        });
        let payload = shape_event_payload(&event).unwrap();
        // Fixture uses "status" not "state"
        assert!(payload["task"]["status"]
            .as_str()
            .unwrap()
            .contains("completed"));
    }

    #[test]
    fn approval_required_matches_fixture_shape() {
        let event = RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: ProjectKey::new("t", "w", "p"),
            approval_id: ApprovalId::new("appr_1"),
            run_id: None,
            task_id: Some(TaskId::new("task_1")),
            requirement: ApprovalRequirement::Required,
        });
        let payload = shape_event_payload(&event).unwrap();
        assert!(payload.get("approval").is_some());
        // Fixture uses "id" not "approvalId"
        assert_eq!(payload["approval"]["id"], "appr_1");
        assert_eq!(payload["approval"]["status"], "pending");
    }

    #[test]
    fn task_update_prefers_store_record_when_available() {
        use cairn_store::projections::TaskRecord;

        let event = RuntimeEvent::TaskCreated(TaskCreated {
            project: ProjectKey::new("t", "w", "p"),
            task_id: TaskId::new("task_1"),
            parent_run_id: None,
            parent_task_id: None,
        });
        let record = TaskRecord {
            task_id: TaskId::new("task_1"),
            project: ProjectKey::new("t", "w", "p"),
            parent_run_id: None,
            parent_task_id: None,
            state: TaskState::Running,
            failure_class: None,
            lease_owner: None,
            lease_expires_at: None,
            title: Some("Draft weekly digest".to_owned()),
            description: Some("Collect updates and prepare digest.".to_owned()),
            version: 2,
            created_at: 1000,
            updated_at: 1500,
        };

        let payload = shape_event_payload_with_records(&event, Some(&record), None).unwrap();

        assert_eq!(payload["task"]["id"], "task_1");
        assert_eq!(payload["task"]["status"], "running");
        assert_eq!(payload["task"]["title"], "Draft weekly digest");
        assert_eq!(payload["task"]["description"], "Collect updates and prepare digest.");
        assert_eq!(payload["task"]["createdAt"], "1000");
        assert_eq!(payload["task"]["updatedAt"], "1500");
    }

    #[test]
    fn approval_required_prefers_store_record_when_available() {
        use cairn_store::projections::ApprovalRecord;

        let event = RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: ProjectKey::new("t", "w", "p"),
            approval_id: ApprovalId::new("appr_1"),
            run_id: None,
            task_id: Some(TaskId::new("task_1")),
            requirement: ApprovalRequirement::Required,
        });
        let record = ApprovalRecord {
            approval_id: ApprovalId::new("appr_1"),
            project: ProjectKey::new("t", "w", "p"),
            run_id: None,
            task_id: Some(TaskId::new("task_1")),
            requirement: ApprovalRequirement::Required,
            decision: None,
            title: Some("Approve GitHub write action".to_owned()),
            description: Some("Agent wants to create a PR.".to_owned()),
            version: 1,
            created_at: 2000,
            updated_at: 2000,
        };

        let payload = shape_event_payload_with_records(&event, None, Some(&record)).unwrap();

        assert_eq!(payload["approval"]["id"], "appr_1");
        assert_eq!(payload["approval"]["status"], "pending");
        assert_eq!(
            payload["approval"]["title"],
            "Approve GitHub write action"
        );
        assert_eq!(payload["approval"]["description"], "Agent wants to create a PR.");
        assert_eq!(payload["approval"]["createdAt"], "2000");
    }

    #[test]
    fn tool_call_started_matches_fixture_shape() {
        let event = RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
            project: ProjectKey::new("t", "w", "p"),
            invocation_id: ToolInvocationId::new("inv_1"),
            session_id: None,
            run_id: None,
            task_id: Some(TaskId::new("task_1")),
            target: ToolInvocationTarget::Builtin {
                tool_name: "fs.read".to_owned(),
            },
            execution_class: ExecutionClass::SupervisedProcess,
            requested_at_ms: 100,
            started_at_ms: 101,
        });
        let payload = shape_event_payload(&event).unwrap();
        assert_eq!(payload["taskId"], "task_1");
        assert_eq!(payload["toolName"], "fs.read");
        // Fixture uses "start" not "started"
        assert_eq!(payload["phase"], "start");
    }

    #[test]
    fn session_events_return_none() {
        let event = RuntimeEvent::SessionCreated(cairn_domain::events::SessionCreated {
            project: ProjectKey::new("t", "w", "p"),
            session_id: "sess_1".into(),
        });
        assert!(shape_event_payload(&event).is_none());
    }

    #[test]
    fn feed_update_frame_has_item_wrapper() {
        let item = crate::feed::FeedItem {
            id: "feed_1".to_owned(),
            source: "rss".to_owned(),
            kind: None,
            title: Some("News".to_owned()),
            body: None,
            url: None,
            author: None,
            avatar_url: None,
            repo_full_name: None,
            is_read: false,
            is_archived: false,
            group_key: None,
            created_at: "2026-04-03T09:30:00Z".to_owned(),
        };
        let frame = build_feed_update_frame(item, Some("evt_1".to_owned()));
        assert_eq!(frame.event, crate::sse::SseEventName::FeedUpdate);
        assert!(frame.data.get("item").is_some());
        assert_eq!(frame.data["item"]["id"], "feed_1");
    }

    #[test]
    fn poll_completed_frame_has_source_and_count() {
        let frame = build_poll_completed_frame("rss_feed_1", 5, None);
        assert_eq!(frame.event, crate::sse::SseEventName::PollCompleted);
        assert_eq!(frame.data["source"], "rss_feed_1");
        assert_eq!(frame.data["newCount"], 5);
    }

    #[test]
    fn assistant_delta_matches_fixture_shape() {
        let payload = AssistantDeltaPayload {
            task_id: "task_assistant_001".to_owned(),
            delta_text: "The current deploy is blocked by".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["taskId"], "task_assistant_001");
        assert_eq!(json["deltaText"], "The current deploy is blocked by");
    }

    #[test]
    fn assistant_end_matches_fixture_shape() {
        let payload = AssistantEndPayload {
            task_id: "task_assistant_001".to_owned(),
            message_text: "The deploy is blocked by a pending approval.".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["taskId"], "task_assistant_001");
        assert!(json.get("messageText").is_some());
    }

    #[test]
    fn assistant_reasoning_matches_fixture_shape() {
        let payload = AssistantReasoningPayload {
            task_id: "task_assistant_001".to_owned(),
            round: 1,
            thought: "I should inspect the current approvals.".to_owned(),
        };
        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["taskId"], "task_assistant_001");
        assert_eq!(json["round"], 1);
        assert!(json.get("thought").is_some());
    }

    #[test]
    fn streaming_output_delta_builds_sse_frame() {
        use cairn_agent::streaming::{AssistantDelta, StreamingOutput};
        use cairn_domain::{RunId, SessionId};

        let output = StreamingOutput::AssistantDelta(AssistantDelta {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            content: "Hello".to_owned(),
            index: 0,
        });
        let frame = build_streaming_sse_frame(&output, "task_1", None).unwrap();
        assert_eq!(frame.event, crate::sse::SseEventName::AssistantDelta);
        assert_eq!(frame.data["taskId"], "task_1");
        assert_eq!(frame.data["deltaText"], "Hello");
    }

    #[test]
    fn streaming_output_tool_result_returns_none() {
        use cairn_agent::streaming::{StreamingOutput, ToolResult};
        use cairn_domain::{RunId, SessionId};

        let output = StreamingOutput::ToolResult(ToolResult {
            session_id: SessionId::new("s1"),
            run_id: RunId::new("r1"),
            tool_call_id: "tc_1".to_owned(),
            content: serde_json::json!({}),
            is_error: false,
        });
        assert!(build_streaming_sse_frame(&output, "task_1", None).is_none());
    }

    #[test]
    fn enriched_tool_call_from_lifecycle_output() {
        let lifecycle = cairn_tools::runtime_service::ToolLifecycleOutput::started(
            "git.status",
            Some(serde_json::json!({"path": "/repo"})),
        );
        let frame = build_enriched_tool_call_frame(&lifecycle, Some("task_1"), None);
        assert_eq!(frame.event, crate::sse::SseEventName::AssistantToolCall);
        assert_eq!(frame.data["toolName"], "git.status");
        assert_eq!(frame.data["phase"], "start");
        assert_eq!(frame.data["args"]["path"], "/repo");
        assert_eq!(frame.data["taskId"], "task_1");
    }

    #[test]
    fn enriched_tool_call_completed_with_no_args() {
        let lifecycle = cairn_tools::runtime_service::ToolLifecycleOutput::completed(
            "fs.read",
            Some(serde_json::json!({"text": "file contents"})),
        );
        let frame = build_enriched_tool_call_frame(&lifecycle, None, Some("evt_5".to_owned()));
        assert_eq!(frame.data["phase"], "completed");
        assert_eq!(frame.data["toolName"], "fs.read");
        assert!(frame.data.get("args").is_none());
        assert_eq!(frame.id, Some("evt_5".to_owned()));
    }

    #[test]
    fn enriched_task_update_from_store_record() {
        use cairn_store::projections::TaskRecord;

        let record = TaskRecord {
            task_id: cairn_domain::ids::TaskId::new("task_001"),
            project: ProjectKey::new("t", "w", "p"),
            parent_run_id: None,
            parent_task_id: None,
            state: cairn_domain::lifecycle::TaskState::Running,
            failure_class: None,
            lease_owner: None,
            lease_expires_at: None,
            title: Some("Draft weekly digest".to_owned()),
            description: Some("Collect updates and prepare digest.".to_owned()),
            version: 2,
            created_at: 1000,
            updated_at: 1500,
        };

        let frame = build_enriched_task_update_frame(&record, Some("evt_10".to_owned()));
        assert_eq!(frame.event, crate::sse::SseEventName::TaskUpdate);
        assert_eq!(frame.data["task"]["id"], "task_001");
        assert_eq!(frame.data["task"]["status"], "running");
        assert_eq!(frame.data["task"]["title"], "Draft weekly digest");
        assert_eq!(
            frame.data["task"]["description"],
            "Collect updates and prepare digest."
        );
        assert!(frame.data["task"]["createdAt"].as_str().is_some());
    }

    #[test]
    fn enriched_approval_from_store_record() {
        use cairn_store::projections::ApprovalRecord;

        let record = ApprovalRecord {
            approval_id: cairn_domain::ids::ApprovalId::new("appr_001"),
            project: ProjectKey::new("t", "w", "p"),
            run_id: None,
            task_id: Some(cairn_domain::ids::TaskId::new("task_001")),
            requirement: cairn_domain::policy::ApprovalRequirement::Required,
            decision: None,
            title: Some("Approve GitHub write action".to_owned()),
            description: Some("Agent wants to create a PR.".to_owned()),
            version: 1,
            created_at: 2000,
            updated_at: 2000,
        };

        let frame = build_enriched_approval_frame(&record, None);
        assert_eq!(frame.event, crate::sse::SseEventName::ApprovalRequired);
        assert_eq!(frame.data["approval"]["id"], "appr_001");
        assert_eq!(frame.data["approval"]["status"], "pending");
        assert_eq!(
            frame.data["approval"]["title"],
            "Approve GitHub write action"
        );
        assert_eq!(
            frame.data["approval"]["description"],
            "Agent wants to create a PR."
        );
    }

    #[test]
    fn enriched_assistant_end_with_assembled_text() {
        let frame = build_enriched_assistant_end_frame(
            "task_assistant_001",
            "The deploy is blocked by a pending approval from ops.",
            Some("evt_20".to_owned()),
        );
        assert_eq!(frame.event, crate::sse::SseEventName::AssistantEnd);
        assert_eq!(frame.data["taskId"], "task_assistant_001");
        assert_eq!(
            frame.data["messageText"],
            "The deploy is blocked by a pending approval from ops."
        );
        assert_eq!(frame.id, Some("evt_20".to_owned()));
    }
}
