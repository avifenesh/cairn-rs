//! RFC 014 entitlement gating and usage metering.
//!
//! ## Architecture
//!
//! ```text
//! EntitlementService
//!  ├─ PlanTier (Free / Pro / Enterprise)
//!  ├─ PlanLimits (max sessions, runs/day, tokens/month, features)
//!  ├─ UsageMeter (atomic counters per tenant)
//!  ├─ check_entitlement(tenant, feature) → Result / EntitlementError
//!  ├─ check_session_limit(tenant) → Result / EntitlementError
//!  ├─ check_run_limit(tenant) → Result / EntitlementError
//!  ├─ check_token_limit(tenant, n) → Result / EntitlementError
//!  └─ get_usage(tenant) → UsageReport
//! ```

use std::collections::HashMap;
use std::sync::RwLock;

use serde::{Deserialize, Serialize};

// ── Plan tiers ───────────────────────────────────────────────────────────────

/// Product plan tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanTier {
    Free,
    Pro,
    Enterprise,
}

/// Resource limits for a plan tier.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanLimits {
    pub tier: PlanTier,
    pub max_sessions: u32,
    pub max_runs_per_day: u32,
    pub max_tokens_per_month: u64,
    pub features_enabled: Vec<String>,
}

impl PlanLimits {
    pub fn free() -> Self {
        Self {
            tier: PlanTier::Free,
            max_sessions: 10,
            max_runs_per_day: 50,
            max_tokens_per_month: 100_000,
            features_enabled: vec!["runtime_core".into(), "eval_matrices".into()],
        }
    }

    pub fn pro() -> Self {
        Self {
            tier: PlanTier::Pro,
            max_sessions: 100,
            max_runs_per_day: 1_000,
            max_tokens_per_month: 5_000_000,
            features_enabled: vec![
                "runtime_core".into(),
                "eval_matrices".into(),
                "multi_provider".into(),
                "retrieval_core".into(),
                "credential_management".into(),
                "advanced_admin".into(),
            ],
        }
    }

    pub fn enterprise() -> Self {
        Self {
            tier: PlanTier::Enterprise,
            max_sessions: 0, // 0 = unlimited
            max_runs_per_day: 0,
            max_tokens_per_month: 0,
            features_enabled: vec![
                "runtime_core".into(),
                "eval_matrices".into(),
                "multi_provider".into(),
                "retrieval_core".into(),
                "credential_management".into(),
                "advanced_admin".into(),
                "advanced_audit_export".into(),
                "compliance_policy_packs".into(),
                "approval_hardening".into(),
            ],
        }
    }

    pub fn for_tier(tier: PlanTier) -> Self {
        match tier {
            PlanTier::Free => Self::free(),
            PlanTier::Pro => Self::pro(),
            PlanTier::Enterprise => Self::enterprise(),
        }
    }

    pub fn has_feature(&self, feature: &str) -> bool {
        self.features_enabled.iter().any(|f| f == feature)
    }

    /// True when a limit value of 0 means unlimited.
    fn is_unlimited(limit: u32) -> bool {
        limit == 0
    }

    fn is_unlimited_u64(limit: u64) -> bool {
        limit == 0
    }
}

// ── Entitlement errors ───────────────────────────────────────────────────────

/// Entitlement check failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntitlementError {
    /// Feature not available on the tenant's plan (HTTP 402).
    FeatureNotAvailable {
        feature: String,
        current_tier: PlanTier,
    },
    /// Usage limit exceeded (HTTP 429).
    LimitExceeded {
        resource: String,
        current: u64,
        limit: u64,
        tier: PlanTier,
    },
    /// No plan assigned to tenant.
    NoPlan { tenant_id: String },
}

impl EntitlementError {
    /// Suggested HTTP status code for this error.
    pub fn status_code(&self) -> u16 {
        match self {
            EntitlementError::FeatureNotAvailable { .. } => 402,
            EntitlementError::LimitExceeded { .. } => 429,
            EntitlementError::NoPlan { .. } => 402,
        }
    }

    /// Human-readable message for API responses.
    pub fn message(&self) -> String {
        match self {
            EntitlementError::FeatureNotAvailable {
                feature,
                current_tier,
            } => format!(
                "Feature '{feature}' is not available on the {current_tier:?} plan. \
                 Upgrade to access this feature."
            ),
            EntitlementError::LimitExceeded {
                resource,
                current,
                limit,
                tier,
            } => format!(
                "{resource} limit exceeded: {current}/{limit} on the {tier:?} plan. \
                 Upgrade for higher limits."
            ),
            EntitlementError::NoPlan { tenant_id } => {
                format!("No plan assigned to tenant '{tenant_id}'. Activate a plan first.")
            }
        }
    }
}

impl std::fmt::Display for EntitlementError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message())
    }
}

impl std::error::Error for EntitlementError {}

// ── Usage counters ───────────────────────────────────────────────────────────

/// Per-tenant usage counters.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct TenantUsage {
    pub sessions_created: u32,
    pub runs_today: u32,
    pub tokens_this_month: u64,
    /// Unix ms when the daily run counter was last reset.
    pub day_reset_ms: u64,
    /// Unix ms when the monthly token counter was last reset.
    pub month_reset_ms: u64,
}

/// Usage report combining counters with plan limits.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageReport {
    pub tenant_id: String,
    pub tier: PlanTier,
    pub sessions_used: u32,
    pub max_sessions: u32,
    pub runs_today: u32,
    pub max_runs_per_day: u32,
    pub tokens_this_month: u64,
    pub max_tokens_per_month: u64,
    pub features_enabled: Vec<String>,
}

/// Detailed usage breakdown for the /v1/entitlements/usage endpoint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DetailedUsageReport {
    pub tenant_id: String,
    pub tier: PlanTier,
    pub sessions: ResourceUsage,
    pub runs: ResourceUsage,
    pub tokens: ResourceUsageU64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub used: u32,
    pub limit: u32,
    pub remaining: u32,
    pub percent_used: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceUsageU64 {
    pub used: u64,
    pub limit: u64,
    pub remaining: u64,
    pub percent_used: f64,
}

// ── One day / one month in ms ────────────────────────────────────────────────

const DAY_MS: u64 = 86_400_000;
const MONTH_MS: u64 = 30 * DAY_MS;

// ── Entitlement service ──────────────────────────────────────────────────────

/// In-memory entitlement and usage metering service.
///
/// Thread-safe — designed to be shared via `Arc<EntitlementService>`.
pub struct EntitlementService {
    /// Tenant → plan assignment.
    plans: RwLock<HashMap<String, PlanTier>>,
    /// Tenant → usage counters.
    usage: RwLock<HashMap<String, TenantUsage>>,
}

impl EntitlementService {
    pub fn new() -> Self {
        Self {
            plans: RwLock::new(HashMap::new()),
            usage: RwLock::new(HashMap::new()),
        }
    }

    // ── Plan management ──────────────────────────────────────────────────

    /// Assign a plan tier to a tenant.
    pub fn set_plan(&self, tenant_id: &str, tier: PlanTier) {
        self.plans
            .write()
            .unwrap()
            .insert(tenant_id.to_owned(), tier);
    }

    /// Get the plan tier for a tenant.
    pub fn get_plan(&self, tenant_id: &str) -> Option<PlanTier> {
        self.plans.read().unwrap().get(tenant_id).copied()
    }

    /// Get the plan limits for a tenant. Returns `None` if no plan assigned.
    pub fn get_limits(&self, tenant_id: &str) -> Option<PlanLimits> {
        self.get_plan(tenant_id).map(PlanLimits::for_tier)
    }

    // ── Entitlement checks ───────────────────────────────────────────────

    /// Check whether a tenant has access to a feature.
    pub fn check_entitlement(
        &self,
        tenant_id: &str,
        feature: &str,
    ) -> Result<(), EntitlementError> {
        let tier = self
            .get_plan(tenant_id)
            .ok_or_else(|| EntitlementError::NoPlan {
                tenant_id: tenant_id.to_owned(),
            })?;

        let limits = PlanLimits::for_tier(tier);
        if limits.has_feature(feature) {
            Ok(())
        } else {
            Err(EntitlementError::FeatureNotAvailable {
                feature: feature.to_owned(),
                current_tier: tier,
            })
        }
    }

    /// Check whether creating a new session is allowed.
    pub fn check_session_limit(&self, tenant_id: &str) -> Result<(), EntitlementError> {
        let tier = self
            .get_plan(tenant_id)
            .ok_or_else(|| EntitlementError::NoPlan {
                tenant_id: tenant_id.to_owned(),
            })?;
        let limits = PlanLimits::for_tier(tier);
        if PlanLimits::is_unlimited(limits.max_sessions) {
            return Ok(());
        }

        let usage = self.get_usage_counters(tenant_id);
        if usage.sessions_created >= limits.max_sessions {
            return Err(EntitlementError::LimitExceeded {
                resource: "sessions".into(),
                current: usage.sessions_created as u64,
                limit: limits.max_sessions as u64,
                tier,
            });
        }
        Ok(())
    }

    /// Check whether starting a new run is allowed.
    pub fn check_run_limit(&self, tenant_id: &str) -> Result<(), EntitlementError> {
        let tier = self
            .get_plan(tenant_id)
            .ok_or_else(|| EntitlementError::NoPlan {
                tenant_id: tenant_id.to_owned(),
            })?;
        let limits = PlanLimits::for_tier(tier);
        if PlanLimits::is_unlimited(limits.max_runs_per_day) {
            return Ok(());
        }

        let mut map = self.usage.write().unwrap();
        let usage = map.entry(tenant_id.to_owned()).or_default();
        maybe_reset_daily(usage);

        if usage.runs_today >= limits.max_runs_per_day {
            return Err(EntitlementError::LimitExceeded {
                resource: "runs_per_day".into(),
                current: usage.runs_today as u64,
                limit: limits.max_runs_per_day as u64,
                tier,
            });
        }
        Ok(())
    }

    /// Check whether consuming `additional` tokens is allowed.
    pub fn check_token_limit(
        &self,
        tenant_id: &str,
        additional: u64,
    ) -> Result<(), EntitlementError> {
        let tier = self
            .get_plan(tenant_id)
            .ok_or_else(|| EntitlementError::NoPlan {
                tenant_id: tenant_id.to_owned(),
            })?;
        let limits = PlanLimits::for_tier(tier);
        if PlanLimits::is_unlimited_u64(limits.max_tokens_per_month) {
            return Ok(());
        }

        let mut map = self.usage.write().unwrap();
        let usage = map.entry(tenant_id.to_owned()).or_default();
        maybe_reset_monthly(usage);

        let projected = usage.tokens_this_month.saturating_add(additional);
        if projected > limits.max_tokens_per_month {
            return Err(EntitlementError::LimitExceeded {
                resource: "tokens_per_month".into(),
                current: usage.tokens_this_month,
                limit: limits.max_tokens_per_month,
                tier,
            });
        }
        Ok(())
    }

    // ── Usage metering ───────────────────────────────────────────────────

    /// Record a session creation.
    pub fn record_session(&self, tenant_id: &str) {
        let mut map = self.usage.write().unwrap();
        let usage = map.entry(tenant_id.to_owned()).or_default();
        usage.sessions_created += 1;
    }

    /// Record a run start.
    pub fn record_run(&self, tenant_id: &str) {
        let mut map = self.usage.write().unwrap();
        let usage = map.entry(tenant_id.to_owned()).or_default();
        maybe_reset_daily(usage);
        usage.runs_today += 1;
    }

    /// Record token consumption.
    pub fn record_tokens(&self, tenant_id: &str, tokens: u64) {
        let mut map = self.usage.write().unwrap();
        let usage = map.entry(tenant_id.to_owned()).or_default();
        maybe_reset_monthly(usage);
        usage.tokens_this_month = usage.tokens_this_month.saturating_add(tokens);
    }

    // ── Reporting ────────────────────────────────────────────────────────

    fn get_usage_counters(&self, tenant_id: &str) -> TenantUsage {
        self.usage
            .read()
            .unwrap()
            .get(tenant_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Get a usage report combining counters with plan limits.
    pub fn get_usage(&self, tenant_id: &str) -> Option<UsageReport> {
        let tier = self.get_plan(tenant_id)?;
        let limits = PlanLimits::for_tier(tier);
        let usage = self.get_usage_counters(tenant_id);

        Some(UsageReport {
            tenant_id: tenant_id.to_owned(),
            tier,
            sessions_used: usage.sessions_created,
            max_sessions: limits.max_sessions,
            runs_today: usage.runs_today,
            max_runs_per_day: limits.max_runs_per_day,
            tokens_this_month: usage.tokens_this_month,
            max_tokens_per_month: limits.max_tokens_per_month,
            features_enabled: limits.features_enabled,
        })
    }

    /// Get detailed usage breakdown with remaining capacity and percentages.
    pub fn get_detailed_usage(&self, tenant_id: &str) -> Option<DetailedUsageReport> {
        let tier = self.get_plan(tenant_id)?;
        let limits = PlanLimits::for_tier(tier);
        let usage = self.get_usage_counters(tenant_id);

        Some(DetailedUsageReport {
            tenant_id: tenant_id.to_owned(),
            tier,
            sessions: resource_usage(usage.sessions_created, limits.max_sessions),
            runs: resource_usage(usage.runs_today, limits.max_runs_per_day),
            tokens: resource_usage_u64(usage.tokens_this_month, limits.max_tokens_per_month),
        })
    }
}

impl Default for EntitlementService {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn maybe_reset_daily(usage: &mut TenantUsage) {
    let now = now_ms();
    if now.saturating_sub(usage.day_reset_ms) >= DAY_MS {
        usage.runs_today = 0;
        usage.day_reset_ms = now;
    }
}

fn maybe_reset_monthly(usage: &mut TenantUsage) {
    let now = now_ms();
    if now.saturating_sub(usage.month_reset_ms) >= MONTH_MS {
        usage.tokens_this_month = 0;
        usage.month_reset_ms = now;
    }
}

fn resource_usage(used: u32, limit: u32) -> ResourceUsage {
    let remaining = if limit == 0 {
        u32::MAX
    } else {
        limit.saturating_sub(used)
    };
    let percent = if limit == 0 {
        0.0
    } else {
        (used as f64 / limit as f64 * 100.0).min(100.0)
    };
    ResourceUsage {
        used,
        limit,
        remaining,
        percent_used: percent,
    }
}

fn resource_usage_u64(used: u64, limit: u64) -> ResourceUsageU64 {
    let remaining = if limit == 0 {
        u64::MAX
    } else {
        limit.saturating_sub(used)
    };
    let percent = if limit == 0 {
        0.0
    } else {
        (used as f64 / limit as f64 * 100.0).min(100.0)
    };
    ResourceUsageU64 {
        used,
        limit,
        remaining,
        percent_used: percent,
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

    // ── Plan defaults ────────────────────────────────────────────────────

    #[test]
    fn free_plan_has_limited_features() {
        let limits = PlanLimits::free();
        assert_eq!(limits.tier, PlanTier::Free);
        assert_eq!(limits.max_sessions, 10);
        assert_eq!(limits.max_runs_per_day, 50);
        assert!(limits.has_feature("runtime_core"));
        assert!(!limits.has_feature("advanced_admin"));
    }

    #[test]
    fn pro_plan_has_more_features() {
        let limits = PlanLimits::pro();
        assert_eq!(limits.tier, PlanTier::Pro);
        assert!(limits.max_sessions > PlanLimits::free().max_sessions);
        assert!(limits.has_feature("advanced_admin"));
        assert!(!limits.has_feature("compliance_policy_packs"));
    }

    #[test]
    fn enterprise_plan_is_unlimited() {
        let limits = PlanLimits::enterprise();
        assert_eq!(limits.max_sessions, 0); // 0 = unlimited
        assert_eq!(limits.max_runs_per_day, 0);
        assert_eq!(limits.max_tokens_per_month, 0);
        assert!(limits.has_feature("compliance_policy_packs"));
        assert!(limits.has_feature("approval_hardening"));
    }

    #[test]
    fn for_tier_returns_correct_plan() {
        assert_eq!(PlanLimits::for_tier(PlanTier::Free).tier, PlanTier::Free);
        assert_eq!(PlanLimits::for_tier(PlanTier::Pro).tier, PlanTier::Pro);
        assert_eq!(
            PlanLimits::for_tier(PlanTier::Enterprise).tier,
            PlanTier::Enterprise
        );
    }

    // ── Feature entitlement checks ───────────────────────────────────────

    #[test]
    fn check_entitlement_passes_for_included_feature() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Free);
        assert!(svc.check_entitlement("t1", "runtime_core").is_ok());
    }

    #[test]
    fn check_entitlement_fails_for_excluded_feature() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Free);
        let err = svc.check_entitlement("t1", "advanced_admin").unwrap_err();
        assert_eq!(err.status_code(), 402);
        assert!(matches!(
            err,
            EntitlementError::FeatureNotAvailable {
                ref feature,
                current_tier: PlanTier::Free,
            } if feature == "advanced_admin"
        ));
    }

    #[test]
    fn check_entitlement_no_plan_returns_402() {
        let svc = EntitlementService::new();
        let err = svc
            .check_entitlement("unknown", "runtime_core")
            .unwrap_err();
        assert_eq!(err.status_code(), 402);
        assert!(matches!(err, EntitlementError::NoPlan { .. }));
    }

    #[test]
    fn pro_can_access_advanced_admin() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Pro);
        assert!(svc.check_entitlement("t1", "advanced_admin").is_ok());
    }

    #[test]
    fn enterprise_can_access_everything() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Enterprise);
        assert!(svc
            .check_entitlement("t1", "compliance_policy_packs")
            .is_ok());
        assert!(svc.check_entitlement("t1", "approval_hardening").is_ok());
        assert!(svc.check_entitlement("t1", "runtime_core").is_ok());
    }

    // ── Session limit checks ─────────────────────────────────────────────

    #[test]
    fn session_limit_enforced_on_free() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Free);

        for _ in 0..10 {
            svc.record_session("t1");
        }

        let err = svc.check_session_limit("t1").unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(matches!(
            err,
            EntitlementError::LimitExceeded {
                ref resource,
                current: 10,
                limit: 10,
                tier: PlanTier::Free,
            } if resource == "sessions"
        ));
    }

    #[test]
    fn enterprise_session_limit_unlimited() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Enterprise);
        for _ in 0..1000 {
            svc.record_session("t1");
        }
        assert!(svc.check_session_limit("t1").is_ok());
    }

    // ── Run limit checks ─────────────────────────────────────────────────

    #[test]
    fn run_limit_enforced_on_free() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Free);

        for _ in 0..50 {
            svc.record_run("t1");
        }

        let err = svc.check_run_limit("t1").unwrap_err();
        assert_eq!(err.status_code(), 429);
        assert!(matches!(
            err,
            EntitlementError::LimitExceeded {
                ref resource,
                ..
            } if resource == "runs_per_day"
        ));
    }

    // ── Token limit checks ───────────────────────────────────────────────

    #[test]
    fn token_limit_enforced_on_free() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Free);

        svc.record_tokens("t1", 90_000);
        assert!(svc.check_token_limit("t1", 5_000).is_ok());
        assert!(svc.check_token_limit("t1", 20_000).is_err());
    }

    #[test]
    fn enterprise_token_limit_unlimited() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Enterprise);
        svc.record_tokens("t1", u64::MAX / 2);
        assert!(svc.check_token_limit("t1", 1000).is_ok());
    }

    // ── Usage reporting ──────────────────────────────────────────────────

    #[test]
    fn get_usage_returns_correct_report() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Pro);

        svc.record_session("t1");
        svc.record_session("t1");
        svc.record_run("t1");
        svc.record_tokens("t1", 500);

        let report = svc.get_usage("t1").unwrap();
        assert_eq!(report.tier, PlanTier::Pro);
        assert_eq!(report.sessions_used, 2);
        assert_eq!(report.max_sessions, 100);
        assert_eq!(report.runs_today, 1);
        assert_eq!(report.tokens_this_month, 500);
        assert!(report
            .features_enabled
            .contains(&"advanced_admin".to_owned()));
    }

    #[test]
    fn get_usage_no_plan_returns_none() {
        let svc = EntitlementService::new();
        assert!(svc.get_usage("unknown").is_none());
    }

    #[test]
    fn detailed_usage_report_has_percentages() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Free);
        svc.record_session("t1");
        svc.record_session("t1");

        let report = svc.get_detailed_usage("t1").unwrap();
        assert_eq!(report.sessions.used, 2);
        assert_eq!(report.sessions.limit, 10);
        assert_eq!(report.sessions.remaining, 8);
        assert!((report.sessions.percent_used - 20.0).abs() < 0.1);
    }

    #[test]
    fn detailed_usage_unlimited_shows_max_remaining() {
        let svc = EntitlementService::new();
        svc.set_plan("t1", PlanTier::Enterprise);

        let report = svc.get_detailed_usage("t1").unwrap();
        assert_eq!(report.sessions.limit, 0);
        assert_eq!(report.sessions.remaining, u32::MAX);
        assert!((report.sessions.percent_used - 0.0).abs() < 0.01);
    }

    // ── Error messages ───────────────────────────────────────────────────

    #[test]
    fn error_messages_are_descriptive() {
        let err = EntitlementError::FeatureNotAvailable {
            feature: "compliance".into(),
            current_tier: PlanTier::Free,
        };
        assert!(err.message().contains("compliance"));
        assert!(err.message().contains("Free"));
        assert!(err.message().contains("Upgrade"));

        let err = EntitlementError::LimitExceeded {
            resource: "sessions".into(),
            current: 10,
            limit: 10,
            tier: PlanTier::Free,
        };
        assert!(err.message().contains("10/10"));
        assert!(err.message().contains("Upgrade"));
    }

    // ── Integration ──────────────────────────────────────────────────────

    #[test]
    fn full_lifecycle() {
        let svc = EntitlementService::new();

        // New tenant — no plan.
        assert!(svc.check_entitlement("t1", "runtime_core").is_err());

        // Assign Free plan.
        svc.set_plan("t1", PlanTier::Free);
        assert!(svc.check_entitlement("t1", "runtime_core").is_ok());
        assert!(svc.check_entitlement("t1", "advanced_admin").is_err());

        // Use some resources.
        for _ in 0..10 {
            assert!(svc.check_session_limit("t1").is_ok());
            svc.record_session("t1");
        }
        // 11th session blocked.
        assert!(svc.check_session_limit("t1").is_err());

        // Upgrade to Pro.
        svc.set_plan("t1", PlanTier::Pro);
        assert!(svc.check_entitlement("t1", "advanced_admin").is_ok());
        // Session limit is now 100, so we're at 10/100.
        assert!(svc.check_session_limit("t1").is_ok());

        // Report reflects upgrade.
        let report = svc.get_usage("t1").unwrap();
        assert_eq!(report.tier, PlanTier::Pro);
        assert_eq!(report.max_sessions, 100);
    }
}
