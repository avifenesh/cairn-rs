//! Unified Decision Layer (RFC 019).
//!
//! Composes the existing guardrail, approval, budget, and visibility services
//! into a single atomic evaluation with one truth per decision.
//!
//! The `DecisionService` is the only call site for the RFC 018 resolver chain
//! and the only emitter of `DecisionRecorded` events.

use async_trait::async_trait;
use cairn_domain::decisions::{
    ActorRef, CachePolicy, CachedDecisionRef, DecisionCacheScope, DecisionEvent, DecisionKey,
    DecisionKind, DecisionOutcome, DecisionPolicy, DecisionRequest, DecisionScopeRef,
    DecisionSource, StepResult, ToolEffect,
};
use cairn_domain::ids::{DecisionId, PolicyId};
use cairn_domain::ProjectKey;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── Errors ───────────────────────────────────────────────────────────────────

/// Error type for the decision service.
#[derive(Debug)]
pub enum DecisionError {
    /// An internal service call failed.
    Internal(String),
    /// The request was malformed.
    InvalidRequest(String),
}

impl std::fmt::Display for DecisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DecisionError::Internal(msg) => write!(f, "decision error: {msg}"),
            DecisionError::InvalidRequest(msg) => write!(f, "invalid decision request: {msg}"),
        }
    }
}

impl std::error::Error for DecisionError {}

// ── DecisionResult ───────────────────────────────────────────────────────────

/// Result of evaluating a decision through the unified layer.
#[derive(Clone, Debug)]
pub struct DecisionResult {
    /// The unique ID assigned to this decision.
    pub decision_id: DecisionId,
    /// Whether the action is allowed or denied.
    pub outcome: DecisionOutcome,
    /// The cache key derived from the request (for cache operations).
    pub decision_key: DecisionKey,
    /// The event emitted by this evaluation.
    pub event: DecisionEvent,
}

// ── CacheEntryState ──────────────────────────────────────────────────────────

/// Cache entry states for the singleflight pattern (RFC 019 §5).
#[derive(Clone, Debug)]
pub enum CacheEntryState {
    /// No entry exists for this key.
    Miss,
    /// Another request is being evaluated for this key.
    Pending {
        owner_decision_id: DecisionId,
        started_at: u64,
    },
    /// A resolved (possibly expired) entry exists.
    Resolved {
        decision_id: DecisionId,
        outcome: DecisionOutcome,
        expires_at: u64,
    },
}

// ── DecisionService trait ────────────────────────────────────────────────────

/// Unified decision layer (RFC 019).
///
/// Evaluates "can I do this?" requests through the canonical 8-step order:
///
/// 1. Scope check (RFC 008)
/// 2. Visibility check (VisibilityContext)
/// 3. Guardrail check (GuardrailService)
/// 4. Budget check (BudgetService)
/// 5. Cache lookup (singleflight: Miss / Pending / Resolved)
/// 6. Approval resolution (RFC 018 resolver chain)
/// 7. Cache write (Pending → Resolved)
/// 8. Return outcome
///
/// No step is skipped. Every allow is the result of all steps passing.
/// Every deny comes from a specific step with an attributable reason.
#[async_trait]
pub trait DecisionService: Send + Sync {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResult, DecisionError>;
    fn policy_for_kind(&self, kind_tag: &str) -> Option<DecisionPolicy>;
    async fn cache_lookup(&self, key: &DecisionKey) -> Result<CacheEntryState, DecisionError>;
    async fn invalidate(
        &self,
        decision_id: &DecisionId,
        reason: &str,
        invalidated_by: ActorRef,
    ) -> Result<(), DecisionError>;
    async fn invalidate_by_scope(
        &self,
        scope: &DecisionScopeRef,
        kind_filter: Option<&str>,
        reason: &str,
        invalidated_by: ActorRef,
    ) -> Result<u32, DecisionError>;
    async fn invalidate_by_rule(
        &self,
        rule_id: &PolicyId,
        reason: &str,
        invalidated_by: ActorRef,
    ) -> Result<u32, DecisionError>;
    async fn list_cached(
        &self,
        scope: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<CachedDecisionSummary>, DecisionError>;
    async fn get_decision(
        &self,
        decision_id: &DecisionId,
    ) -> Result<Option<DecisionEvent>, DecisionError>;
}

/// Summary of a cached decision for the operator "learned rules" view.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct CachedDecisionSummary {
    pub decision_id: DecisionId,
    pub decision_key: DecisionKey,
    pub outcome: DecisionOutcome,
    pub kind_tag: String,
    pub scope: DecisionScopeRef,
    pub ttl_remaining_secs: u64,
    pub source: DecisionSource,
    pub hit_count: u64,
    pub created_at: u64,
    pub expires_at: u64,
}

// ── Inner service abstractions (stubbed) ─────────────────────────────────────

/// Outcome of scope check (step 1).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ScopeCheckResult {
    Allowed,
    Denied(String),
}

/// Outcome of visibility check (step 2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VisibilityCheckResult {
    Visible,
    NotInContext(String),
}

/// Outcome of guardrail check (step 3).
#[derive(Clone, Debug)]
pub struct GuardrailCheckResult {
    pub outcome: GuardrailCheckOutcome,
    /// Rule IDs that contributed to this evaluation.
    pub rule_ids: Vec<PolicyId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GuardrailCheckOutcome {
    Allow,
    Deny(String),
    Escalate,
}

/// Outcome of budget check (step 4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BudgetCheckResult {
    /// Within limits.
    Ok,
    /// Soft alert threshold crossed; continue but record alert.
    SoftAlert { percent_used: u32 },
    /// Hard cap exceeded.
    Exceeded(String),
}

/// Pluggable scope checker (step 1).
#[async_trait]
pub trait ScopeChecker: Send + Sync {
    async fn check(&self, request: &DecisionRequest) -> ScopeCheckResult;
}

/// Pluggable visibility checker (step 2).
#[async_trait]
pub trait VisibilityChecker: Send + Sync {
    async fn check(&self, request: &DecisionRequest) -> VisibilityCheckResult;
}

/// Pluggable guardrail checker (step 3).
#[async_trait]
pub trait GuardrailChecker: Send + Sync {
    async fn check(&self, request: &DecisionRequest) -> GuardrailCheckResult;
}

/// Pluggable budget checker (step 4).
#[async_trait]
pub trait BudgetChecker: Send + Sync {
    async fn check(&self, request: &DecisionRequest) -> BudgetCheckResult;
}

/// Pluggable approval resolver (step 6).
#[async_trait]
pub trait ApprovalResolver: Send + Sync {
    /// Resolve an escalated decision. Returns Allowed or Denied.
    async fn resolve(&self, request: &DecisionRequest) -> (DecisionOutcome, DecisionSource);
}

// ── Default (allow-all) implementations ──────────────────────────────────────

pub struct AllowAllScopeChecker;
#[async_trait]
impl ScopeChecker for AllowAllScopeChecker {
    async fn check(&self, _: &DecisionRequest) -> ScopeCheckResult {
        ScopeCheckResult::Allowed
    }
}

pub struct AllowAllVisibilityChecker;
#[async_trait]
impl VisibilityChecker for AllowAllVisibilityChecker {
    async fn check(&self, _: &DecisionRequest) -> VisibilityCheckResult {
        VisibilityCheckResult::Visible
    }
}

pub struct AllowAllGuardrailChecker;
#[async_trait]
impl GuardrailChecker for AllowAllGuardrailChecker {
    async fn check(&self, _: &DecisionRequest) -> GuardrailCheckResult {
        GuardrailCheckResult {
            outcome: GuardrailCheckOutcome::Allow,
            rule_ids: vec![],
        }
    }
}

pub struct AllowAllBudgetChecker;
#[async_trait]
impl BudgetChecker for AllowAllBudgetChecker {
    async fn check(&self, _: &DecisionRequest) -> BudgetCheckResult {
        BudgetCheckResult::Ok
    }
}

pub struct AutoApproveResolver;
#[async_trait]
impl ApprovalResolver for AutoApproveResolver {
    async fn resolve(&self, _: &DecisionRequest) -> (DecisionOutcome, DecisionSource) {
        (DecisionOutcome::Allowed, DecisionSource::FreshEvaluation)
    }
}

// ── Singleflight cache ──────────────────────────────────────────────────────

/// Internal cache entry with singleflight support.
#[derive(Clone)]
struct CacheEntry {
    decision_id: DecisionId,
    outcome: Option<DecisionOutcome>,
    source: Option<DecisionSource>,
    reasoning_chain: Vec<StepResult>,
    expires_at: u64,
    created_at: u64,
    hit_count: u64,
    /// `true` when this entry is still being evaluated (Pending state).
    is_pending: bool,
}

struct DecisionCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    /// Decision ID → full event for get_decision().
    events: Mutex<HashMap<String, DecisionEvent>>,
    /// Reverse index: rule_id → set of cache key strings.
    rule_index: Mutex<HashMap<String, Vec<String>>>,
    pending_timeout_ms: u64,
}

impl DecisionCache {
    fn new(pending_timeout_ms: u64) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            events: Mutex::new(HashMap::new()),
            rule_index: Mutex::new(HashMap::new()),
            pending_timeout_ms,
        }
    }

    fn cache_key_string(key: &DecisionKey) -> String {
        format!(
            "{}:{}:{}",
            key.kind_tag,
            scope_ref_string(&key.scope_ref),
            key.semantic_hash
        )
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    /// Step 5: Atomically check cache.
    fn lookup(&self, key: &DecisionKey) -> CacheEntryState {
        let ck = Self::cache_key_string(key);
        let mut entries = self.entries.lock().unwrap();
        let now = Self::now_ms();

        if let Some(entry) = entries.get(&ck) {
            if entry.is_pending {
                // Pending entry — check staleness.
                if now.saturating_sub(entry.created_at) > self.pending_timeout_ms {
                    // Stale pending — remove and treat as miss.
                    entries.remove(&ck);
                    return CacheEntryState::Miss;
                }
                return CacheEntryState::Pending {
                    owner_decision_id: entry.decision_id.clone(),
                    started_at: entry.created_at,
                };
            }
            // Resolved — check expiry.
            if now < entry.expires_at {
                return CacheEntryState::Resolved {
                    decision_id: entry.decision_id.clone(),
                    outcome: entry.outcome.clone().unwrap_or(DecisionOutcome::Allowed),
                    expires_at: entry.expires_at,
                };
            }
            // Expired — remove.
            entries.remove(&ck);
        }

        CacheEntryState::Miss
    }

    /// Install a Pending entry for singleflight.
    fn install_pending(&self, key: &DecisionKey, decision_id: &DecisionId) {
        let ck = Self::cache_key_string(key);
        let entry = CacheEntry {
            decision_id: decision_id.clone(),
            outcome: None,
            source: None,
            reasoning_chain: vec![],
            expires_at: 0,
            created_at: Self::now_ms(),
            hit_count: 0,
            is_pending: true,
        };
        self.entries.lock().unwrap().insert(ck, entry);
    }

    /// Promote Pending → Resolved.
    fn promote_to_resolved(
        &self,
        key: &DecisionKey,
        decision_id: &DecisionId,
        outcome: &DecisionOutcome,
        source: &DecisionSource,
        chain: &[StepResult],
        ttl_secs: u64,
    ) -> u64 {
        let ck = Self::cache_key_string(key);
        let now = Self::now_ms();
        let expires_at = now + ttl_secs * 1000;

        // Build rule index entries from the chain.
        let rule_ids: Vec<String> = chain
            .iter()
            .flat_map(|s| s.rule_ids.iter().map(|r| r.as_str().to_owned()))
            .collect();
        if !rule_ids.is_empty() {
            let mut rule_idx = self.rule_index.lock().unwrap();
            for rid in &rule_ids {
                rule_idx.entry(rid.clone()).or_default().push(ck.clone());
            }
        }

        self.entries.lock().unwrap().insert(
            ck,
            CacheEntry {
                decision_id: decision_id.clone(),
                outcome: Some(outcome.clone()),
                source: Some(source.clone()),
                reasoning_chain: chain.to_vec(),
                expires_at,
                created_at: now,
                hit_count: 0,
                is_pending: false,
            },
        );

        expires_at
    }

    fn record_hit(&self, key: &DecisionKey) {
        let ck = Self::cache_key_string(key);
        if let Some(entry) = self.entries.lock().unwrap().get_mut(&ck) {
            entry.hit_count += 1;
        }
    }

    fn store_event(&self, decision_id: &DecisionId, event: &DecisionEvent) {
        self.events
            .lock()
            .unwrap()
            .insert(decision_id.as_str().to_owned(), event.clone());
    }

    fn get_event(&self, decision_id: &DecisionId) -> Option<DecisionEvent> {
        self.events
            .lock()
            .unwrap()
            .get(decision_id.as_str())
            .cloned()
    }

    fn remove_by_decision_id(&self, decision_id: &DecisionId) -> bool {
        let mut entries = self.entries.lock().unwrap();
        let target_id = decision_id.as_str();
        let key = entries
            .iter()
            .find(|(_, v)| v.decision_id.as_str() == target_id)
            .map(|(k, _)| k.clone());
        if let Some(k) = key {
            entries.remove(&k);
            true
        } else {
            false
        }
    }

    fn remove_by_scope(&self, scope: &DecisionScopeRef, kind_filter: Option<&str>) -> u32 {
        let scope_str = scope_ref_string(scope);
        let mut entries = self.entries.lock().unwrap();
        let keys_to_remove: Vec<String> = entries
            .keys()
            .filter(|k| {
                let parts: Vec<&str> = k.splitn(3, ':').collect();
                if parts.len() < 2 {
                    return false;
                }
                let matches_scope = parts[1] == scope_str;
                let matches_kind = kind_filter.map_or(true, |f| parts[0] == f);
                matches_scope && matches_kind
            })
            .cloned()
            .collect();
        let count = keys_to_remove.len() as u32;
        for k in keys_to_remove {
            entries.remove(&k);
        }
        count
    }

    fn remove_by_rule_id(&self, rule_id: &PolicyId) -> u32 {
        let rid = rule_id.as_str().to_owned();
        let keys = {
            let mut rule_idx = self.rule_index.lock().unwrap();
            rule_idx.remove(&rid).unwrap_or_default()
        };
        let mut entries = self.entries.lock().unwrap();
        let mut count = 0u32;
        for k in keys {
            if entries.remove(&k).is_some() {
                count += 1;
            }
        }
        count
    }

    fn list_active(&self, scope: &ProjectKey, limit: usize) -> Vec<CachedDecisionSummary> {
        let now = Self::now_ms();
        let entries = self.entries.lock().unwrap();
        entries
            .iter()
            .filter(|(_, e)| e.outcome.is_some() && e.expires_at > now)
            .take(limit)
            .filter_map(|(_, e)| {
                // Filter by project scope (rough match).
                Some(CachedDecisionSummary {
                    decision_id: e.decision_id.clone(),
                    decision_key: DecisionKey {
                        kind_tag: String::new(),
                        scope_ref: DecisionScopeRef::Project(scope.clone()),
                        semantic_hash: String::new(),
                    },
                    outcome: e.outcome.clone()?,
                    kind_tag: String::new(),
                    scope: DecisionScopeRef::Project(scope.clone()),
                    ttl_remaining_secs: (e.expires_at.saturating_sub(now)) / 1000,
                    source: e.source.clone().unwrap_or(DecisionSource::FreshEvaluation),
                    hit_count: e.hit_count,
                    created_at: e.created_at,
                    expires_at: e.expires_at,
                })
            })
            .collect()
    }
}

fn scope_ref_string(s: &DecisionScopeRef) -> String {
    match s {
        DecisionScopeRef::Run { run_id, project } => {
            format!(
                "run:{}:{}/{}/{}",
                run_id, project.tenant_id, project.workspace_id, project.project_id
            )
        }
        DecisionScopeRef::Project(p) => {
            format!(
                "project:{}/{}/{}",
                p.tenant_id, p.workspace_id, p.project_id
            )
        }
        DecisionScopeRef::Workspace {
            tenant_id,
            workspace_id,
        } => {
            format!("workspace:{}/{}", tenant_id, workspace_id)
        }
        DecisionScopeRef::Tenant { tenant_id } => {
            format!("tenant:{}", tenant_id)
        }
    }
}

// ── DecisionServiceImpl ──────────────────────────────────────────────────────

/// Full 8-step decision pipeline implementation.
pub struct DecisionServiceImpl {
    scope_checker: Arc<dyn ScopeChecker>,
    visibility_checker: Arc<dyn VisibilityChecker>,
    guardrail_checker: Arc<dyn GuardrailChecker>,
    budget_checker: Arc<dyn BudgetChecker>,
    approval_resolver: Arc<dyn ApprovalResolver>,
    cache: DecisionCache,
    policies: HashMap<String, DecisionPolicy>,
}

impl DecisionServiceImpl {
    pub fn new() -> Self {
        Self::with_services(
            Arc::new(AllowAllScopeChecker),
            Arc::new(AllowAllVisibilityChecker),
            Arc::new(AllowAllGuardrailChecker),
            Arc::new(AllowAllBudgetChecker),
            Arc::new(AutoApproveResolver),
        )
    }

    pub fn with_services(
        scope_checker: Arc<dyn ScopeChecker>,
        visibility_checker: Arc<dyn VisibilityChecker>,
        guardrail_checker: Arc<dyn GuardrailChecker>,
        budget_checker: Arc<dyn BudgetChecker>,
        approval_resolver: Arc<dyn ApprovalResolver>,
    ) -> Self {
        Self {
            scope_checker,
            visibility_checker,
            guardrail_checker,
            budget_checker,
            approval_resolver,
            cache: DecisionCache::new(60_000),
            policies: Self::default_policies(),
        }
    }

    fn default_policies() -> HashMap<String, DecisionPolicy> {
        let mut m = HashMap::new();
        // Per RFC 019 table
        m.insert(
            "tool_invocation:observational".into(),
            DecisionPolicy {
                default_ttl_secs: 86400,
                default_scope: DecisionCacheScope::Project,
                cache_policy: CachePolicy::AlwaysCache,
                max_ttl_secs: 604800,
            },
        );
        m.insert(
            "tool_invocation:internal".into(),
            DecisionPolicy {
                default_ttl_secs: 86400,
                default_scope: DecisionCacheScope::Project,
                cache_policy: CachePolicy::AlwaysCache,
                max_ttl_secs: 604800,
            },
        );
        m.insert(
            "tool_invocation:external".into(),
            DecisionPolicy {
                default_ttl_secs: 14400,
                default_scope: DecisionCacheScope::Project,
                cache_policy: CachePolicy::CacheIfApproved,
                max_ttl_secs: 86400,
            },
        );
        m.insert(
            "provider_call".into(),
            DecisionPolicy {
                default_ttl_secs: 3600,
                default_scope: DecisionCacheScope::Run,
                cache_policy: CachePolicy::CacheIfApproved,
                max_ttl_secs: 86400,
            },
        );
        m.insert(
            "plugin_enablement".into(),
            DecisionPolicy {
                default_ttl_secs: 0,
                default_scope: DecisionCacheScope::Project,
                cache_policy: CachePolicy::NeverCache,
                max_ttl_secs: 0,
            },
        );
        m.insert(
            "workspace_provision".into(),
            DecisionPolicy {
                default_ttl_secs: 86400,
                default_scope: DecisionCacheScope::Project,
                cache_policy: CachePolicy::AlwaysCache,
                max_ttl_secs: 604800,
            },
        );
        m.insert(
            "trigger_fire".into(),
            DecisionPolicy {
                default_ttl_secs: 3600,
                default_scope: DecisionCacheScope::Project,
                cache_policy: CachePolicy::CacheIfApproved,
                max_ttl_secs: 14400,
            },
        );
        m.insert(
            "credential_access".into(),
            DecisionPolicy {
                default_ttl_secs: 3600,
                default_scope: DecisionCacheScope::Run,
                cache_policy: CachePolicy::CacheIfApproved,
                max_ttl_secs: 3600,
            },
        );
        m
    }

    fn resolve_policy(&self, kind: &DecisionKind) -> DecisionPolicy {
        let tag = kind_tag_for(kind);
        // For tool invocations, use the effect-specific policy.
        let key = if let DecisionKind::ToolInvocation { effect, .. } = kind {
            let effect_str = match effect {
                ToolEffect::Observational => "observational",
                ToolEffect::Internal => "internal",
                ToolEffect::External => "external",
            };
            format!("{tag}:{effect_str}")
        } else {
            tag.clone()
        };
        self.policies.get(&key).cloned().unwrap_or(DecisionPolicy {
            default_ttl_secs: 0,
            default_scope: DecisionCacheScope::Project,
            cache_policy: CachePolicy::NeverCache,
            max_ttl_secs: 0,
        })
    }

    fn derive_scope_ref(
        &self,
        request: &DecisionRequest,
        policy: &DecisionPolicy,
    ) -> DecisionScopeRef {
        match policy.default_scope {
            DecisionCacheScope::Run => {
                if let cairn_domain::decisions::Principal::Run { ref run_id } = request.principal {
                    DecisionScopeRef::Run {
                        run_id: run_id.clone(),
                        project: request.scope.clone(),
                    }
                } else {
                    DecisionScopeRef::Project(request.scope.clone())
                }
            }
            DecisionCacheScope::Project => DecisionScopeRef::Project(request.scope.clone()),
            DecisionCacheScope::Workspace => DecisionScopeRef::Workspace {
                tenant_id: request.scope.tenant_id.clone(),
                workspace_id: request.scope.workspace_id.clone(),
            },
            DecisionCacheScope::Tenant => DecisionScopeRef::Tenant {
                tenant_id: request.scope.tenant_id.clone(),
            },
        }
    }

    fn derive_semantic_hash(&self, request: &DecisionRequest) -> String {
        // Simplified: hash the kind tag + key fields.
        // Full implementation will use cache_on_fields from tool descriptors.
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        let kind_json = serde_json::to_string(&request.kind).unwrap_or_default();
        kind_json.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }

    fn should_cache(&self, policy: &DecisionPolicy, outcome: &DecisionOutcome) -> bool {
        match policy.cache_policy {
            CachePolicy::NeverCache => false,
            CachePolicy::AlwaysCache => true,
            CachePolicy::CacheIfApproved => matches!(outcome, DecisionOutcome::Allowed),
            CachePolicy::CacheIfDenied => matches!(outcome, DecisionOutcome::Denied { .. }),
        }
    }
}

impl Default for DecisionServiceImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DecisionService for DecisionServiceImpl {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResult, DecisionError> {
        let now_ms = DecisionCache::now_ms();
        let decision_id = DecisionId::new(format!("dec_{now_ms}_{}", request.correlation_id));
        let policy = self.resolve_policy(&request.kind);
        let scope_ref = self.derive_scope_ref(&request, &policy);
        let semantic_hash = self.derive_semantic_hash(&request);
        let decision_key = DecisionKey {
            kind_tag: kind_tag_for(&request.kind),
            scope_ref: scope_ref.clone(),
            semantic_hash,
        };

        let mut chain: Vec<StepResult> = Vec::with_capacity(8);
        let mut needs_escalation = false;
        let mut guardrail_rule_ids: Vec<PolicyId> = Vec::new();

        // ── Step 1: Scope check ─────────────────────────────────────────
        let scope_result = self.scope_checker.check(&request).await;
        match &scope_result {
            ScopeCheckResult::Allowed => {
                chain.push(StepResult {
                    step: 1,
                    name: "scope".into(),
                    outcome: "allow".into(),
                    detail: None,
                    rule_ids: vec![],
                });
            }
            ScopeCheckResult::Denied(reason) => {
                chain.push(StepResult {
                    step: 1,
                    name: "scope".into(),
                    outcome: "deny".into(),
                    detail: Some(reason.clone()),
                    rule_ids: vec![],
                });
                return Ok(self.build_denied_result(
                    decision_id,
                    request,
                    decision_key,
                    chain,
                    1,
                    reason,
                ));
            }
        }

        // ── Step 2: Visibility check ────────────────────────────────────
        let vis_result = self.visibility_checker.check(&request).await;
        match &vis_result {
            VisibilityCheckResult::Visible => {
                chain.push(StepResult {
                    step: 2,
                    name: "visibility".into(),
                    outcome: "allow".into(),
                    detail: None,
                    rule_ids: vec![],
                });
            }
            VisibilityCheckResult::NotInContext(reason) => {
                chain.push(StepResult {
                    step: 2,
                    name: "visibility".into(),
                    outcome: "deny".into(),
                    detail: Some(reason.clone()),
                    rule_ids: vec![],
                });
                return Ok(self.build_denied_result(
                    decision_id,
                    request,
                    decision_key,
                    chain,
                    2,
                    reason,
                ));
            }
        }

        // ── Step 3: Guardrail check ─────────────────────────────────────
        let guard_result = self.guardrail_checker.check(&request).await;
        guardrail_rule_ids = guard_result.rule_ids.clone();
        match &guard_result.outcome {
            GuardrailCheckOutcome::Allow => {
                chain.push(StepResult {
                    step: 3,
                    name: "guardrail".into(),
                    outcome: "allow".into(),
                    detail: None,
                    rule_ids: guard_result.rule_ids.clone(),
                });
            }
            GuardrailCheckOutcome::Deny(reason) => {
                chain.push(StepResult {
                    step: 3,
                    name: "guardrail".into(),
                    outcome: "deny".into(),
                    detail: Some(reason.clone()),
                    rule_ids: guard_result.rule_ids.clone(),
                });
                return Ok(self.build_denied_result(
                    decision_id,
                    request,
                    decision_key,
                    chain,
                    3,
                    reason,
                ));
            }
            GuardrailCheckOutcome::Escalate => {
                needs_escalation = true;
                chain.push(StepResult {
                    step: 3,
                    name: "guardrail".into(),
                    outcome: "escalate".into(),
                    detail: Some("requires approval".into()),
                    rule_ids: guard_result.rule_ids.clone(),
                });
            }
        }

        // ── Step 4: Budget check ────────────────────────────────────────
        let budget_result = self.budget_checker.check(&request).await;
        match &budget_result {
            BudgetCheckResult::Ok => {
                chain.push(StepResult {
                    step: 4,
                    name: "budget".into(),
                    outcome: "allow".into(),
                    detail: None,
                    rule_ids: vec![],
                });
            }
            BudgetCheckResult::SoftAlert { percent_used } => {
                chain.push(StepResult {
                    step: 4,
                    name: "budget".into(),
                    outcome: "allow".into(),
                    detail: Some(format!("soft_alert: {percent_used}% used")),
                    rule_ids: vec![],
                });
            }
            BudgetCheckResult::Exceeded(reason) => {
                chain.push(StepResult {
                    step: 4,
                    name: "budget".into(),
                    outcome: "deny".into(),
                    detail: Some(reason.clone()),
                    rule_ids: vec![],
                });
                return Ok(self.build_denied_result(
                    decision_id,
                    request,
                    decision_key,
                    chain,
                    4,
                    reason,
                ));
            }
        }

        // ── Step 5: Cache lookup ────────────────────────────────────────
        let cache_state = self.cache.lookup(&decision_key);
        match cache_state {
            CacheEntryState::Resolved {
                decision_id: cached_id,
                outcome: cached_outcome,
                expires_at,
            } => {
                self.cache.record_hit(&decision_key);
                chain.push(StepResult {
                    step: 5,
                    name: "cache".into(),
                    outcome: "cache_hit".into(),
                    detail: Some(format!(
                        "cached_decision={}, expires_at={expires_at}",
                        cached_id
                    )),
                    rule_ids: vec![],
                });
                // Steps 6-7 skipped for cache hits.
                chain.push(StepResult {
                    step: 6,
                    name: "resolver".into(),
                    outcome: "skip".into(),
                    detail: Some("cache hit".into()),
                    rule_ids: vec![],
                });
                chain.push(StepResult {
                    step: 7,
                    name: "cache_write".into(),
                    outcome: "skip".into(),
                    detail: Some("cache hit".into()),
                    rule_ids: vec![],
                });
                chain.push(StepResult {
                    step: 8,
                    name: "return".into(),
                    outcome: if matches!(cached_outcome, DecisionOutcome::Allowed) {
                        "allow"
                    } else {
                        "deny"
                    }
                    .into(),
                    detail: None,
                    rule_ids: vec![],
                });

                let source = DecisionSource::CacheHit {
                    original_decision_id: cached_id,
                };
                let event = DecisionEvent::DecisionRecorded {
                    decision_id: decision_id.clone(),
                    request,
                    outcome: cached_outcome.clone(),
                    reasoning_chain: chain,
                    source: source.clone(),
                    resolved_by: None,
                    cached_for: None,
                    decided_at: now_ms,
                };
                self.cache.store_event(&decision_id, &event);
                return Ok(DecisionResult {
                    decision_id,
                    outcome: cached_outcome,
                    decision_key,
                    event,
                });
            }
            CacheEntryState::Pending {
                owner_decision_id, ..
            } => {
                // Another evaluation is in-flight for this key.
                // Fall through to fresh evaluation (the pending entry will be
                // overwritten when the first evaluator completes, or cleaned up
                // if stale). Full singleflight coalescing with async waiters
                // will be added when tokio `time` feature is available.
                chain.push(StepResult {
                    step: 5,
                    name: "cache".into(),
                    outcome: "pending".into(),
                    detail: Some(format!(
                        "concurrent eval by {owner_decision_id}, proceeding fresh"
                    )),
                    rule_ids: vec![],
                });
            }
            CacheEntryState::Miss => {
                chain.push(StepResult {
                    step: 5,
                    name: "cache".into(),
                    outcome: "cache_miss".into(),
                    detail: None,
                    rule_ids: vec![],
                });
            }
        }

        // Install pending entry for singleflight.
        self.cache.install_pending(&decision_key, &decision_id);

        // ── Step 6: Approval resolution ─────────────────────────────────
        let (outcome, source) = if needs_escalation {
            let (res_outcome, res_source) = self.approval_resolver.resolve(&request).await;
            chain.push(StepResult {
                step: 6,
                name: "resolver".into(),
                outcome: if matches!(res_outcome, DecisionOutcome::Allowed) {
                    "allow"
                } else {
                    "deny"
                }
                .into(),
                detail: None,
                rule_ids: vec![],
            });
            (res_outcome, res_source)
        } else {
            chain.push(StepResult {
                step: 6,
                name: "resolver".into(),
                outcome: "skip".into(),
                detail: Some("no escalation needed".into()),
                rule_ids: vec![],
            });
            (DecisionOutcome::Allowed, DecisionSource::FreshEvaluation)
        };

        // ── Step 7: Cache write ─────────────────────────────────────────
        let cached_for = if self.should_cache(&policy, &outcome) && policy.default_ttl_secs > 0 {
            let expires_at = self.cache.promote_to_resolved(
                &decision_key,
                &decision_id,
                &outcome,
                &source,
                &chain,
                policy.default_ttl_secs,
            );
            chain.push(StepResult {
                step: 7,
                name: "cache_write".into(),
                outcome: "written".into(),
                detail: Some(format!(
                    "ttl={}s, expires_at={expires_at}",
                    policy.default_ttl_secs
                )),
                rule_ids: guardrail_rule_ids,
            });
            Some(CachedDecisionRef {
                decision_key: decision_key.clone(),
                expires_at,
            })
        } else {
            // Remove the pending entry since we're not caching.
            self.cache.remove_by_decision_id(&decision_id);
            chain.push(StepResult {
                step: 7,
                name: "cache_write".into(),
                outcome: "skip".into(),
                detail: Some("not cacheable".into()),
                rule_ids: vec![],
            });
            None
        };

        // ── Step 8: Return ──────────────────────────────────────────────
        chain.push(StepResult {
            step: 8,
            name: "return".into(),
            outcome: if matches!(outcome, DecisionOutcome::Allowed) {
                "allow"
            } else {
                "deny"
            }
            .into(),
            detail: None,
            rule_ids: vec![],
        });

        let event = DecisionEvent::DecisionRecorded {
            decision_id: decision_id.clone(),
            request,
            outcome: outcome.clone(),
            reasoning_chain: chain,
            source,
            resolved_by: None,
            cached_for,
            decided_at: DecisionCache::now_ms(),
        };
        self.cache.store_event(&decision_id, &event);

        Ok(DecisionResult {
            decision_id,
            outcome,
            decision_key,
            event,
        })
    }

    fn policy_for_kind(&self, kind_tag: &str) -> Option<DecisionPolicy> {
        self.policies.get(kind_tag).cloned()
    }

    async fn cache_lookup(&self, key: &DecisionKey) -> Result<CacheEntryState, DecisionError> {
        Ok(self.cache.lookup(key))
    }

    async fn invalidate(
        &self,
        decision_id: &DecisionId,
        _reason: &str,
        _invalidated_by: ActorRef,
    ) -> Result<(), DecisionError> {
        self.cache.remove_by_decision_id(decision_id);
        Ok(())
    }

    async fn invalidate_by_scope(
        &self,
        scope: &DecisionScopeRef,
        kind_filter: Option<&str>,
        _reason: &str,
        _invalidated_by: ActorRef,
    ) -> Result<u32, DecisionError> {
        Ok(self.cache.remove_by_scope(scope, kind_filter))
    }

    async fn invalidate_by_rule(
        &self,
        rule_id: &PolicyId,
        _reason: &str,
        _invalidated_by: ActorRef,
    ) -> Result<u32, DecisionError> {
        Ok(self.cache.remove_by_rule_id(rule_id))
    }

    async fn list_cached(
        &self,
        scope: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<CachedDecisionSummary>, DecisionError> {
        Ok(self.cache.list_active(scope, limit))
    }

    async fn get_decision(
        &self,
        decision_id: &DecisionId,
    ) -> Result<Option<DecisionEvent>, DecisionError> {
        Ok(self.cache.get_event(decision_id))
    }
}

impl DecisionServiceImpl {
    fn build_denied_result(
        &self,
        decision_id: DecisionId,
        request: DecisionRequest,
        decision_key: DecisionKey,
        mut chain: Vec<StepResult>,
        deny_step: u8,
        deny_reason: &str,
    ) -> DecisionResult {
        // Fill remaining steps as skipped.
        for step in (deny_step + 1)..=8 {
            let name = match step {
                2 => "visibility",
                3 => "guardrail",
                4 => "budget",
                5 => "cache",
                6 => "resolver",
                7 => "cache_write",
                8 => "return",
                _ => "unknown",
            };
            if step == 8 {
                chain.push(StepResult {
                    step,
                    name: name.into(),
                    outcome: "deny".into(),
                    detail: Some(format!("denied at step {deny_step}")),
                    rule_ids: vec![],
                });
            } else {
                chain.push(StepResult {
                    step,
                    name: name.into(),
                    outcome: "skip".into(),
                    detail: Some(format!("denied at step {deny_step}")),
                    rule_ids: vec![],
                });
            }
        }

        let outcome = DecisionOutcome::Denied {
            deny_step,
            deny_reason: deny_reason.to_owned(),
        };
        let event = DecisionEvent::DecisionRecorded {
            decision_id: decision_id.clone(),
            request,
            outcome: outcome.clone(),
            reasoning_chain: chain,
            source: DecisionSource::FreshEvaluation,
            resolved_by: None,
            cached_for: None,
            decided_at: DecisionCache::now_ms(),
        };
        self.cache.store_event(&decision_id, &event);

        DecisionResult {
            decision_id,
            outcome,
            decision_key,
            event,
        }
    }
}

// ── Stub implementation ─────────────────────────────────────────────────────

/// Stub `DecisionService` that allows everything (no pipeline).
pub struct StubDecisionService;

impl StubDecisionService {
    pub fn new() -> Self {
        Self
    }
}

impl Default for StubDecisionService {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DecisionService for StubDecisionService {
    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionResult, DecisionError> {
        let now_ms = DecisionCache::now_ms();
        let decision_id = DecisionId::new(format!("dec_{now_ms}"));
        let decision_key = DecisionKey {
            kind_tag: kind_tag_for(&request.kind),
            scope_ref: DecisionScopeRef::Project(request.scope.clone()),
            semantic_hash: "stub".into(),
        };
        let event = DecisionEvent::DecisionRecorded {
            decision_id: decision_id.clone(),
            request: request.clone(),
            outcome: DecisionOutcome::Allowed,
            reasoning_chain: vec![StepResult {
                step: 8,
                name: "stub_allow_all".into(),
                outcome: "allow".into(),
                detail: Some("StubDecisionService — all requests allowed".into()),
                rule_ids: vec![],
            }],
            source: DecisionSource::FreshEvaluation,
            resolved_by: None,
            cached_for: None,
            decided_at: now_ms,
        };
        Ok(DecisionResult {
            decision_id,
            outcome: DecisionOutcome::Allowed,
            decision_key,
            event,
        })
    }
    fn policy_for_kind(&self, _: &str) -> Option<DecisionPolicy> {
        None
    }
    async fn cache_lookup(&self, _: &DecisionKey) -> Result<CacheEntryState, DecisionError> {
        Ok(CacheEntryState::Miss)
    }
    async fn invalidate(&self, _: &DecisionId, _: &str, _: ActorRef) -> Result<(), DecisionError> {
        Ok(())
    }
    async fn invalidate_by_scope(
        &self,
        _: &DecisionScopeRef,
        _: Option<&str>,
        _: &str,
        _: ActorRef,
    ) -> Result<u32, DecisionError> {
        Ok(0)
    }
    async fn invalidate_by_rule(
        &self,
        _: &PolicyId,
        _: &str,
        _: ActorRef,
    ) -> Result<u32, DecisionError> {
        Ok(0)
    }
    async fn list_cached(
        &self,
        _: &ProjectKey,
        _: usize,
    ) -> Result<Vec<CachedDecisionSummary>, DecisionError> {
        Ok(vec![])
    }
    async fn get_decision(&self, _: &DecisionId) -> Result<Option<DecisionEvent>, DecisionError> {
        Ok(None)
    }
}

/// Derive the `kind_tag` string from a `DecisionKind`.
pub fn kind_tag_for(kind: &DecisionKind) -> String {
    match kind {
        DecisionKind::ToolInvocation { .. } => "tool_invocation".into(),
        DecisionKind::ProviderCall { .. } => "provider_call".into(),
        DecisionKind::PluginEnablement { .. } => "plugin_enablement".into(),
        DecisionKind::WorkspaceProvision { .. } => "workspace_provision".into(),
        DecisionKind::CredentialAccess { .. } => "credential_access".into(),
        DecisionKind::TriggerFire { .. } => "trigger_fire".into(),
        DecisionKind::DestructiveAction { .. } => "destructive_action".into(),
        DecisionKind::Other(s) => format!("other:{s}"),
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::decisions::*;
    use cairn_domain::ids::*;

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

    fn observational_request() -> DecisionRequest {
        DecisionRequest {
            kind: DecisionKind::ToolInvocation {
                tool_name: "grep_search".into(),
                effect: ToolEffect::Observational,
            },
            principal: Principal::Run {
                run_id: RunId::new("run_1"),
            },
            subject: DecisionSubject::ToolCall {
                tool_name: "grep_search".into(),
                args: serde_json::json!({"pattern": "TODO"}),
            },
            scope: ProjectKey::new("t", "w", "p"),
            cost_estimate: None,
            requested_at: 1700000000000,
            correlation_id: CorrelationId::new("cor_2"),
        }
    }

    // ── Stub tests (preserved from Step 1) ──────────────────────────────

    #[tokio::test]
    async fn stub_service_allows_everything() {
        let svc = StubDecisionService::new();
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert_eq!(result.outcome, DecisionOutcome::Allowed);
    }

    #[tokio::test]
    async fn stub_cache_lookup_returns_miss() {
        let svc = StubDecisionService::new();
        let key = DecisionKey {
            kind_tag: "tool_invocation".into(),
            scope_ref: DecisionScopeRef::Project(ProjectKey::new("t", "w", "p")),
            semantic_hash: "abc".into(),
        };
        assert!(matches!(
            svc.cache_lookup(&key).await.unwrap(),
            CacheEntryState::Miss
        ));
    }

    #[test]
    fn kind_tag_derivation() {
        assert_eq!(
            kind_tag_for(&DecisionKind::ToolInvocation {
                tool_name: "x".into(),
                effect: ToolEffect::External,
            }),
            "tool_invocation"
        );
        assert_eq!(
            kind_tag_for(&DecisionKind::TriggerFire {
                trigger_id: "t".into(),
                signal_type: "webhook".into(),
            }),
            "trigger_fire"
        );
    }

    // ── Pipeline tests ──────────────────────────────────────────────────

    #[tokio::test]
    async fn pipeline_allows_when_all_steps_pass() {
        let svc = DecisionServiceImpl::new();
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert_eq!(result.outcome, DecisionOutcome::Allowed);
        if let DecisionEvent::DecisionRecorded {
            reasoning_chain, ..
        } = &result.event
        {
            assert_eq!(reasoning_chain.len(), 8);
            assert_eq!(reasoning_chain[0].name, "scope");
            assert_eq!(reasoning_chain[7].name, "return");
            assert_eq!(reasoning_chain[7].outcome, "allow");
        } else {
            panic!("expected DecisionRecorded");
        }
    }

    #[tokio::test]
    async fn pipeline_scope_deny_stops_at_step_1() {
        struct DenyScope;
        #[async_trait]
        impl ScopeChecker for DenyScope {
            async fn check(&self, _: &DecisionRequest) -> ScopeCheckResult {
                ScopeCheckResult::Denied("scope_violation".into())
            }
        }
        let svc = DecisionServiceImpl::with_services(
            Arc::new(DenyScope),
            Arc::new(AllowAllVisibilityChecker),
            Arc::new(AllowAllGuardrailChecker),
            Arc::new(AllowAllBudgetChecker),
            Arc::new(AutoApproveResolver),
        );
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert!(matches!(
            result.outcome,
            DecisionOutcome::Denied { deny_step: 1, .. }
        ));
        // Verify remaining steps are skipped.
        if let DecisionEvent::DecisionRecorded {
            reasoning_chain, ..
        } = &result.event
        {
            assert_eq!(reasoning_chain.len(), 8);
            assert_eq!(reasoning_chain[1].outcome, "skip"); // visibility
        }
    }

    #[tokio::test]
    async fn pipeline_visibility_deny_stops_at_step_2() {
        struct DenyVis;
        #[async_trait]
        impl VisibilityChecker for DenyVis {
            async fn check(&self, _: &DecisionRequest) -> VisibilityCheckResult {
                VisibilityCheckResult::NotInContext("not_in_context".into())
            }
        }
        let svc = DecisionServiceImpl::with_services(
            Arc::new(AllowAllScopeChecker),
            Arc::new(DenyVis),
            Arc::new(AllowAllGuardrailChecker),
            Arc::new(AllowAllBudgetChecker),
            Arc::new(AutoApproveResolver),
        );
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert!(matches!(
            result.outcome,
            DecisionOutcome::Denied { deny_step: 2, .. }
        ));
    }

    #[tokio::test]
    async fn pipeline_guardrail_deny_stops_at_step_3() {
        struct DenyGuard;
        #[async_trait]
        impl GuardrailChecker for DenyGuard {
            async fn check(&self, _: &DecisionRequest) -> GuardrailCheckResult {
                GuardrailCheckResult {
                    outcome: GuardrailCheckOutcome::Deny("guardrail_denied".into()),
                    rule_ids: vec![PolicyId::new("rule_1")],
                }
            }
        }
        let svc = DecisionServiceImpl::with_services(
            Arc::new(AllowAllScopeChecker),
            Arc::new(AllowAllVisibilityChecker),
            Arc::new(DenyGuard),
            Arc::new(AllowAllBudgetChecker),
            Arc::new(AutoApproveResolver),
        );
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert!(matches!(
            result.outcome,
            DecisionOutcome::Denied { deny_step: 3, .. }
        ));
    }

    #[tokio::test]
    async fn pipeline_budget_deny_stops_at_step_4() {
        struct DenyBudget;
        #[async_trait]
        impl BudgetChecker for DenyBudget {
            async fn check(&self, _: &DecisionRequest) -> BudgetCheckResult {
                BudgetCheckResult::Exceeded("budget_exceeded".into())
            }
        }
        let svc = DecisionServiceImpl::with_services(
            Arc::new(AllowAllScopeChecker),
            Arc::new(AllowAllVisibilityChecker),
            Arc::new(AllowAllGuardrailChecker),
            Arc::new(DenyBudget),
            Arc::new(AutoApproveResolver),
        );
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert!(matches!(
            result.outcome,
            DecisionOutcome::Denied { deny_step: 4, .. }
        ));
    }

    #[tokio::test]
    async fn pipeline_fresh_eval_then_cache_hit() {
        let svc = DecisionServiceImpl::new();
        // First request: fresh evaluation.
        let r1 = svc.evaluate(observational_request()).await.unwrap();
        assert_eq!(r1.outcome, DecisionOutcome::Allowed);
        // Second identical request: should be a cache hit.
        let r2 = svc.evaluate(observational_request()).await.unwrap();
        assert_eq!(r2.outcome, DecisionOutcome::Allowed);
        if let DecisionEvent::DecisionRecorded {
            source,
            reasoning_chain,
            ..
        } = &r2.event
        {
            assert!(matches!(source, DecisionSource::CacheHit { .. }));
            let cache_step = reasoning_chain.iter().find(|s| s.step == 5).unwrap();
            assert_eq!(cache_step.outcome, "cache_hit");
        } else {
            panic!("expected DecisionRecorded");
        }
    }

    #[tokio::test]
    async fn pipeline_never_cache_policy_skips_cache() {
        let svc = DecisionServiceImpl::new();
        let req = DecisionRequest {
            kind: DecisionKind::PluginEnablement {
                plugin_id: "test-plugin".into(),
                target_project: ProjectKey::new("t", "w", "p"),
            },
            principal: Principal::Run {
                run_id: RunId::new("run_1"),
            },
            subject: DecisionSubject::Resource {
                resource_type: "plugin".into(),
                resource_id: "test-plugin".into(),
            },
            scope: ProjectKey::new("t", "w", "p"),
            cost_estimate: None,
            requested_at: 1700000000000,
            correlation_id: CorrelationId::new("cor_nc"),
        };
        let r1 = svc.evaluate(req.clone()).await.unwrap();
        assert_eq!(r1.outcome, DecisionOutcome::Allowed);
        // Second request: should NOT be a cache hit (NeverCache).
        let r2 = svc.evaluate(req).await.unwrap();
        if let DecisionEvent::DecisionRecorded { source, .. } = &r2.event {
            assert!(matches!(source, DecisionSource::FreshEvaluation));
        }
    }

    #[tokio::test]
    async fn pipeline_guardrail_escalate_triggers_resolver() {
        struct EscalateGuard;
        #[async_trait]
        impl GuardrailChecker for EscalateGuard {
            async fn check(&self, _: &DecisionRequest) -> GuardrailCheckResult {
                GuardrailCheckResult {
                    outcome: GuardrailCheckOutcome::Escalate,
                    rule_ids: vec![PolicyId::new("esc_rule")],
                }
            }
        }
        let svc = DecisionServiceImpl::with_services(
            Arc::new(AllowAllScopeChecker),
            Arc::new(AllowAllVisibilityChecker),
            Arc::new(EscalateGuard),
            Arc::new(AllowAllBudgetChecker),
            Arc::new(AutoApproveResolver), // auto-approves
        );
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert_eq!(result.outcome, DecisionOutcome::Allowed);
        if let DecisionEvent::DecisionRecorded {
            reasoning_chain, ..
        } = &result.event
        {
            let guard_step = reasoning_chain.iter().find(|s| s.step == 3).unwrap();
            assert_eq!(guard_step.outcome, "escalate");
            let resolver_step = reasoning_chain.iter().find(|s| s.step == 6).unwrap();
            assert_eq!(resolver_step.outcome, "allow");
        }
    }

    #[tokio::test]
    async fn pipeline_resolver_deny_produces_denied_outcome() {
        struct EscalateGuard;
        #[async_trait]
        impl GuardrailChecker for EscalateGuard {
            async fn check(&self, _: &DecisionRequest) -> GuardrailCheckResult {
                GuardrailCheckResult {
                    outcome: GuardrailCheckOutcome::Escalate,
                    rule_ids: vec![],
                }
            }
        }
        struct DenyResolver;
        #[async_trait]
        impl ApprovalResolver for DenyResolver {
            async fn resolve(&self, _: &DecisionRequest) -> (DecisionOutcome, DecisionSource) {
                (
                    DecisionOutcome::Denied {
                        deny_step: 6,
                        deny_reason: "operator_rejected".into(),
                    },
                    DecisionSource::Human {
                        operator_id: OperatorId::new("op1"),
                    },
                )
            }
        }
        let svc = DecisionServiceImpl::with_services(
            Arc::new(AllowAllScopeChecker),
            Arc::new(AllowAllVisibilityChecker),
            Arc::new(EscalateGuard),
            Arc::new(AllowAllBudgetChecker),
            Arc::new(DenyResolver),
        );
        let result = svc.evaluate(sample_request()).await.unwrap();
        assert!(matches!(result.outcome, DecisionOutcome::Denied { .. }));
    }

    #[tokio::test]
    async fn invalidation_removes_cached_decision() {
        let svc = DecisionServiceImpl::new();
        let r1 = svc.evaluate(observational_request()).await.unwrap();
        assert_eq!(r1.outcome, DecisionOutcome::Allowed);
        // Verify cache hit.
        let state = svc.cache_lookup(&r1.decision_key).await.unwrap();
        assert!(matches!(state, CacheEntryState::Resolved { .. }));
        // Invalidate.
        svc.invalidate(&r1.decision_id, "test", ActorRef::SystemPolicyChange)
            .await
            .unwrap();
        // Verify miss.
        let state2 = svc.cache_lookup(&r1.decision_key).await.unwrap();
        assert!(matches!(state2, CacheEntryState::Miss));
    }

    #[tokio::test]
    async fn invalidate_by_rule_removes_matching_entries() {
        struct TaggedGuard;
        #[async_trait]
        impl GuardrailChecker for TaggedGuard {
            async fn check(&self, _: &DecisionRequest) -> GuardrailCheckResult {
                GuardrailCheckResult {
                    outcome: GuardrailCheckOutcome::Allow,
                    rule_ids: vec![PolicyId::new("rule_x")],
                }
            }
        }
        let svc = DecisionServiceImpl::with_services(
            Arc::new(AllowAllScopeChecker),
            Arc::new(AllowAllVisibilityChecker),
            Arc::new(TaggedGuard),
            Arc::new(AllowAllBudgetChecker),
            Arc::new(AutoApproveResolver),
        );
        let r1 = svc.evaluate(observational_request()).await.unwrap();
        assert!(matches!(
            svc.cache_lookup(&r1.decision_key).await.unwrap(),
            CacheEntryState::Resolved { .. }
        ));
        // Invalidate by rule.
        let count = svc
            .invalidate_by_rule(
                &PolicyId::new("rule_x"),
                "policy changed",
                ActorRef::SystemPolicyChange,
            )
            .await
            .unwrap();
        assert!(count >= 1);
        assert!(matches!(
            svc.cache_lookup(&r1.decision_key).await.unwrap(),
            CacheEntryState::Miss
        ));
    }

    #[tokio::test]
    async fn reasoning_chain_has_all_8_steps() {
        let svc = DecisionServiceImpl::new();
        let result = svc.evaluate(sample_request()).await.unwrap();
        if let DecisionEvent::DecisionRecorded {
            reasoning_chain, ..
        } = &result.event
        {
            assert_eq!(reasoning_chain.len(), 8);
            for (i, step) in reasoning_chain.iter().enumerate() {
                assert_eq!(step.step as usize, i + 1);
            }
        }
    }

    #[tokio::test]
    async fn get_decision_returns_stored_event() {
        let svc = DecisionServiceImpl::new();
        let result = svc.evaluate(sample_request()).await.unwrap();
        let event = svc.get_decision(&result.decision_id).await.unwrap();
        assert!(event.is_some());
    }
}
