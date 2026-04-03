//! Signal service boundary per RFC 002.
//!
//! Signals are external events ingested into the runtime for durable
//! recording and downstream processing.

use async_trait::async_trait;
use cairn_domain::{ProjectKey, SignalId, SignalRecord};

use crate::error::RuntimeError;

/// Signal service boundary.
///
/// Per RFC 002, signals have `current_state_plus_audit` durability.
/// The runtime ingests external signals, persists them as events, and
/// exposes current-state reads via the projection.
#[async_trait]
pub trait SignalService: Send + Sync {
    /// Ingest an external signal into the runtime.
    async fn ingest(
        &self,
        project: &ProjectKey,
        signal_id: SignalId,
        source: String,
        payload: serde_json::Value,
        timestamp_ms: u64,
    ) -> Result<SignalRecord, RuntimeError>;

    /// Get a signal by ID.
    async fn get(&self, signal_id: &SignalId) -> Result<Option<SignalRecord>, RuntimeError>;

    /// List signals for a project.
    async fn list_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SignalRecord>, RuntimeError>;
}
