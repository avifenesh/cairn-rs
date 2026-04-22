//! Quota / admission service — thin shim over [`ControlPlaneBackend`].
//!
//! FF-shaped logic (key builders, FCALL dispatch, envelope parsing)
//! lives in the backend impl (see
//! `engine/valkey_control_plane_impl.rs`). This service keeps only
//! scope-convenience constructors and the
//! [`Self::check_admission_for_run`] helper that derives the
//! ExecutionId from `(project, session_id, run_id)` so the quota's
//! concurrency counters hit the same FF flow partition as the run
//! itself.
//!
//! **Lean-bridge silence (intentional).** None of this service's
//! methods emit `BridgeEvent`s — quota state (policies, rate-window
//! counters, concurrency gauges) is FF-owned with no cairn-store
//! projection. See `docs/design/bridge-event-audit.md` §2.7.
use std::sync::Arc;

use ff_core::types::{ExecutionId, QuotaPolicyId};

use crate::boot::FabricRuntime;
use crate::engine::control_plane::ControlPlaneBackend;
use crate::engine::control_plane_types::QuotaAdmission;
use crate::error::FabricError;
use crate::id_map;

/// Re-export of the mirror type under the historical service-level
/// name. Existing callers that imported
/// `crate::services::quota_service::AdmissionResult` keep working.
pub type AdmissionResult = QuotaAdmission;

pub struct FabricQuotaService {
    backend: Arc<dyn ControlPlaneBackend>,
    runtime: Arc<FabricRuntime>,
}

impl FabricQuotaService {
    pub fn new(backend: Arc<dyn ControlPlaneBackend>, runtime: Arc<FabricRuntime>) -> Self {
        Self { backend, runtime }
    }

    pub async fn create_quota_policy(
        &self,
        scope_type: &str,
        scope_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        self.backend
            .create_quota_policy(
                scope_type,
                scope_id,
                window_seconds,
                max_requests_per_window,
                max_concurrent,
            )
            .await
    }

    pub async fn create_tenant_quota(
        &self,
        tenant_id: &cairn_domain::TenantId,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        self.create_quota_policy(
            "tenant",
            tenant_id.as_str(),
            window_seconds,
            max_requests_per_window,
            max_concurrent,
        )
        .await
    }

    pub async fn create_workspace_quota(
        &self,
        workspace_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        self.create_quota_policy(
            "workspace",
            workspace_id,
            window_seconds,
            max_requests_per_window,
            max_concurrent,
        )
        .await
    }

    pub async fn create_user_quota(
        &self,
        user_id: &str,
        window_seconds: u64,
        max_requests_per_window: u64,
        max_concurrent: u64,
    ) -> Result<QuotaPolicyId, FabricError> {
        self.create_quota_policy(
            "user",
            user_id,
            window_seconds,
            max_requests_per_window,
            max_concurrent,
        )
        .await
    }

    pub async fn check_admission(
        &self,
        quota_policy_id: &QuotaPolicyId,
        execution_id: &ExecutionId,
        window_seconds: u64,
        rate_limit: u64,
        concurrency_cap: u64,
    ) -> Result<AdmissionResult, FabricError> {
        self.backend
            .check_admission(
                quota_policy_id,
                execution_id,
                window_seconds,
                rate_limit,
                concurrency_cap,
            )
            .await
    }

    /// Quota admission check for a session-scoped run. Mints the
    /// matching ExecutionId via `id_map::session_run_to_execution_id`
    /// so the quota's concurrency counters hit the same FF flow
    /// partition as the run itself.
    pub async fn check_admission_for_run(
        &self,
        quota_policy_id: &QuotaPolicyId,
        project: &cairn_domain::tenancy::ProjectKey,
        session_id: &cairn_domain::SessionId,
        run_id: &cairn_domain::RunId,
        window_seconds: u64,
        rate_limit: u64,
        concurrency_cap: u64,
    ) -> Result<AdmissionResult, FabricError> {
        let eid = id_map::session_run_to_execution_id(
            project,
            session_id,
            run_id,
            &self.runtime.partition_config,
        );
        self.check_admission(
            quota_policy_id,
            &eid,
            window_seconds,
            rate_limit,
            concurrency_cap,
        )
        .await
    }
}
