//! In-memory store implementation for testing and local-mode use.
//!
//! Provides a single `InMemoryStore` that implements `EventLog` and all
//! entity read-model traits. Event append atomically updates sync projections.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::*;

use crate::error::StoreError;
use crate::event_log::*;
use crate::projections::*;

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

struct State {
    events: Vec<StoredEvent>,
    next_position: u64,
    sessions: HashMap<String, SessionRecord>,
    runs: HashMap<String, RunRecord>,
    tasks: HashMap<String, TaskRecord>,
    approvals: HashMap<String, ApprovalRecord>,
    checkpoints: HashMap<String, CheckpointRecord>,
    mailbox_messages: HashMap<String, MailboxRecord>,
    tool_invocations: HashMap<String, ToolInvocationRecord>,
    signals: HashMap<String, cairn_domain::SignalRecord>,
}

pub struct InMemoryStore {
    state: Mutex<State>,
}

impl InMemoryStore {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State {
                events: Vec::new(),
                next_position: 1,
                sessions: HashMap::new(),
                runs: HashMap::new(),
                tasks: HashMap::new(),
                approvals: HashMap::new(),
                checkpoints: HashMap::new(),
                mailbox_messages: HashMap::new(),
                tool_invocations: HashMap::new(),
                signals: HashMap::new(),
            }),
        }
    }

    fn apply_projection(state: &mut State, event: &StoredEvent) {
        let now = event.stored_at;
        match &event.envelope.payload {
            RuntimeEvent::SessionCreated(e) => {
                state.sessions.insert(
                    e.session_id.as_str().to_owned(),
                    SessionRecord {
                        session_id: e.session_id.clone(),
                        project: e.project.clone(),
                        state: SessionState::Open,
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::SessionStateChanged(e) => {
                if let Some(rec) = state.sessions.get_mut(e.session_id.as_str()) {
                    rec.state = e.transition.to;
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::RunCreated(e) => {
                state.runs.insert(
                    e.run_id.as_str().to_owned(),
                    RunRecord {
                        run_id: e.run_id.clone(),
                        session_id: e.session_id.clone(),
                        parent_run_id: e.parent_run_id.clone(),
                        project: e.project.clone(),
                        state: RunState::Pending,
                        failure_class: None,
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::RunStateChanged(e) => {
                if let Some(rec) = state.runs.get_mut(e.run_id.as_str()) {
                    rec.state = e.transition.to;
                    rec.failure_class = e.failure_class;
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::TaskCreated(e) => {
                state.tasks.insert(
                    e.task_id.as_str().to_owned(),
                    TaskRecord {
                        task_id: e.task_id.clone(),
                        project: e.project.clone(),
                        parent_run_id: e.parent_run_id.clone(),
                        parent_task_id: e.parent_task_id.clone(),
                        state: TaskState::Queued,
                        failure_class: None,
                        lease_owner: None,
                        lease_expires_at: None,
                        title: None,
                        description: None,
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::TaskStateChanged(e) => {
                if let Some(rec) = state.tasks.get_mut(e.task_id.as_str()) {
                    rec.state = e.transition.to;
                    rec.failure_class = e.failure_class;
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::ApprovalRequested(e) => {
                state.approvals.insert(
                    e.approval_id.as_str().to_owned(),
                    ApprovalRecord {
                        approval_id: e.approval_id.clone(),
                        project: e.project.clone(),
                        run_id: e.run_id.clone(),
                        task_id: e.task_id.clone(),
                        requirement: e.requirement,
                        decision: None,
                        title: None,
                        description: None,
                        version: 1,
                        created_at: now,
                        updated_at: now,
                    },
                );
            }
            RuntimeEvent::ApprovalResolved(e) => {
                if let Some(rec) = state.approvals.get_mut(e.approval_id.as_str()) {
                    rec.decision = Some(e.decision);
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::CheckpointRecorded(e) => {
                // Supersede any existing latest checkpoint for this run.
                if e.disposition == CheckpointDisposition::Latest {
                    for cp in state.checkpoints.values_mut() {
                        if cp.run_id == e.run_id && cp.disposition == CheckpointDisposition::Latest
                        {
                            cp.disposition = CheckpointDisposition::Superseded;
                            cp.version += 1;
                        }
                    }
                }
                state.checkpoints.insert(
                    e.checkpoint_id.as_str().to_owned(),
                    CheckpointRecord {
                        checkpoint_id: e.checkpoint_id.clone(),
                        project: e.project.clone(),
                        run_id: e.run_id.clone(),
                        disposition: e.disposition,
                        version: 1,
                        created_at: now,
                    },
                );
            }
            RuntimeEvent::MailboxMessageAppended(e) => {
                state.mailbox_messages.insert(
                    e.message_id.as_str().to_owned(),
                    MailboxRecord {
                        message_id: e.message_id.clone(),
                        project: e.project.clone(),
                        run_id: e.run_id.clone(),
                        task_id: e.task_id.clone(),
                        version: 1,
                        created_at: now,
                    },
                );
            }
            RuntimeEvent::TaskLeaseClaimed(e) => {
                if let Some(rec) = state.tasks.get_mut(e.task_id.as_str()) {
                    rec.lease_owner = Some(e.lease_owner.clone());
                    rec.lease_expires_at = Some(e.lease_expires_at_ms);
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::TaskLeaseHeartbeated(e) => {
                if let Some(rec) = state.tasks.get_mut(e.task_id.as_str()) {
                    rec.lease_expires_at = Some(e.lease_expires_at_ms);
                    rec.version += 1;
                    rec.updated_at = now;
                }
            }
            RuntimeEvent::ToolInvocationStarted(e) => {
                let requested = ToolInvocationRecord::new_requested(
                    e.invocation_id.clone(),
                    e.project.clone(),
                    e.session_id.clone(),
                    e.run_id.clone(),
                    e.task_id.clone(),
                    e.target.clone(),
                    e.execution_class,
                    e.requested_at_ms,
                );
                let started = requested
                    .mark_started(e.started_at_ms)
                    .expect("tool invocation started event should always be a valid requested->started transition");
                state
                    .tool_invocations
                    .insert(e.invocation_id.as_str().to_owned(), started);
            }
            RuntimeEvent::ToolInvocationCompleted(e) => {
                if let Some(rec) = state.tool_invocations.get_mut(e.invocation_id.as_str()) {
                    *rec = rec.mark_finished(e.outcome, None, e.finished_at_ms).expect(
                        "tool invocation completed event should preserve valid terminal transition",
                    );
                }
            }
            RuntimeEvent::ToolInvocationFailed(e) => {
                if let Some(rec) = state.tool_invocations.get_mut(e.invocation_id.as_str()) {
                    *rec = rec
                        .mark_finished(e.outcome, e.error_message.clone(), e.finished_at_ms)
                        .expect("tool invocation failed event should preserve valid terminal transition");
                }
            }
            RuntimeEvent::SignalIngested(e) => {
                state.signals.insert(
                    e.signal_id.as_str().to_owned(),
                    cairn_domain::SignalRecord {
                        id: e.signal_id.clone(),
                        project: e.project.clone(),
                        source: e.source.clone(),
                        payload: e.payload.clone(),
                        timestamp_ms: e.timestamp_ms,
                    },
                );
            }
            // Audit/linkage events that don't update core projections.
            RuntimeEvent::CheckpointRestored(_)
            | RuntimeEvent::ExternalWorkerReported(_)
            | RuntimeEvent::SubagentSpawned(_)
            | RuntimeEvent::RecoveryAttempted(_)
            | RuntimeEvent::RecoveryCompleted(_) => {}
        }
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

// -- EventLog --

#[async_trait]
impl EventLog for InMemoryStore {
    async fn append(
        &self,
        events: &[EventEnvelope<RuntimeEvent>],
    ) -> Result<Vec<EventPosition>, StoreError> {
        let mut state = self.state.lock().unwrap();
        let now = now_millis();
        let mut positions = Vec::with_capacity(events.len());

        for envelope in events {
            let pos = EventPosition(state.next_position);
            state.next_position += 1;

            let stored = StoredEvent {
                position: pos,
                envelope: envelope.clone(),
                stored_at: now,
            };

            Self::apply_projection(&mut state, &stored);
            state.events.push(stored);
            positions.push(pos);
        }

        Ok(positions)
    }

    async fn read_by_entity(
        &self,
        entity: &EntityRef,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let state = self.state.lock().unwrap();
        let min_pos = after.map(|p| p.0).unwrap_or(0);

        let results: Vec<StoredEvent> = state
            .events
            .iter()
            .filter(|e| e.position.0 > min_pos)
            .filter(|e| event_matches_entity(&e.envelope.payload, entity))
            .take(limit)
            .cloned()
            .collect();

        Ok(results)
    }

    async fn read_stream(
        &self,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let state = self.state.lock().unwrap();
        let min_pos = after.map(|p| p.0).unwrap_or(0);

        let results: Vec<StoredEvent> = state
            .events
            .iter()
            .filter(|e| e.position.0 > min_pos)
            .take(limit)
            .cloned()
            .collect();

        Ok(results)
    }

    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.events.last().map(|e| e.position))
    }
}

fn event_matches_entity(event: &RuntimeEvent, entity: &EntityRef) -> bool {
    match (event, entity) {
        (RuntimeEvent::SessionCreated(e), EntityRef::Session(id)) => e.session_id == *id,
        (RuntimeEvent::SessionStateChanged(e), EntityRef::Session(id)) => e.session_id == *id,
        (RuntimeEvent::RunCreated(e), EntityRef::Run(id)) => e.run_id == *id,
        (RuntimeEvent::RunStateChanged(e), EntityRef::Run(id)) => e.run_id == *id,
        (RuntimeEvent::TaskCreated(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::TaskLeaseClaimed(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::TaskLeaseHeartbeated(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::TaskStateChanged(e), EntityRef::Task(id)) => e.task_id == *id,
        (RuntimeEvent::ApprovalRequested(e), EntityRef::Approval(id)) => e.approval_id == *id,
        (RuntimeEvent::ApprovalResolved(e), EntityRef::Approval(id)) => e.approval_id == *id,
        (RuntimeEvent::CheckpointRecorded(e), EntityRef::Checkpoint(id)) => e.checkpoint_id == *id,
        (RuntimeEvent::CheckpointRestored(e), EntityRef::Checkpoint(id)) => e.checkpoint_id == *id,
        (RuntimeEvent::MailboxMessageAppended(e), EntityRef::Mailbox(id)) => e.message_id == *id,
        (RuntimeEvent::ToolInvocationStarted(e), EntityRef::ToolInvocation(id)) => {
            e.invocation_id == *id
        }
        (RuntimeEvent::ToolInvocationCompleted(e), EntityRef::ToolInvocation(id)) => {
            e.invocation_id == *id
        }
        (RuntimeEvent::ToolInvocationFailed(e), EntityRef::ToolInvocation(id)) => {
            e.invocation_id == *id
        }
        (RuntimeEvent::SignalIngested(e), EntityRef::Signal(id)) => e.signal_id == *id,
        _ => false,
    }
}

// -- SessionReadModel --

#[async_trait]
impl SessionReadModel for InMemoryStore {
    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.sessions.get(session_id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<SessionRecord> = state
            .sessions
            .values()
            .filter(|s| s.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|s| (s.created_at, s.session_id.as_str().to_owned()));
        let results: Vec<SessionRecord> = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- RunReadModel --

#[async_trait]
impl RunReadModel for InMemoryStore {
    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.runs.get(run_id.as_str()).cloned())
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<RunRecord> = state
            .runs
            .values()
            .filter(|r| r.session_id == *session_id)
            .cloned()
            .collect();
        results.sort_by_key(|r| (r.created_at, r.run_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn any_non_terminal(&self, session_id: &SessionId) -> Result<bool, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .runs
            .values()
            .any(|r| r.session_id == *session_id && !r.state.is_terminal()))
    }

    async fn latest_root_run(
        &self,
        session_id: &SessionId,
    ) -> Result<Option<RunRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .runs
            .values()
            .filter(|r| r.session_id == *session_id && r.parent_run_id.is_none())
            .max_by_key(|r| r.created_at)
            .cloned())
    }

    async fn list_by_state(
        &self,
        state: cairn_domain::RunState,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        let store = self.state.lock().unwrap();
        let mut results: Vec<RunRecord> = store
            .runs
            .values()
            .filter(|r| r.state == state)
            .cloned()
            .collect();
        results.sort_by_key(|r| r.created_at);
        results.truncate(limit);
        Ok(results)
    }
}

// -- TaskReadModel --

#[async_trait]
impl TaskReadModel for InMemoryStore {
    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.tasks.get(task_id.as_str()).cloned())
    }

    async fn list_by_state(
        &self,
        project: &ProjectKey,
        task_state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<TaskRecord> = state
            .tasks
            .values()
            .filter(|t| t.project == *project && t.state == task_state)
            .cloned()
            .collect();
        results.sort_by_key(|t| (t.created_at, t.task_id.as_str().to_owned()));
        results.truncate(limit);
        Ok(results)
    }

    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<TaskRecord> = state
            .tasks
            .values()
            .filter(|t| {
                t.state == TaskState::Leased && t.lease_expires_at.map_or(false, |exp| exp < now)
            })
            .cloned()
            .collect();
        results.sort_by_key(|t| {
            (
                t.lease_expires_at.unwrap_or(0),
                t.task_id.as_str().to_owned(),
            )
        });
        results.truncate(limit);
        Ok(results)
    }

    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<TaskRecord> = state
            .tasks
            .values()
            .filter(|t| t.parent_run_id.as_ref() == Some(parent_run_id))
            .cloned()
            .collect();
        results.sort_by_key(|t| (t.created_at, t.task_id.as_str().to_owned()));
        results.truncate(limit);
        Ok(results)
    }

    async fn any_non_terminal_children(&self, parent_run_id: &RunId) -> Result<bool, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .tasks
            .values()
            .any(|t| t.parent_run_id.as_ref() == Some(parent_run_id) && !t.state.is_terminal()))
    }
}

// -- ApprovalReadModel --

#[async_trait]
impl ApprovalReadModel for InMemoryStore {
    async fn get(&self, approval_id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.approvals.get(approval_id.as_str()).cloned())
    }

    async fn list_pending(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<ApprovalRecord> = state
            .approvals
            .values()
            .filter(|a| a.project == *project && a.decision.is_none())
            .cloned()
            .collect();
        results.sort_by_key(|a| (a.created_at, a.approval_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- CheckpointReadModel --

#[async_trait]
impl CheckpointReadModel for InMemoryStore {
    async fn get(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<Option<CheckpointRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.checkpoints.get(checkpoint_id.as_str()).cloned())
    }

    async fn latest_for_run(&self, run_id: &RunId) -> Result<Option<CheckpointRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state
            .checkpoints
            .values()
            .find(|c| c.run_id == *run_id && c.disposition == CheckpointDisposition::Latest)
            .cloned())
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<CheckpointRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<CheckpointRecord> = state
            .checkpoints
            .values()
            .filter(|c| c.run_id == *run_id)
            .cloned()
            .collect();
        results.sort_by_key(|c| (c.created_at, c.checkpoint_id.as_str().to_owned()));
        results.truncate(limit);
        Ok(results)
    }
}

// -- MailboxReadModel --

#[async_trait]
impl MailboxReadModel for InMemoryStore {
    async fn get(
        &self,
        message_id: &MailboxMessageId,
    ) -> Result<Option<MailboxRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.mailbox_messages.get(message_id.as_str()).cloned())
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<MailboxRecord> = state
            .mailbox_messages
            .values()
            .filter(|m| m.run_id.as_ref() == Some(run_id))
            .cloned()
            .collect();
        results.sort_by_key(|m| (m.created_at, m.message_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    async fn list_by_task(
        &self,
        task_id: &TaskId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<MailboxRecord> = state
            .mailbox_messages
            .values()
            .filter(|m| m.task_id.as_ref() == Some(task_id))
            .cloned()
            .collect();
        results.sort_by_key(|m| (m.created_at, m.message_id.as_str().to_owned()));
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- ToolInvocationReadModel --

#[async_trait]
impl ToolInvocationReadModel for InMemoryStore {
    async fn get(
        &self,
        invocation_id: &ToolInvocationId,
    ) -> Result<Option<ToolInvocationRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.tool_invocations.get(invocation_id.as_str()).cloned())
    }

    async fn list_by_run(
        &self,
        run_id: &RunId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<ToolInvocationRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<ToolInvocationRecord> = state
            .tool_invocations
            .values()
            .filter(|record| record.run_id.as_ref() == Some(run_id))
            .cloned()
            .collect();
        results.sort_by_key(|record| record.requested_at_ms);
        Ok(results.into_iter().skip(offset).take(limit).collect())
    }
}

// -- SignalReadModel --

#[async_trait]
impl SignalReadModel for InMemoryStore {
    async fn get(
        &self,
        signal_id: &cairn_domain::SignalId,
    ) -> Result<Option<cairn_domain::SignalRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        Ok(state.signals.get(signal_id.as_str()).cloned())
    }

    async fn list_by_project(
        &self,
        project: &cairn_domain::ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<cairn_domain::SignalRecord>, StoreError> {
        let state = self.state.lock().unwrap();
        let mut results: Vec<cairn_domain::SignalRecord> = state
            .signals
            .values()
            .filter(|s| s.project == *project)
            .cloned()
            .collect();
        results.sort_by_key(|s| s.timestamp_ms);
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }
}

// -- Lease helpers (not trait-based, used by runtime directly) --

impl InMemoryStore {
    /// Set lease fields on a task. Used by runtime TaskService for claim/heartbeat.
    pub async fn set_task_lease(
        &self,
        task_id: &TaskId,
        owner: String,
        expires_at: u64,
    ) -> Result<(), StoreError> {
        let mut state = self.state.lock().unwrap();
        let rec = state
            .tasks
            .get_mut(task_id.as_str())
            .ok_or_else(|| StoreError::NotFound {
                entity: "task",
                id: task_id.to_string(),
            })?;
        rec.lease_owner = Some(owner);
        rec.lease_expires_at = Some(expires_at);
        Ok(())
    }

    /// Clear lease fields on a task.
    pub async fn clear_task_lease(&self, task_id: &TaskId) -> Result<(), StoreError> {
        let mut state = self.state.lock().unwrap();
        if let Some(rec) = state.tasks.get_mut(task_id.as_str()) {
            rec.lease_owner = None;
            rec.lease_expires_at = None;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant", "workspace", "project")
    }

    fn make_envelope(event: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
        EventEnvelope::for_runtime_event(EventId::new("evt_test"), EventSource::Runtime, event)
    }

    #[tokio::test]
    async fn append_and_read_session_lifecycle() {
        let store = InMemoryStore::new();
        let project = test_project();
        let session_id = SessionId::new("sess_1");

        // Create session
        let positions = store
            .append(&[make_envelope(RuntimeEvent::SessionCreated(
                SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                },
            ))])
            .await
            .unwrap();

        assert_eq!(positions.len(), 1);
        assert_eq!(positions[0], EventPosition(1));

        // Read projection
        let session = SessionReadModel::get(&store, &session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Open);
        assert_eq!(session.version, 1);

        // Change state
        store
            .append(&[make_envelope(RuntimeEvent::SessionStateChanged(
                SessionStateChanged {
                    project: project.clone(),
                    session_id: session_id.clone(),
                    transition: StateTransition {
                        from: Some(SessionState::Open),
                        to: SessionState::Completed,
                    },
                },
            ))])
            .await
            .unwrap();

        let session = SessionReadModel::get(&store, &session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Completed);
        assert_eq!(session.version, 2);
    }

    #[tokio::test]
    async fn append_and_read_run_lifecycle() {
        let store = InMemoryStore::new();
        let project = test_project();
        let session_id = SessionId::new("sess_1");
        let run_id = RunId::new("run_1");

        store
            .append(&[make_envelope(RuntimeEvent::RunCreated(RunCreated {
                project: project.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: None,
            }))])
            .await
            .unwrap();

        let run = RunReadModel::get(&store, &run_id).await.unwrap().unwrap();
        assert_eq!(run.state, RunState::Pending);

        // Advance to running then completed
        store
            .append(&[
                make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                })),
                make_envelope(RuntimeEvent::RunStateChanged(RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::Completed,
                    },
                    failure_class: None,
                })),
            ])
            .await
            .unwrap();

        let run = RunReadModel::get(&store, &run_id).await.unwrap().unwrap();
        assert_eq!(run.state, RunState::Completed);
        assert_eq!(run.version, 3);
    }

    #[tokio::test]
    async fn task_lifecycle_with_lease() {
        let store = InMemoryStore::new();
        let project = test_project();
        let task_id = TaskId::new("task_1");

        store
            .append(&[make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: task_id.clone(),
                parent_run_id: None,
                parent_task_id: None,
            }))])
            .await
            .unwrap();

        let task = TaskReadModel::get(&store, &task_id).await.unwrap().unwrap();
        assert_eq!(task.state, TaskState::Queued);

        // Claim lease via event (Worker 2 added TaskLeaseClaimed)
        store
            .append(&[
                make_envelope(RuntimeEvent::TaskLeaseClaimed(TaskLeaseClaimed {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    lease_owner: "worker-a".to_owned(),
                    lease_token: 1,
                    lease_expires_at_ms: 9999999999,
                })),
                make_envelope(RuntimeEvent::TaskStateChanged(TaskStateChanged {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    transition: StateTransition {
                        from: Some(TaskState::Queued),
                        to: TaskState::Leased,
                    },
                    failure_class: None,
                })),
            ])
            .await
            .unwrap();

        let task = TaskReadModel::get(&store, &task_id).await.unwrap().unwrap();
        assert_eq!(task.state, TaskState::Leased);
        assert_eq!(task.lease_owner.as_deref(), Some("worker-a"));
    }

    #[tokio::test]
    async fn checkpoint_supersedes_previous_latest() {
        let store = InMemoryStore::new();
        let project = test_project();
        let run_id = RunId::new("run_1");

        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: CheckpointId::new("cp_1"),
                    disposition: CheckpointDisposition::Latest,
                },
            ))])
            .await
            .unwrap();

        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: CheckpointId::new("cp_2"),
                    disposition: CheckpointDisposition::Latest,
                },
            ))])
            .await
            .unwrap();

        let cp1 = CheckpointReadModel::get(&store, &CheckpointId::new("cp_1"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cp1.disposition, CheckpointDisposition::Superseded);

        let latest = store.latest_for_run(&run_id).await.unwrap().unwrap();
        assert_eq!(latest.checkpoint_id, CheckpointId::new("cp_2"));
    }

    #[tokio::test]
    async fn tool_invocation_projection_tracks_terminal_outcome() {
        let store = InMemoryStore::new();
        let project = test_project();
        let invocation_id = ToolInvocationId::new("tool_1");
        let run_id = RunId::new("run_1");

        store
            .append(&[
                make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project.clone(),
                    invocation_id: invocation_id.clone(),
                    session_id: Some(SessionId::new("sess_1")),
                    run_id: Some(run_id.clone()),
                    task_id: Some(TaskId::new("task_1")),
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "fs.read".to_owned(),
                    },
                    execution_class: ExecutionClass::SupervisedProcess,
                    requested_at_ms: 100,
                    started_at_ms: 101,
                })),
                make_envelope(RuntimeEvent::ToolInvocationCompleted(
                    ToolInvocationCompleted {
                        project,
                        invocation_id: invocation_id.clone(),
                        task_id: Some(TaskId::new("task_1")),
                        tool_name: "fs.read".to_owned(),
                        finished_at_ms: 105,
                        outcome: ToolInvocationOutcomeKind::Success,
                    },
                )),
            ])
            .await
            .unwrap();

        let record = ToolInvocationReadModel::get(&store, &invocation_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.state, ToolInvocationState::Completed);
        assert_eq!(record.outcome, Some(ToolInvocationOutcomeKind::Success));
        assert_eq!(record.finished_at_ms, Some(105));

        let listed = ToolInvocationReadModel::list_by_run(&store, &run_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].invocation_id, invocation_id);
    }

    #[tokio::test]
    async fn tool_invocation_projection_preserves_canceled_state_and_orders_by_request_time() {
        let store = InMemoryStore::new();
        let project = test_project();
        let run_id = RunId::new("run_1");
        let older_invocation = ToolInvocationId::new("tool_old");
        let newer_invocation = ToolInvocationId::new("tool_new");

        store
            .append(&[
                make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project: project.clone(),
                    invocation_id: newer_invocation.clone(),
                    session_id: Some(SessionId::new("sess_1")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "shell.exec".to_owned(),
                    },
                    execution_class: ExecutionClass::SandboxedProcess,
                    requested_at_ms: 200,
                    started_at_ms: 201,
                })),
                make_envelope(RuntimeEvent::ToolInvocationFailed(ToolInvocationFailed {
                    project: project.clone(),
                    invocation_id: newer_invocation.clone(),
                    task_id: None,
                    tool_name: "shell.exec".to_owned(),
                    finished_at_ms: 205,
                    outcome: ToolInvocationOutcomeKind::Canceled,
                    error_message: Some("canceled".to_owned()),
                })),
                make_envelope(RuntimeEvent::ToolInvocationStarted(ToolInvocationStarted {
                    project,
                    invocation_id: older_invocation.clone(),
                    session_id: Some(SessionId::new("sess_1")),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    target: ToolInvocationTarget::Builtin {
                        tool_name: "fs.read".to_owned(),
                    },
                    execution_class: ExecutionClass::SupervisedProcess,
                    requested_at_ms: 100,
                    started_at_ms: 101,
                })),
            ])
            .await
            .unwrap();

        let canceled = ToolInvocationReadModel::get(&store, &newer_invocation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(canceled.state, ToolInvocationState::Canceled);
        assert_eq!(canceled.outcome, Some(ToolInvocationOutcomeKind::Canceled));
        assert_eq!(canceled.error_message.as_deref(), Some("canceled"));

        let listed = ToolInvocationReadModel::list_by_run(&store, &run_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].invocation_id, older_invocation);
        assert_eq!(listed[1].invocation_id, newer_invocation);

        let paged = ToolInvocationReadModel::list_by_run(&store, &run_id, 1, 1)
            .await
            .unwrap();
        assert_eq!(paged.len(), 1);
        assert_eq!(paged[0].invocation_id, newer_invocation);
    }

    #[tokio::test]
    async fn event_stream_read() {
        let store = InMemoryStore::new();
        let project = test_project();

        store
            .append(&[
                make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: SessionId::new("s1"),
                })),
                make_envelope(RuntimeEvent::SessionCreated(SessionCreated {
                    project: project.clone(),
                    session_id: SessionId::new("s2"),
                })),
            ])
            .await
            .unwrap();

        let all = store.read_stream(None, 100).await.unwrap();
        assert_eq!(all.len(), 2);

        let after_first = store
            .read_stream(Some(EventPosition(1)), 100)
            .await
            .unwrap();
        assert_eq!(after_first.len(), 1);
    }

    /// Full lifecycle integration test: session -> run -> task -> approval -> checkpoint -> mailbox.
    /// Validates all projections are correct after a realistic event sequence.
    #[tokio::test]
    async fn full_lifecycle_projection_correctness() {
        let store = InMemoryStore::new();
        let project = test_project();
        let session_id = SessionId::new("sess_int");
        let run_id = RunId::new("run_int");
        let task_id = TaskId::new("task_int");
        let approval_id = ApprovalId::new("approval_int");
        let checkpoint_id_1 = CheckpointId::new("cp_int_1");
        let checkpoint_id_2 = CheckpointId::new("cp_int_2");
        let message_id = MailboxMessageId::new("msg_int");

        // 1. Create session.
        store
            .append(&[make_envelope(RuntimeEvent::SessionCreated(
                SessionCreated {
                    project: project.clone(),
                    session_id: session_id.clone(),
                },
            ))])
            .await
            .unwrap();

        // 2. Create run in session.
        store
            .append(&[make_envelope(RuntimeEvent::RunCreated(RunCreated {
                project: project.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: None,
            }))])
            .await
            .unwrap();

        // 3. Start run.
        store
            .append(&[make_envelope(RuntimeEvent::RunStateChanged(
                RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Pending),
                        to: RunState::Running,
                    },
                    failure_class: None,
                },
            ))])
            .await
            .unwrap();

        // 4. Create task.
        store
            .append(&[make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: task_id.clone(),
                parent_run_id: Some(run_id.clone()),
                parent_task_id: None,
            }))])
            .await
            .unwrap();

        // 5. Claim task lease.
        store
            .append(&[make_envelope(RuntimeEvent::TaskLeaseClaimed(
                TaskLeaseClaimed {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    lease_owner: "worker-alpha".to_owned(),
                    lease_token: 1,
                    lease_expires_at_ms: 9999999999,
                },
            ))])
            .await
            .unwrap();

        // 6. Task starts running.
        store
            .append(&[make_envelope(RuntimeEvent::TaskStateChanged(
                TaskStateChanged {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    transition: StateTransition {
                        from: Some(TaskState::Leased),
                        to: TaskState::Running,
                    },
                    failure_class: None,
                },
            ))])
            .await
            .unwrap();

        // 7. Request approval.
        store
            .append(&[make_envelope(RuntimeEvent::ApprovalRequested(
                ApprovalRequested {
                    project: project.clone(),
                    approval_id: approval_id.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: Some(task_id.clone()),
                    requirement: ApprovalRequirement::Required,
                },
            ))])
            .await
            .unwrap();

        // 8. Save checkpoint.
        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: checkpoint_id_1.clone(),
                    disposition: CheckpointDisposition::Latest,
                },
            ))])
            .await
            .unwrap();

        // 9. Save second checkpoint (supersedes first).
        store
            .append(&[make_envelope(RuntimeEvent::CheckpointRecorded(
                CheckpointRecorded {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    checkpoint_id: checkpoint_id_2.clone(),
                    disposition: CheckpointDisposition::Latest,
                },
            ))])
            .await
            .unwrap();

        // 10. Resolve approval.
        store
            .append(&[make_envelope(RuntimeEvent::ApprovalResolved(
                ApprovalResolved {
                    project: project.clone(),
                    approval_id: approval_id.clone(),
                    decision: ApprovalDecision::Approved,
                },
            ))])
            .await
            .unwrap();

        // 11. Send mailbox message.
        store
            .append(&[make_envelope(RuntimeEvent::MailboxMessageAppended(
                MailboxMessageAppended {
                    project: project.clone(),
                    message_id: message_id.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: Some(task_id.clone()),
                },
            ))])
            .await
            .unwrap();

        // 12. Complete task.
        store
            .append(&[make_envelope(RuntimeEvent::TaskStateChanged(
                TaskStateChanged {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    transition: StateTransition {
                        from: Some(TaskState::Running),
                        to: TaskState::Completed,
                    },
                    failure_class: None,
                },
            ))])
            .await
            .unwrap();

        // 13. Complete run.
        store
            .append(&[make_envelope(RuntimeEvent::RunStateChanged(
                RunStateChanged {
                    project: project.clone(),
                    run_id: run_id.clone(),
                    transition: StateTransition {
                        from: Some(RunState::Running),
                        to: RunState::Completed,
                    },
                    failure_class: None,
                },
            ))])
            .await
            .unwrap();

        // --- Verify all projections ---

        // Session: still open (derived from run state, not explicit close).
        let session = SessionReadModel::get(&store, &session_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(session.state, SessionState::Open);

        // Run: completed.
        let run = RunReadModel::get(&store, &run_id).await.unwrap().unwrap();
        assert_eq!(run.state, RunState::Completed);
        assert!(run.state.is_terminal());
        assert!(run.parent_run_id.is_none());

        // Task: completed with lease info preserved.
        let task = TaskReadModel::get(&store, &task_id).await.unwrap().unwrap();
        assert_eq!(task.state, TaskState::Completed);
        assert!(task.state.is_terminal());
        assert_eq!(task.lease_owner.as_deref(), Some("worker-alpha"));
        assert_eq!(task.parent_run_id.as_ref(), Some(&run_id));

        // Approval: resolved as approved.
        let approval = ApprovalReadModel::get(&store, &approval_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(approval.decision, Some(ApprovalDecision::Approved));
        assert_eq!(approval.run_id.as_ref(), Some(&run_id));

        // Checkpoint 1: superseded.
        let cp1 = CheckpointReadModel::get(&store, &checkpoint_id_1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cp1.disposition, CheckpointDisposition::Superseded);

        // Checkpoint 2: latest.
        let cp2 = CheckpointReadModel::get(&store, &checkpoint_id_2)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cp2.disposition, CheckpointDisposition::Latest);

        // Latest checkpoint for run is cp2.
        let latest = store.latest_for_run(&run_id).await.unwrap().unwrap();
        assert_eq!(latest.checkpoint_id, checkpoint_id_2);

        // Mailbox: message linked to run and task.
        let msg = MailboxReadModel::get(&store, &message_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg.run_id.as_ref(), Some(&run_id));
        assert_eq!(msg.task_id.as_ref(), Some(&task_id));

        // No non-terminal runs remain.
        assert!(!store.any_non_terminal(&session_id).await.unwrap());

        // Event stream has all 13 events.
        let all = store.read_stream(None, 100).await.unwrap();
        assert_eq!(all.len(), 13);

        // Entity-filtered read for run events.
        let run_events = store
            .read_by_entity(&EntityRef::Run(run_id.clone()), None, 100)
            .await
            .unwrap();
        assert!(run_events.len() >= 3); // created + 2 state changes
    }

    /// Expired lease detection for recovery sweeps.
    #[tokio::test]
    async fn expired_lease_detection() {
        let store = InMemoryStore::new();
        let project = test_project();

        // Create two tasks with leases.
        for (id, expires) in [("t1", 100u64), ("t2", 9999999999u64)] {
            let task_id = TaskId::new(id);
            store
                .append(&[make_envelope(RuntimeEvent::TaskCreated(TaskCreated {
                    project: project.clone(),
                    task_id: task_id.clone(),
                    parent_run_id: None,
                    parent_task_id: None,
                }))])
                .await
                .unwrap();

            store
                .append(&[make_envelope(RuntimeEvent::TaskLeaseClaimed(
                    TaskLeaseClaimed {
                        project: project.clone(),
                        task_id: task_id.clone(),
                        lease_owner: "w".to_owned(),
                        lease_token: 1,
                        lease_expires_at_ms: expires,
                    },
                ))])
                .await
                .unwrap();

            store
                .append(&[make_envelope(RuntimeEvent::TaskStateChanged(
                    TaskStateChanged {
                        project: project.clone(),
                        task_id,
                        transition: StateTransition {
                            from: Some(TaskState::Queued),
                            to: TaskState::Leased,
                        },
                        failure_class: None,
                    },
                ))])
                .await
                .unwrap();
        }

        // t1 expired (lease at 100, now is 500), t2 still valid.
        let expired = store.list_expired_leases(500, 100).await.unwrap();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].task_id, TaskId::new("t1"));
    }
}
