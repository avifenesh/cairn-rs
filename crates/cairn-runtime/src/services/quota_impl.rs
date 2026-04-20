use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::{RuntimeEvent, TenantId, TenantQuota, TenantQuotaSet, TenantQuotaViolated};
use cairn_store::projections::QuotaReadModel;
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;
use crate::quotas::QuotaService;

pub struct QuotaServiceImpl<S> {
    store: Arc<S>,
}

impl<S> QuotaServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub(crate) async fn enforce_run_quota<S>(
    store: &S,
    tenant_id: &TenantId,
) -> Result<(), RuntimeError>
where
    S: EventLog + QuotaReadModel + Send + Sync,
{
    enforce_quota(store, tenant_id, "max_concurrent_runs").await
}

pub(crate) async fn enforce_session_quota<S>(
    store: &S,
    tenant_id: &TenantId,
) -> Result<(), RuntimeError>
where
    S: EventLog + QuotaReadModel + Send + Sync,
{
    enforce_quota(store, tenant_id, "max_sessions_per_hour").await
}

async fn enforce_quota<S>(
    store: &S,
    tenant_id: &TenantId,
    quota_type: &str,
) -> Result<(), RuntimeError>
where
    S: EventLog + QuotaReadModel + Send + Sync,
{
    let Some(quota) = QuotaReadModel::get_quota(store, tenant_id).await? else {
        return Ok(());
    };

    let (current, limit) = match quota_type {
        "max_concurrent_runs" => (quota.current_active_runs, quota.max_concurrent_runs),
        "max_sessions_per_hour" => (quota.sessions_this_hour, quota.max_sessions_per_hour),
        _ => return Ok(()),
    };

    if limit > 0 && current >= limit {
        let event = make_envelope(RuntimeEvent::TenantQuotaViolated(TenantQuotaViolated {
            tenant_id: tenant_id.clone(),
            quota_type: quota_type.to_owned(),
            current,
            limit,
            occurred_at_ms: now_millis(),
        }));
        store.append(&[event]).await?;
        return Err(RuntimeError::QuotaExceeded {
            tenant_id: tenant_id.to_string(),
            quota_type: quota_type.to_owned(),
            current,
            limit,
        });
    }

    Ok(())
}

#[async_trait]
impl<S> QuotaService for QuotaServiceImpl<S>
where
    S: EventLog + QuotaReadModel + Send + Sync + 'static,
{
    async fn set_quota(
        &self,
        tenant_id: TenantId,
        max_concurrent_runs: u32,
        max_sessions_per_hour: u32,
        max_tasks_per_run: u32,
    ) -> Result<TenantQuota, RuntimeError> {
        let event = make_envelope(RuntimeEvent::TenantQuotaSet(TenantQuotaSet {
            tenant_id: tenant_id.clone(),
            max_concurrent_runs,
            max_sessions_per_hour,
            max_tasks_per_run,
        }));
        self.store.append(&[event]).await?;
        self.get_quota(&tenant_id)
            .await?
            .ok_or_else(|| RuntimeError::Internal("tenant quota not found after set".to_owned()))
    }

    async fn get_quota(&self, tenant_id: &TenantId) -> Result<Option<TenantQuota>, RuntimeError> {
        Ok(QuotaReadModel::get_quota(self.store.as_ref(), tenant_id).await?)
    }

    async fn check_run_quota(&self, tenant_id: &TenantId) -> Result<(), RuntimeError> {
        enforce_run_quota(self.store.as_ref(), tenant_id).await
    }

    async fn check_session_quota(&self, tenant_id: &TenantId) -> Result<(), RuntimeError> {
        enforce_session_quota(self.store.as_ref(), tenant_id).await
    }
}
