//! Cursor table for the FF lease_history subscriber.
//!
//! The subscriber in `cairn-fabric` tails FF's per-execution
//! `lease_history` streams and emits `BridgeEvent`s for transitions that
//! never flow through a cairn service call (FF-initiated lease expiry
//! and reclaim). A persistent per-stream cursor lets the subscriber
//! resume across process restarts instead of re-processing or missing
//! frames.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::StoreError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FfLeaseHistoryCursor {
    pub partition_id: String,
    pub execution_id: String,
    pub last_stream_id: String,
    pub updated_at_ms: u64,
}

#[async_trait]
pub trait FfLeaseHistoryCursorStore: Send + Sync {
    /// Look up the cursor for a single stream, or `None` if never recorded.
    async fn get(
        &self,
        partition_id: &str,
        execution_id: &str,
    ) -> Result<Option<FfLeaseHistoryCursor>, StoreError>;

    /// All cursors recorded for a partition. Used by the subscriber at
    /// startup to rebuild its in-memory cursor map for a partition's
    /// XREAD call without hitting Valkey first.
    async fn list_by_partition(
        &self,
        partition_id: &str,
    ) -> Result<Vec<FfLeaseHistoryCursor>, StoreError>;

    /// Upsert the cursor for a stream. Called once per consumed frame.
    async fn upsert(&self, cursor: &FfLeaseHistoryCursor) -> Result<(), StoreError>;

    /// Remove a cursor. Called when the execution terminates and its
    /// lease_history stream is no longer tailable. Keeping stale rows
    /// around would grow the table without bound.
    async fn delete(&self, partition_id: &str, execution_id: &str) -> Result<(), StoreError>;
}
