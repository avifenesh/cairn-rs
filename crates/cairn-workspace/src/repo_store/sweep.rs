use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use cairn_domain::TenantId;

use crate::error::SweepError;
use crate::repo_store::{ProjectRepoAccessService, RepoCloneCache, RepoStoreEvent, SweepId};
use crate::sandbox::RepoId;

#[async_trait]
pub trait ActiveSandboxRepoSource: Send + Sync {
    async fn active_repo_references(&self) -> Result<HashSet<(TenantId, RepoId)>, SweepError>;
}

pub trait RepoStoreEventSink: Send + Sync + 'static {
    fn publish(&self, event: RepoStoreEvent);
}

#[derive(Debug, Default)]
pub struct BufferedRepoStoreEventSink {
    events: Mutex<Vec<RepoStoreEvent>>,
}

impl BufferedRepoStoreEventSink {
    pub fn drain(&self) -> Vec<RepoStoreEvent> {
        let mut guard = self
            .events
            .lock()
            .expect("repo store event buffer poisoned");
        std::mem::take(&mut *guard)
    }
}

impl RepoStoreEventSink for BufferedRepoStoreEventSink {
    fn publish(&self, event: RepoStoreEvent) {
        self.events
            .lock()
            .expect("repo store event buffer poisoned")
            .push(event);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SweepRunSummary {
    pub sweep_id: SweepId,
    pub deleted: u32,
    pub skipped_active_sandboxes: u32,
    pub skipped_active_allowlists: u32,
}

pub struct RepoCloneSweepTask {
    pub cache: Arc<RepoCloneCache>,
    pub access: Arc<ProjectRepoAccessService>,
    pub sandbox_source: Arc<dyn ActiveSandboxRepoSource>,
    pub event_sink: Arc<dyn RepoStoreEventSink>,
    pub interval: Duration,
}

impl RepoCloneSweepTask {
    pub fn new(
        cache: Arc<RepoCloneCache>,
        access: Arc<ProjectRepoAccessService>,
        sandbox_source: Arc<dyn ActiveSandboxRepoSource>,
        event_sink: Arc<dyn RepoStoreEventSink>,
        interval: Duration,
    ) -> Self {
        Self {
            cache,
            access,
            sandbox_source,
            event_sink,
            interval,
        }
    }

    pub async fn run_once(&self) -> Result<SweepRunSummary, SweepError> {
        let sweep_id = SweepId::new();
        let started_at = now_millis();
        self.event_sink
            .publish(RepoStoreEvent::RepoCloneSweepStarted {
                sweep_id: sweep_id.clone(),
                started_at,
            });

        let cloned = self.cache.cloned_set().await;
        let allowlists = self.access.list_all().await;
        let active = self.sandbox_source.active_repo_references().await?;

        let referenced_by_allowlist: HashSet<(TenantId, RepoId)> = allowlists
            .iter()
            .flat_map(|(project, repos)| {
                repos
                    .iter()
                    .map(move |repo_id| (project.tenant_id.clone(), repo_id.clone()))
            })
            .collect();

        let mut deleted = 0;
        let mut skipped_active_sandboxes = 0;
        let mut skipped_active_allowlists = 0;

        for pair in cloned {
            if active.contains(&pair) {
                skipped_active_sandboxes += 1;
                continue;
            }
            if referenced_by_allowlist.contains(&pair) {
                skipped_active_allowlists += 1;
                continue;
            }

            self.cache.delete(&pair.0, &pair.1).await?;
            deleted += 1;
            self.event_sink.publish(RepoStoreEvent::RepoCloneDeleted {
                tenant: pair.0.clone(),
                repo_id: pair.1.clone(),
                sweep_id: Some(sweep_id.clone()),
                at: now_millis(),
            });
        }

        let completed_at = now_millis();
        self.event_sink
            .publish(RepoStoreEvent::RepoCloneSweepCompleted {
                sweep_id: sweep_id.clone(),
                deleted,
                skipped_active_sandboxes,
                skipped_active_allowlists,
                completed_at,
            });

        Ok(SweepRunSummary {
            sweep_id,
            deleted,
            skipped_active_sandboxes,
            skipped_active_allowlists,
        })
    }

    pub async fn run_loop(self) -> Result<(), SweepError> {
        loop {
            self.run_once().await?;
            tokio::time::sleep(self.interval).await;
        }
    }
}

fn now_millis() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::fs;
    use std::path::PathBuf;
    use std::process;
    use std::sync::Arc;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use async_trait::async_trait;
    use cairn_domain::{ActorRef, OperatorId, ProjectKey, RepoAccessContext, TenantId};

    use super::{
        ActiveSandboxRepoSource, BufferedRepoStoreEventSink, RepoCloneSweepTask, SweepRunSummary,
    };
    use crate::error::SweepError;
    use crate::repo_store::{ProjectRepoAccessService, RepoCloneCache, RepoStoreEvent};
    use crate::sandbox::RepoId;

    struct TestDir {
        path: PathBuf,
    }

    impl TestDir {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "cairn-workspace-{label}-{}-{unique}",
                process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[derive(Clone, Debug, Default)]
    struct StaticActiveSandboxRepoSource {
        active: HashSet<(TenantId, RepoId)>,
        fail: Option<String>,
    }

    #[async_trait]
    impl ActiveSandboxRepoSource for StaticActiveSandboxRepoSource {
        async fn active_repo_references(&self) -> Result<HashSet<(TenantId, RepoId)>, SweepError> {
            if let Some(message) = &self.fail {
                return Err(SweepError::ActiveSandboxQuery(message.clone()));
            }
            Ok(self.active.clone())
        }
    }

    fn actor() -> ActorRef {
        ActorRef::Operator {
            operator_id: OperatorId::new("op"),
        }
    }

    fn ctx(project: &str) -> RepoAccessContext {
        RepoAccessContext {
            project: ProjectKey::new("tenant-a", "workspace-a", project),
        }
    }

    async fn sweep_task(
        temp_dir: &TestDir,
        active: HashSet<(TenantId, RepoId)>,
    ) -> (
        RepoCloneSweepTask,
        Arc<RepoCloneCache>,
        Arc<ProjectRepoAccessService>,
        Arc<BufferedRepoStoreEventSink>,
    ) {
        let cache = Arc::new(RepoCloneCache::new(&temp_dir.path));
        let access = Arc::new(ProjectRepoAccessService::new());
        let sink = Arc::new(BufferedRepoStoreEventSink::default());
        let source = Arc::new(StaticActiveSandboxRepoSource { active, fail: None });
        (
            RepoCloneSweepTask::new(
                cache.clone(),
                access.clone(),
                source,
                sink.clone(),
                Duration::from_secs(3600),
            ),
            cache,
            access,
            sink,
        )
    }

    #[tokio::test]
    async fn run_once_deletes_only_unreferenced_clones() {
        let temp_dir = TestDir::new("sweep-run-once");
        let tenant = TenantId::new("tenant-a");
        let repo_keep_allowlist = RepoId::new("octocat/keep-allow");
        let repo_keep_active = RepoId::new("octocat/keep-active");
        let repo_delete = RepoId::new("octocat/delete-me");

        let (task, cache, access, sink) = sweep_task(
            &temp_dir,
            HashSet::from([(tenant.clone(), repo_keep_active.clone())]),
        )
        .await;

        cache
            .ensure_cloned(&tenant, &repo_keep_allowlist)
            .await
            .unwrap();
        cache
            .ensure_cloned(&tenant, &repo_keep_active)
            .await
            .unwrap();
        cache.ensure_cloned(&tenant, &repo_delete).await.unwrap();

        access
            .allow(&ctx("project-a"), &repo_keep_allowlist, actor())
            .await
            .unwrap();

        let summary = task.run_once().await.unwrap();

        assert_eq!(
            summary,
            SweepRunSummary {
                sweep_id: summary.sweep_id.clone(),
                deleted: 1,
                skipped_active_sandboxes: 1,
                skipped_active_allowlists: 1,
            }
        );
        assert!(cache.is_cloned(&tenant, &repo_keep_allowlist).await);
        assert!(cache.is_cloned(&tenant, &repo_keep_active).await);
        assert!(!cache.is_cloned(&tenant, &repo_delete).await);

        let events = sink.drain();
        assert!(matches!(
            events.first(),
            Some(RepoStoreEvent::RepoCloneSweepStarted { .. })
        ));
        assert!(events.iter().any(|event| matches!(
            event,
            RepoStoreEvent::RepoCloneDeleted {
                tenant: deleted_tenant,
                repo_id,
                sweep_id: Some(_),
                ..
            } if deleted_tenant == &tenant && repo_id == &repo_delete
        )));
        assert!(matches!(
            events.last(),
            Some(RepoStoreEvent::RepoCloneSweepCompleted {
                deleted: 1,
                skipped_active_sandboxes: 1,
                skipped_active_allowlists: 1,
                ..
            })
        ));
    }

    #[tokio::test]
    async fn run_once_surfaces_active_source_errors() {
        let temp_dir = TestDir::new("sweep-error");
        let cache = Arc::new(RepoCloneCache::new(&temp_dir.path));
        let access = Arc::new(ProjectRepoAccessService::new());
        let sink = Arc::new(BufferedRepoStoreEventSink::default());
        let source = Arc::new(StaticActiveSandboxRepoSource {
            active: HashSet::new(),
            fail: Some("boom".to_string()),
        });
        let task = RepoCloneSweepTask::new(cache, access, source, sink, Duration::from_secs(3600));

        let error = task.run_once().await.unwrap_err();
        assert_eq!(error, SweepError::ActiveSandboxQuery("boom".to_string()));
    }
}
