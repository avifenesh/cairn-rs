use async_trait::async_trait;
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::{EventEnvelope, RuntimeEvent};

use super::projections::SqliteSyncProjection;
use crate::error::StoreError;
use crate::event_log::{EntityRef, EventLog, EventPosition, StoredEvent};

/// SQLite-backed append-only event log for local-mode.
///
/// Appends events and updates synchronous projections within a single
/// transaction so reads can never observe an event position that hasn't
/// been projected yet. Mirrors the Postgres backend contract.
pub struct SqliteEventLog {
    pool: SqlitePool,
}

impl SqliteEventLog {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl EventLog for SqliteEventLog {
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
            let source_meta = serde_json::to_string(&event.source)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            let ownership = serde_json::to_string(&event.ownership)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;
            let payload = serde_json::to_string(&event.payload)
                .map_err(|e| StoreError::Serialization(e.to_string()))?;

            let row: (i64,) = sqlx::query_as(
                "INSERT INTO event_log (event_id, source_type, source_meta, ownership, causation_id, correlation_id, payload, stored_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 RETURNING position",
            )
            .bind(event_id)
            .bind(source_type)
            .bind(&source_meta)
            .bind(&ownership)
            .bind(event.causation_id.as_ref().map(|id| id.as_str()))
            .bind(event.correlation_id.as_deref())
            .bind(&payload)
            .bind(now)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;

            let pos = EventPosition(row.0 as u64);

            // Apply synchronous projections within the same transaction.
            // Pre-T2-C1 this call was missing and every SQLite-backed
            // projection table stayed empty in production — see audit queue
            // at .claude/audit-state/review-queue.md §T2-C1.
            let stored = StoredEvent {
                position: pos,
                envelope: event.clone(),
                stored_at: now as u64,
            };
            SqliteSyncProjection::apply_async(&mut tx, &stored).await?;

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

        let sql = format!(
            "SELECT position, event_id, source_meta, ownership, causation_id, correlation_id, payload, stored_at
             FROM event_log
             WHERE position > $1
               AND json_extract(payload, '$.{id_field}') = $2
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
            "SELECT position, event_id, source_meta, ownership, causation_id, correlation_id, payload, stored_at
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
        // `MAX(position)` on empty table yields NULL (decoded as
        // `Some((None,))` by sqlx-SQLite). Decode into `Option<i64>` and
        // filter on the inner option; decoding into plain `i64` would
        // error on NULL.
        let row: Option<(Option<i64>,)> = sqlx::query_as("SELECT MAX(position) FROM event_log")
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;

        Ok(row.and_then(|(pos,)| pos.map(|p| EventPosition(p as u64))))
    }

    async fn find_by_causation_id(
        &self,
        causation_id: &str,
    ) -> Result<Option<EventPosition>, StoreError> {
        // `MIN(position)` on an empty match always returns one row with a
        // NULL value — `fetch_optional` reports `Some(...)` for the row and
        // sqlx decodes the NULL into `Option<i64>`. Filter on the inner
        // `Option` rather than checking for a sentinel. Pre-T2-M7 a
        // `pos > 0` guard tried to approximate this but discarded the
        // legitimate position-0 edge, diverging from the PG backend.
        let row: Option<(Option<i64>,)> =
            sqlx::query_as("SELECT MIN(position) FROM event_log WHERE causation_id = ?")
                .bind(causation_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;

        Ok(row.and_then(|(pos,)| pos.map(|p| EventPosition(p as u64))))
    }
}

#[derive(sqlx::FromRow)]
struct EventRow {
    position: i64,
    event_id: String,
    source_meta: String,
    ownership: String,
    causation_id: Option<String>,
    correlation_id: Option<String>,
    payload: String,
    stored_at: i64,
}

impl EventRow {
    fn into_stored_event(self) -> Result<StoredEvent, StoreError> {
        let source = serde_json::from_str(&self.source_meta)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        let ownership: cairn_domain::OwnershipKey = serde_json::from_str(&self.ownership)
            .map_err(|e| StoreError::Serialization(e.to_string()))?;
        let payload: RuntimeEvent = serde_json::from_str(&self.payload)
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
