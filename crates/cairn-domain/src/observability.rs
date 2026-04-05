//! LLM observability types (GAP-010).
//!
//! Structured traces of every LLM provider call, indexed by session and run
//! for operator visibility into latency, token usage, and cost.

use serde::{Deserialize, Serialize};

use crate::ids::{RunId, SessionId};

/// A single LLM provider call trace record.
///
/// Derived from a `ProviderCallCompleted` event. Stored in the
/// `LlmCallTraceReadModel` projection for low-latency operator queries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCallTrace {
    /// Stable ID — equals the `provider_call_id` of the originating event.
    pub trace_id: String,
    /// Model identifier as reported by the provider (e.g. `claude-sonnet-4-6`).
    pub model_id: String,
    /// Input/prompt tokens consumed by this call.
    pub prompt_tokens: u32,
    /// Output/completion tokens produced by this call.
    pub completion_tokens: u32,
    /// Wall-clock latency of the provider API call in milliseconds.
    pub latency_ms: u64,
    /// Estimated cost of this call in micro-USD (0 for free/flat-rate models).
    pub cost_micros: u64,
    /// Session this call belongs to, if known.
    pub session_id: Option<SessionId>,
    /// Run this call belongs to, if known.
    pub run_id: Option<RunId>,
    /// Unix epoch milliseconds when the call completed.
    pub created_at_ms: u64,
    /// True when the provider call failed (status != Succeeded).
    #[serde(default)]
    pub is_error: bool,
}

impl LlmCallTrace {
    /// Total tokens (prompt + completion).
    pub fn total_tokens(&self) -> u32 {
        self.prompt_tokens.saturating_add(self.completion_tokens)
    }
}
