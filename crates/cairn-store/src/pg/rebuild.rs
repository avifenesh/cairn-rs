use sqlx::PgPool;

use crate::error::StoreError;
use crate::event_log::{EventPosition, StoredEvent};

use super::projections::PgSyncProjection;

/// Rebuilds synchronous projections from the event log.
///
/// Per RFC 002, synchronous projections may be rebuilt from the canonical
/// event log. This rebuilder truncates current-state tables and replays
/// all events through `PgSyncProjection::apply_async`.
pub struct ProjectionRebuilder {
    pool: PgPool,
}

/// Tables that hold synchronous projection state.
const PROJECTION_TABLES: &[&str] = &[
    "tool_invocations",
    "mailbox_messages",
    "checkpoints",
    "approvals",
    "tasks",
    "runs",
    "sessions",
];

impl ProjectionRebuilder {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Rebuild all projections from scratch.
    ///
    /// 1. Truncates all projection tables (in FK-safe order)
    /// 2. Reads the entire event log in batches
    /// 3. Replays each event through PgSyncProjection::apply_async
    ///
    /// This is an expensive operation meant for recovery, migration,
    /// or development — not for routine use.
    pub async fn rebuild_all(&self) -> Result<RebuildReport, StoreError> {
        // Truncate in reverse FK order.
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;

        for table in PROJECTION_TABLES {
            let sql = format!("DELETE FROM {table}");
            sqlx::query(&sql)
                .execute(&mut *tx)
                .await
                .map_err(|e| StoreError::Internal(format!("truncate {table}: {e}")))?;
        }

        tx.commit()
            .await
            .map_err(|e| StoreError::Internal(e.to_string()))?;

        // Replay all events in batches.
        let batch_size: usize = 500;
        let mut cursor: Option<EventPosition> = None;
        let mut total_events: u64 = 0;
        let mut total_errors: u64 = 0;

        loop {
            let events = self.read_batch(cursor, batch_size).await?;
            if events.is_empty() {
                break;
            }

            let batch_len = events.len();

            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;

            for event in &events {
                match PgSyncProjection::apply_async(&mut tx, event).await {
                    Ok(()) => {}
                    Err(_e) => {
                        total_errors += 1;
                        // Continue rebuilding — log but don't abort.
                    }
                }
                total_events += 1;
            }

            tx.commit()
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;

            cursor = events.last().map(|e| e.position);

            if batch_len < batch_size {
                break;
            }
        }

        Ok(RebuildReport {
            events_replayed: total_events,
            errors: total_errors,
            last_position: cursor,
        })
    }

    /// Incremental rebuild: replay events after a given position.
    ///
    /// Useful for catching up projections that fell behind,
    /// without the cost of a full truncate+replay.
    pub async fn rebuild_from(&self, after: EventPosition) -> Result<RebuildReport, StoreError> {
        let batch_size: usize = 500;
        let mut cursor = Some(after);
        let mut total_events: u64 = 0;
        let mut total_errors: u64 = 0;

        loop {
            let events = self.read_batch(cursor, batch_size).await?;
            if events.is_empty() {
                break;
            }

            let batch_len = events.len();

            let mut tx = self
                .pool
                .begin()
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;

            for event in &events {
                match PgSyncProjection::apply_async(&mut tx, event).await {
                    Ok(()) => {}
                    Err(_e) => {
                        total_errors += 1;
                    }
                }
                total_events += 1;
            }

            tx.commit()
                .await
                .map_err(|e| StoreError::Internal(e.to_string()))?;

            cursor = events.last().map(|e| e.position);

            if batch_len < batch_size {
                break;
            }
        }

        Ok(RebuildReport {
            events_replayed: total_events,
            errors: total_errors,
            last_position: cursor,
        })
    }

    async fn read_batch(
        &self,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        use super::event_log::PgEventLog;
        use crate::event_log::EventLog;

        let log = PgEventLog::new(self.pool.clone());
        log.read_stream(after, limit).await
    }
}

/// Report from a projection rebuild operation.
#[derive(Clone, Debug)]
pub struct RebuildReport {
    pub events_replayed: u64,
    pub errors: u64,
    pub last_position: Option<EventPosition>,
}

impl RebuildReport {
    pub fn is_clean(&self) -> bool {
        self.errors == 0
    }
}
