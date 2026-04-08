//! Spend alert service boundary — GAP-006.
//!
//! Monitors per-session LLM spend against a per-tenant threshold.
//! When a session's total cost crosses the threshold, emits a
//! `SpendAlertTriggered` event (at most once per session).
//!
//! Mirrors `cairn/internal/agent/spend_alert.go` at the service boundary.

use async_trait::async_trait;
use cairn_domain::{providers::SpendAlert, SessionId, TenantId};

use crate::error::RuntimeError;

/// Service for configuring spend thresholds and checking session spend.
#[async_trait]
pub trait SpendAlertService: Send + Sync {
    /// Set (or replace) the alert threshold for a tenant.
    ///
    /// When any session for this tenant's total cost exceeds `threshold_micros`,
    /// `SpendAlertTriggered` is emitted.
    async fn set_threshold(
        &self,
        tenant_id: TenantId,
        threshold_micros: u64,
    ) -> Result<(), RuntimeError>;

    /// Check the session's current cost against the tenant threshold.
    ///
    /// Returns `Some(SpendAlert)` and emits `SpendAlertTriggered` if the
    /// threshold is exceeded and has not already been triggered for this session.
    /// Returns `None` if below threshold or no threshold is configured.
    async fn check_session_spend(
        &self,
        session_id: &SessionId,
        tenant_id: &TenantId,
    ) -> Result<Option<SpendAlert>, RuntimeError>;
}
