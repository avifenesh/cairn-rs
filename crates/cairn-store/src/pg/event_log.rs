use async_trait::async_trait;
use sqlx::PgPool;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{EventEnvelope, RuntimeEvent};

use crate::error::StoreError;
use crate::event_log::{EntityRef, EventLog, EventPosition, StoredEvent};
use super::projections::PgSyncProjection;

/// Postgres-backed append-only event log.
///
/// Appends events to the `event_log` table and updates synchronous
/// projections within the same transaction.
pub struct PgEventLog {
    pool: PgPool,
}

impl PgEventLog {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventLog for PgEventLog {
    async fn append(
        &self,
        events: &[EventEnvelope<RuntimeEvent>],
    ) -> Result<Vec<EventPosition>, StoreError> {
        if events.is_empty() {
            return Ok(vec![]);
        }

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;

        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;

        let mut positions = Vec::with_capacity(events.len());

        for event in events {
            let event_id = event.event_id.as_str();
            let source_type = source_type_str(&event.source);
            let source_meta = serde_json::to_value(&event.source)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            let ownership = serde_json::to_value(&event.ownership)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            let payload = serde_json::to_value(&event.payload)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;

            let row: (i64,) = sqlx::query_as(
                "INSERT INTO event_log (event_id, source_type, source_meta, ownership, causation_id, correlation_id, payload, stored_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 RETURNING position",
            )
            .bind(event_id)
            .bind(source_type)
            .bind(source_meta)
            .bind(ownership)
            .bind(event.causation_id.as_ref().map(|id| id.as_str()))
            .bind(event.correlation_id.as_deref())
            .bind(payload)
            .bind(now)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;

            let pos = EventPosition(row.0 as u64);

            // Apply synchronous projections within the same transaction.
            // This guarantees that current-state tables (sessions, runs, tasks, …)
            // are always consistent with the event log — reads can never see a
            // position that hasn't been projected yet.
            let stored = StoredEvent {
                position: pos,
                envelope: event.clone(),
                stored_at: now as u64,
            };
            PgSyncProjection::apply_async(&mut tx, &stored).await?;

            positions.push(pos);
        }

        tx.commit()
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;

        Ok(positions)
    }

    async fn read_by_entity(
        &self,
        entity: &EntityRef,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let after_pos = after.map(|p| p.0 as i64).unwrap_or(0);
        let (id_field, id_value) = entity_ref_filter(entity);

        // Filter events by payload JSON field matching the entity ID.
        let sql = format!(
            "SELECT position, event_id, source_type, source_meta, ownership, causation_id, correlation_id, payload, stored_at
             FROM event_log
             WHERE position > $1
               AND payload->>'{id_field}' = $2
             ORDER BY position ASC
             LIMIT $3"
        );

        let rows = sqlx::query_as::<_, EventRow>(&sql)
            .bind(after_pos)
            .bind(&id_value)
            .bind(limit as i64)
            .fetch_all(&self.pool)
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(|r| r.into_stored_event()).collect()
    }

    async fn read_stream(
        &self,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        let after_pos = after.map(|p| p.0 as i64).unwrap_or(0);

        let rows = sqlx::query_as::<_, EventRow>(
            "SELECT position, event_id, source_type, source_meta, ownership, causation_id, correlation_id, payload, stored_at
             FROM event_log
             WHERE position > $1
             ORDER BY position ASC
             LIMIT $2",
        )
        .bind(after_pos)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| StoreError::Internal(e.to_string()))?;

        rows.into_iter().map(|r| r.into_stored_event()).collect()
    }

    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
        let row: Option<(i64,)> = sqlx::query_as("SELECT MAX(position) FROM event_log")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;

        Ok(row.and_then(|(pos,)| {
            if pos > 0 {
                Some(EventPosition(pos as u64))
            } else {
                None
            }
        }))
    }

    async fn find_by_causation_id(
        &self,
        causation_id: &str,
    ) -> Result<Option<EventPosition>, StoreError> {
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT position FROM event_log WHERE causation_id = $1 LIMIT 1")
                .bind(causation_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;
        Ok(row.map(|(pos,)| EventPosition(pos as u64)))
    }
}

/// Raw row from the event_log table.
#[derive(sqlx::FromRow)]
struct EventRow {
    position: i64,
    #[allow(dead_code)]
    event_id: String,
    #[allow(dead_code)]
    source_type: String,
    source_meta: serde_json::Value,
    ownership: serde_json::Value,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    payload: serde_json::Value,
    stored_at: i64,
}

impl EventRow {
    fn into_stored_event(self) -> Result<StoredEvent, StoreError> {
        let source = serde_json::from_value(self.source_meta)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        let ownership: cairn_domain::OwnershipKey = serde_json::from_value(self.ownership)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        let payload: RuntimeEvent = serde_json::from_value(self.payload)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;

        // Row rehydration stays explicit here: source/ownership/ids are
        // persisted separately in the event log and need to be reconstructed
        // from stored columns rather than re-derived only from the payload.
        let mut envelope = EventEnvelope::new(
            cairn_domain::EventId::new(self.event_id),
            source,
            ownership,
            payload,
        );

        if let Some(causation_id) = self.causation_id {
            envelope = envelope.with_causation_id(cairn_domain::CommandId::new(causation_id));
        }

        if let Some(correlation_id) = self.correlation_id {
            envelope = envelope.with_correlation_id(correlation_id);
        }

        Ok(StoredEvent {
            position: EventPosition(self.position as u64),
            envelope,
            stored_at: self.stored_at as u64,
        })
    }
}

fn source_type_str(source: &cairn_domain::EventSource) -> &'static str {
    match source {
        cairn_domain::EventSource::Operator { .. } => "operator",
        cairn_domain::EventSource::Runtime => "runtime",
        cairn_domain::EventSource::Scheduler => "scheduler",
        cairn_domain::EventSource::ExternalWorker { .. } => "external_worker",
        cairn_domain::EventSource::System => "system",
    }
}

fn entity_ref_filter(entity: &EntityRef) -> (&'static str, String) {
    match entity {
        EntityRef::Session(id) => ("session_id", id.to_string()),
        EntityRef::Run(id) => ("run_id", id.to_string()),
        EntityRef::Task(id) => ("task_id", id.to_string()),
        EntityRef::Approval(id) => ("approval_id", id.to_string()),
        EntityRef::Checkpoint(id) => ("checkpoint_id", id.to_string()),
        EntityRef::Mailbox(id) => ("message_id", id.to_string()),
        EntityRef::ToolInvocation(id) => ("invocation_id", id.to_string()),
        EntityRef::Signal(id) => ("signal_id", id.to_string()),
        EntityRef::IngestJob(id) => ("job_id", id.to_string()),
        EntityRef::EvalRun(id) => ("eval_run_id", id.to_string()),
        EntityRef::PromptAsset(id) => ("prompt_asset_id", id.to_string()),
        EntityRef::PromptVersion(id) => ("prompt_version_id", id.to_string()),
        EntityRef::PromptRelease(id) => ("prompt_release_id", id.to_string()),
    }
}
