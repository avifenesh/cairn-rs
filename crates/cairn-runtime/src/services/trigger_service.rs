//! Trigger service — RFC 022: Binding Signals to Runs.
//!
//! A Trigger is a project-scoped declarative rule: "when a signal of type X
//! arrives matching condition Y, create a run from template Z."
//!
//! A RunTemplate is a reusable run configuration that a Trigger references.
//!
//! The trigger evaluator is a runtime worker that subscribes to the signal
//! router (RFC 015) and creates runs for matching triggers.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use cairn_domain::decisions::RunMode;
use cairn_domain::ids::{
    ApprovalId, DecisionId, OperatorId, RunId, RunTemplateId, SignalId, TriggerId,
};
use cairn_domain::tenancy::ProjectKey;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

// ── Trigger Entity (RFC 022 §"The Trigger Entity") ──────────────────────────

/// A project-scoped rule that binds signals to runs.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trigger {
    pub id: TriggerId,
    pub project: ProjectKey,
    pub name: String,
    pub description: Option<String>,
    pub signal_pattern: SignalPattern,
    pub conditions: Vec<TriggerCondition>,
    pub run_template_id: RunTemplateId,
    pub state: TriggerState,
    pub rate_limit: RateLimitConfig,
    pub max_chain_depth: u8,
    pub created_by: OperatorId,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Which signals this trigger matches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalPattern {
    /// Signal type (exact match in v1, e.g. "github.issue.labeled").
    pub signal_type: String,
    /// Optional plugin ID restriction. If set, only signals from this plugin match.
    pub plugin_id: Option<String>,
}

/// Trigger lifecycle state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum TriggerState {
    Enabled,
    Disabled {
        reason: Option<String>,
        since: u64,
    },
    Suspended {
        reason: SuspensionReason,
        since: u64,
    },
}

/// Why a trigger was automatically suspended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuspensionReason {
    RateLimitExceeded,
    BudgetExceeded,
    RepeatedFailures { failure_count: u32 },
    OperatorPaused,
}

// ── Trigger Condition DSL (RFC 022 §"The Trigger Entity") ────────────────────

/// Condition for matching a signal payload.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerCondition {
    /// JSON path equals value: payload.action == "labeled"
    Equals {
        path: String,
        value: serde_json::Value,
    },
    /// JSON path's array contains a value: payload.labels[].name contains "cairn-ready"
    Contains {
        path: String,
        value: serde_json::Value,
    },
    /// JSON path is non-null
    Exists { path: String },
    /// Negate a child condition
    Not(Box<TriggerCondition>),
}

impl Serialize for TriggerCondition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        trigger_condition_to_value(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for TriggerCondition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        trigger_condition_from_value(value).map_err(serde::de::Error::custom)
    }
}

fn trigger_condition_to_value(condition: &TriggerCondition) -> serde_json::Value {
    match condition {
        TriggerCondition::Equals { path, value } => serde_json::json!({
            "type": "equals",
            "path": path,
            "value": value,
        }),
        TriggerCondition::Contains { path, value } => serde_json::json!({
            "type": "contains",
            "path": path,
            "value": value,
        }),
        TriggerCondition::Exists { path } => serde_json::json!({
            "type": "exists",
            "path": path,
        }),
        TriggerCondition::Not(inner) => serde_json::json!({
            "type": "not",
            "condition": trigger_condition_to_value(inner),
        }),
    }
}

fn trigger_condition_from_value(value: serde_json::Value) -> Result<TriggerCondition, String> {
    let object = value
        .as_object()
        .ok_or_else(|| "trigger condition must be an object".to_owned())?;
    let kind = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "trigger condition is missing string field `type`".to_owned())?;

    match kind {
        "equals" => Ok(TriggerCondition::Equals {
            path: condition_path(object)?,
            value: condition_value(object)?,
        }),
        "contains" => Ok(TriggerCondition::Contains {
            path: condition_path(object)?,
            value: condition_value(object)?,
        }),
        "exists" => Ok(TriggerCondition::Exists {
            path: condition_path(object)?,
        }),
        "not" => {
            let nested = object
                .get("condition")
                .or_else(|| object.get("inner"))
                .cloned()
                .ok_or_else(|| {
                    "trigger condition `not` is missing object field `condition`".to_owned()
                })?;
            Ok(TriggerCondition::Not(Box::new(
                trigger_condition_from_value(nested)?,
            )))
        }
        other => Err(format!("unsupported trigger condition type `{other}`")),
    }
}

fn condition_path(object: &serde_json::Map<String, serde_json::Value>) -> Result<String, String> {
    object
        .get("path")
        .and_then(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| "trigger condition is missing string field `path`".to_owned())
}

fn condition_value(
    object: &serde_json::Map<String, serde_json::Value>,
) -> Result<serde_json::Value, String> {
    object
        .get("value")
        .cloned()
        .ok_or_else(|| "trigger condition is missing field `value`".to_owned())
}

/// Rate limit configuration for a trigger (token bucket).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateLimitConfig {
    pub max_per_minute: u32,
    pub max_burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_per_minute: 10,
            max_burst: 20,
        }
    }
}

// ── RunTemplate Entity (RFC 022 §"The RunTemplate Entity") ──────────────────

/// A reusable run configuration that a Trigger references.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunTemplate {
    pub id: RunTemplateId,
    pub project: ProjectKey,
    pub name: String,
    pub description: Option<String>,
    pub default_mode: RunMode,
    pub system_prompt: String,
    pub initial_user_message: Option<String>,
    pub plugin_allowlist: Option<Vec<String>>,
    pub tool_allowlist: Option<Vec<String>>,
    pub budget: TemplateBudget,
    pub sandbox_hint: Option<String>,
    pub required_fields: Vec<String>,
    pub created_by: OperatorId,
    pub created_at: u64,
    pub updated_at: u64,
}

/// Default budget caps for runs created from a template.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TemplateBudget {
    pub max_tokens: Option<u64>,
    pub max_wall_clock_ms: Option<u64>,
    pub max_iterations: Option<u32>,
    pub exploration_budget_share: Option<f32>,
}

// ── Trigger Events (RFC 022 §"Events") ──────────────────────────────────────

/// Events emitted by the trigger service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TriggerEvent {
    TriggerCreated {
        trigger_id: TriggerId,
        project: ProjectKey,
        signal_pattern: SignalPattern,
        run_template_id: RunTemplateId,
        created_by: OperatorId,
        created_at: u64,
    },
    TriggerUpdated {
        trigger_id: TriggerId,
        updated_by: OperatorId,
        updated_at: u64,
    },
    TriggerEnabled {
        trigger_id: TriggerId,
        by: OperatorId,
        at: u64,
    },
    TriggerDisabled {
        trigger_id: TriggerId,
        by: OperatorId,
        reason: Option<String>,
        at: u64,
    },
    TriggerSuspended {
        trigger_id: TriggerId,
        reason: SuspensionReason,
        at: u64,
    },
    TriggerResumed {
        trigger_id: TriggerId,
        at: u64,
    },
    TriggerDeleted {
        trigger_id: TriggerId,
        by: OperatorId,
        at: u64,
    },
    TriggerFired {
        trigger_id: TriggerId,
        signal_id: SignalId,
        signal_type: String,
        run_id: RunId,
        chain_depth: u8,
        fired_at: u64,
    },
    TriggerSkipped {
        trigger_id: TriggerId,
        signal_id: SignalId,
        reason: SkipReason,
        skipped_at: u64,
    },
    TriggerDenied {
        trigger_id: TriggerId,
        signal_id: SignalId,
        decision_id: DecisionId,
        reason: String,
        denied_at: u64,
    },
    TriggerRateLimited {
        trigger_id: TriggerId,
        signal_id: SignalId,
        bucket_remaining: u32,
        bucket_capacity: u32,
        rate_limited_at: u64,
    },
    TriggerPendingApproval {
        trigger_id: TriggerId,
        signal_id: SignalId,
        approval_id: ApprovalId,
        pending_at: u64,
    },
    RunTemplateCreated {
        template_id: RunTemplateId,
        project: ProjectKey,
        name: String,
        default_mode: RunMode,
        created_by: OperatorId,
        created_at: u64,
    },
    RunTemplateUpdated {
        template_id: RunTemplateId,
        updated_by: OperatorId,
        updated_at: u64,
    },
    RunTemplateDeleted {
        template_id: RunTemplateId,
        by: OperatorId,
        at: u64,
    },
}

/// Reason a trigger fire was skipped.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    ConditionMismatch,
    ChainTooDeep,
    AlreadyFired,
    MissingRequiredField { field: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TriggerPreDecisionStatus {
    Ready,
    Skipped(SkipReason),
    RateLimited { bucket_capacity: u32 },
    BudgetExceeded,
}

// ── Condition Evaluator ─────────────────────────────────────────────────────

/// Evaluate a trigger condition against a JSON payload.
pub fn evaluate_condition(condition: &TriggerCondition, payload: &serde_json::Value) -> bool {
    match condition {
        TriggerCondition::Equals { path, value } => resolve_path(payload, path) == Some(value),
        TriggerCondition::Contains { path, value } => {
            // For array paths like "labels[].name", check if any element matches
            resolve_array_path(payload, path).iter().any(|v| v == value)
        }
        TriggerCondition::Exists { path } => resolve_path(payload, path).is_some(),
        TriggerCondition::Not(inner) => !evaluate_condition(inner, payload),
    }
}

/// Evaluate all conditions — all must pass (AND semantics).
pub fn evaluate_conditions(conditions: &[TriggerCondition], payload: &serde_json::Value) -> bool {
    conditions.iter().all(|c| evaluate_condition(c, payload))
}

/// Resolve a dot-notation path to a JSON value.
/// E.g. "issue.number" resolves `{"issue": {"number": 42}}` to `42`.
fn resolve_path<'a>(value: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
    let mut current = value;
    for segment in path.split('.') {
        // Handle array access like "labels[]"
        if segment.ends_with("[]") {
            return None; // Array paths use resolve_array_path
        }
        current = current.get(segment)?;
    }
    Some(current)
}

/// Resolve a dot-notation path with array expansion.
/// E.g. "labels[].name" on `{"labels": [{"name": "bug"}, {"name": "cairn-ready"}]}`
/// returns `["bug", "cairn-ready"]`.
fn resolve_array_path(value: &serde_json::Value, path: &str) -> Vec<serde_json::Value> {
    let parts: Vec<&str> = path.splitn(2, "[].").collect();
    if parts.len() != 2 {
        // No array expansion — fall back to scalar
        return resolve_path(value, path).cloned().into_iter().collect();
    }

    let array_path = parts[0];
    let field_path = parts[1];

    let array = match resolve_path(value, array_path) {
        Some(serde_json::Value::Array(arr)) => arr,
        _ => return Vec::new(),
    };

    array
        .iter()
        .filter_map(|item| resolve_path(item, field_path).cloned())
        .collect()
}

// ── Variable Substitution (RFC 022 §"Variable Substitution") ────────────────

/// Substitute `{{path.to.field}}` placeholders with values from the signal payload.
///
/// Returns the expanded string and a list of missing required fields (if any).
pub fn substitute_variables(
    template: &str,
    payload: &serde_json::Value,
    required_fields: &[String],
) -> Result<String, Vec<String>> {
    let result = template.to_string();
    let mut missing = Vec::new();

    // Find all {{...}} patterns
    let mut start = 0;
    let mut output = String::with_capacity(template.len());

    while let Some(open) = result[start..].find("{{") {
        let abs_open = start + open;
        output.push_str(&result[start..abs_open]);

        if let Some(close) = result[abs_open + 2..].find("}}") {
            let abs_close = abs_open + 2 + close;
            let path = &result[abs_open + 2..abs_close];

            // Resolve the path
            let value = if path.contains("[].") {
                let values = resolve_array_path(payload, path);
                if values.is_empty() {
                    String::new()
                } else {
                    values
                        .iter()
                        .map(value_to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            } else {
                resolve_path(payload, path)
                    .map(value_to_string)
                    .unwrap_or_default()
            };

            output.push_str(&value);
            start = abs_close + 2;
        } else {
            // No closing braces — leave as-is
            output.push_str("{{");
            start = abs_open + 2;
        }
    }
    output.push_str(&result[start..]);

    // Check required fields
    for field in required_fields {
        if resolve_path(payload, field).is_none() {
            missing.push(field.clone());
        }
    }

    if missing.is_empty() {
        Ok(output)
    } else {
        Err(missing)
    }
}

fn value_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

// ── Decision Layer Integration (RFC 019 × RFC 022) ──────────────────────────

/// Outcome of submitting a trigger fire to the decision layer.
///
/// In production, this is the result of `DecisionService::evaluate()` for
/// `DecisionKind::TriggerFire`. In tests, callers can supply a closure
/// that returns the desired outcome.
#[derive(Clone, Debug)]
pub enum TriggerDecisionOutcome {
    /// Decision layer approved the fire.
    Approved { decision_id: DecisionId },
    /// Decision layer denied the fire.
    Denied {
        decision_id: DecisionId,
        reason: String,
    },
    /// Decision layer requires human/guardian approval before proceeding.
    PendingApproval { approval_id: ApprovalId },
}

/// Default decision function that auto-approves all trigger fires.
/// Used in tests and when no DecisionService is configured.
pub fn auto_approve_decision(
    _trigger_id: &TriggerId,
    _signal_type: &str,
) -> TriggerDecisionOutcome {
    TriggerDecisionOutcome::Approved {
        decision_id: DecisionId::new(format!("auto_{}", now_ms())),
    }
}

// ── TriggerService ──────────────────────────────────────────────────────────

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// In-memory trigger service managing triggers and run templates.
pub struct TriggerService {
    triggers: HashMap<TriggerId, Trigger>,
    templates: HashMap<RunTemplateId, RunTemplate>,
    /// Durable fire ledger: (trigger_id, signal_id) → fired_at.
    /// Prevents duplicate runs on webhook retry or signal replay.
    fire_ledger: HashMap<(TriggerId, SignalId), u64>,
    /// Per-trigger fire counts in the current rate-limit window.
    fire_counts: HashMap<TriggerId, Vec<u64>>,
    /// Per-project trigger budget: total fires in the last hour.
    project_budgets: HashMap<ProjectKey, Vec<u64>>,
    /// Default per-project trigger budget (runs/hour).
    pub default_project_budget: u32,
}

impl TriggerService {
    pub fn new() -> Self {
        Self {
            triggers: HashMap::new(),
            templates: HashMap::new(),
            fire_ledger: HashMap::new(),
            fire_counts: HashMap::new(),
            project_budgets: HashMap::new(),
            default_project_budget: 100,
        }
    }

    // ── Template CRUD ────────────────────────────────────────────────

    pub fn create_template(&mut self, template: RunTemplate) -> TriggerEvent {
        let event = TriggerEvent::RunTemplateCreated {
            template_id: template.id.clone(),
            project: template.project.clone(),
            name: template.name.clone(),
            default_mode: template.default_mode.clone(),
            created_by: template.created_by.clone(),
            created_at: template.created_at,
        };
        self.templates.insert(template.id.clone(), template);
        event
    }

    pub fn get_template(&self, id: &RunTemplateId) -> Option<&RunTemplate> {
        self.templates.get(id)
    }

    pub fn delete_template(
        &mut self,
        id: &RunTemplateId,
        by: OperatorId,
    ) -> Result<TriggerEvent, TriggerError> {
        // Block deletion if any trigger references this template
        let referencing: Vec<_> = self
            .triggers
            .values()
            .filter(|t| &t.run_template_id == id)
            .map(|t| t.id.clone())
            .collect();

        if !referencing.is_empty() {
            return Err(TriggerError::TemplateInUse {
                template_id: id.clone(),
                trigger_ids: referencing,
            });
        }

        self.templates
            .remove(id)
            .ok_or_else(|| TriggerError::TemplateNotFound(id.clone()))?;

        Ok(TriggerEvent::RunTemplateDeleted {
            template_id: id.clone(),
            by,
            at: now_ms(),
        })
    }

    pub fn list_templates_for_project(&self, project: &ProjectKey) -> Vec<&RunTemplate> {
        self.templates
            .values()
            .filter(|t| &t.project == project)
            .collect()
    }

    // ── Trigger CRUD ────────────────────────────────────────────────

    pub fn create_trigger(&mut self, trigger: Trigger) -> Result<TriggerEvent, TriggerError> {
        // Verify template exists
        if !self.templates.contains_key(&trigger.run_template_id) {
            return Err(TriggerError::TemplateNotFound(
                trigger.run_template_id.clone(),
            ));
        }

        let event = TriggerEvent::TriggerCreated {
            trigger_id: trigger.id.clone(),
            project: trigger.project.clone(),
            signal_pattern: trigger.signal_pattern.clone(),
            run_template_id: trigger.run_template_id.clone(),
            created_by: trigger.created_by.clone(),
            created_at: trigger.created_at,
        };
        self.triggers.insert(trigger.id.clone(), trigger);
        Ok(event)
    }

    pub fn get_trigger(&self, id: &TriggerId) -> Option<&Trigger> {
        self.triggers.get(id)
    }

    pub fn enable_trigger(
        &mut self,
        id: &TriggerId,
        by: OperatorId,
    ) -> Result<TriggerEvent, TriggerError> {
        let trigger = self
            .triggers
            .get_mut(id)
            .ok_or_else(|| TriggerError::TriggerNotFound(id.clone()))?;
        trigger.state = TriggerState::Enabled;
        trigger.updated_at = now_ms();
        Ok(TriggerEvent::TriggerEnabled {
            trigger_id: id.clone(),
            by,
            at: trigger.updated_at,
        })
    }

    pub fn disable_trigger(
        &mut self,
        id: &TriggerId,
        by: OperatorId,
        reason: Option<String>,
    ) -> Result<TriggerEvent, TriggerError> {
        let trigger = self
            .triggers
            .get_mut(id)
            .ok_or_else(|| TriggerError::TriggerNotFound(id.clone()))?;
        let now = now_ms();
        trigger.state = TriggerState::Disabled {
            reason: reason.clone(),
            since: now,
        };
        trigger.updated_at = now;
        Ok(TriggerEvent::TriggerDisabled {
            trigger_id: id.clone(),
            by,
            reason,
            at: now,
        })
    }

    pub fn resume_trigger(&mut self, id: &TriggerId) -> Result<TriggerEvent, TriggerError> {
        let trigger = self
            .triggers
            .get_mut(id)
            .ok_or_else(|| TriggerError::TriggerNotFound(id.clone()))?;

        if !matches!(trigger.state, TriggerState::Suspended { .. }) {
            return Err(TriggerError::NotSuspended(id.clone()));
        }

        let now = now_ms();
        trigger.state = TriggerState::Enabled;
        trigger.updated_at = now;
        Ok(TriggerEvent::TriggerResumed {
            trigger_id: id.clone(),
            at: now,
        })
    }

    /// Restore a trigger state from durable event history.
    pub fn restore_trigger_state(
        &mut self,
        id: &TriggerId,
        state: TriggerState,
        updated_at: u64,
    ) -> Result<(), TriggerError> {
        let trigger = self
            .triggers
            .get_mut(id)
            .ok_or_else(|| TriggerError::TriggerNotFound(id.clone()))?;
        trigger.state = state;
        trigger.updated_at = updated_at;
        Ok(())
    }

    pub fn delete_trigger(
        &mut self,
        id: &TriggerId,
        by: OperatorId,
    ) -> Result<TriggerEvent, TriggerError> {
        self.triggers
            .remove(id)
            .ok_or_else(|| TriggerError::TriggerNotFound(id.clone()))?;
        Ok(TriggerEvent::TriggerDeleted {
            trigger_id: id.clone(),
            by,
            at: now_ms(),
        })
    }

    pub fn list_triggers_for_project(&self, project: &ProjectKey) -> Vec<&Trigger> {
        self.triggers
            .values()
            .filter(|t| &t.project == project)
            .collect()
    }

    // ── Fire Ledger Snapshot / Restore (for recovery — RFC 020) ──────

    /// Snapshot the fire ledger for durable persistence.
    /// Returns all (trigger_id, signal_id) → fired_at entries.
    pub fn fire_ledger_snapshot(&self) -> HashMap<(TriggerId, SignalId), u64> {
        self.fire_ledger.clone()
    }

    /// Restore the fire ledger from a persisted snapshot.
    /// Used during recovery to prevent duplicate fires after restart.
    pub fn restore_fire_ledger(&mut self, ledger: HashMap<(TriggerId, SignalId), u64>) {
        self.fire_ledger = ledger;
    }

    /// Rebuild fire-ledger and rolling counters from a durable TriggerFired event.
    pub fn restore_fired_trigger(
        &mut self,
        project: &ProjectKey,
        trigger_id: &TriggerId,
        signal_id: &SignalId,
        fired_at: u64,
    ) {
        self.fire_ledger
            .insert((trigger_id.clone(), signal_id.clone()), fired_at);
        self.fire_counts
            .entry(trigger_id.clone())
            .or_default()
            .push(fired_at);
        self.project_budgets
            .entry(project.clone())
            .or_default()
            .push(fired_at);
    }

    fn matching_trigger_ids(
        &self,
        project: &ProjectKey,
        signal_type: &str,
        plugin_id: &str,
    ) -> Vec<TriggerId> {
        self.triggers
            .values()
            .filter(|trigger| {
                &trigger.project == project
                    && matches!(trigger.state, TriggerState::Enabled)
                    && trigger.signal_pattern.signal_type == signal_type
                    && trigger
                        .signal_pattern
                        .plugin_id
                        .as_ref()
                        .is_none_or(|pid| pid == plugin_id)
            })
            .map(|trigger| trigger.id.clone())
            .collect()
    }

    fn pre_decision_status(
        &self,
        project: &ProjectKey,
        trigger_id: &TriggerId,
        signal_id: &SignalId,
        payload: &serde_json::Value,
        source_run_chain_depth: Option<u8>,
        now: u64,
    ) -> Option<TriggerPreDecisionStatus> {
        let trigger = self.triggers.get(trigger_id)?;

        let ledger_key = (trigger.id.clone(), signal_id.clone());
        if self.fire_ledger.contains_key(&ledger_key) {
            return Some(TriggerPreDecisionStatus::Skipped(SkipReason::AlreadyFired));
        }

        if !evaluate_conditions(&trigger.conditions, payload) {
            return Some(TriggerPreDecisionStatus::Skipped(
                SkipReason::ConditionMismatch,
            ));
        }

        let next_depth = source_run_chain_depth.map_or(1u8, |depth| depth.saturating_add(1));
        if next_depth > trigger.max_chain_depth {
            return Some(TriggerPreDecisionStatus::Skipped(SkipReason::ChainTooDeep));
        }

        let window_start = now.saturating_sub(60_000);
        let current_trigger_count = self
            .fire_counts
            .get(trigger_id)
            .map(|counts| counts.iter().filter(|&&ts| ts > window_start).count())
            .unwrap_or(0);
        if current_trigger_count as u32 >= trigger.rate_limit.max_per_minute {
            return Some(TriggerPreDecisionStatus::RateLimited {
                bucket_capacity: trigger.rate_limit.max_per_minute,
            });
        }

        let hour_ago = now.saturating_sub(3_600_000);
        let current_project_budget = self
            .project_budgets
            .get(project)
            .map(|entries| entries.iter().filter(|&&ts| ts > hour_ago).count())
            .unwrap_or(0);
        if current_project_budget as u32 >= self.default_project_budget {
            return Some(TriggerPreDecisionStatus::BudgetExceeded);
        }

        let template = self.templates.get(&trigger.run_template_id)?;
        if let Some(field) = template
            .required_fields
            .iter()
            .find(|field| resolve_path(payload, field).is_none())
        {
            return Some(TriggerPreDecisionStatus::Skipped(
                SkipReason::MissingRequiredField {
                    field: field.clone(),
                },
            ));
        }

        Some(TriggerPreDecisionStatus::Ready)
    }

    /// Preview which triggers are eligible for decision-layer evaluation for a signal.
    ///
    /// This runs the pre-decision checks without mutating trigger state so callers
    /// can consult an async decision service outside the trigger mutex, then call
    /// `evaluate_signal()` with the resulting outcomes to apply the durable events.
    pub fn decision_candidates_for_signal(
        &self,
        project: &ProjectKey,
        signal_id: &SignalId,
        signal_type: &str,
        plugin_id: &str,
        payload: &serde_json::Value,
        source_run_chain_depth: Option<u8>,
    ) -> Vec<TriggerId> {
        let now = now_ms();
        self.matching_trigger_ids(project, signal_type, plugin_id)
            .into_iter()
            .filter(|trigger_id| {
                matches!(
                    self.pre_decision_status(
                        project,
                        trigger_id,
                        signal_id,
                        payload,
                        source_run_chain_depth,
                        now,
                    ),
                    Some(TriggerPreDecisionStatus::Ready)
                )
            })
            .collect()
    }

    // ── Trigger Evaluation ──────────────────────────────────────────

    /// Evaluate a signal against all enabled triggers in the project.
    ///
    /// The `decision_fn` callback is called for each trigger that passes
    /// condition matching, chain depth, and rate limit checks. It integrates
    /// with RFC 019's decision layer — in production this calls
    /// `DecisionService::evaluate()` with `DecisionKind::TriggerFire`.
    ///
    /// Use `auto_approve_decision` for tests or when no decision layer is configured.
    pub fn evaluate_signal(
        &mut self,
        project: &ProjectKey,
        signal_id: &SignalId,
        signal_type: &str,
        plugin_id: &str,
        payload: &serde_json::Value,
        source_run_chain_depth: Option<u8>,
        decision_fn: &dyn Fn(&TriggerId, &str) -> TriggerDecisionOutcome,
    ) -> Vec<TriggerEvent> {
        let now = now_ms();
        let mut events = Vec::new();

        let matching_triggers = self.matching_trigger_ids(project, signal_type, plugin_id);

        for trigger_id in matching_triggers {
            match self.pre_decision_status(
                project,
                &trigger_id,
                signal_id,
                payload,
                source_run_chain_depth,
                now,
            ) {
                Some(TriggerPreDecisionStatus::Ready) => {}
                Some(TriggerPreDecisionStatus::Skipped(reason)) => {
                    events.push(TriggerEvent::TriggerSkipped {
                        trigger_id,
                        signal_id: signal_id.clone(),
                        reason,
                        skipped_at: now,
                    });
                    continue;
                }
                Some(TriggerPreDecisionStatus::RateLimited { bucket_capacity }) => {
                    events.push(TriggerEvent::TriggerRateLimited {
                        trigger_id,
                        signal_id: signal_id.clone(),
                        bucket_remaining: 0,
                        bucket_capacity,
                        rate_limited_at: now,
                    });
                    continue;
                }
                Some(TriggerPreDecisionStatus::BudgetExceeded) => {
                    if let Some(trigger) = self.triggers.get_mut(&trigger_id) {
                        trigger.state = TriggerState::Suspended {
                            reason: SuspensionReason::BudgetExceeded,
                            since: now,
                        };
                        trigger.updated_at = now;
                    }
                    events.push(TriggerEvent::TriggerSuspended {
                        trigger_id,
                        reason: SuspensionReason::BudgetExceeded,
                        at: now,
                    });
                    continue;
                }
                None => continue,
            }

            // Decision layer check (RFC 019 integration).
            // The decision_fn callback simulates DecisionService::evaluate()
            // for the TriggerFire decision kind. In production, this calls
            // the actual DecisionService; in tests it can be overridden.
            let next_depth = source_run_chain_depth.map_or(1u8, |depth| depth.saturating_add(1));
            let decision_outcome = (decision_fn)(&trigger_id, signal_type);

            match &decision_outcome {
                TriggerDecisionOutcome::Approved { .. } => {
                    // Approved — proceed to fire
                }
                TriggerDecisionOutcome::Denied {
                    decision_id,
                    reason,
                } => {
                    events.push(TriggerEvent::TriggerDenied {
                        trigger_id,
                        signal_id: signal_id.clone(),
                        decision_id: decision_id.clone(),
                        reason: reason.clone(),
                        denied_at: now,
                    });
                    continue;
                }
                TriggerDecisionOutcome::PendingApproval { approval_id } => {
                    events.push(TriggerEvent::TriggerPendingApproval {
                        trigger_id,
                        signal_id: signal_id.clone(),
                        approval_id: approval_id.clone(),
                        pending_at: now,
                    });
                    continue;
                }
            }

            // Fire! Create a synthetic run_id (real impl integrates with RunService)
            let run_id = RunId::new(format!("run_trigger_{}_{}", trigger_id.as_str(), now));

            // Record in fire ledger
            self.fire_ledger
                .insert((trigger_id.clone(), signal_id.clone()), now);

            // Record fire count
            self.fire_counts
                .entry(trigger_id.clone())
                .or_default()
                .push(now);

            // Record project budget
            self.project_budgets
                .entry(project.clone())
                .or_default()
                .push(now);

            events.push(TriggerEvent::TriggerFired {
                trigger_id,
                signal_id: signal_id.clone(),
                signal_type: signal_type.to_string(),
                run_id,
                chain_depth: next_depth,
                fired_at: now,
            });
        }

        events
    }
}

impl Default for TriggerService {
    fn default() -> Self {
        Self::new()
    }
}

// ── Errors ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum TriggerError {
    TriggerNotFound(TriggerId),
    TemplateNotFound(RunTemplateId),
    TemplateInUse {
        template_id: RunTemplateId,
        trigger_ids: Vec<TriggerId>,
    },
    NotSuspended(TriggerId),
}

impl std::fmt::Display for TriggerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TriggerNotFound(id) => write!(f, "trigger not found: {id}"),
            Self::TemplateNotFound(id) => write!(f, "run template not found: {id}"),
            Self::TemplateInUse {
                template_id,
                trigger_ids,
            } => write!(
                f,
                "template {template_id} is referenced by triggers: {}",
                trigger_ids
                    .iter()
                    .map(|id| id.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::NotSuspended(id) => write!(f, "trigger {id} is not suspended"),
        }
    }
}

impl std::error::Error for TriggerError {}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn operator() -> OperatorId {
        OperatorId::new("op-1")
    }

    fn project() -> ProjectKey {
        ProjectKey::new("t1", "w1", "p1")
    }

    fn make_template(id: &str) -> RunTemplate {
        RunTemplate {
            id: RunTemplateId::new(id),
            project: project(),
            name: format!("Template {id}"),
            description: None,
            default_mode: RunMode::Direct,
            system_prompt: "You are responding to {{action}} on issue #{{issue.number}}".into(),
            initial_user_message: None,
            plugin_allowlist: None,
            tool_allowlist: None,
            budget: TemplateBudget::default(),
            sandbox_hint: None,
            required_fields: Vec::new(),
            created_by: operator(),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn make_trigger(id: &str, template_id: &str) -> Trigger {
        Trigger {
            id: TriggerId::new(id),
            project: project(),
            name: format!("Trigger {id}"),
            description: None,
            signal_pattern: SignalPattern {
                signal_type: "github.issue.labeled".into(),
                plugin_id: Some("github".into()),
            },
            conditions: vec![TriggerCondition::Contains {
                path: "labels[].name".into(),
                value: json!("cairn-ready"),
            }],
            run_template_id: RunTemplateId::new(template_id),
            state: TriggerState::Enabled,
            rate_limit: RateLimitConfig::default(),
            max_chain_depth: 5,
            created_by: operator(),
            created_at: 0,
            updated_at: 0,
        }
    }

    // ── Condition DSL Tests ──────────────────────────────────────────

    #[test]
    fn condition_equals_matches() {
        let payload = json!({"action": "labeled"});
        let cond = TriggerCondition::Equals {
            path: "action".into(),
            value: json!("labeled"),
        };
        assert!(evaluate_condition(&cond, &payload));
    }

    #[test]
    fn condition_equals_mismatches() {
        let payload = json!({"action": "opened"});
        let cond = TriggerCondition::Equals {
            path: "action".into(),
            value: json!("labeled"),
        };
        assert!(!evaluate_condition(&cond, &payload));
    }

    #[test]
    fn condition_contains_array() {
        let payload = json!({
            "labels": [{"name": "bug"}, {"name": "cairn-ready"}]
        });
        let cond = TriggerCondition::Contains {
            path: "labels[].name".into(),
            value: json!("cairn-ready"),
        };
        assert!(evaluate_condition(&cond, &payload));
    }

    #[test]
    fn condition_contains_array_no_match() {
        let payload = json!({
            "labels": [{"name": "bug"}, {"name": "enhancement"}]
        });
        let cond = TriggerCondition::Contains {
            path: "labels[].name".into(),
            value: json!("cairn-ready"),
        };
        assert!(!evaluate_condition(&cond, &payload));
    }

    #[test]
    fn condition_exists() {
        let payload = json!({"issue": {"number": 42}});
        assert!(evaluate_condition(
            &TriggerCondition::Exists {
                path: "issue.number".into()
            },
            &payload
        ));
        assert!(!evaluate_condition(
            &TriggerCondition::Exists {
                path: "issue.title".into()
            },
            &payload
        ));
    }

    #[test]
    fn condition_not() {
        let payload = json!({"action": "opened"});
        let cond = TriggerCondition::Not(Box::new(TriggerCondition::Equals {
            path: "action".into(),
            value: json!("labeled"),
        }));
        assert!(evaluate_condition(&cond, &payload));
    }

    #[test]
    fn condition_serializes_and_roundtrips_with_nested_not() {
        let cond = TriggerCondition::Not(Box::new(TriggerCondition::Contains {
            path: "labels[].name".into(),
            value: json!("cairn-ready"),
        }));

        let json = serde_json::to_value(&cond).expect("trigger condition should serialize");
        assert_eq!(json["type"], json!("not"));
        assert_eq!(json["condition"]["type"], json!("contains"));

        let restored: TriggerCondition =
            serde_json::from_value(json).expect("trigger condition should deserialize");
        assert_eq!(restored, cond);
    }

    // ── Variable Substitution Tests ─────────────────────────────────

    #[test]
    fn substitution_replaces_scalars() {
        let payload = json!({
            "action": "labeled",
            "issue": {"number": 42, "title": "Fix login bug"},
            "repository": {"full_name": "org/dogfood"}
        });

        let template = "Issue #{{issue.number}} in {{repository.full_name}}: {{issue.title}}";
        let result = substitute_variables(template, &payload, &[]).unwrap();
        assert_eq!(result, "Issue #42 in org/dogfood: Fix login bug");
    }

    #[test]
    fn substitution_replaces_arrays() {
        let payload = json!({
            "issue": {
                "labels": [{"name": "bug"}, {"name": "cairn-ready"}]
            }
        });

        let template = "Labels: {{issue.labels[].name}}";
        let result = substitute_variables(template, &payload, &[]).unwrap();
        assert_eq!(result, "Labels: bug, cairn-ready");
    }

    #[test]
    fn substitution_missing_field_empty_string() {
        let payload = json!({"action": "labeled"});
        let result = substitute_variables("Value: {{nonexistent}}", &payload, &[]).unwrap();
        assert_eq!(result, "Value: ");
    }

    #[test]
    fn substitution_required_field_missing_errors() {
        let payload = json!({"action": "labeled"});
        let result =
            substitute_variables("{{issue.number}}", &payload, &["issue.number".to_string()]);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err(), vec!["issue.number".to_string()]);
    }

    // ── Trigger Service Tests ───────────────────────────────────────

    #[test]
    fn create_trigger_requires_template() {
        let mut svc = TriggerService::new();
        let trigger = make_trigger("t1", "nonexistent");
        assert!(matches!(
            svc.create_trigger(trigger),
            Err(TriggerError::TemplateNotFound(_))
        ));
    }

    #[test]
    fn create_and_evaluate_trigger() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));
        svc.create_trigger(make_trigger("t1", "tmpl-1")).unwrap();

        let payload = json!({
            "action": "labeled",
            "labels": [{"name": "cairn-ready"}]
        });

        let events = svc.evaluate_signal(
            &project(),
            &SignalId::new("sig-1"),
            "github.issue.labeled",
            "github",
            &payload,
            None,
            &auto_approve_decision,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            TriggerEvent::TriggerFired { chain_depth: 1, .. }
        ));
    }

    #[test]
    fn condition_mismatch_skips() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));
        svc.create_trigger(make_trigger("t1", "tmpl-1")).unwrap();

        // No "cairn-ready" label
        let payload = json!({
            "action": "labeled",
            "labels": [{"name": "bug"}]
        });

        let events = svc.evaluate_signal(
            &project(),
            &SignalId::new("sig-2"),
            "github.issue.labeled",
            "github",
            &payload,
            None,
            &auto_approve_decision,
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            TriggerEvent::TriggerSkipped {
                reason: SkipReason::ConditionMismatch,
                ..
            }
        ));
    }

    #[test]
    fn fire_ledger_dedup_prevents_duplicate() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));
        svc.create_trigger(make_trigger("t1", "tmpl-1")).unwrap();

        let payload = json!({"labels": [{"name": "cairn-ready"}]});
        let signal_id = SignalId::new("sig-dup");

        // First eval fires
        let events1 = svc.evaluate_signal(
            &project(),
            &signal_id,
            "github.issue.labeled",
            "github",
            &payload,
            None,
            &auto_approve_decision,
        );
        assert!(matches!(&events1[0], TriggerEvent::TriggerFired { .. }));

        // Second eval with same signal_id is deduped
        let events2 = svc.evaluate_signal(
            &project(),
            &signal_id,
            "github.issue.labeled",
            "github",
            &payload,
            None,
            &auto_approve_decision,
        );
        assert!(matches!(
            &events2[0],
            TriggerEvent::TriggerSkipped {
                reason: SkipReason::AlreadyFired,
                ..
            }
        ));
    }

    #[test]
    fn chain_depth_prevents_loops() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));

        let mut trigger = make_trigger("t1", "tmpl-1");
        trigger.max_chain_depth = 3;
        svc.create_trigger(trigger).unwrap();

        let payload = json!({"labels": [{"name": "cairn-ready"}]});

        // Depth 3 (source at 2, +1 = 3) — at limit, should still fire
        let events = svc.evaluate_signal(
            &project(),
            &SignalId::new("sig-depth3"),
            "github.issue.labeled",
            "github",
            &payload,
            Some(2),
            &auto_approve_decision,
        );
        assert!(matches!(
            &events[0],
            TriggerEvent::TriggerFired { chain_depth: 3, .. }
        ));

        // Depth 4 (source at 3, +1 = 4) — exceeds limit
        let events = svc.evaluate_signal(
            &project(),
            &SignalId::new("sig-depth4"),
            "github.issue.labeled",
            "github",
            &payload,
            Some(3),
            &auto_approve_decision,
        );
        assert!(matches!(
            &events[0],
            TriggerEvent::TriggerSkipped {
                reason: SkipReason::ChainTooDeep,
                ..
            }
        ));
    }

    #[test]
    fn multiple_triggers_fan_out() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));
        svc.create_template(make_template("tmpl-2"));
        svc.create_trigger(make_trigger("t1", "tmpl-1")).unwrap();
        svc.create_trigger(make_trigger("t2", "tmpl-2")).unwrap();

        let payload = json!({"labels": [{"name": "cairn-ready"}]});

        let events = svc.evaluate_signal(
            &project(),
            &SignalId::new("sig-fan"),
            "github.issue.labeled",
            "github",
            &payload,
            None,
            &auto_approve_decision,
        );

        // Both triggers should fire
        let fired_count = events
            .iter()
            .filter(|e| matches!(e, TriggerEvent::TriggerFired { .. }))
            .count();
        assert_eq!(fired_count, 2);
    }

    #[test]
    fn delete_template_blocked_by_trigger() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));
        svc.create_trigger(make_trigger("t1", "tmpl-1")).unwrap();

        let result = svc.delete_template(&RunTemplateId::new("tmpl-1"), operator());
        assert!(matches!(result, Err(TriggerError::TemplateInUse { .. })));
    }

    #[test]
    fn delete_template_succeeds_after_trigger_removed() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));
        svc.create_trigger(make_trigger("t1", "tmpl-1")).unwrap();

        svc.delete_trigger(&TriggerId::new("t1"), operator())
            .unwrap();
        let result = svc.delete_template(&RunTemplateId::new("tmpl-1"), operator());
        assert!(matches!(
            result,
            Ok(TriggerEvent::RunTemplateDeleted { .. })
        ));
    }

    #[test]
    fn cross_project_isolation() {
        let mut svc = TriggerService::new();
        svc.create_template(make_template("tmpl-1"));
        svc.create_trigger(make_trigger("t1", "tmpl-1")).unwrap();

        let payload = json!({"labels": [{"name": "cairn-ready"}]});
        let other_project = ProjectKey::new("t1", "w1", "p2");

        // Signal in the wrong project → no triggers match
        let events = svc.evaluate_signal(
            &other_project,
            &SignalId::new("sig-other"),
            "github.issue.labeled",
            "github",
            &payload,
            None,
            &auto_approve_decision,
        );
        assert!(events.is_empty());
    }
}
