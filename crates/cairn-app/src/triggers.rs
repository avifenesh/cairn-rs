//! Trigger state management and event mapping.

use std::collections::HashMap;

use tokio::task::JoinSet;

use cairn_domain::{
    CorrelationId, DecisionId, ProjectKey, RunId, RuntimeEvent, SessionId, SignalId,
};
use cairn_runtime::{DefaultsService, SessionService};
use cairn_store::projections::RunRecord;

use crate::now_ms;
use crate::state::AppState;

fn run_default_key(run_id: &RunId, suffix: &str) -> String {
    format!("run:{}:{suffix}", run_id.as_str())
}

pub(crate) struct PendingTriggeredRun {
    pub(crate) trigger_id: cairn_domain::TriggerId,
    pub(crate) run_id: RunId,
    pub(crate) template: cairn_runtime::RunTemplate,
}

pub(crate) fn trigger_condition_values(
    conditions: &[cairn_runtime::TriggerCondition],
) -> Vec<serde_json::Value> {
    conditions
        .iter()
        .map(|condition| serde_json::to_value(condition).unwrap_or(serde_json::Value::Null))
        .collect()
}

pub(crate) fn trigger_conditions_from_values(
    values: &[serde_json::Value],
) -> Result<Vec<cairn_runtime::TriggerCondition>, String> {
    values
        .iter()
        .cloned()
        .map(|value| {
            serde_json::from_value(value)
                .map_err(|error| format!("invalid persisted trigger condition: {error}"))
        })
        .collect()
}

pub(crate) fn domain_trigger_skip_reason(
    reason: &cairn_runtime::SkipReason,
) -> cairn_domain::events::TriggerSkipReason {
    match reason {
        cairn_runtime::SkipReason::ConditionMismatch => {
            cairn_domain::events::TriggerSkipReason::ConditionMismatch
        }
        cairn_runtime::SkipReason::ChainTooDeep => {
            cairn_domain::events::TriggerSkipReason::ChainTooDeep
        }
        cairn_runtime::SkipReason::AlreadyFired => {
            cairn_domain::events::TriggerSkipReason::AlreadyFired
        }
        cairn_runtime::SkipReason::MissingRequiredField { field } => {
            cairn_domain::events::TriggerSkipReason::MissingRequiredField {
                field: field.clone(),
            }
        }
    }
}

pub(crate) fn domain_trigger_suspension_reason(
    reason: &cairn_runtime::SuspensionReason,
) -> cairn_domain::events::TriggerSuspensionReason {
    match reason {
        cairn_runtime::SuspensionReason::RateLimitExceeded => {
            cairn_domain::events::TriggerSuspensionReason::RateLimitExceeded
        }
        cairn_runtime::SuspensionReason::BudgetExceeded => {
            cairn_domain::events::TriggerSuspensionReason::BudgetExceeded
        }
        cairn_runtime::SuspensionReason::RepeatedFailures { failure_count } => {
            cairn_domain::events::TriggerSuspensionReason::RepeatedFailures {
                failure_count: *failure_count,
            }
        }
        cairn_runtime::SuspensionReason::OperatorPaused => {
            cairn_domain::events::TriggerSuspensionReason::OperatorPaused
        }
    }
}

pub(crate) fn runtime_trigger_suspension_reason(
    reason: &cairn_domain::events::TriggerSuspensionReason,
) -> cairn_runtime::SuspensionReason {
    match reason {
        cairn_domain::events::TriggerSuspensionReason::RateLimitExceeded => {
            cairn_runtime::SuspensionReason::RateLimitExceeded
        }
        cairn_domain::events::TriggerSuspensionReason::BudgetExceeded => {
            cairn_runtime::SuspensionReason::BudgetExceeded
        }
        cairn_domain::events::TriggerSuspensionReason::RepeatedFailures { failure_count } => {
            cairn_runtime::SuspensionReason::RepeatedFailures {
                failure_count: *failure_count,
            }
        }
        cairn_domain::events::TriggerSuspensionReason::OperatorPaused => {
            cairn_runtime::SuspensionReason::OperatorPaused
        }
    }
}

pub(crate) fn runtime_event_for_run_template_created(
    template: &cairn_runtime::RunTemplate,
) -> RuntimeEvent {
    RuntimeEvent::RunTemplateCreated(cairn_domain::events::RunTemplateCreated {
        project: template.project.clone(),
        template_id: template.id.clone(),
        name: template.name.clone(),
        description: template.description.clone(),
        default_mode: template.default_mode.clone(),
        system_prompt: template.system_prompt.clone(),
        initial_user_message: template.initial_user_message.clone(),
        plugin_allowlist: template.plugin_allowlist.clone(),
        tool_allowlist: template.tool_allowlist.clone(),
        budget_max_tokens: template.budget.max_tokens,
        budget_max_wall_clock_ms: template.budget.max_wall_clock_ms,
        budget_max_iterations: template.budget.max_iterations,
        budget_exploration_budget_share: template.budget.exploration_budget_share,
        sandbox_hint: template.sandbox_hint.clone(),
        required_fields: template.required_fields.clone(),
        created_by: template.created_by.clone(),
        created_at: template.created_at,
    })
}

pub(crate) fn runtime_event_for_trigger_created(trigger: &cairn_runtime::Trigger) -> RuntimeEvent {
    RuntimeEvent::TriggerCreated(cairn_domain::events::TriggerCreated {
        project: trigger.project.clone(),
        trigger_id: trigger.id.clone(),
        name: trigger.name.clone(),
        description: trigger.description.clone(),
        signal_type: trigger.signal_pattern.signal_type.clone(),
        plugin_id: trigger.signal_pattern.plugin_id.clone(),
        conditions: trigger_condition_values(&trigger.conditions),
        run_template_id: trigger.run_template_id.clone(),
        max_per_minute: trigger.rate_limit.max_per_minute,
        max_burst: trigger.rate_limit.max_burst,
        max_chain_depth: trigger.max_chain_depth,
        created_by: trigger.created_by.clone(),
        created_at: trigger.created_at,
    })
}

pub(crate) fn runtime_event_for_trigger_service_event(
    project: &ProjectKey,
    event: &cairn_runtime::TriggerEvent,
) -> Option<RuntimeEvent> {
    match event {
        cairn_runtime::TriggerEvent::TriggerEnabled { trigger_id, by, at } => Some(
            RuntimeEvent::TriggerEnabled(cairn_domain::events::TriggerEnabled {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                by: by.clone(),
                at: *at,
            }),
        ),
        cairn_runtime::TriggerEvent::TriggerDisabled {
            trigger_id,
            by,
            reason,
            at,
        } => Some(RuntimeEvent::TriggerDisabled(
            cairn_domain::events::TriggerDisabled {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                by: by.clone(),
                reason: reason.clone(),
                at: *at,
            },
        )),
        cairn_runtime::TriggerEvent::TriggerSuspended {
            trigger_id,
            reason,
            at,
        } => Some(RuntimeEvent::TriggerSuspended(
            cairn_domain::events::TriggerSuspended {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                reason: domain_trigger_suspension_reason(reason),
                at: *at,
            },
        )),
        cairn_runtime::TriggerEvent::TriggerResumed { trigger_id, at } => Some(
            RuntimeEvent::TriggerResumed(cairn_domain::events::TriggerResumed {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                at: *at,
            }),
        ),
        cairn_runtime::TriggerEvent::TriggerDeleted { trigger_id, by, at } => Some(
            RuntimeEvent::TriggerDeleted(cairn_domain::events::TriggerDeleted {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                by: by.clone(),
                at: *at,
            }),
        ),
        cairn_runtime::TriggerEvent::TriggerFired {
            trigger_id,
            signal_id,
            signal_type,
            run_id,
            chain_depth,
            fired_at,
        } => Some(RuntimeEvent::TriggerFired(
            cairn_domain::events::TriggerFired {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                signal_id: signal_id.clone(),
                signal_type: signal_type.clone(),
                run_id: run_id.clone(),
                chain_depth: *chain_depth,
                fired_at: *fired_at,
            },
        )),
        cairn_runtime::TriggerEvent::TriggerSkipped {
            trigger_id,
            signal_id,
            reason,
            skipped_at,
        } => Some(RuntimeEvent::TriggerSkipped(
            cairn_domain::events::TriggerSkipped {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                signal_id: signal_id.clone(),
                reason: domain_trigger_skip_reason(reason),
                skipped_at: *skipped_at,
            },
        )),
        cairn_runtime::TriggerEvent::TriggerDenied {
            trigger_id,
            signal_id,
            decision_id,
            reason,
            denied_at,
        } => Some(RuntimeEvent::TriggerDenied(
            cairn_domain::events::TriggerDenied {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                signal_id: signal_id.clone(),
                decision_id: decision_id.clone(),
                reason: reason.clone(),
                denied_at: *denied_at,
            },
        )),
        cairn_runtime::TriggerEvent::TriggerRateLimited {
            trigger_id,
            signal_id,
            bucket_remaining,
            bucket_capacity,
            rate_limited_at,
        } => Some(RuntimeEvent::TriggerRateLimited(
            cairn_domain::events::TriggerRateLimited {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                signal_id: signal_id.clone(),
                bucket_remaining: *bucket_remaining,
                bucket_capacity: *bucket_capacity,
                rate_limited_at: *rate_limited_at,
            },
        )),
        cairn_runtime::TriggerEvent::TriggerPendingApproval {
            trigger_id,
            signal_id,
            approval_id,
            pending_at,
        } => Some(RuntimeEvent::TriggerPendingApproval(
            cairn_domain::events::TriggerPendingApproval {
                project: project.clone(),
                trigger_id: trigger_id.clone(),
                signal_id: signal_id.clone(),
                approval_id: approval_id.clone(),
                pending_at: *pending_at,
            },
        )),
        cairn_runtime::TriggerEvent::RunTemplateDeleted {
            template_id,
            by,
            at,
        } => Some(RuntimeEvent::RunTemplateDeleted(
            cairn_domain::events::RunTemplateDeleted {
                project: project.clone(),
                template_id: template_id.clone(),
                by: by.clone(),
                at: *at,
            },
        )),
        cairn_runtime::TriggerEvent::TriggerCreated { .. }
        | cairn_runtime::TriggerEvent::TriggerUpdated { .. }
        | cairn_runtime::TriggerEvent::RunTemplateCreated { .. }
        | cairn_runtime::TriggerEvent::RunTemplateUpdated { .. } => None,
    }
}

pub(crate) async fn persist_trigger_run_defaults(
    state: &AppState,
    project: &ProjectKey,
    run_id: &RunId,
    trigger_id: &cairn_domain::TriggerId,
    template: &cairn_runtime::RunTemplate,
) -> Result<(), cairn_runtime::RuntimeError> {
    let scope = cairn_domain::tenancy::Scope::Tenant;
    let scope_id = project.tenant_id.as_str().to_owned();
    let goal = template
        .initial_user_message
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("Triggered run from template {}", template.name));
    let mut settings = vec![
        (
            run_default_key(run_id, "created_by_trigger_id"),
            serde_json::json!(trigger_id.as_str()),
        ),
        (
            run_default_key(run_id, "run_template_id"),
            serde_json::json!(template.id.as_str()),
        ),
        (run_default_key(run_id, "goal"), serde_json::json!(goal)),
        (
            run_default_key(run_id, "run_mode"),
            serde_json::to_value(&template.default_mode).unwrap_or(serde_json::Value::Null),
        ),
        (
            run_default_key(run_id, "system_prompt"),
            serde_json::json!(template.system_prompt.clone()),
        ),
        (
            run_default_key(run_id, "required_fields"),
            serde_json::json!(template.required_fields.clone()),
        ),
        (
            run_default_key(run_id, "template_budget"),
            serde_json::to_value(&template.budget).unwrap_or(serde_json::Value::Null),
        ),
    ];

    if let Some(initial_user_message) = template.initial_user_message.as_ref() {
        settings.push((
            run_default_key(run_id, "initial_user_message"),
            serde_json::json!(initial_user_message),
        ));
    }
    if let Some(plugin_allowlist) = template.plugin_allowlist.as_ref() {
        settings.push((
            run_default_key(run_id, "plugin_allowlist"),
            serde_json::json!(plugin_allowlist),
        ));
    }
    if let Some(tool_allowlist) = template.tool_allowlist.as_ref() {
        settings.push((
            run_default_key(run_id, "tool_allowlist"),
            serde_json::json!(tool_allowlist),
        ));
    }
    if let Some(sandbox_hint) = template.sandbox_hint.as_ref() {
        settings.push((
            run_default_key(run_id, "sandbox_hint"),
            serde_json::json!(sandbox_hint),
        ));
    }

    for (key, value) in settings {
        state
            .runtime
            .defaults
            .set(scope, scope_id.clone(), key, value)
            .await?;
    }

    Ok(())
}

pub(crate) async fn materialize_triggered_run(
    state: &AppState,
    project: &ProjectKey,
    triggered: PendingTriggeredRun,
) -> Result<RunRecord, cairn_runtime::RuntimeError> {
    let session_id = SessionId::new(format!("sess_trigger_{}", triggered.run_id.as_str()));
    match state
        .runtime
        .sessions
        .create(project, session_id.clone())
        .await
    {
        Ok(_) => {}
        Err(cairn_runtime::RuntimeError::Conflict { .. }) => {}
        Err(err) => return Err(err),
    }

    let command = cairn_domain::commands::StartRun {
        project: project.clone(),
        session_id,
        run_id: triggered.run_id,
        parent_run_id: None,
    };
    let run = state.runtime.runs.start_command(command).await?;
    persist_trigger_run_defaults(
        state,
        project,
        &run.run_id,
        &triggered.trigger_id,
        &triggered.template,
    )
    .await?;
    Ok(run)
}

pub(crate) fn build_trigger_fire_decision_request(
    project: &ProjectKey,
    trigger_id: &cairn_domain::TriggerId,
    signal_id: &SignalId,
    signal_type: &str,
) -> cairn_domain::decisions::DecisionRequest {
    cairn_domain::decisions::DecisionRequest {
        kind: cairn_domain::decisions::DecisionKind::TriggerFire {
            trigger_id: trigger_id.as_str().to_owned(),
            signal_type: signal_type.to_owned(),
        },
        principal: cairn_domain::decisions::Principal::System,
        subject: cairn_domain::decisions::DecisionSubject::Resource {
            resource_type: "signal".to_owned(),
            resource_id: signal_id.as_str().to_owned(),
        },
        scope: project.clone(),
        cost_estimate: None,
        requested_at: now_ms(),
        correlation_id: CorrelationId::new(format!(
            "trigger_fire_{}_{}",
            trigger_id.as_str(),
            signal_id.as_str()
        )),
    }
}

pub(crate) fn unavailable_trigger_decision(
    trigger_id: &cairn_domain::TriggerId,
    reason: String,
) -> cairn_runtime::services::trigger_service::TriggerDecisionOutcome {
    cairn_runtime::services::trigger_service::TriggerDecisionOutcome::Denied {
        decision_id: DecisionId::new(format!(
            "dec_trigger_unavailable_{}_{}",
            trigger_id.as_str(),
            now_ms()
        )),
        reason,
    }
}

pub(crate) async fn trigger_decision_outcomes_for_signal(
    state: &AppState,
    project: &ProjectKey,
    signal_id: &SignalId,
    signal_type: &str,
    candidate_trigger_ids: Vec<cairn_domain::TriggerId>,
) -> HashMap<
    cairn_domain::TriggerId,
    cairn_runtime::services::trigger_service::TriggerDecisionOutcome,
> {
    let decision_service = state.runtime.decision_service.clone();
    let project = project.clone();
    let signal_id = signal_id.clone();
    let signal_type = signal_type.to_owned();
    let mut outcomes = HashMap::new();
    let mut evaluations = JoinSet::new();

    for trigger_id in candidate_trigger_ids {
        let decision_service = decision_service.clone();
        let project = project.clone();
        let signal_id = signal_id.clone();
        let signal_type = signal_type.clone();
        evaluations.spawn(async move {
            let request = build_trigger_fire_decision_request(
                &project,
                &trigger_id,
                &signal_id,
                &signal_type,
            );
            let outcome = match decision_service.evaluate(request).await {
                Ok(result) => match result.outcome {
                    cairn_domain::decisions::DecisionOutcome::Allowed => {
                        cairn_runtime::services::trigger_service::TriggerDecisionOutcome::Approved {
                            decision_id: result.decision_id,
                        }
                    }
                    cairn_domain::decisions::DecisionOutcome::Denied { deny_reason, .. } => {
                        cairn_runtime::services::trigger_service::TriggerDecisionOutcome::Denied {
                            decision_id: result.decision_id,
                            reason: deny_reason,
                        }
                    }
                },
                Err(error) => {
                    tracing::warn!(
                        project = ?project,
                        trigger_id = %trigger_id,
                        signal_id = %signal_id,
                        error = %error,
                        "decision service failed during trigger fire evaluation"
                    );
                    unavailable_trigger_decision(
                        &trigger_id,
                        format!("decision_service_error: {error}"),
                    )
                }
            };
            (trigger_id, outcome)
        });
    }

    while let Some(result) = evaluations.join_next().await {
        if let Ok((trigger_id, outcome)) = result {
            outcomes.insert(trigger_id, outcome);
        }
    }

    outcomes
}
