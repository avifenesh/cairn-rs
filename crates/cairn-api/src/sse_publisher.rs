use async_trait::async_trait;
use cairn_domain::events::RuntimeEvent;
use cairn_domain::{ApprovalId, TaskId};
use cairn_store::error::StoreError;
use cairn_store::event_log::{EventPosition, StoredEvent};
use cairn_store::projections::{ApprovalReadModel, ApprovalRecord, TaskReadModel, TaskRecord};
use cairn_store::EventLog;
use std::sync::Arc;

use crate::sse::{SseEventName, SseFrame};

/// Maps a `RuntimeEvent` to an `SseEventName` for preserved compatibility.
///
/// Returns `None` for events that do not have a corresponding SSE surface.
pub fn map_event_to_sse_name(event: &RuntimeEvent) -> Option<SseEventName> {
    match event {
        RuntimeEvent::SessionCreated(_) | RuntimeEvent::SessionStateChanged(_) => None,
        RuntimeEvent::RunCreated(_) => Some(SseEventName::AgentProgress),
        RuntimeEvent::RunStateChanged(_) => None,
        RuntimeEvent::TaskCreated(_)
        | RuntimeEvent::TaskStateChanged(_)
        | RuntimeEvent::TaskDependencyAdded(_)
        | RuntimeEvent::TaskDependencyResolved(_)
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
        | RuntimeEvent::PromptReleaseTransitioned(_)
        | RuntimeEvent::TenantCreated(_)
        | RuntimeEvent::WorkspaceCreated(_)
        | RuntimeEvent::ProjectCreated(_)
        | RuntimeEvent::RouteDecisionMade(_)
        | RuntimeEvent::ProviderCallCompleted(_) => None,
        _ => None,
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

fn task_id_for_event(event: &RuntimeEvent) -> Option<TaskId> {
    match event {
        RuntimeEvent::TaskCreated(event) => Some(event.task_id.clone()),
        RuntimeEvent::TaskStateChanged(event) => Some(event.task_id.clone()),
        RuntimeEvent::TaskDependencyAdded(event) => {
            Some(if event.dependent_task_id.as_str().is_empty() {
                event.task_id.clone()
            } else {
                event.dependent_task_id.clone()
            })
        }
        RuntimeEvent::TaskDependencyResolved(event) => {
            Some(if event.dependent_task_id.as_str().is_empty() {
                event.task_id.clone()
            } else {
                event.dependent_task_id.clone()
            })
        }
        RuntimeEvent::TaskLeaseClaimed(event) => Some(event.task_id.clone()),
        RuntimeEvent::TaskLeaseHeartbeated(event) => Some(event.task_id.clone()),
        _ => None,
    }
}

fn approval_id_for_event(event: &RuntimeEvent) -> Option<ApprovalId> {
    match event {
        RuntimeEvent::ApprovalRequested(event) => Some(event.approval_id.clone()),
        RuntimeEvent::ApprovalResolved(event) => Some(event.approval_id.clone()),
        _ => None,
    }
}

/// Builds an SSE frame by hydrating task/approval current-state records from the store
/// before falling back to the thinner raw runtime-event payload path.
pub async fn build_sse_frame_with_store_state<S>(
    store: &S,
    stored: &StoredEvent,
) -> Result<Option<SseFrame>, StoreError>
where
    S: TaskReadModel + ApprovalReadModel + Send + Sync,
{
    let task_record = match task_id_for_event(&stored.envelope.payload) {
        Some(task_id) => match TaskReadModel::get(store, &task_id).await {
            Ok(record) => record,
            Err(_) => return Ok(build_sse_frame(stored)),
        },
        None => None,
    };

    let approval_record = match approval_id_for_event(&stored.envelope.payload) {
        Some(approval_id) => match ApprovalReadModel::get(store, &approval_id).await {
            Ok(record) => record,
            Err(_) => return Ok(build_sse_frame(stored)),
        },
        None => None,
    };

    Ok(build_sse_frame_with_current_state(
        stored,
        task_record.as_ref(),
        approval_record.as_ref(),
    ))
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

/// Store-backed SSE replay publisher that enriches task and approval events from current-state
/// read models before serializing them to preserved SSE frames.
pub struct ReadModelBackedSsePublisher<S> {
    store: Arc<S>,
}

impl<S> ReadModelBackedSsePublisher<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> SsePublisher for ReadModelBackedSsePublisher<S>
where
    S: EventLog + TaskReadModel + ApprovalReadModel + Send + Sync,
{
    async fn replay(&self, query: &SseReplayQuery) -> Result<Vec<SseFrame>, StoreError> {
        let events = self
            .store
            .read_stream(query.after_position, query.limit)
            .await?;
        let mut frames = Vec::with_capacity(events.len());
        for stored in events {
            if let Some(frame) =
                build_sse_frame_with_store_state(self.store.as_ref(), &stored).await?
            {
                frames.push(frame);
            }
        }
        Ok(frames)
    }

    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
        self.store.head_position().await
    }
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
    use async_trait::async_trait;
    use cairn_domain::events::StateTransition;
    use cairn_domain::events::{
        ApprovalRequested, EventEnvelope, EventSource, RuntimeEvent, TaskCreated, TaskStateChanged,
    };
    use cairn_domain::ids::{ApprovalId, EventId, TaskId};
    use cairn_domain::lifecycle::TaskState;
    use cairn_domain::policy::ApprovalRequirement;
    use cairn_domain::tenancy::ProjectKey;
    use cairn_store::event_log::{EventPosition, StoredEvent};
    use cairn_store::InMemoryStore;

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
            title: None,
            description: None,
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
        assert_eq!(
            frame.data["task"]["description"],
            "Collect updates and prepare digest."
        );
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
            title: None,
            description: None,
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
        assert_eq!(
            frame.data["approval"]["title"],
            "Approve GitHub write action"
        );
        assert_eq!(
            frame.data["approval"]["description"],
            "Agent wants to create a PR."
        );
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

    #[tokio::test]
    async fn build_sse_frame_with_store_state_uses_task_projection() {
        let store = InMemoryStore::new();
        let project = ProjectKey::new("t", "w", "p");
        let envelope = EventEnvelope::for_runtime_event(
            EventId::new("evt_task_store"),
            EventSource::Runtime,
            RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new("task_store"),
                parent_run_id: None,
                parent_task_id: None,
                prompt_release_id: None,
            }),
        );
        let positions = store.append(&[envelope]).await.unwrap();
        let stored = store
            .read_stream(Some(EventPosition(positions[0].0.saturating_sub(1))), 1)
            .await
            .unwrap()
            .pop()
            .expect("stored event");

        let frame = build_sse_frame_with_store_state(&store, &stored)
            .await
            .unwrap()
            .expect("task frame");
        assert_eq!(frame.event, SseEventName::TaskUpdate);
        assert!(frame.data["task"]["createdAt"].as_str().is_some());
    }

    #[tokio::test]
    async fn build_sse_frame_with_store_state_enriches_task_dependency_events() {
        let store = InMemoryStore::new();
        let project = ProjectKey::new("t", "w", "p");
        let created = EventEnvelope::for_runtime_event(
            EventId::new("evt_task_dependency_created"),
            EventSource::Runtime,
            RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new("task_dependency"),
                parent_run_id: None,
                parent_task_id: None,
                prompt_release_id: None,
            }),
        );
        let dependency = EventEnvelope::for_runtime_event(
            EventId::new("evt_task_dependency_added"),
            EventSource::Runtime,
            RuntimeEvent::TaskDependencyAdded(cairn_domain::events::TaskDependencyAdded {
                task_id: TaskId::new("task_dependency"),
                depends_on: TaskId::new("task_upstream"),
                added_at_ms: 123,
                dependent_task_id: TaskId::new("task_dependency"),
                depends_on_task_id: TaskId::new("task_upstream"),
            }),
        );
        store.append(&[created, dependency]).await.unwrap();
        let stored = store
            .read_stream(Some(EventPosition(0)), 16)
            .await
            .unwrap()
            .into_iter()
            .find(|event| event.envelope.event_id.as_str() == "evt_task_dependency_added")
            .expect("dependency event");

        let frame = build_sse_frame_with_store_state(&store, &stored)
            .await
            .unwrap()
            .expect("task dependency frame");
        assert_eq!(frame.event, SseEventName::TaskUpdate);
        assert_eq!(frame.data["task"]["id"], "task_dependency");
        assert!(frame.data["task"]["createdAt"].as_str().is_some());
    }

    #[tokio::test]
    async fn task_dependency_events_fall_back_to_task_id_when_aliases_are_missing() {
        let store = InMemoryStore::new();
        let project = ProjectKey::new("t", "w", "p");
        let created = EventEnvelope::for_runtime_event(
            EventId::new("evt_task_dependency_created_legacy"),
            EventSource::Runtime,
            RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: TaskId::new("task_dependency_legacy"),
                parent_run_id: None,
                parent_task_id: None,
                prompt_release_id: None,
            }),
        );
        let dependency = EventEnvelope::for_runtime_event(
            EventId::new("evt_task_dependency_added_legacy"),
            EventSource::Runtime,
            RuntimeEvent::TaskDependencyAdded(cairn_domain::events::TaskDependencyAdded {
                task_id: TaskId::new("task_dependency_legacy"),
                depends_on: TaskId::new("task_upstream"),
                added_at_ms: 456,
                dependent_task_id: TaskId::default(),
                depends_on_task_id: TaskId::default(),
            }),
        );
        store.append(&[created, dependency]).await.unwrap();
        let stored = store
            .read_stream(Some(EventPosition(0)), 16)
            .await
            .unwrap()
            .into_iter()
            .find(|event| event.envelope.event_id.as_str() == "evt_task_dependency_added_legacy")
            .expect("dependency event");

        let frame = build_sse_frame_with_store_state(&store, &stored)
            .await
            .unwrap()
            .expect("task dependency frame");
        assert_eq!(frame.event, SseEventName::TaskUpdate);
        assert_eq!(frame.data["task"]["id"], "task_dependency_legacy");
        assert!(frame.data["task"]["createdAt"].as_str().is_some());
    }

    struct FailingProjectionStore;

    #[async_trait]
    impl TaskReadModel for FailingProjectionStore {
        async fn get(&self, _task_id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
            Err(StoreError::Internal(
                "task projection unavailable".to_owned(),
            ))
        }

        async fn list_by_state(
            &self,
            _project: &ProjectKey,
            _state: TaskState,
            _limit: usize,
        ) -> Result<Vec<TaskRecord>, StoreError> {
            Ok(Vec::new())
        }

        async fn list_expired_leases(
            &self,
            _now: u64,
            _limit: usize,
        ) -> Result<Vec<TaskRecord>, StoreError> {
            Ok(Vec::new())
        }

        async fn list_by_parent_run(
            &self,
            _parent_run_id: &cairn_domain::RunId,
            _limit: usize,
        ) -> Result<Vec<TaskRecord>, StoreError> {
            Ok(Vec::new())
        }

        async fn any_non_terminal_children(
            &self,
            _parent_run_id: &cairn_domain::RunId,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
    }

    #[async_trait]
    impl ApprovalReadModel for FailingProjectionStore {
        async fn get(
            &self,
            _approval_id: &ApprovalId,
        ) -> Result<Option<ApprovalRecord>, StoreError> {
            Err(StoreError::Internal(
                "approval projection unavailable".to_owned(),
            ))
        }

        async fn list_pending(
            &self,
            _project: &ProjectKey,
            _limit: usize,
            _offset: usize,
        ) -> Result<Vec<ApprovalRecord>, StoreError> {
            Ok(Vec::new())
        }

        async fn list_all(
            &self,
            _project: &ProjectKey,
            _limit: usize,
            _offset: usize,
        ) -> Result<Vec<ApprovalRecord>, StoreError> {
            Ok(Vec::new())
        }

        async fn has_pending_for_run(
            &self,
            _run_id: &cairn_domain::RunId,
        ) -> Result<bool, StoreError> {
            Ok(false)
        }
    }

    #[tokio::test]
    async fn build_sse_frame_with_store_state_falls_back_when_projection_lookup_fails() {
        let stored = test_stored_event(
            RuntimeEvent::TaskCreated(TaskCreated {
                project: ProjectKey::new("t", "w", "p"),
                task_id: TaskId::new("task_projection_error"),
                parent_run_id: None,
                parent_task_id: None,
                prompt_release_id: None,
            }),
            9,
        );

        let frame = build_sse_frame_with_store_state(&FailingProjectionStore, &stored)
            .await
            .unwrap()
            .expect("fallback frame");
        assert_eq!(frame.event, SseEventName::TaskUpdate);
        assert_eq!(frame.data["task"]["id"], "task_projection_error");
        assert!(frame.data["task"]["createdAt"].is_null());
    }

    #[tokio::test]
    async fn read_model_backed_sse_publisher_replays_enriched_approval_frames() {
        let store = Arc::new(InMemoryStore::new());
        let project = ProjectKey::new("t", "w", "p");
        let envelope = EventEnvelope::for_runtime_event(
            EventId::new("evt_approval_store"),
            EventSource::Runtime,
            RuntimeEvent::ApprovalRequested(ApprovalRequested {
                project,
                approval_id: ApprovalId::new("appr_store"),
                run_id: None,
                task_id: Some(TaskId::new("task_store")),
                requirement: ApprovalRequirement::Required,
                title: None,
                description: None,
            }),
        );
        store.append(&[envelope]).await.unwrap();

        let publisher = ReadModelBackedSsePublisher::new(store);
        let frames = publisher.replay(&SseReplayQuery::default()).await.unwrap();
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].event, SseEventName::ApprovalRequired);
        assert!(frames[0].data["approval"]["createdAt"].as_str().is_some());
    }
}
