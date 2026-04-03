use crate::ids::{
    ProjectId, PromptReleaseId, ProviderBindingId, ProviderCallId, ProviderConnectionId,
    ProviderModelId, RouteAttemptId, RouteDecisionId, RunId, TaskId,
};
use crate::selectors::SelectorContext;
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
