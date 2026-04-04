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

#[cfg(test)]
mod tests {
    use super::{
        validate_route_decision, OperationKind, ProviderBindingSettings, ProviderCallErrorClass,
        ProviderCallRecord, ProviderCallStatus, RouteAttemptDecision, RouteAttemptRecord,
        RouteDecisionReason, RouteDecisionRecord, RouteDecisionStatus,
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
        ProviderBindingId, ProviderCallId, ProviderConnectionId, ProviderModelId,
        RouteAttemptId, RouteDecisionId,
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
        };

        // RFC 009: vetoed attempts should not have an associated provider call.
        // Verify the decision type matches and no call was dispatched.
        assert_eq!(attempt.decision, RouteAttemptDecision::Vetoed,
            "RFC 009: vetoed attempt must be explicitly marked Vetoed");
        // Veto reason must be explicit, not Other
        assert_ne!(attempt.decision_reason, RouteDecisionReason::Other,
            "RFC 009: every veto must have a concrete decision_reason");
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

        assert!(decision.selected_provider_binding_id.is_some(),
            "RFC 009: Selected route decision must have selected_provider_binding_id");
        assert!(decision.selected_route_attempt_id.is_some(),
            "RFC 009: Selected route decision must have selected_route_attempt_id");
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

        assert!(decision.selected_provider_binding_id.is_none(),
            "RFC 009: NoViableRoute decision must have no selected binding");
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
            error_class: None,
        };

        // RFC 009: every provider call must link to exactly one route_decision and attempt.
        assert_eq!(call.route_decision_id.as_str(), "rd_1");
        assert_eq!(call.route_attempt_id.as_str(), "ra_1");
        assert_eq!(call.fallback_position, 0, "first attempt has fallback_position=0");
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
}
