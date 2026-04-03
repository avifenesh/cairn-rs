use async_trait::async_trait;
use cairn_domain::{
    ApprovalId, CheckpointId, EvalRunId, EventEnvelope, IngestJobId, MailboxMessageId, RunId,
    RuntimeEvent, SessionId, SignalId, TaskId, ToolInvocationId,
};
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

/// Monotonic position in the append-only event log.
///
/// Positions are globally ordered: a higher value means a later event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EventPosition(pub u64);

/// RFC 002 durability classification for entities.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurabilityClass {
    /// Every accepted event retained; entity state reconstructable from history.
    FullHistory,
    /// Durable current state with audit events; no full replay requirement.
    CurrentStatePlusAudit,
}

/// Reference to a specific entity for event filtering.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntityRef {
    Session(SessionId),
    Run(RunId),
    Task(TaskId),
    Approval(ApprovalId),
    Checkpoint(CheckpointId),
    Mailbox(MailboxMessageId),
    ToolInvocation(ToolInvocationId),
    Signal(SignalId),
    IngestJob(IngestJobId),
    EvalRun(EvalRunId),
}

/// An event persisted in the log with storage metadata.
#[derive(Clone, Debug)]
pub struct StoredEvent {
    pub position: EventPosition,
    pub envelope: EventEnvelope<RuntimeEvent>,
    /// Unix milliseconds when the event was persisted.
    pub stored_at: u64,
}

/// Durable append-only event log for runtime events.
///
/// Per RFC 002:
/// - append-only for critical runtime facts
/// - per-entity and global stream reads
/// - cursor-based replay with a minimum 72-hour SSE replay window
///
/// Synchronous projections are applied within the same transaction as
/// `append`. Concrete implementations coordinate this through the DB
/// adapter's transaction boundary.
#[async_trait]
pub trait EventLog: Send + Sync {
    /// Atomically append events and return assigned positions.
    ///
    /// Implementations must ensure synchronous projections are updated
    /// within the same transaction.
    async fn append(
        &self,
        events: &[EventEnvelope<RuntimeEvent>],
    ) -> Result<Vec<EventPosition>, StoreError>;

    /// Read events for a specific entity, optionally starting after a position.
    async fn read_by_entity(
        &self,
        entity: &EntityRef,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Read the global event stream starting after a position.
    async fn read_stream(
        &self,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError>;

    /// Current head position of the log, or `None` if empty.
    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError>;
}

#[cfg(test)]
mod tests {
    use super::{DurabilityClass, EntityRef, EventPosition};
    use cairn_domain::SessionId;

    #[test]
    fn event_positions_are_ordered() {
        assert!(EventPosition(1) < EventPosition(2));
        assert_eq!(EventPosition(5), EventPosition(5));
    }

    #[test]
    fn entity_ref_carries_domain_id() {
        let entity = EntityRef::Session(SessionId::new("sess_1"));
        assert!(matches!(entity, EntityRef::Session(_)));
    }

    #[test]
    fn durability_classes_are_distinct() {
        assert_ne!(
            DurabilityClass::FullHistory,
            DurabilityClass::CurrentStatePlusAudit
        );
    }
}
