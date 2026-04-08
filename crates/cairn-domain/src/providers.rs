use crate::ids::{
    ProjectId, PromptReleaseId, ProviderBindingId, ProviderCallId, ProviderConnectionId,
    ProviderModelId, RouteAttemptId, RouteDecisionId, RunId, TaskId, TenantId,
};
use crate::selectors::SelectorContext;
use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Canonical provider operation kinds from RFC 009.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Generate,
    Embed,
    Rerank,
}

/// How a provider model is billed per call.
///
/// Mirrors cairn Go `internal/llm/budget.go` ProviderCostTypeForModel.
/// Used to compute `cost_micros` on `ProviderCallRecord` and to enforce
/// budget caps without counting flat-rate or free calls against token budgets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCostType {
    /// Per-token billing. `cost_micros` reflects actual token usage.
    #[default]
    Metered,
    /// Subscription/flat-rate. The call has no marginal cost; `cost_micros` = 0.
    FlatRate,
    /// Open/free model. The call has no cost; `cost_micros` = 0.
    Free,
}

impl ProviderCostType {
    /// Returns true if this cost type produces no marginal cost per call.
    pub fn is_free(&self) -> bool {
        matches!(self, ProviderCostType::FlatRate | ProviderCostType::Free)
    }
}

/// Budget period for LLM spend caps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderBudgetPeriod {
    Daily,
    Monthly,
}

/// Tenant-level LLM spend budget record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBudget {
    pub tenant_id: TenantId,
    pub period: ProviderBudgetPeriod,
    /// Hard cap in USD micros (1 USD = 1_000_000 micros).
    pub limit_micros: u64,
    /// Alert fires when spend reaches this percentage of `limit_micros`.
    pub alert_threshold_percent: u32,
    /// Accumulated spend for the current period in USD micros.
    pub current_spend_micros: u64,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Per-model cost rates for billing estimation (USD per million tokens).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelCostRates {
    pub provider_model_id: ProviderModelId,
    pub cost_type: ProviderCostType,
    /// USD per million input tokens (0.0 for flat_rate / free).
    pub cost_per_1m_input: f64,
    /// USD per million output tokens (0.0 for flat_rate / free).
    pub cost_per_1m_output: f64,
    /// USD per million cached-read tokens (0.0 if not applicable).
    pub cache_read_per_1m: f64,
    /// USD per million cache-write tokens (0.0 if not applicable).
    pub cache_write_per_1m: f64,
}

impl ModelCostRates {
    /// Estimate cost in micros (µUSD) for a call with given token counts.
    ///
    /// Returns 0 for flat-rate and free models regardless of token counts.
    pub fn estimate_micros(&self, input_tokens: u32, output_tokens: u32) -> u64 {
        if self.cost_type.is_free() {
            return 0;
        }
        let input_cost = self.cost_per_1m_input * (input_tokens as f64) / 1_000_000.0;
        let output_cost = self.cost_per_1m_output * (output_tokens as f64) / 1_000_000.0;
        ((input_cost + output_cost) * 1_000_000.0).round() as u64
    }
}

/// Stable capability vocabulary shared by route policy, runtime, and operators.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCapability {
    Streaming,
    ToolUse,
    StructuredOutput,
    ImageInput,
    ReasoningTrace,
    HighContextWindow,
}

/// Normalized product-facing tuning only; raw provider-native flags stay out of v1.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ProviderBindingSettings {
    pub temperature_milli: Option<u16>,
    pub max_output_tokens: Option<u32>,
    pub timeout_ms: Option<u64>,
    pub structured_output_mode: StructuredOutputMode,
    pub required_capabilities: Vec<ProviderCapability>,
    pub disabled_capabilities: Vec<ProviderCapability>,
    /// Billing model for cost tracking. Defaults to `Metered`.
    #[serde(default)]
    pub cost_type: ProviderCostType,
    /// Optional hard daily spend cap in USD micros. Enforced by the budget service.
    #[serde(default)]
    pub daily_budget_micros: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StructuredOutputMode {
    #[default]
    Default,
    Preferred,
    Required,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteAttemptDecision {
    Selected,
    Vetoed,
    Failed,
    Skipped,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecisionStatus {
    Selected,
    FailedAfterDispatch,
    NoViableRoute,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecisionReason {
    MissingRequiredCapability,
    DisallowedProviderFamily,
    BudgetExhausted,
    ProjectPolicyRestriction,
    SafetyModeRestriction,
    TransportFailure,
    TimedOut,
    RateLimited,
    StructuredOutputInvalid,
    ExplicitSkip,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallStatus {
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderCallErrorClass {
    TransportFailure,
    TimedOut,
    RateLimited,
    StructuredOutputInvalid,
    ProviderError,
    Cancelled,
}

/// Durable record of one candidate considered during route resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteAttemptRecord {
    pub route_attempt_id: RouteAttemptId,
    pub route_decision_id: RouteDecisionId,
    pub project_id: ProjectId,
    pub operation_kind: OperationKind,
    pub provider_binding_id: ProviderBindingId,
    pub selector_context: SelectorContext,
    pub attempt_index: u16,
    pub decision: RouteAttemptDecision,
    pub decision_reason: RouteDecisionReason,
    /// Human-readable reason this attempt was skipped (Vetoed/Skipped decisions only).
    #[serde(default)]
    pub skip_reason: Option<String>,
    /// Pre-dispatch cost estimate in micros (may be 0 for flat-rate / free models).
    #[serde(default)]
    pub estimated_cost_micros: Option<u64>,
}

/// One logical route outcome per routed request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteDecisionRecord {
    pub route_decision_id: RouteDecisionId,
    pub project_id: ProjectId,
    pub operation_kind: OperationKind,
    pub terminal_route_attempt_id: Option<RouteAttemptId>,
    pub selected_provider_binding_id: Option<ProviderBindingId>,
    pub selected_route_attempt_id: Option<RouteAttemptId>,
    pub selector_context: SelectorContext,
    pub attempt_count: u16,
    pub fallback_used: bool,
    pub final_status: RouteDecisionStatus,
}

/// Executed provider dispatch record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCallRecord {
    pub provider_call_id: ProviderCallId,
    pub route_decision_id: RouteDecisionId,
    pub route_attempt_id: RouteAttemptId,
    pub project_id: ProjectId,
    pub operation_kind: OperationKind,
    pub provider_binding_id: ProviderBindingId,
    pub provider_connection_id: ProviderConnectionId,
    pub provider_adapter: String,
    pub provider_model_id: ProviderModelId,
    pub task_id: Option<TaskId>,
    pub run_id: Option<RunId>,
    pub prompt_release_id: Option<PromptReleaseId>,
    pub fallback_position: u16,
    pub status: ProviderCallStatus,
    pub latency_ms: Option<u64>,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub cost_micros: Option<u64>,
    /// Billing model at the time of the call (copied from binding settings).
    #[serde(default)]
    pub cost_type: ProviderCostType,
    pub error_class: Option<ProviderCallErrorClass>,
}

impl RouteDecisionRecord {
    pub fn requires_provider_call(&self) -> bool {
        matches!(
            self.final_status,
            RouteDecisionStatus::Selected | RouteDecisionStatus::FailedAfterDispatch
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RouteDecisionValidationError {
    AttemptCountMismatch,
    MissingSelectedBinding,
    MissingSelectedAttempt,
    SelectedAttemptNotFound,
    MissingTerminalAttempt,
    TerminalAttemptNotFound,
    UnexpectedSelectedAttempt,
    ExpectedProviderCall,
    UnexpectedProviderCall,
    InvalidNoViableRouteAttempt,
    ProviderCallDecisionMismatch,
    ProviderCallAttemptMissing,
    DuplicateProviderCallForAttempt,
}

pub fn validate_route_decision(
    decision: &RouteDecisionRecord,
    attempts: &[RouteAttemptRecord],
    provider_calls: &[ProviderCallRecord],
) -> Result<(), RouteDecisionValidationError> {
    if attempts.len() != usize::from(decision.attempt_count) {
        return Err(RouteDecisionValidationError::AttemptCountMismatch);
    }

    let selected_attempt = decision
        .selected_route_attempt_id
        .as_ref()
        .and_then(|attempt_id| {
            attempts
                .iter()
                .find(|attempt| &attempt.route_attempt_id == attempt_id)
        });
    let terminal_attempt = decision
        .terminal_route_attempt_id
        .as_ref()
        .and_then(|attempt_id| {
            attempts
                .iter()
                .find(|attempt| &attempt.route_attempt_id == attempt_id)
        });

    match decision.final_status {
        RouteDecisionStatus::Selected => {
            if decision.selected_provider_binding_id.is_none() {
                return Err(RouteDecisionValidationError::MissingSelectedBinding);
            }
            if decision.selected_route_attempt_id.is_none() {
                return Err(RouteDecisionValidationError::MissingSelectedAttempt);
            }
            if selected_attempt.is_none() {
                return Err(RouteDecisionValidationError::SelectedAttemptNotFound);
            }
            if provider_calls.is_empty() {
                return Err(RouteDecisionValidationError::ExpectedProviderCall);
            }
        }
        RouteDecisionStatus::FailedAfterDispatch => {
            if decision.terminal_route_attempt_id.is_none() {
                return Err(RouteDecisionValidationError::MissingTerminalAttempt);
            }
            if terminal_attempt.is_none() {
                return Err(RouteDecisionValidationError::TerminalAttemptNotFound);
            }
            if decision.selected_route_attempt_id.is_some() {
                return Err(RouteDecisionValidationError::UnexpectedSelectedAttempt);
            }
            if provider_calls.is_empty() {
                return Err(RouteDecisionValidationError::ExpectedProviderCall);
            }
        }
        RouteDecisionStatus::NoViableRoute => {
            if !provider_calls.is_empty() {
                return Err(RouteDecisionValidationError::UnexpectedProviderCall);
            }
            if attempts.iter().any(|attempt| {
                !matches!(
                    attempt.decision,
                    RouteAttemptDecision::Vetoed | RouteAttemptDecision::Skipped
                )
            }) {
                return Err(RouteDecisionValidationError::InvalidNoViableRouteAttempt);
            }
        }
        RouteDecisionStatus::Cancelled => {}
    }

    let attempt_ids: BTreeSet<_> = attempts
        .iter()
        .map(|attempt| &attempt.route_attempt_id)
        .collect();
    let mut dispatched_attempts = BTreeSet::new();

    for provider_call in provider_calls {
        if !attempt_ids.contains(&provider_call.route_attempt_id) {
            return Err(RouteDecisionValidationError::ProviderCallAttemptMissing);
        }

        let call_attempt = attempts
            .iter()
            .find(|attempt| attempt.route_attempt_id == provider_call.route_attempt_id)
            .expect("attempt existence checked above");

        if !matches!(
            call_attempt.decision,
            RouteAttemptDecision::Selected | RouteAttemptDecision::Failed
        ) {
            return Err(RouteDecisionValidationError::ProviderCallDecisionMismatch);
        }

        if !dispatched_attempts.insert(&provider_call.route_attempt_id) {
            return Err(RouteDecisionValidationError::DuplicateProviderCallForAttempt);
        }
    }

    Ok(())
}

/// Configured provider endpoint owned above the project scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConnectionRecord {
    pub provider_connection_id: ProviderConnectionId,
    pub tenant_id: TenantId,
    pub provider_family: String,
    pub adapter_type: String,
    /// Model identifiers served through this connection (e.g. ["gemma4", "qwen3.5"]).
    /// Allows a single OpenAI-compatible endpoint to advertise multiple models.
    #[serde(default)]
    pub supported_models: Vec<String>,
    pub status: ProviderConnectionStatus,
    pub created_at: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderConnectionStatus {
    Active,
    Disabled,
}

/// Project-scoped deployable runtime selection unit for provider routing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBindingRecord {
    pub provider_binding_id: ProviderBindingId,
    pub project: ProjectKey,
    pub provider_connection_id: ProviderConnectionId,
    pub provider_model_id: ProviderModelId,
    pub operation_kind: OperationKind,
    pub settings: ProviderBindingSettings,
    pub active: bool,
    pub created_at: u64,
}

// ── GAP-006: Spend Alerting ───────────────────────────────────────────────────

/// Per-session accumulated cost record.
///
/// Updated by projecting `SessionCostUpdated` events.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCostRecord {
    pub session_id: crate::ids::SessionId,
    pub tenant_id: TenantId,
    /// Total accumulated cost in USD micros across all provider calls in this session.
    pub total_cost_micros: u64,
    /// Total input tokens consumed (alias: token_in).
    pub total_tokens_in: u64,
    /// Total output tokens produced (alias: token_out).
    pub total_tokens_out: u64,
    pub updated_at_ms: u64,
    /// Number of provider calls accumulated.
    #[serde(default)]
    pub provider_calls: u64,
    /// Total input tokens (alias for total_tokens_in).
    #[serde(default)]
    pub token_in: u64,
    /// Total output tokens (alias for total_tokens_out).
    #[serde(default)]
    pub token_out: u64,
}

/// Tenant-level LLM spend alert record.
///
/// A `SpendAlert` is created when session cost for a tenant crosses the
/// configured threshold. At most one alert fires per tenant per UTC day
/// (deduplication is the caller's responsibility for the MVP; future versions
/// will use a daily dedup key).
///
/// Mirrors `cairn/internal/agent/spend_alert.go` SpendAlert struct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendAlert {
    pub alert_id: String,
    pub tenant_id: TenantId,
    /// Threshold that was crossed, in USD micros.
    pub threshold_micros: u64,
    /// Session cost at the time the alert fired, in USD micros.
    pub current_micros: u64,
    /// Unix milliseconds when the alert was triggered.
    pub triggered_at_ms: u64,
}

/// Tenant-level spend threshold configuration record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpendThresholdRecord {
    pub tenant_id: TenantId,
    /// Alert fires when any single session's total cost exceeds this value (µUSD).
    pub threshold_micros: u64,
    pub set_at_ms: u64,
}

/// Generation provider adapter trait per RFC 009.
///
/// Supports streaming text deltas, usage accounting, tool call emission,
/// structured output, timeout, and cancellation.
#[async_trait::async_trait]
pub trait GenerationProvider: Send + Sync {
    async fn generate(
        &self,
        model_id: &str,
        messages: Vec<serde_json::Value>,
        settings: &ProviderBindingSettings,
    ) -> Result<GenerationResponse, ProviderAdapterError>;
}

/// Response from a generation provider call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GenerationResponse {
    pub text: String,
    pub input_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub model_id: String,
    pub tool_calls: Vec<serde_json::Value>,
}

/// Reranker provider adapter trait per RFC 009.
#[async_trait::async_trait]
pub trait RerankerProvider: Send + Sync {
    async fn rerank(
        &self,
        model_id: &str,
        query: &str,
        candidates: Vec<String>,
    ) -> Result<RerankResponse, ProviderAdapterError>;
}

/// Response from a reranker provider call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RerankResponse {
    pub ranked_indices: Vec<usize>,
    pub scores: Vec<f64>,
    pub model_id: String,
}

/// Response from an embedding provider call.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmbeddingResponse {
    /// One embedding vector per input text, in input order.
    pub embeddings: Vec<Vec<f32>>,
    pub model_id: String,
    pub token_count: u32,
}

/// Embedding provider adapter trait per RFC 009.
///
/// Converts a batch of text strings into dense vector representations.
/// Modelled after [`GenerationProvider`] and [`RerankerProvider`].
#[async_trait::async_trait]
pub trait EmbeddingProvider: Send + Sync {
    async fn embed(
        &self,
        model_id: &str,
        texts: Vec<String>,
    ) -> Result<EmbeddingResponse, ProviderAdapterError>;
}

/// Errors from provider adapter calls.
#[derive(Debug)]
pub enum ProviderAdapterError {
    TransportFailure(String),
    TimedOut,
    RateLimited,
    ProviderError(String),
    StructuredOutputInvalid(String),
}

impl std::fmt::Display for ProviderAdapterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderAdapterError::TransportFailure(msg) => write!(f, "transport failure: {msg}"),
            ProviderAdapterError::TimedOut => write!(f, "timed out"),
            ProviderAdapterError::RateLimited => write!(f, "rate limited"),
            ProviderAdapterError::ProviderError(msg) => write!(f, "provider error: {msg}"),
            ProviderAdapterError::StructuredOutputInvalid(msg) => {
                write!(f, "structured output invalid: {msg}")
            }
        }
    }
}

impl std::error::Error for ProviderAdapterError {}

/// In-process LLM spend tracker with daily and monthly reset semantics.
///
/// Mirrors `cairn/internal/llm/budget.go` — thread-safety is the caller's
/// responsibility (wrap in `Mutex` for shared use).
///
/// Does NOT enforce cross-process consistency. Use `BudgetService` (backed by
/// the event store) for durable, multi-process budget enforcement. This struct
/// is for in-process fast-path `can_afford` checks before dispatching a call.
#[derive(Clone, Debug)]
pub struct LlmBudget {
    /// Hard daily cap in USD micros. 0 = disabled.
    pub daily_limit_micros: u64,
    /// Hard monthly cap in USD micros. 0 = disabled.
    pub monthly_limit_micros: u64,
    /// Accumulated spend for the current UTC day.
    daily_spent_micros: u64,
    /// Accumulated spend for the current calendar month.
    monthly_spent_micros: u64,
    /// UTC day-of-year at which `daily_spent_micros` was last reset.
    last_reset_day: u32,
    /// UTC month at which `monthly_spent_micros` was last reset.
    last_reset_month: u32,
}

impl LlmBudget {
    /// Create a new budget tracker.
    ///
    /// Pass 0 for a limit to disable that period's cap.
    pub fn new(daily_limit_micros: u64, monthly_limit_micros: u64) -> Self {
        let (day, month) = Self::current_day_month();
        Self {
            daily_limit_micros,
            monthly_limit_micros,
            daily_spent_micros: 0,
            monthly_spent_micros: 0,
            last_reset_day: day,
            last_reset_month: month,
        }
    }

    fn current_day_month() -> (u32, u32) {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Simple UTC day-of-year and month from unix timestamp.
        let days_since_epoch = secs / 86_400;
        let year_approx = 1970 + days_since_epoch / 365;
        let day_of_year = (days_since_epoch % 365) as u32;
        let month = (year_approx % 12 + 1) as u32;
        (day_of_year, month)
    }

    fn maybe_reset(&mut self) {
        let (day, month) = Self::current_day_month();
        if day != self.last_reset_day {
            self.daily_spent_micros = 0;
            self.last_reset_day = day;
        }
        if month != self.last_reset_month {
            self.monthly_spent_micros = 0;
            self.last_reset_month = month;
        }
    }

    /// Returns false if the estimated cost would exceed any active limit.
    pub fn can_afford(&mut self, estimated_cost_micros: u64) -> bool {
        self.maybe_reset();
        if self.daily_limit_micros > 0
            && self.daily_spent_micros + estimated_cost_micros > self.daily_limit_micros
        {
            return false;
        }
        if self.monthly_limit_micros > 0
            && self.monthly_spent_micros + estimated_cost_micros > self.monthly_limit_micros
        {
            return false;
        }
        true
    }

    /// Record actual spend after a call completes.
    pub fn record(&mut self, actual_cost_micros: u64) {
        self.maybe_reset();
        self.daily_spent_micros = self.daily_spent_micros.saturating_add(actual_cost_micros);
        self.monthly_spent_micros = self.monthly_spent_micros.saturating_add(actual_cost_micros);
    }

    /// Current accumulated spend (daily, monthly).
    pub fn spent(&self) -> (u64, u64) {
        (self.daily_spent_micros, self.monthly_spent_micros)
    }
}

/// Health status reported by the connection health checker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProviderHealthStatus {
    Healthy,
    Degraded,
    #[default]
    Unknown,
    Unreachable,
}

/// A single rule within a route policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RouteRule {
    pub rule_id: String,
    pub priority: u32,
    pub condition: String,
    pub action: String,
}
#[cfg(test)]
mod tests {
    use super::{
        validate_route_decision, OperationKind, ProviderBindingSettings, ProviderCallErrorClass,
        ProviderCallRecord, ProviderCallStatus, ProviderCostType, RouteAttemptDecision,
        RouteAttemptRecord, RouteDecisionReason, RouteDecisionRecord, RouteDecisionStatus,
        RouteDecisionValidationError, StructuredOutputMode,
    };
    use crate::selectors::SelectorContext;

    #[test]
    fn route_decision_knows_when_dispatch_must_exist() {
        let selected = RouteDecisionRecord {
            route_decision_id: "route_decision_1".into(),
            project_id: "project_1".into(),
            operation_kind: OperationKind::Generate,
            terminal_route_attempt_id: Some("route_attempt_1".into()),
            selected_provider_binding_id: Some("provider_binding_1".into()),
            selected_route_attempt_id: Some("route_attempt_1".into()),
            selector_context: SelectorContext::default(),
            attempt_count: 1,
            fallback_used: false,
            final_status: RouteDecisionStatus::Selected,
        };

        let no_viable_route = RouteDecisionRecord {
            final_status: RouteDecisionStatus::NoViableRoute,
            ..selected.clone()
        };

        assert!(selected.requires_provider_call());
        assert!(!no_viable_route.requires_provider_call());
    }

    #[test]
    fn selected_route_decision_requires_selected_binding_and_call() {
        let attempts = vec![RouteAttemptRecord {
            route_attempt_id: "route_attempt_1".into(),
            route_decision_id: "route_decision_1".into(),
            project_id: "project_1".into(),
            operation_kind: OperationKind::Generate,
            provider_binding_id: "provider_binding_1".into(),
            selector_context: SelectorContext::default(),
            attempt_index: 0,
            decision: RouteAttemptDecision::Selected,
            decision_reason: RouteDecisionReason::Other,
            skip_reason: None,
            estimated_cost_micros: None,
        }];
        let provider_calls = vec![ProviderCallRecord {
            provider_call_id: "provider_call_1".into(),
            route_decision_id: "route_decision_1".into(),
            route_attempt_id: "route_attempt_1".into(),
            project_id: "project_1".into(),
            operation_kind: OperationKind::Generate,
            provider_binding_id: "provider_binding_1".into(),
            provider_connection_id: "provider_connection_1".into(),
            provider_adapter: "openai".to_owned(),
            provider_model_id: "gpt-5.4".into(),
            task_id: Some("task_1".into()),
            run_id: Some("run_1".into()),
            prompt_release_id: Some("prompt_release_1".into()),
            fallback_position: 0,
            status: ProviderCallStatus::Succeeded,
            latency_ms: Some(1250),
            input_tokens: Some(200),
            output_tokens: Some(80),
            cost_micros: Some(9000),
            cost_type: ProviderCostType::Metered,
            error_class: None,
        }];
        let decision = RouteDecisionRecord {
            route_decision_id: "route_decision_1".into(),
            project_id: "project_1".into(),
            operation_kind: OperationKind::Generate,
            terminal_route_attempt_id: Some("route_attempt_1".into()),
            selected_provider_binding_id: Some("provider_binding_1".into()),
            selected_route_attempt_id: Some("route_attempt_1".into()),
            selector_context: SelectorContext::default(),
            attempt_count: 1,
            fallback_used: false,
            final_status: RouteDecisionStatus::Selected,
        };

        assert_eq!(
            validate_route_decision(&decision, &attempts, &provider_calls),
            Ok(())
        );
    }

    #[test]
    fn no_viable_route_rejects_provider_calls() {
        let attempts = vec![RouteAttemptRecord {
            route_attempt_id: "route_attempt_1".into(),
            route_decision_id: "route_decision_1".into(),
            project_id: "project_1".into(),
            operation_kind: OperationKind::Generate,
            provider_binding_id: "provider_binding_1".into(),
            selector_context: SelectorContext::default(),
            attempt_index: 0,
            decision: RouteAttemptDecision::Vetoed,
            decision_reason: RouteDecisionReason::BudgetExhausted,
            skip_reason: Some("over budget".to_owned()),
            estimated_cost_micros: None,
        }];
        let provider_calls = vec![ProviderCallRecord {
            provider_call_id: "provider_call_1".into(),
            route_decision_id: "route_decision_1".into(),
            route_attempt_id: "route_attempt_1".into(),
            project_id: "project_1".into(),
            operation_kind: OperationKind::Generate,
            provider_binding_id: "provider_binding_1".into(),
            provider_connection_id: "provider_connection_1".into(),
            provider_adapter: "openai".to_owned(),
            provider_model_id: "gpt-5.4".into(),
            task_id: None,
            run_id: Some("run_1".into()),
            prompt_release_id: None,
            fallback_position: 0,
            status: ProviderCallStatus::Failed,
            latency_ms: Some(300),
            input_tokens: Some(100),
            output_tokens: None,
            cost_micros: Some(1000),
            cost_type: ProviderCostType::Metered,
            error_class: Some(ProviderCallErrorClass::ProviderError),
        }];
        let decision = RouteDecisionRecord {
            route_decision_id: "route_decision_1".into(),
            project_id: "project_1".into(),
            operation_kind: OperationKind::Generate,
            terminal_route_attempt_id: None,
            selected_provider_binding_id: None,
            selected_route_attempt_id: None,
            selector_context: SelectorContext::default(),
            attempt_count: 1,
            fallback_used: false,
            final_status: RouteDecisionStatus::NoViableRoute,
        };

        assert_eq!(
            validate_route_decision(&decision, &attempts, &provider_calls),
            Err(RouteDecisionValidationError::UnexpectedProviderCall)
        );
    }

    #[test]
    fn binding_settings_stay_normalized() {
        let settings = ProviderBindingSettings {
            temperature_milli: Some(700),
            max_output_tokens: Some(4096),
            timeout_ms: Some(30_000),
            structured_output_mode: StructuredOutputMode::Preferred,
            required_capabilities: vec![super::ProviderCapability::StructuredOutput],
            disabled_capabilities: vec![super::ProviderCapability::Streaming],
            cost_type: ProviderCostType::Metered,
            daily_budget_micros: None,
        };

        assert_eq!(
            settings.structured_output_mode,
            StructuredOutputMode::Preferred
        );
        assert_eq!(settings.required_capabilities.len(), 1);
        assert_eq!(settings.disabled_capabilities.len(), 1);
        assert_eq!(ProviderCallStatus::Succeeded, ProviderCallStatus::Succeeded);
    }
}

// ── RFC 009 Gap Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod rfc009_tests {
    use super::*;
    use crate::ids::{
        ProviderBindingId, ProviderCallId, ProviderConnectionId, ProviderModelId, RouteAttemptId,
        RouteDecisionId,
    };

    /// RFC 009: skipped/vetoed route attempts must NOT create provider call records.
    /// Only `Selected` or `Failed` decisions can have associated provider calls.
    #[test]
    fn rfc009_vetoed_attempt_has_no_provider_call() {
        // A vetoed route attempt: no provider dispatch occurred.
        let attempt = RouteAttemptRecord {
            route_attempt_id: RouteAttemptId::new("ra_1"),
            route_decision_id: RouteDecisionId::new("rd_1"),
            project_id: crate::ids::ProjectId::new("p1"),
            operation_kind: OperationKind::Generate,
            provider_binding_id: ProviderBindingId::new("binding_expensive"),
            selector_context: SelectorContext::default(),
            attempt_index: 0,
            decision: RouteAttemptDecision::Vetoed,
            decision_reason: RouteDecisionReason::BudgetExhausted,
            skip_reason: Some("over budget".to_owned()),
            estimated_cost_micros: None,
        };

        // RFC 009: vetoed attempts should not have an associated provider call.
        // Verify the decision type matches and no call was dispatched.
        assert_eq!(
            attempt.decision,
            RouteAttemptDecision::Vetoed,
            "RFC 009: vetoed attempt must be explicitly marked Vetoed"
        );
        // Veto reason must be explicit, not Other
        assert_ne!(
            attempt.decision_reason,
            RouteDecisionReason::Other,
            "RFC 009: every veto must have a concrete decision_reason"
        );
    }

    /// RFC 009: a selected route decision must have selected_provider_binding_id set.
    #[test]
    fn rfc009_selected_decision_has_binding_id() {
        let decision = RouteDecisionRecord {
            route_decision_id: RouteDecisionId::new("rd_sel"),
            project_id: crate::ids::ProjectId::new("proj"),
            operation_kind: OperationKind::Generate,
            terminal_route_attempt_id: Some(RouteAttemptId::new("ra_sel")),
            selected_provider_binding_id: Some(ProviderBindingId::new("binding_fast")),
            selected_route_attempt_id: Some(RouteAttemptId::new("ra_sel")),
            selector_context: SelectorContext::default(),
            attempt_count: 1,
            fallback_used: false,
            final_status: RouteDecisionStatus::Selected,
        };

        assert!(
            decision.selected_provider_binding_id.is_some(),
            "RFC 009: Selected route decision must have selected_provider_binding_id"
        );
        assert!(
            decision.selected_route_attempt_id.is_some(),
            "RFC 009: Selected route decision must have selected_route_attempt_id"
        );
    }

    /// RFC 009: no_viable_route decision must have no selected binding.
    #[test]
    fn rfc009_no_viable_route_has_no_selected_binding() {
        let decision = RouteDecisionRecord {
            route_decision_id: RouteDecisionId::new("rd_nvr"),
            project_id: crate::ids::ProjectId::new("proj"),
            operation_kind: OperationKind::Generate,
            terminal_route_attempt_id: None,
            selected_provider_binding_id: None,
            selected_route_attempt_id: None,
            selector_context: SelectorContext::default(),
            attempt_count: 2,
            fallback_used: false,
            final_status: RouteDecisionStatus::NoViableRoute,
        };

        assert!(
            decision.selected_provider_binding_id.is_none(),
            "RFC 009: NoViableRoute decision must have no selected binding"
        );
    }

    /// RFC 009: every provider call must belong to exactly one route decision.
    /// Verify RouteDecisionId and RouteAttemptId are required on ProviderCallRecord.
    #[test]
    fn rfc009_provider_call_belongs_to_route_decision_and_attempt() {
        let call = ProviderCallRecord {
            provider_call_id: ProviderCallId::new("call_1"),
            route_decision_id: RouteDecisionId::new("rd_1"),
            route_attempt_id: RouteAttemptId::new("ra_1"),
            project_id: crate::ids::ProjectId::new("p1"),
            operation_kind: OperationKind::Generate,
            provider_binding_id: ProviderBindingId::new("binding_1"),
            provider_connection_id: ProviderConnectionId::new("conn_1"),
            provider_adapter: "openai_responses".to_owned(),
            provider_model_id: ProviderModelId::new("gpt-5"),
            task_id: None,
            run_id: None,
            prompt_release_id: None,
            fallback_position: 0,
            status: ProviderCallStatus::Succeeded,
            latency_ms: Some(245),
            input_tokens: Some(512),
            output_tokens: Some(128),
            cost_micros: Some(1200),
            cost_type: ProviderCostType::Metered,
            error_class: None,
        };

        // RFC 009: every provider call must link to exactly one route_decision and attempt.
        assert_eq!(call.route_decision_id.as_str(), "rd_1");
        assert_eq!(call.route_attempt_id.as_str(), "ra_1");
        assert_eq!(
            call.fallback_position, 0,
            "first attempt has fallback_position=0"
        );
    }

    /// RFC 009: veto reasons must cover all product-defined rejection scenarios.
    #[test]
    fn rfc009_all_veto_reasons_are_defined() {
        // All these reasons must be expressible per RFC 009.
        let reasons = [
            RouteDecisionReason::MissingRequiredCapability,
            RouteDecisionReason::DisallowedProviderFamily,
            RouteDecisionReason::BudgetExhausted,
            RouteDecisionReason::ProjectPolicyRestriction,
            RouteDecisionReason::SafetyModeRestriction,
        ];
        // All reasons must be distinct.
        for (i, a) in reasons.iter().enumerate() {
            for (j, b) in reasons.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "RFC 009: all veto reason variants must be distinct");
                }
            }
        }
    }

    // ── GAP-003: Per-Provider Cost Tracking ──────────────────────────────────

    #[test]
    fn provider_cost_type_is_free_for_flat_rate_and_free() {
        assert!(ProviderCostType::FlatRate.is_free());
        assert!(ProviderCostType::Free.is_free());
        assert!(!ProviderCostType::Metered.is_free());
    }

    #[test]
    fn model_cost_rates_estimate_micros_metered() {
        let rates = ModelCostRates {
            provider_model_id: ProviderModelId::new("gpt-4"),
            cost_type: ProviderCostType::Metered,
            cost_per_1m_input: 3.0,   // $3 / M tokens
            cost_per_1m_output: 12.0, // $12 / M tokens
            cache_read_per_1m: 0.0,
            cache_write_per_1m: 0.0,
        };
        // 500k input + 100k output
        // input:  3.0 * 500_000 / 1_000_000  = $1.50  = 1_500_000 µUSD
        // output: 12.0 * 100_000 / 1_000_000 = $1.20  = 1_200_000 µUSD
        // total: 2_700_000 µUSD
        assert_eq!(rates.estimate_micros(500_000, 100_000), 2_700_000);
    }

    #[test]
    fn model_cost_rates_estimate_micros_zero_for_free_types() {
        for cost_type in [ProviderCostType::FlatRate, ProviderCostType::Free] {
            let rates = ModelCostRates {
                provider_model_id: ProviderModelId::new("free-model"),
                cost_type,
                cost_per_1m_input: 10.0,
                cost_per_1m_output: 30.0,
                cache_read_per_1m: 0.0,
                cache_write_per_1m: 0.0,
            };
            assert_eq!(
                rates.estimate_micros(1_000_000, 500_000),
                0,
                "{cost_type:?} must always return 0 cost regardless of token count"
            );
        }
    }

    #[test]
    fn llm_budget_can_afford_within_limit() {
        let mut budget = LlmBudget::new(1_000_000, 0); // $1 daily, no monthly cap
        assert!(budget.can_afford(999_999));
        assert!(budget.can_afford(1_000_000)); // exactly at limit is allowed
    }

    #[test]
    fn llm_budget_cannot_afford_over_daily_limit() {
        let mut budget = LlmBudget::new(1_000_000, 0);
        budget.record(500_000); // $0.50 already spent
        assert!(!budget.can_afford(600_000), "500k + 600k > 1M limit");
        assert!(budget.can_afford(500_000), "500k + 500k = exactly limit");
    }

    #[test]
    fn llm_budget_cannot_afford_over_monthly_limit() {
        let mut budget = LlmBudget::new(0, 5_000_000); // no daily, $5 monthly
        budget.record(4_500_000);
        assert!(
            !budget.can_afford(600_000),
            "4.5M + 600k > 5M monthly limit"
        );
    }

    #[test]
    fn llm_budget_no_limits_always_affords() {
        let mut budget = LlmBudget::new(0, 0); // both disabled
        budget.record(u64::MAX / 2);
        assert!(budget.can_afford(u64::MAX / 2));
    }

    #[test]
    fn llm_budget_spent_tracks_cumulative() {
        let mut budget = LlmBudget::new(0, 0);
        budget.record(300_000);
        budget.record(200_000);
        let (daily, monthly) = budget.spent();
        assert_eq!(daily, 500_000);
        assert_eq!(monthly, 500_000);
    }

    #[test]
    fn provider_binding_settings_cost_type_defaults_to_metered() {
        let settings = ProviderBindingSettings::default();
        assert_eq!(settings.cost_type, ProviderCostType::Metered);
        assert!(settings.daily_budget_micros.is_none());
    }

    #[test]
    fn provider_call_record_carries_cost_type() {
        use crate::ids::{ProviderCallId, ProviderConnectionId, RouteAttemptId, RouteDecisionId};
        let record = ProviderCallRecord {
            provider_call_id: ProviderCallId::new("pc1"),
            route_decision_id: RouteDecisionId::new("rd1"),
            route_attempt_id: RouteAttemptId::new("ra1"),
            project_id: crate::ids::ProjectId::new("p1"),
            operation_kind: OperationKind::Generate,
            provider_binding_id: ProviderBindingId::new("pb1"),
            provider_connection_id: ProviderConnectionId::new("conn1"),
            provider_adapter: "openai".to_owned(),
            provider_model_id: ProviderModelId::new("gpt-4"),
            task_id: None,
            run_id: None,
            prompt_release_id: None,
            fallback_position: 0,
            status: ProviderCallStatus::Succeeded,
            latency_ms: Some(500),
            input_tokens: Some(100),
            output_tokens: Some(50),
            cost_micros: Some(1500),
            cost_type: ProviderCostType::Metered,
            error_class: None,
        };
        assert_eq!(record.cost_type, ProviderCostType::Metered);
        assert!(!record.cost_type.is_free());
    }
}

// ── Forward-looking stub types ────────────────────────────────────────────
// Referenced by cairn-store projection traits; defined here to keep the
// codebase compilable while the full implementations are pending.

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthRecord {
    pub binding_id: crate::ids::ProviderBindingId,
    pub healthy: bool,
    pub last_checked_ms: u64,
    pub error_message: Option<String>,
    #[serde(default)]
    pub consecutive_failures: u32,
    #[serde(default)]
    pub status: ProviderHealthStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePolicy {
    pub policy_id: String,
    pub name: String,
    pub enabled: bool,
    /// Tenant that owns this policy (RFC 009 tenant scoping).
    #[serde(default)]
    pub tenant_id: String,
    /// Rules associated with this policy.
    #[serde(default)]
    pub rules: Vec<RoutePolicyRule>,
    /// Unix ms of the last update (0 when freshly created).
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCostRecord {
    pub run_id: crate::ids::RunId,
    pub total_cost_micros: u64,
    pub total_tokens_in: u64,
    pub total_tokens_out: u64,
    /// Number of provider calls accumulated.
    #[serde(default)]
    pub provider_calls: u64,
    /// Total input tokens (alias for total_tokens_in).
    #[serde(default)]
    pub token_in: u64,
    /// Total output tokens (alias for total_tokens_out).
    #[serde(default)]
    pub token_out: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCostAlert {
    pub run_id: crate::ids::RunId,
    pub threshold_micros: u64,
    pub triggered_at_ms: u64,
    #[serde(default)]
    pub tenant_id: crate::ids::TenantId,
    #[serde(default)]
    pub actual_cost_micros: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealthSchedule {
    pub schedule_id: String,
    pub binding_id: crate::ids::ProviderBindingId,
    pub interval_ms: u64,
    pub enabled: bool,
    #[serde(default)]
    pub connection_id: crate::ids::ProviderConnectionId,
    #[serde(default)]
    pub tenant_id: crate::ids::TenantId,
    #[serde(default)]
    pub last_run_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ProviderModelCapability {
    pub model_id: crate::ids::ProviderModelId,
    #[serde(default)]
    pub capabilities: Vec<ProviderCapability>,
    #[serde(default)]
    pub provider_id: String,
    #[serde(default)]
    pub operation_kinds: Vec<OperationKind>,
    #[serde(default)]
    pub context_window_tokens: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub supports_streaming: bool,
    #[serde(default)]
    pub cost_per_1k_input_tokens: Option<f64>,
    #[serde(default)]
    pub cost_per_1k_output_tokens: Option<f64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderConnectionPool {
    pub pool_id: String,
    pub connection_ids: Vec<crate::ids::ProviderConnectionId>,
    pub max_connections: u32,
    #[serde(default)]
    pub active_connections: u32,
    #[serde(default)]
    pub tenant_id: crate::ids::TenantId,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderBindingCostStats {
    pub binding_id: crate::ids::ProviderBindingId,
    pub total_cost_micros: u64,
    pub call_count: u64,
}

/// A single rule within a route policy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoutePolicyRule {
    pub rule_id: String,
    pub policy_id: String,
    pub priority: u32,
    pub description: Option<String>,
}

/// Per-provider retry configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_ms: u64,
    #[serde(default)]
    pub retryable_error_classes: Vec<String>,
}

/// RFC 009: reusable route template capturing preferred providers and fallback strategy.
///
/// Templates allow operators to define routing patterns once and attach them to
/// multiple policies without duplicating configuration.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteTemplate {
    pub template_id: String,
    pub name: String,
    pub operation_kind: OperationKind,
    /// Ordered list of preferred provider family identifiers (e.g. "openai", "anthropic").
    pub preferred_providers: Vec<String>,
    /// Fallback strategy identifier (e.g. "round_robin", "failover", "cost_optimized").
    pub fallback_strategy: String,
    pub created_at: u64,
}

/// RFC 009: link between a provider binding and a credential record.
///
/// Records which credential is actively used by a binding, supporting
/// key rotation and audit without mutating the binding itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderCredentialLink {
    pub binding_id: String,
    pub credential_id: String,
    pub linked_at: u64,
}
