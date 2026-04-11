//! RFC 008 tenant isolation and workspace quota enforcement.
//!
//! ## Gaps implemented
//!
//! 1. **Workspace quotas** — `WorkspaceQuotaPolicy` with max_runs_per_hour,
//!    max_concurrent_runs, max_storage_mb, max_tokens_per_day. Enforced via
//!    `check_run_quota()` / `check_token_quota()`.
//! 2. **Cross-tenant access policies** — `TenantAccessPolicy` defines which
//!    tenants may share resources. Default: no cross-tenant access.
//! 3. **Workspace-level isolation** — `WorkspaceIsolationGuard` verifies
//!    that a `ProjectKey` belongs to the expected workspace before any
//!    store query.
//! 4. **Quota usage tracking** — `WorkspaceUsage` tracks current run count,
//!    token usage, and storage per workspace. `usage_report()` returns
//!    current vs limits.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use cairn_domain::{TenantId, WorkspaceId};
use serde::{Deserialize, Serialize};

// ── Gap 1: Workspace Quota Policy ────────────────────────────────────────────

/// Workspace-level quota policy per RFC 008.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceQuotaPolicy {
    pub workspace_id: WorkspaceId,
    pub tenant_id: TenantId,
    pub max_runs_per_hour: u32,
    pub max_concurrent_runs: u32,
    pub max_storage_mb: u64,
    pub max_tokens_per_day: u64,
}

impl WorkspaceQuotaPolicy {
    pub fn new(workspace_id: WorkspaceId, tenant_id: TenantId) -> Self {
        Self {
            workspace_id,
            tenant_id,
            max_runs_per_hour: 0,
            max_concurrent_runs: 0,
            max_storage_mb: 0,
            max_tokens_per_day: 0,
        }
    }

    /// True when all limits are 0 (disabled).
    pub fn is_unlimited(&self) -> bool {
        self.max_runs_per_hour == 0
            && self.max_concurrent_runs == 0
            && self.max_storage_mb == 0
            && self.max_tokens_per_day == 0
    }
}

// ── Gap 4: Workspace Usage Tracking ──────────────────────────────────────────

/// Current usage counters for a single workspace.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WorkspaceUsage {
    pub workspace_id: String,
    pub active_runs: u32,
    pub runs_this_hour: u32,
    pub tokens_today: u64,
    pub storage_mb: u64,
    /// Unix ms when the hourly run counter was last reset.
    pub hour_reset_ms: u64,
    /// Unix ms when the daily token counter was last reset.
    pub day_reset_ms: u64,
}

/// Usage report combining current usage with the configured limits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorkspaceUsageReport {
    pub workspace_id: String,
    pub active_runs: u32,
    pub max_concurrent_runs: u32,
    pub runs_this_hour: u32,
    pub max_runs_per_hour: u32,
    pub tokens_today: u64,
    pub max_tokens_per_day: u64,
    pub storage_mb: u64,
    pub max_storage_mb: u64,
}

// ── Gap 2: Cross-Tenant Access Policy ────────────────────────────────────────

/// Defines which tenants a given tenant may share resources with.
///
/// Default: no cross-tenant access. Each tenant must explicitly allow
/// sharing with specific other tenants.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TenantAccessPolicy {
    pub tenant_id: TenantId,
    /// Set of tenant IDs that this tenant permits resource sharing with.
    pub allowed_tenants: HashSet<String>,
}

impl TenantAccessPolicy {
    /// Create a default policy that permits no cross-tenant access.
    pub fn deny_all(tenant_id: TenantId) -> Self {
        Self {
            tenant_id,
            allowed_tenants: HashSet::new(),
        }
    }

    pub fn allow_tenant(&mut self, other_tenant_id: &str) {
        self.allowed_tenants.insert(other_tenant_id.to_owned());
    }

    pub fn revoke_tenant(&mut self, other_tenant_id: &str) {
        self.allowed_tenants.remove(other_tenant_id);
    }

    /// Check whether cross-tenant access is permitted from this tenant
    /// to `target_tenant_id`.
    pub fn can_access(&self, target_tenant_id: &str) -> bool {
        // Same-tenant always allowed.
        if self.tenant_id.as_str() == target_tenant_id {
            return true;
        }
        self.allowed_tenants.contains(target_tenant_id)
    }
}

// ── Gap 3: Workspace Isolation Guard ─────────────────────────────────────────

/// Verification error when a request violates tenant/workspace isolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationViolation {
    /// The workspace_id in the project key does not match the expected scope.
    WorkspaceMismatch { expected: String, actual: String },
    /// Cross-tenant access denied.
    TenantAccessDenied {
        source_tenant: String,
        target_tenant: String,
    },
}

impl std::fmt::Display for IsolationViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolationViolation::WorkspaceMismatch { expected, actual } => {
                write!(f, "workspace mismatch: expected {expected}, got {actual}")
            }
            IsolationViolation::TenantAccessDenied {
                source_tenant,
                target_tenant,
            } => {
                write!(
                    f,
                    "cross-tenant access denied: {source_tenant} → {target_tenant}"
                )
            }
        }
    }
}

impl std::error::Error for IsolationViolation {}

/// Verify that a `ProjectKey` is scoped to the expected workspace.
pub fn assert_workspace_scope(
    project: &cairn_domain::ProjectKey,
    expected_workspace: &WorkspaceId,
) -> Result<(), IsolationViolation> {
    if project.workspace_id != *expected_workspace {
        return Err(IsolationViolation::WorkspaceMismatch {
            expected: expected_workspace.as_str().to_owned(),
            actual: project.workspace_id.as_str().to_owned(),
        });
    }
    Ok(())
}

/// Verify cross-tenant access using the access policy.
pub fn assert_tenant_access(
    policy: &TenantAccessPolicy,
    target_tenant: &TenantId,
) -> Result<(), IsolationViolation> {
    if !policy.can_access(target_tenant.as_str()) {
        return Err(IsolationViolation::TenantAccessDenied {
            source_tenant: policy.tenant_id.as_str().to_owned(),
            target_tenant: target_tenant.as_str().to_owned(),
        });
    }
    Ok(())
}

// ── Quota + Usage Manager ────────────────────────────────────────────────────

/// Quota violation detail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaViolation {
    pub workspace_id: String,
    pub quota_type: String,
    pub current: u64,
    pub limit: u64,
}

impl std::fmt::Display for QuotaViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "workspace {} quota exceeded: {} ({}/{})",
            self.workspace_id, self.quota_type, self.current, self.limit
        )
    }
}

impl std::error::Error for QuotaViolation {}

/// One hour in milliseconds.
const HOUR_MS: u64 = 3_600_000;
/// One day in milliseconds.
const DAY_MS: u64 = 86_400_000;

/// In-memory workspace quota and usage manager.
///
/// Thread-safe — designed to be shared via `Arc<WorkspaceQuotaManager>`.
pub struct WorkspaceQuotaManager {
    policies: RwLock<HashMap<String, WorkspaceQuotaPolicy>>,
    usage: RwLock<HashMap<String, WorkspaceUsage>>,
    tenant_access: RwLock<HashMap<String, TenantAccessPolicy>>,
}

impl WorkspaceQuotaManager {
    pub fn new() -> Self {
        Self {
            policies: RwLock::new(HashMap::new()),
            usage: RwLock::new(HashMap::new()),
            tenant_access: RwLock::new(HashMap::new()),
        }
    }

    // ── Policy management ────────────────────────────────────────────────

    /// Set the quota policy for a workspace.
    pub fn set_policy(&self, policy: WorkspaceQuotaPolicy) {
        self.policies
            .write()
            .unwrap()
            .insert(policy.workspace_id.as_str().to_owned(), policy);
    }

    /// Get the quota policy for a workspace.
    pub fn get_policy(&self, workspace_id: &WorkspaceId) -> Option<WorkspaceQuotaPolicy> {
        self.policies
            .read()
            .unwrap()
            .get(workspace_id.as_str())
            .cloned()
    }

    // ── Tenant access policy management ──────────────────────────────────

    /// Set the cross-tenant access policy for a tenant.
    pub fn set_tenant_access(&self, policy: TenantAccessPolicy) {
        self.tenant_access
            .write()
            .unwrap()
            .insert(policy.tenant_id.as_str().to_owned(), policy);
    }

    /// Get the access policy for a tenant (returns deny-all if not set).
    pub fn get_tenant_access(&self, tenant_id: &TenantId) -> TenantAccessPolicy {
        self.tenant_access
            .read()
            .unwrap()
            .get(tenant_id.as_str())
            .cloned()
            .unwrap_or_else(|| TenantAccessPolicy::deny_all(tenant_id.clone()))
    }

    /// Check whether cross-tenant access is permitted.
    pub fn check_tenant_access(
        &self,
        source_tenant: &TenantId,
        target_tenant: &TenantId,
    ) -> Result<(), IsolationViolation> {
        let policy = self.get_tenant_access(source_tenant);
        assert_tenant_access(&policy, target_tenant)
    }

    // ── Usage tracking ───────────────────────────────────────────────────

    fn get_or_create_usage(&self, workspace_id: &str) -> WorkspaceUsage {
        let mut map = self.usage.write().unwrap();
        map.entry(workspace_id.to_owned())
            .or_insert_with(|| WorkspaceUsage {
                workspace_id: workspace_id.to_owned(),
                ..Default::default()
            })
            .clone()
    }

    fn maybe_reset_counters(usage: &mut WorkspaceUsage) {
        let now = now_ms();
        if now.saturating_sub(usage.hour_reset_ms) >= HOUR_MS {
            usage.runs_this_hour = 0;
            usage.hour_reset_ms = now;
        }
        if now.saturating_sub(usage.day_reset_ms) >= DAY_MS {
            usage.tokens_today = 0;
            usage.day_reset_ms = now;
        }
    }

    /// Record that a new run started in a workspace.
    pub fn record_run_started(&self, workspace_id: &str) {
        let mut map = self.usage.write().unwrap();
        let usage = map
            .entry(workspace_id.to_owned())
            .or_insert_with(|| WorkspaceUsage {
                workspace_id: workspace_id.to_owned(),
                ..Default::default()
            });
        Self::maybe_reset_counters(usage);
        usage.active_runs += 1;
        usage.runs_this_hour += 1;
    }

    /// Record that a run completed (decrement active count).
    pub fn record_run_completed(&self, workspace_id: &str) {
        let mut map = self.usage.write().unwrap();
        if let Some(usage) = map.get_mut(workspace_id) {
            usage.active_runs = usage.active_runs.saturating_sub(1);
        }
    }

    /// Record token consumption.
    pub fn record_tokens(&self, workspace_id: &str, tokens: u64) {
        let mut map = self.usage.write().unwrap();
        let usage = map
            .entry(workspace_id.to_owned())
            .or_insert_with(|| WorkspaceUsage {
                workspace_id: workspace_id.to_owned(),
                ..Default::default()
            });
        Self::maybe_reset_counters(usage);
        usage.tokens_today = usage.tokens_today.saturating_add(tokens);
    }

    /// Record storage increase.
    pub fn record_storage(&self, workspace_id: &str, mb: u64) {
        let mut map = self.usage.write().unwrap();
        let usage = map
            .entry(workspace_id.to_owned())
            .or_insert_with(|| WorkspaceUsage {
                workspace_id: workspace_id.to_owned(),
                ..Default::default()
            });
        usage.storage_mb = usage.storage_mb.saturating_add(mb);
    }

    /// Get current usage for a workspace.
    pub fn get_usage(&self, workspace_id: &str) -> WorkspaceUsage {
        self.get_or_create_usage(workspace_id)
    }

    // ── Quota enforcement ────────────────────────────────────────────────

    /// Check whether starting a new run would violate workspace quotas.
    ///
    /// Checks both `max_concurrent_runs` and `max_runs_per_hour`.
    pub fn check_run_quota(&self, workspace_id: &WorkspaceId) -> Result<(), QuotaViolation> {
        let ws = workspace_id.as_str();
        let policy = match self.get_policy(workspace_id) {
            Some(p) => p,
            None => return Ok(()), // no policy = unlimited
        };

        let mut map = self.usage.write().unwrap();
        let usage = map.entry(ws.to_owned()).or_insert_with(|| WorkspaceUsage {
            workspace_id: ws.to_owned(),
            ..Default::default()
        });
        Self::maybe_reset_counters(usage);

        if policy.max_concurrent_runs > 0 && usage.active_runs >= policy.max_concurrent_runs {
            return Err(QuotaViolation {
                workspace_id: ws.to_owned(),
                quota_type: "max_concurrent_runs".to_owned(),
                current: usage.active_runs as u64,
                limit: policy.max_concurrent_runs as u64,
            });
        }

        if policy.max_runs_per_hour > 0 && usage.runs_this_hour >= policy.max_runs_per_hour {
            return Err(QuotaViolation {
                workspace_id: ws.to_owned(),
                quota_type: "max_runs_per_hour".to_owned(),
                current: usage.runs_this_hour as u64,
                limit: policy.max_runs_per_hour as u64,
            });
        }

        Ok(())
    }

    /// Check whether token usage would violate the daily token quota.
    pub fn check_token_quota(
        &self,
        workspace_id: &WorkspaceId,
        additional_tokens: u64,
    ) -> Result<(), QuotaViolation> {
        let ws = workspace_id.as_str();
        let policy = match self.get_policy(workspace_id) {
            Some(p) => p,
            None => return Ok(()),
        };

        if policy.max_tokens_per_day == 0 {
            return Ok(());
        }

        let mut map = self.usage.write().unwrap();
        let usage = map.entry(ws.to_owned()).or_insert_with(|| WorkspaceUsage {
            workspace_id: ws.to_owned(),
            ..Default::default()
        });
        Self::maybe_reset_counters(usage);

        let projected = usage.tokens_today.saturating_add(additional_tokens);
        if projected > policy.max_tokens_per_day {
            return Err(QuotaViolation {
                workspace_id: ws.to_owned(),
                quota_type: "max_tokens_per_day".to_owned(),
                current: usage.tokens_today,
                limit: policy.max_tokens_per_day,
            });
        }

        Ok(())
    }

    /// Check whether storage usage is within limits.
    pub fn check_storage_quota(&self, workspace_id: &WorkspaceId) -> Result<(), QuotaViolation> {
        let ws = workspace_id.as_str();
        let policy = match self.get_policy(workspace_id) {
            Some(p) => p,
            None => return Ok(()),
        };

        if policy.max_storage_mb == 0 {
            return Ok(());
        }

        let map = self.usage.read().unwrap();
        let storage = map.get(ws).map(|u| u.storage_mb).unwrap_or(0);

        if storage >= policy.max_storage_mb {
            return Err(QuotaViolation {
                workspace_id: ws.to_owned(),
                quota_type: "max_storage_mb".to_owned(),
                current: storage,
                limit: policy.max_storage_mb,
            });
        }

        Ok(())
    }

    // ── Gap 4: Usage report ──────────────────────────────────────────────

    /// Build a usage report combining current counters with policy limits.
    pub fn usage_report(&self, workspace_id: &WorkspaceId) -> WorkspaceUsageReport {
        let ws = workspace_id.as_str();
        let usage = self.get_or_create_usage(ws);
        let policy = self.get_policy(workspace_id);

        let (max_concurrent, max_hour, max_tokens, max_storage) = match policy {
            Some(p) => (
                p.max_concurrent_runs,
                p.max_runs_per_hour,
                p.max_tokens_per_day,
                p.max_storage_mb,
            ),
            None => (0, 0, 0, 0),
        };

        WorkspaceUsageReport {
            workspace_id: ws.to_owned(),
            active_runs: usage.active_runs,
            max_concurrent_runs: max_concurrent,
            runs_this_hour: usage.runs_this_hour,
            max_runs_per_hour: max_hour,
            tokens_today: usage.tokens_today,
            max_tokens_per_day: max_tokens,
            storage_mb: usage.storage_mb,
            max_storage_mb: max_storage,
        }
    }
}

impl Default for WorkspaceQuotaManager {
    fn default() -> Self {
        Self::new()
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{ProjectKey, TenantId, WorkspaceId};

    fn ws(id: &str) -> WorkspaceId {
        WorkspaceId::new(id)
    }

    fn tenant(id: &str) -> TenantId {
        TenantId::new(id)
    }

    fn policy(ws_id: &str, t_id: &str) -> WorkspaceQuotaPolicy {
        WorkspaceQuotaPolicy {
            workspace_id: ws(ws_id),
            tenant_id: tenant(t_id),
            max_runs_per_hour: 10,
            max_concurrent_runs: 3,
            max_storage_mb: 500,
            max_tokens_per_day: 1_000_000,
        }
    }

    // ── Gap 1: Workspace quota enforcement ───────────────────────────────

    #[test]
    fn no_policy_means_unlimited() {
        let mgr = WorkspaceQuotaManager::new();
        assert!(mgr.check_run_quota(&ws("ws1")).is_ok());
        assert!(mgr.check_token_quota(&ws("ws1"), 999_999).is_ok());
        assert!(mgr.check_storage_quota(&ws("ws1")).is_ok());
    }

    #[test]
    fn concurrent_run_quota_enforced() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(WorkspaceQuotaPolicy {
            max_concurrent_runs: 2,
            ..policy("ws1", "t1")
        });

        mgr.record_run_started("ws1");
        mgr.record_run_started("ws1");
        assert!(mgr.check_run_quota(&ws("ws1")).is_err());

        let err = mgr.check_run_quota(&ws("ws1")).unwrap_err();
        assert_eq!(err.quota_type, "max_concurrent_runs");
        assert_eq!(err.current, 2);
        assert_eq!(err.limit, 2);
    }

    #[test]
    fn concurrent_run_quota_recovers_after_completion() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(WorkspaceQuotaPolicy {
            max_concurrent_runs: 2,
            ..policy("ws1", "t1")
        });

        mgr.record_run_started("ws1");
        mgr.record_run_started("ws1");
        assert!(mgr.check_run_quota(&ws("ws1")).is_err());

        mgr.record_run_completed("ws1");
        assert!(mgr.check_run_quota(&ws("ws1")).is_ok());
    }

    #[test]
    fn runs_per_hour_quota_enforced() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(WorkspaceQuotaPolicy {
            max_runs_per_hour: 3,
            max_concurrent_runs: 100, // not the bottleneck
            ..policy("ws1", "t1")
        });

        for _ in 0..3 {
            mgr.record_run_started("ws1");
            mgr.record_run_completed("ws1"); // complete immediately
        }

        let err = mgr.check_run_quota(&ws("ws1")).unwrap_err();
        assert_eq!(err.quota_type, "max_runs_per_hour");
    }

    #[test]
    fn token_quota_enforced() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(WorkspaceQuotaPolicy {
            max_tokens_per_day: 1000,
            ..policy("ws1", "t1")
        });

        mgr.record_tokens("ws1", 800);
        assert!(mgr.check_token_quota(&ws("ws1"), 100).is_ok());
        assert!(mgr.check_token_quota(&ws("ws1"), 300).is_err());

        let err = mgr.check_token_quota(&ws("ws1"), 300).unwrap_err();
        assert_eq!(err.quota_type, "max_tokens_per_day");
        assert_eq!(err.current, 800);
        assert_eq!(err.limit, 1000);
    }

    #[test]
    fn storage_quota_enforced() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(WorkspaceQuotaPolicy {
            max_storage_mb: 100,
            ..policy("ws1", "t1")
        });

        mgr.record_storage("ws1", 50);
        assert!(mgr.check_storage_quota(&ws("ws1")).is_ok());

        mgr.record_storage("ws1", 60);
        let err = mgr.check_storage_quota(&ws("ws1")).unwrap_err();
        assert_eq!(err.quota_type, "max_storage_mb");
    }

    #[test]
    fn unlimited_policy_allows_everything() {
        let p = WorkspaceQuotaPolicy::new(ws("ws1"), tenant("t1"));
        assert!(p.is_unlimited());

        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(p);
        // All checks pass because limits are 0 (disabled).
        mgr.record_run_started("ws1");
        assert!(mgr.check_run_quota(&ws("ws1")).is_ok());
        assert!(mgr.check_token_quota(&ws("ws1"), u64::MAX).is_ok());
        assert!(mgr.check_storage_quota(&ws("ws1")).is_ok());
    }

    // ── Gap 2: Cross-tenant access ───────────────────────────────────────

    #[test]
    fn default_policy_denies_cross_tenant_access() {
        let policy = TenantAccessPolicy::deny_all(tenant("t1"));
        assert!(policy.can_access("t1")); // same-tenant always OK
        assert!(!policy.can_access("t2"));
    }

    #[test]
    fn explicit_allow_grants_cross_tenant_access() {
        let mut policy = TenantAccessPolicy::deny_all(tenant("t1"));
        policy.allow_tenant("t2");
        assert!(policy.can_access("t2"));
        assert!(!policy.can_access("t3")); // still denied
    }

    #[test]
    fn revoke_removes_access() {
        let mut policy = TenantAccessPolicy::deny_all(tenant("t1"));
        policy.allow_tenant("t2");
        assert!(policy.can_access("t2"));
        policy.revoke_tenant("t2");
        assert!(!policy.can_access("t2"));
    }

    #[test]
    fn manager_check_tenant_access() {
        let mgr = WorkspaceQuotaManager::new();
        let mut policy = TenantAccessPolicy::deny_all(tenant("t1"));
        policy.allow_tenant("t2");
        mgr.set_tenant_access(policy);

        assert!(mgr
            .check_tenant_access(&tenant("t1"), &tenant("t2"))
            .is_ok());
        assert!(mgr
            .check_tenant_access(&tenant("t1"), &tenant("t3"))
            .is_err());
    }

    #[test]
    fn manager_returns_deny_all_for_unknown_tenant() {
        let mgr = WorkspaceQuotaManager::new();
        assert!(mgr
            .check_tenant_access(&tenant("unknown"), &tenant("other"))
            .is_err());
        // Same-tenant is always OK even without explicit policy.
        assert!(mgr
            .check_tenant_access(&tenant("t1"), &tenant("t1"))
            .is_ok());
    }

    // ── Gap 3: Workspace isolation ───────────────────────────────────────

    #[test]
    fn workspace_scope_assertion_passes() {
        let project = ProjectKey::new("t1", "ws1", "p1");
        assert!(assert_workspace_scope(&project, &ws("ws1")).is_ok());
    }

    #[test]
    fn workspace_scope_assertion_fails_on_mismatch() {
        let project = ProjectKey::new("t1", "ws1", "p1");
        let err = assert_workspace_scope(&project, &ws("ws_other")).unwrap_err();
        assert_eq!(
            err,
            IsolationViolation::WorkspaceMismatch {
                expected: "ws_other".into(),
                actual: "ws1".into(),
            }
        );
    }

    #[test]
    fn tenant_access_assertion_passes() {
        let mut policy = TenantAccessPolicy::deny_all(tenant("t1"));
        policy.allow_tenant("t2");
        assert!(assert_tenant_access(&policy, &tenant("t2")).is_ok());
    }

    #[test]
    fn tenant_access_assertion_fails() {
        let policy = TenantAccessPolicy::deny_all(tenant("t1"));
        let err = assert_tenant_access(&policy, &tenant("t2")).unwrap_err();
        assert_eq!(
            err,
            IsolationViolation::TenantAccessDenied {
                source_tenant: "t1".into(),
                target_tenant: "t2".into(),
            }
        );
    }

    // ── Gap 4: Usage report ──────────────────────────────────────────────

    #[test]
    fn usage_report_combines_usage_and_limits() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(policy("ws1", "t1"));

        mgr.record_run_started("ws1");
        mgr.record_tokens("ws1", 5000);
        mgr.record_storage("ws1", 25);

        let report = mgr.usage_report(&ws("ws1"));
        assert_eq!(report.active_runs, 1);
        assert_eq!(report.max_concurrent_runs, 3);
        assert_eq!(report.runs_this_hour, 1);
        assert_eq!(report.max_runs_per_hour, 10);
        assert_eq!(report.tokens_today, 5000);
        assert_eq!(report.max_tokens_per_day, 1_000_000);
        assert_eq!(report.storage_mb, 25);
        assert_eq!(report.max_storage_mb, 500);
    }

    #[test]
    fn usage_report_without_policy_shows_zero_limits() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.record_run_started("ws_nopolicy");

        let report = mgr.usage_report(&ws("ws_nopolicy"));
        assert_eq!(report.active_runs, 1);
        assert_eq!(report.max_concurrent_runs, 0); // 0 = unlimited
        assert_eq!(report.max_runs_per_hour, 0);
    }

    // ── Integration ──────────────────────────────────────────────────────

    #[test]
    fn full_quota_lifecycle() {
        let mgr = WorkspaceQuotaManager::new();
        mgr.set_policy(WorkspaceQuotaPolicy {
            workspace_id: ws("ws1"),
            tenant_id: tenant("t1"),
            max_runs_per_hour: 100,
            max_concurrent_runs: 2,
            max_storage_mb: 1000,
            max_tokens_per_day: 50_000,
        });

        // Start 2 runs.
        assert!(mgr.check_run_quota(&ws("ws1")).is_ok());
        mgr.record_run_started("ws1");
        assert!(mgr.check_run_quota(&ws("ws1")).is_ok());
        mgr.record_run_started("ws1");

        // Third run blocked.
        assert!(mgr.check_run_quota(&ws("ws1")).is_err());

        // Complete one, third now allowed.
        mgr.record_run_completed("ws1");
        assert!(mgr.check_run_quota(&ws("ws1")).is_ok());
        mgr.record_run_started("ws1");

        // Use tokens.
        mgr.record_tokens("ws1", 40_000);
        assert!(mgr.check_token_quota(&ws("ws1"), 9_000).is_ok());
        assert!(mgr.check_token_quota(&ws("ws1"), 11_000).is_err());

        // Check report.
        let report = mgr.usage_report(&ws("ws1"));
        assert_eq!(report.active_runs, 2);
        assert_eq!(report.tokens_today, 40_000);
    }
}
