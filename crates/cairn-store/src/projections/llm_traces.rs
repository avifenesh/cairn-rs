use async_trait::async_trait;
use cairn_domain::{LlmCallTrace, SessionId};

use crate::error::StoreError;

/// Read model for LLM call traces (GAP-010 observability).
#[async_trait]
pub trait LlmCallTraceReadModel: Send + Sync {
    /// Insert a new LLM call trace.
    async fn insert_trace(&self, trace: LlmCallTrace) -> Result<(), StoreError>;

    /// List traces for a session, most-recent first.
    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
    ) -> Result<Vec<LlmCallTrace>, StoreError>;

    /// List all traces, most-recent first (operator-level view).
    async fn list_all_traces(&self, limit: usize) -> Result<Vec<LlmCallTrace>, StoreError>;
}
