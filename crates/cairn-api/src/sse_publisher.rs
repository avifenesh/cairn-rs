use async_trait::async_trait;
use cairn_domain::events::RuntimeEvent;
use cairn_store::error::StoreError;
use cairn_store::event_log::{EventPosition, StoredEvent};
use cairn_store::projections::{ApprovalRecord, TaskRecord};

use crate::sse::{SseEventName, SseFrame};

/// Maps a `RuntimeEvent` to an `SseEventName` for preserved compatibility.
///
/// Returns `None` for events that do not have a corresponding SSE surface.
pub fn map_event_to_sse_name(event: &RuntimeEvent) -> Option<SseEventName> {
    match event {
        RuntimeEvent::SessionCreated(_) | RuntimeEvent::SessionStateChanged(_) => None,
        RuntimeEvent::RunCreated(_) | RuntimeEvent::RunStateChanged(_) => None,
        RuntimeEvent::TaskCreated(_)
        | RuntimeEvent::TaskStateChanged(_)
        | RuntimeEvent::TaskLeaseClaimed(_)
        | RuntimeEvent::TaskLeaseHeartbeated(_) => Some(SseEventName::TaskUpdate),
        RuntimeEvent::ApprovalRequested(_) => Some(SseEventName::ApprovalRequired),
        RuntimeEvent::ApprovalResolved(_) => None,
        RuntimeEvent::CheckpointRecorded(_) | RuntimeEvent::CheckpointRestored(_) => None,
        RuntimeEvent::MailboxMessageAppended(_) => None,
        RuntimeEvent::ToolInvocationStarted(_)
        | RuntimeEvent::ToolInvocationCompleted(_)
        | RuntimeEvent::ToolInvocationFailed(_) => Some(SseEventName::AssistantToolCall),
        RuntimeEvent::ExternalWorkerReported(_) => Some(SseEventName::AgentProgress),
        RuntimeEvent::SubagentSpawned(_) => Some(SseEventName::AgentProgress),
        RuntimeEvent::RecoveryAttempted(_)
        | RuntimeEvent::RecoveryCompleted(_)
        | RuntimeEvent::SignalIngested(_)
        | RuntimeEvent::UserMessageAppended(_)
        | RuntimeEvent::IngestJobStarted(_)
        | RuntimeEvent::IngestJobCompleted(_)
        | RuntimeEvent::EvalRunStarted(_)
        | RuntimeEvent::EvalRunCompleted(_)
        | RuntimeEvent::PromptAssetCreated(_)
        | RuntimeEvent::PromptVersionCreated(_)
        | RuntimeEvent::PromptReleaseCreated(_)
        | RuntimeEvent::PromptReleaseTransitioned(_) => None,
    }
}

/// Builds an `SseFrame` from a stored event, using the preserved SSE event name
/// and frontend-compatible payload shapes (not raw RuntimeEvent serialization).
///
/// Returns `None` if the event doesn't map to an SSE surface.
pub fn build_sse_frame(stored: &StoredEvent) -> Option<SseFrame> {
    build_sse_frame_with_current_state(stored, None, None)
}

/// Builds an `SseFrame` from a stored event and optional current-state records.
///
/// When the caller already has task or approval read-model rows, this helper
/// can promote `task_update` and `approval_required` to the richer
/// store-backed path. Otherwise it preserves the current thin runtime-event
/// fallback.
pub fn build_sse_frame_with_current_state(
    stored: &StoredEvent,
    task_record: Option<&TaskRecord>,
    approval_record: Option<&ApprovalRecord>,
) -> Option<SseFrame> {
    let name = map_event_to_sse_name(&stored.envelope.payload)?;
    let data = crate::sse_payloads::shape_event_payload_with_records(
        &stored.envelope.payload,
        task_record,
        approval_record,
    )?;
    Some(SseFrame {
        event: name,
        data,
        id: Some(stored.position.0.to_string()),
    })
}

/// Replay query for SSE reconnection via `lastEventId`.
#[derive(Clone, Debug)]
pub struct SseReplayQuery {
    pub after_position: Option<EventPosition>,
    pub limit: usize,
}

impl Default for SseReplayQuery {
    fn default() -> Self {
        Self {
            after_position: None,
            limit: 100,
        }
    }
}

/// Parses a `lastEventId` string from the SSE reconnection header
/// into an `EventPosition`.
pub fn parse_last_event_id(last_event_id: &str) -> Option<EventPosition> {
    last_event_id.parse::<u64>().ok().map(EventPosition)
}

/// Service trait for SSE stream publishing.
///
/// Implementors read from the event log and push SSE frames to connected
/// clients. Supports replay from a given position per the preserved
/// `/v1/stream?lastEventId=<id>` contract.
#[async_trait]
pub trait SsePublisher: Send + Sync {
    /// Replay events since a given position, returning SSE frames.
    async fn replay(&self, query: &SseReplayQuery) -> Result<Vec<SseFrame>, StoreError>;

    /// Get the current head position for initial `ready` event.
    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError>;
}

/// Builds the initial `ready` SSE frame with the current head position.
pub fn build_ready_frame(client_id: &str) -> SseFrame {
    SseFrame {
        event: SseEventName::Ready,
        data: serde_json::json!({"clientId": client_id}),
        id: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::events::StateTransition;
    use cairn_domain::events::{
        ApprovalRequested, EventEnvelope, EventSource, RuntimeEvent, TaskCreated, TaskStateChanged,
    };
    use cairn_domain::ids::{ApprovalId, EventId, TaskId};
    use cairn_domain::lifecycle::TaskState;
    use cairn_domain::policy::ApprovalRequirement;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_store::event_log::{EventPosition, StoredEvent};

    fn test_stored_event(payload: RuntimeEvent, position: u64) -> StoredEvent {
        StoredEvent {
            position: EventPosition(position),
            envelope: EventEnvelope::for_runtime_event(
                EventId::new(format!("evt_{position}")),
                EventSource::Runtime,
                payload,
            ),
            stored_at: 1000 + position,
        }
    }

    #[test]
    fn task_update_maps_to_sse() {
        let event = RuntimeEvent::TaskStateChanged(TaskStateChanged {
            project: ProjectKey::new("t", "w", "p"),
            task_id: TaskId::new("task_1"),
            transition: StateTransition {
                from: Some(TaskState::Running),
                to: TaskState::Completed,
            },
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
        });
        assert_eq!(
            map_event_to_sse_name(&event),
            Some(SseEventName::TaskUpdate)
        );
    }

    #[test]
    fn approval_requested_maps_to_sse() {
        let event = RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: ProjectKey::new("t", "w", "p"),
            approval_id: ApprovalId::new("appr_1"),
            run_id: None,
            task_id: None,
            requirement: ApprovalRequirement::Required,
        });
        assert_eq!(
            map_event_to_sse_name(&event),
            Some(SseEventName::ApprovalRequired)
        );
    }

    #[test]
    fn session_events_have_no_sse_surface() {
        let event = RuntimeEvent::SessionCreated(cairn_domain::events::SessionCreated {
            project: ProjectKey::new("t", "w", "p"),
            session_id: "sess_1".into(),
        });
        assert_eq!(map_event_to_sse_name(&event), None);
    }

    #[test]
    fn build_sse_frame_from_stored_event() {
        let event = RuntimeEvent::TaskCreated(TaskCreated {
            project: ProjectKey::new("t", "w", "p"),
            task_id: TaskId::new("task_1"),
            parent_run_id: None,
            parent_task_id: None,
            prompt_release_id: None,
        });
        let stored = test_stored_event(event, 42);
        let frame = build_sse_frame(&stored).unwrap();

        assert_eq!(frame.event, SseEventName::TaskUpdate);
        assert_eq!(frame.id, Some("42".to_owned()));
    }

    #[test]
    fn build_sse_frame_with_current_state_uses_task_record() {
        use cairn_store::projections::TaskRecord;

        let event = RuntimeEvent::TaskCreated(TaskCreated {
            project: ProjectKey::new("t", "w", "p"),
            task_id: TaskId::new("task_1"),
            parent_run_id: None,
            parent_task_id: None,
            prompt_release_id: None,
        });
        let stored = test_stored_event(event, 7);
        let record = TaskRecord {
            task_id: TaskId::new("task_1"),
            project: ProjectKey::new("t", "w", "p"),
            parent_run_id: None,
            parent_task_id: None,
            state: TaskState::Running,
            prompt_release_id: None,
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
            retry_count: 0,
            lease_owner: None,
            lease_expires_at: None,
            title: Some("Draft weekly digest".to_owned()),
            description: Some("Collect updates and prepare digest.".to_owned()),
            version: 2,
            created_at: 1000,
            updated_at: 1500,
        };

        let frame = build_sse_frame_with_current_state(&stored, Some(&record), None).unwrap();
        assert_eq!(frame.event, SseEventName::TaskUpdate);
        assert_eq!(frame.data["task"]["title"], "Draft weekly digest");
        assert_eq!(frame.data["task"]["description"], "Collect updates and prepare digest.");
        assert_eq!(frame.data["task"]["createdAt"], "1000");
    }

    #[test]
    fn build_sse_frame_with_current_state_uses_approval_record() {
        use cairn_store::projections::ApprovalRecord;

        let event = RuntimeEvent::ApprovalRequested(ApprovalRequested {
            project: ProjectKey::new("t", "w", "p"),
            approval_id: ApprovalId::new("appr_1"),
            run_id: None,
            task_id: Some(TaskId::new("task_1")),
            requirement: ApprovalRequirement::Required,
        });
        let stored = test_stored_event(event, 8);
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

        let frame = build_sse_frame_with_current_state(&stored, None, Some(&record)).unwrap();
        assert_eq!(frame.event, SseEventName::ApprovalRequired);
        assert_eq!(frame.data["approval"]["title"], "Approve GitHub write action");
        assert_eq!(frame.data["approval"]["description"], "Agent wants to create a PR.");
        assert_eq!(frame.data["approval"]["createdAt"], "2000");
    }

    #[test]
    fn parse_last_event_id_valid() {
        assert_eq!(parse_last_event_id("42"), Some(EventPosition(42)));
        assert_eq!(parse_last_event_id("0"), Some(EventPosition(0)));
    }

    #[test]
    fn parse_last_event_id_invalid() {
        assert_eq!(parse_last_event_id("abc"), None);
        assert_eq!(parse_last_event_id(""), None);
    }

    #[test]
    fn ready_frame_construction() {
        let frame = build_ready_frame("client_abc");
        assert_eq!(frame.event, SseEventName::Ready);
        assert_eq!(frame.data["clientId"], "client_abc");
        assert!(frame.id.is_none());
    }
}
