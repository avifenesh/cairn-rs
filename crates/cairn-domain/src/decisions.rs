//! Decision layer and agent loop domain types (RFC 018 + RFC 019).
//!
//! These types underpin the unified decision layer (RFC 019) and the agent
//! loop enhancements (RFC 018: Plan/Execute/Direct modes, tool effect
//! classification).

use crate::ids::{
    CorrelationId, CredentialId, DecisionId, OperatorId, PolicyId, RunId, TenantId, WorkspaceId,
};
use crate::tenancy::ProjectKey;
use serde::{Deserialize, Serialize};
use std::time::Duration;

// ── RunMode (RFC 018) ────────────────────────────────────────────────────────

/// Execution mode for an agent run.
///
/// - `Direct` — the default; agent sees all tools and acts immediately.
/// - `Plan` — agent sees only Observational + Internal tools; produces a plan
///   artifact and terminates with `outcome: plan_proposed`.
/// - `Execute` — agent follows an approved plan from a prior Plan-mode run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunMode {
    #[default]
    Direct,
    Plan,
    Execute {
        /// The Plan-mode run whose approved artifact seeds this execution.
        plan_run_id: RunId,
    },
}

// ── ToolEffect (RFC 018) ─────────────────────────────────────────────────────

/// Side-effect classification for a tool.
///
/// Every built-in tool and plugin tool declares its `ToolEffect`. The prompt
/// builder uses this to filter tools by run mode:
///
/// - **Plan mode**: only `Observational` + `Internal` tools are visible.
/// - **Execute / Direct mode**: all tools are visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolEffect {
    /// Read-only observation — no state changes anywhere.
    ///
    /// Examples: `memory_search`, `grep_search`, `file_read`, `web_fetch`,
    /// `graph_query`, `list_runs`, `get_approvals`, `tool_search`.
    Observational,

    /// Writes to cairn-owned state only (scratch pad, memory, sandbox FS,
    /// cairn task queue). Never touches external systems.
    ///
    /// Examples: `scratch_pad`, `memory_store`, `file_write`, `create_task`.
    Internal,

    /// Touches systems outside cairn's boundary — outbound API calls, shell
    /// execution, notifications to humans, writes to shared resources.
    ///
    /// Examples: `shell_exec`, `http_request`, `notify_operator`.
    External,
}

// ── DecisionScopeRef (RFC 019) ───────────────────────────────────────────────

/// Concrete discriminated scope key for decision cache entries.
///
/// `DecisionRequest.scope` remains `ProjectKey` (execution context); the cache
/// entry's `scope_ref` is derived from the `DecisionKind`'s `default_scope`
/// and the request's execution context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "level", rename_all = "snake_case")]
pub enum DecisionScopeRef {
    Run {
        run_id: RunId,
        project: ProjectKey,
    },
    Project(ProjectKey),
    Workspace {
        tenant_id: TenantId,
        workspace_id: WorkspaceId,
    },
    Tenant {
        tenant_id: TenantId,
    },
}

// ── DecisionKind (RFC 019) ───────────────────────────────────────────────────

/// The kind of action being evaluated by the decision layer.
///
/// Every "can I do this?" question entering the decision service is tagged
/// with a `DecisionKind`. The kind drives cache key derivation, default TTL,
/// scope, and resolver chain selection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum DecisionKind {
    /// Agent invoking a tool (built-in or plugin).
    ToolInvocation {
        tool_name: String,
        effect: ToolEffect,
    },

    /// LLM provider call (generation / embedding).
    ProviderCall {
        model_id: String,
        estimated_tokens: u32,
    },

    /// Enabling a plugin for a target project.
    PluginEnablement {
        plugin_id: String,
        target_project: ProjectKey,
    },

    /// Provisioning a sandbox workspace.
    WorkspaceProvision {
        /// Sandbox isolation strategy identifier (e.g. "container", "nsjail").
        strategy: String,
        /// Base image or template identifier.
        base: String,
    },

    /// Accessing a stored credential.
    CredentialAccess {
        credential_id: CredentialId,
        /// Why the credential is needed (human-readable).
        purpose: String,
    },

    /// Firing a trigger (RFC 022).
    TriggerFire {
        /// The trigger being fired.
        trigger_id: String,
        /// The signal type that caused the trigger.
        signal_type: String,
    },

    /// An action classified as destructive (delete, drop, force-push, etc.).
    DestructiveAction {
        /// Human-readable description of the destructive action.
        action: String,
        /// The resource being acted upon (e.g. "run:abc123", "credential:xyz").
        resource: String,
    },

    /// Catch-all for decision kinds not covered by the named variants.
    Other(String),
}

// ── Principal (RFC 019) ──────────────────────────────────────────────────────

/// Who is making the decision request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Principal {
    /// An agent run requesting permission to act.
    Run { run_id: RunId },
    /// A human operator performing a direct action.
    Operator { operator_id: OperatorId },
    /// The system itself (e.g. scheduled maintenance, policy enforcement).
    System,
}

// ── DecisionSubject (RFC 019) ────────────────────────────────────────────────

/// What the decision is about — the target of the action being evaluated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DecisionSubject {
    /// A specific tool call with its arguments.
    ToolCall {
        tool_name: String,
        args: serde_json::Value,
    },
    /// A resource being acted upon (credential, plugin, workspace, etc.).
    Resource {
        resource_type: String,
        resource_id: String,
    },
    /// A provider call (model inference).
    ProviderCall { model_id: String },
}

// ── CostEstimate (RFC 019) ──────────────────────────────────────────────────

/// Estimated cost for budget pre-check at step 4.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CostEstimate {
    /// Estimated cost in the tenant's billing currency (e.g. USD cents).
    pub amount: f64,
    /// Currency code (e.g. "usd_cents").
    pub currency: String,
    /// What the cost is for (e.g. "gpt-4 generation ~4000 tokens").
    pub description: String,
}

// ── DecisionRequest (RFC 019) ────────────────────────────────────────────────

/// A "can I do this?" question entering the decision service.
///
/// The main call sites are the orchestrator's execute phase (tool invocation),
/// the marketplace layer (plugin enablement), the provider router (LLM calls),
/// and any explicit policy-gated action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DecisionRequest {
    /// What kind of action — drives cache key derivation and policy lookup.
    pub kind: DecisionKind,
    /// Who is asking.
    pub principal: Principal,
    /// What is the target of the action.
    pub subject: DecisionSubject,
    /// Tenant/workspace/project execution context.
    pub scope: ProjectKey,
    /// Optional cost estimate for budget pre-check.
    pub cost_estimate: Option<CostEstimate>,
    /// When the request was made (epoch ms).
    pub requested_at: u64,
    /// Correlation ID for tracing across events.
    pub correlation_id: CorrelationId,
}

// ── DecisionKey (RFC 019) ────────────────────────────────────────────────────

/// Deterministic, stable cache key for decision deduplication.
///
/// Two decisions are equivalent (same key) if they differ only in details
/// that should not change the outcome. The `semantic_hash` is derived from
/// the tool's `cache_on_fields` allowlist.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionKey {
    /// Discriminant tag (e.g. "tool_invocation", "provider_call").
    pub kind_tag: String,
    /// The scope at which this decision is cached.
    pub scope_ref: DecisionScopeRef,
    /// Stable hash of the semantic fingerprint from `cache_on_fields`.
    pub semantic_hash: String,
}

// ── DecisionOutcome (RFC 019) ────────────────────────────────────────────────

/// The result of evaluating a decision request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum DecisionOutcome {
    /// The action is allowed to proceed.
    Allowed,
    /// The action is denied.
    Denied {
        /// Which evaluation step produced the denial (1-based).
        deny_step: u8,
        /// Human-readable reason for the denial.
        deny_reason: String,
    },
}

// ── DecisionCacheScope (RFC 019) ─────────────────────────────────────────────

/// The logical scope at which a cached decision applies.
///
/// This is the *declared* scope (per `DecisionKind` defaults or operator
/// override). The concrete `DecisionScopeRef` on the cache entry is derived
/// from this + the request's execution context.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionCacheScope {
    /// Applies only to the run that made the request.
    Run,
    /// Applies to every run in the project.
    Project,
    /// Applies to every run in the workspace.
    Workspace,
    /// Applies to every run in the tenant.
    Tenant,
}

// ── CachePolicy (RFC 019) ────────────────────────────────────────────────────

/// When to cache a decision outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CachePolicy {
    /// Always cache the outcome regardless of Allowed/Denied.
    AlwaysCache,
    /// Never cache — every request is a fresh evaluation.
    NeverCache,
    /// Cache only if the outcome was Allowed.
    CacheIfApproved,
    /// Cache only if the outcome was Denied.
    CacheIfDenied,
}

// ── DecisionPolicy (RFC 019) ─────────────────────────────────────────────────

/// Per-`DecisionKind` policy controlling TTL, scope, and cacheability.
///
/// Operators can narrow (reduce TTL, tighten scope) but never widen past
/// `max_ttl`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionPolicy {
    /// Default time-to-live for cached decisions (in seconds).
    pub default_ttl_secs: u64,
    /// The scope at which decisions of this kind are cached by default.
    pub default_scope: DecisionCacheScope,
    /// When to write to the cache.
    pub cache_policy: CachePolicy,
    /// Hard cap on TTL that operators cannot exceed (in seconds).
    pub max_ttl_secs: u64,
}

impl DecisionPolicy {
    pub fn default_ttl(&self) -> Duration {
        Duration::from_secs(self.default_ttl_secs)
    }
    pub fn max_ttl(&self) -> Duration {
        Duration::from_secs(self.max_ttl_secs)
    }
}

// ── StepResult (RFC 019) ─────────────────────────────────────────────────────

/// The output of a single evaluation step in the reasoning chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepResult {
    /// Which step (1=scope, 2=visibility, 3=guardrail, 4=budget, 5=cache,
    /// 6=resolver, 7=cache_write, 8=return).
    pub step: u8,
    /// Human-readable step name.
    pub name: String,
    /// Step outcome: "allow", "deny", "escalate", "cache_hit", "cache_miss",
    /// "skip", "pending".
    pub outcome: String,
    /// Optional detail (e.g. matching rule ID, budget remaining, cache key).
    pub detail: Option<String>,
    /// Guardrail rule IDs that contributed to this step's evaluation.
    /// Populated at step 3; used for selective cache invalidation.
    pub rule_ids: Vec<PolicyId>,
}

// ── DecisionSource (RFC 019) ─────────────────────────────────────────────────

/// How a decision was resolved.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum DecisionSource {
    /// The decision was resolved by replaying a cached prior decision.
    CacheHit {
        /// The original decision that was cached.
        original_decision_id: DecisionId,
    },
    /// The decision was evaluated fresh through the full pipeline.
    FreshEvaluation,
    /// The decision was resolved by a guardian model.
    Guardian { model_id: String },
    /// The decision was resolved by a human operator.
    Human { operator_id: OperatorId },
}

// ── ActorRef (RFC 019) ──────────────────────────────────────────────────────

/// Who performed an action (for invalidation audit).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ActorRef {
    Operator { operator_id: OperatorId },
    SystemPolicyChange,
}

// ── CachedDecisionRef (RFC 019) ──────────────────────────────────────────────

/// Reference to a cached decision entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedDecisionRef {
    pub decision_key: DecisionKey,
    pub expires_at: u64,
}

// ── RiskLevel (RFC 018) ──────────────────────────────────────────────────────

/// Risk classification returned by the guardian resolver.
///
/// The guardian evaluates a pending approval and classifies the risk.
/// If the risk exceeds the project's `risk_ceiling`, the guardian falls
/// through to the human resolver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
}

// ── ResolverDecision (RFC 018) ──────────────────────────────────────────────

/// The structured decision returned by an approval resolver.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolverDecision {
    /// Whether the action is approved or denied.
    pub outcome: DecisionOutcome,
    /// Human-readable explanation of why.
    pub rationale: String,
    /// Risk classification.
    pub risk_level: RiskLevel,
    /// Who resolved: "human:operator_id" or "guardian:model_id".
    pub resolved_by: String,
    /// How long to cache this decision (seconds). `None` = use kind default.
    pub ttl_secs: Option<u64>,
}

// ── GuardianConfig (RFC 018) ─────────────────────────────────────────────────

/// Per-project guardian resolver configuration.
///
/// If `model_id` is `None`, the guardian is not in the resolver chain
/// and all approvals go to a human.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardianConfig {
    /// LLM model to use for guardian evaluation. `None` = guardian disabled.
    pub model_id: Option<String>,
    /// Timeout for the guardian LLM call in milliseconds.
    pub timeout_ms: u64,
    /// Maximum risk level the guardian is allowed to approve.
    /// Anything above this falls through to the human resolver.
    pub risk_ceiling: RiskLevel,
    /// Maximum tokens for the guardian prompt context.
    pub max_context_tokens: usize,
}

impl Default for GuardianConfig {
    fn default() -> Self {
        Self {
            model_id: None,
            timeout_ms: 60_000,
            risk_ceiling: RiskLevel::Low,
            max_context_tokens: 16_000,
        }
    }
}

// ── DecisionEvent (RFC 019) ──────────────────────────────────────────────────

/// Events emitted by the decision layer.
///
/// These are embedded in `RuntimeEvent` and flow through the event log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum DecisionEvent {
    /// Single canonical audit record for every decision (Allowed or Denied).
    DecisionRecorded {
        decision_id: DecisionId,
        request: Box<DecisionRequest>,
        outcome: DecisionOutcome,
        /// Full chain from steps 1-7.
        reasoning_chain: Vec<StepResult>,
        source: DecisionSource,
        /// Human ID or guardian model ID that resolved the decision.
        resolved_by: Option<String>,
        /// The cached rule, if step 7 wrote one.
        cached_for: Option<CachedDecisionRef>,
        decided_at: u64,
    },

    /// A new entry was written to the decision cache.
    DecisionCacheUpdated {
        decision_id: DecisionId,
        decision_key: DecisionKey,
        scope: DecisionCacheScope,
        ttl_secs: u64,
        expires_at: u64,
        created_at: u64,
    },

    /// A cached decision was reused for a new request.
    DecisionCacheHit {
        new_correlation_id: CorrelationId,
        cached_decision_id: DecisionId,
        decision_key: DecisionKey,
        hit_at: u64,
    },

    /// A cached decision was invalidated (by operator or policy change).
    DecisionCacheInvalidated {
        decision_id: DecisionId,
        decision_key: DecisionKey,
        invalidated_by: ActorRef,
        reason: String,
        invalidated_at: u64,
    },
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_mode_default_is_direct() {
        assert_eq!(RunMode::default(), RunMode::Direct);
    }

    #[test]
    fn run_mode_serde_roundtrip() {
        let modes = vec![
            RunMode::Direct,
            RunMode::Plan,
            RunMode::Execute {
                plan_run_id: RunId::new("plan_001"),
            },
        ];
        for mode in &modes {
            let json = serde_json::to_string(mode).unwrap();
            let parsed: RunMode = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, mode);
        }
    }

    #[test]
    fn tool_effect_serde_roundtrip() {
        let effects = vec![
            ToolEffect::Observational,
            ToolEffect::Internal,
            ToolEffect::External,
        ];
        let json = serde_json::to_string(&effects).unwrap();
        let parsed: Vec<ToolEffect> = serde_json::from_str(&json).unwrap();
        assert_eq!(effects, parsed);
    }

    #[test]
    fn decision_scope_ref_serde_roundtrip() {
        let scopes = vec![
            DecisionScopeRef::Run {
                run_id: RunId::new("r1"),
                project: ProjectKey::new("t", "w", "p"),
            },
            DecisionScopeRef::Project(ProjectKey::new("t", "w", "p")),
            DecisionScopeRef::Workspace {
                tenant_id: TenantId::new("t"),
                workspace_id: WorkspaceId::new("w"),
            },
            DecisionScopeRef::Tenant {
                tenant_id: TenantId::new("t"),
            },
        ];
        for scope in &scopes {
            let json = serde_json::to_string(scope).unwrap();
            let parsed: DecisionScopeRef = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, scope);
        }
    }

    #[test]
    fn decision_kind_tool_invocation_serde() {
        let kind = DecisionKind::ToolInvocation {
            tool_name: "shell_exec".into(),
            effect: ToolEffect::External,
        };
        let json = serde_json::to_string(&kind).unwrap();
        let parsed: DecisionKind = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, kind);
    }

    #[test]
    fn decision_kind_all_variants_serialize() {
        let kinds = vec![
            DecisionKind::ToolInvocation {
                tool_name: "grep_search".into(),
                effect: ToolEffect::Observational,
            },
            DecisionKind::ProviderCall {
                model_id: "gpt-4".into(),
                estimated_tokens: 1000,
            },
            DecisionKind::PluginEnablement {
                plugin_id: "slack-notifier".into(),
                target_project: ProjectKey::new("t", "w", "p"),
            },
            DecisionKind::WorkspaceProvision {
                strategy: "container".into(),
                base: "ubuntu:22.04".into(),
            },
            DecisionKind::CredentialAccess {
                credential_id: CredentialId::new("cred_1"),
                purpose: "GitHub API access".into(),
            },
            DecisionKind::TriggerFire {
                trigger_id: "trg_001".into(),
                signal_type: "webhook".into(),
            },
            DecisionKind::DestructiveAction {
                action: "delete_run".into(),
                resource: "run:abc123".into(),
            },
            DecisionKind::Other("custom_check".into()),
        ];
        for kind in &kinds {
            let json = serde_json::to_string(kind).unwrap();
            let parsed: DecisionKind = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, kind);
        }
    }

    fn sample_request() -> DecisionRequest {
        DecisionRequest {
            kind: DecisionKind::ToolInvocation {
                tool_name: "shell_exec".into(),
                effect: ToolEffect::External,
            },
            principal: Principal::Run {
                run_id: RunId::new("run_1"),
            },
            subject: DecisionSubject::ToolCall {
                tool_name: "shell_exec".into(),
                args: serde_json::json!({"command": "ls"}),
            },
            scope: ProjectKey::new("t", "w", "p"),
            cost_estimate: None,
            requested_at: 1700000000000,
            correlation_id: CorrelationId::new("cor_1"),
        }
    }

    #[test]
    fn decision_request_serde_roundtrip() {
        let req = sample_request();
        let json = serde_json::to_string(&req).unwrap();
        let parsed: DecisionRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, req);
    }

    #[test]
    fn decision_outcome_allowed_serde() {
        let outcome = DecisionOutcome::Allowed;
        let json = serde_json::to_string(&outcome).unwrap();
        assert!(json.contains("allowed"));
        let parsed: DecisionOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, outcome);
    }

    #[test]
    fn decision_outcome_denied_serde() {
        let outcome = DecisionOutcome::Denied {
            deny_step: 3,
            deny_reason: "guardrail_denied".into(),
        };
        let json = serde_json::to_string(&outcome).unwrap();
        let parsed: DecisionOutcome = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, outcome);
    }

    #[test]
    fn decision_key_hash_equality() {
        let k1 = DecisionKey {
            kind_tag: "tool_invocation".into(),
            scope_ref: DecisionScopeRef::Project(ProjectKey::new("t", "w", "p")),
            semantic_hash: "abc123".into(),
        };
        let k2 = k1.clone();
        assert_eq!(k1, k2);
    }

    #[test]
    fn decision_policy_ttl_conversion() {
        let policy = DecisionPolicy {
            default_ttl_secs: 3600,
            default_scope: DecisionCacheScope::Project,
            cache_policy: CachePolicy::CacheIfApproved,
            max_ttl_secs: 86400,
        };
        assert_eq!(policy.default_ttl(), Duration::from_secs(3600));
        assert_eq!(policy.max_ttl(), Duration::from_secs(86400));
    }

    #[test]
    fn cache_policy_serde_roundtrip() {
        let policies = vec![
            CachePolicy::AlwaysCache,
            CachePolicy::NeverCache,
            CachePolicy::CacheIfApproved,
            CachePolicy::CacheIfDenied,
        ];
        let json = serde_json::to_string(&policies).unwrap();
        let parsed: Vec<CachePolicy> = serde_json::from_str(&json).unwrap();
        assert_eq!(policies, parsed);
    }

    #[test]
    fn principal_variants_serde() {
        let principals = vec![
            Principal::Run {
                run_id: RunId::new("r1"),
            },
            Principal::Operator {
                operator_id: OperatorId::new("op1"),
            },
            Principal::System,
        ];
        for p in &principals {
            let json = serde_json::to_string(p).unwrap();
            let parsed: Principal = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, p);
        }
    }

    #[test]
    fn decision_source_variants_serde() {
        let sources = vec![
            DecisionSource::CacheHit {
                original_decision_id: DecisionId::new("dec_1"),
            },
            DecisionSource::FreshEvaluation,
            DecisionSource::Guardian {
                model_id: "gpt-4".into(),
            },
            DecisionSource::Human {
                operator_id: OperatorId::new("op1"),
            },
        ];
        for src in &sources {
            let json = serde_json::to_string(src).unwrap();
            let parsed: DecisionSource = serde_json::from_str(&json).unwrap();
            assert_eq!(&parsed, src);
        }
    }

    #[test]
    fn step_result_serde() {
        let step = StepResult {
            step: 3,
            name: "guardrail".into(),
            outcome: "escalate".into(),
            detail: Some("rule_abc matched".into()),
            rule_ids: vec![PolicyId::new("rule_abc")],
        };
        let json = serde_json::to_string(&step).unwrap();
        let parsed: StepResult = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, step);
    }

    #[test]
    fn decision_event_recorded_serde() {
        let event = DecisionEvent::DecisionRecorded {
            decision_id: DecisionId::new("dec_1"),
            request: Box::new(sample_request()),
            outcome: DecisionOutcome::Allowed,
            reasoning_chain: vec![StepResult {
                step: 1,
                name: "scope".into(),
                outcome: "allow".into(),
                detail: None,
                rule_ids: vec![],
            }],
            source: DecisionSource::FreshEvaluation,
            resolved_by: None,
            cached_for: None,
            decided_at: 1700000001000,
        };
        let json = serde_json::to_string(&event).unwrap();
        let parsed: DecisionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, event);
    }
}
