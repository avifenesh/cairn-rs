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

#[cfg(all(test, feature = "in-memory-runtime"))]
mod tests {
    use std::sync::Arc;

    use cairn_domain::{ProjectKey, RunId, SessionId, TenantId, WorkspaceId};
    use cairn_store::InMemoryStore;

    use crate::error::RuntimeError;
    use crate::projects::ProjectService;
    use crate::quotas::QuotaService;
    use crate::services::{
        ProjectServiceImpl, QuotaServiceImpl, RunServiceImpl, SessionServiceImpl,
    };
    use crate::sessions::SessionService;
    use crate::tenants::TenantService;
    use crate::workspaces::WorkspaceService;
    use crate::{TenantServiceImpl, WorkspaceServiceImpl};

    /// RFC 008: sessions-per-hour quota MUST be enforced when creating sessions.
    #[tokio::test]
    async fn quota_blocks_session_creation_when_hourly_limit_reached() {
        let store = Arc::new(InMemoryStore::new());
        let tenants = TenantServiceImpl::new(store.clone());
        let workspaces = WorkspaceServiceImpl::new(store.clone());
        let projects = ProjectServiceImpl::new(store.clone());
        let quotas = QuotaServiceImpl::new(store.clone());
        let sessions = SessionServiceImpl::new(store.clone());

        tenants
            .create(TenantId::new("t_session_quota"), "Tenant".to_owned())
            .await
            .unwrap();
        workspaces
            .create(
                TenantId::new("t_session_quota"),
                WorkspaceId::new("ws_sq"),
                "WS".to_owned(),
            )
            .await
            .unwrap();
        let project = ProjectKey::new("t_session_quota", "ws_sq", "proj_sq");
        projects
            .create(project.clone(), "Project".to_owned())
            .await
            .unwrap();

        // Set quota: max 2 sessions per hour, 10 concurrent runs.
        quotas
            .set_quota(TenantId::new("t_session_quota"), 10, 2, 100)
            .await
            .unwrap();

        // First two sessions should succeed.
        sessions
            .create(&project, SessionId::new("sess_sq_1"))
            .await
            .unwrap();
        sessions
            .create(&project, SessionId::new("sess_sq_2"))
            .await
            .unwrap();

        // Third session MUST be rejected — quota exceeded.
        let err = sessions
            .create(&project, SessionId::new("sess_sq_3"))
            .await
            .unwrap_err();
        assert!(
            matches!(
                err,
                RuntimeError::QuotaExceeded {
                    ref quota_type,
                    limit: 2,
                    ..
                } if quota_type == "max_sessions_per_hour"
            ),
            "expected QuotaExceeded for max_sessions_per_hour, got: {err:?}"
        );
    }

    #[tokio::test]
    async fn quota_blocks_third_concurrent_run() {
        let store = Arc::new(InMemoryStore::new());
        let tenants = TenantServiceImpl::new(store.clone());
        let workspaces = WorkspaceServiceImpl::new(store.clone());
        let projects = ProjectServiceImpl::new(store.clone());
        let quotas = QuotaServiceImpl::new(store.clone());
        let sessions = SessionServiceImpl::new(store.clone());
        let runs = RunServiceImpl::new(store.clone());

        tenants
            .create(TenantId::new("tenant_quota"), "Tenant".to_owned())
            .await
            .unwrap();
        workspaces
            .create(
                TenantId::new("tenant_quota"),
                WorkspaceId::new("ws_quota"),
                "Workspace".to_owned(),
            )
            .await
            .unwrap();
        let project = ProjectKey::new("tenant_quota", "ws_quota", "project_quota");
        projects
            .create(project.clone(), "Project".to_owned())
            .await
            .unwrap();
        quotas
            .set_quota(TenantId::new("tenant_quota"), 2, 10, 100)
            .await
            .unwrap();

        for suffix in ["1", "2"] {
            let session_id = SessionId::new(format!("session_quota_{suffix}"));
            sessions.create(&project, session_id.clone()).await.unwrap();
            runs.start(
                &project,
                &session_id,
                RunId::new(format!("run_quota_{suffix}")),
                None,
            )
            .await
            .unwrap();
        }

        let session_id = SessionId::new("session_quota_3");
        sessions.create(&project, session_id.clone()).await.unwrap();
        let err = runs
            .start(&project, &session_id, RunId::new("run_quota_3"), None)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            RuntimeError::QuotaExceeded {
                ref quota_type,
                current: 2,
                limit: 2,
                ..
            } if quota_type == "max_concurrent_runs"
        ));
    }
}
