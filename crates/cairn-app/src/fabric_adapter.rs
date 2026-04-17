//! Bridges trait-based handlers to FabricServices.
//!
//! Reads go to store projection; writes go to Fabric. Enabled per-service via
//! `CAIRN_FABRIC_ENABLED`.
//!
//! This module exists so that HTTP handlers can continue to call
//! `state.runtime.runs.get(...)` (a trait method with bare IDs) while the
//! underlying work is routed to [`cairn_fabric::FabricServices`], which
//! requires a `ProjectKey` for every operation. The adapter resolves the
//! missing project context by reading the cairn-store projection first, then
//! delegates to the Fabric service.
//!
//! **Skeleton only** — every trait method currently returns
//! `unimplemented!()`. Implementation lands after FF B4 (idempotency key ARGV)
//! stabilises and the `CAIRN_FABRIC_ENABLED` flag is wired through
//! [`crate::state::AppState`].
//!
//! Scope per service (see `docs/design/notes/cairn-fabric-handler-wiring.md`):
//!
//! | Method kind     | Routing      | Notes                                         |
//! |-----------------|--------------|-----------------------------------------------|
//! | Mutations       | Fabric       | `start`, `complete`, `fail`, `cancel`, …       |
//! | Bare-ID reads   | Projection   | `get(run_id)` — resolve project then delegate |
//! | Batch/list      | Projection   | FF doesn't index by cairn scope               |
//! | Dependencies    | Fabric (T1)  | FF flow-edge fcalls (not store)               |

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::{
    ApprovalDecision, FailureClass, PauseReason, ProjectKey, ResumeTrigger, RunId, RunResumeTarget,
    SessionId, TaskId, TaskResumeTarget, TaskState,
};
use cairn_fabric::FabricServices;
use cairn_runtime::error::RuntimeError;
use cairn_runtime::runs::RunService;
use cairn_runtime::sessions::SessionService;
use cairn_runtime::tasks::TaskService;
use cairn_store::projections::{
    RunReadModel, RunRecord, SessionReadModel, SessionRecord, TaskDependencyRecord, TaskReadModel,
    TaskRecord,
};
use cairn_store::InMemoryStore;

// ── Project resolvers ────────────────────────────────────────────────────────
//
// The store projections already key records by ID (`HashMap<String, RunRecord>`
// et al.) and each record carries `project: ProjectKey`. No new index is
// required — the resolvers just do the standard `RunReadModel::get(id)` /
// `TaskReadModel::get(id)` / `SessionReadModel::get(id)` lookup and project
// the `project` field out of the returned record.
//
// The projections' `get` methods are `async` (the `RunReadModel` trait
// requires it for Postgres/SQLite backends), so the resolvers are async too.
// Each call is O(1) for InMemoryStore (single mutex-guarded HashMap lookup)
// and a single indexed SELECT for Postgres/SQLite — no I/O amplification.

/// Resolve the owning project for a run from the store projection.
///
/// Returns `Ok(None)` when the run is not in the projection yet (race during
/// create) or when the store has no record of it. Returns `Err` only for
/// store-level failures (e.g. Postgres connection loss).
pub async fn resolve_project_from_run_id(
    store: &Arc<InMemoryStore>,
    run_id: &RunId,
) -> Result<Option<ProjectKey>, RuntimeError> {
    match RunReadModel::get(store.as_ref(), run_id).await? {
        Some(record) => Ok(Some(record.project)),
        None => Ok(None),
    }
}

/// Resolve the owning project for a task from the store projection.
pub async fn resolve_project_from_task_id(
    store: &Arc<InMemoryStore>,
    task_id: &TaskId,
) -> Result<Option<ProjectKey>, RuntimeError> {
    match TaskReadModel::get(store.as_ref(), task_id).await? {
        Some(record) => Ok(Some(record.project)),
        None => Ok(None),
    }
}

/// Resolve the owning project for a session from the store projection.
pub async fn resolve_project_from_session_id(
    store: &Arc<InMemoryStore>,
    session_id: &SessionId,
) -> Result<Option<ProjectKey>, RuntimeError> {
    match SessionReadModel::get(store.as_ref(), session_id).await? {
        Some(record) => Ok(Some(record.project)),
        None => Ok(None),
    }
}

// ── RunService adapter ───────────────────────────────────────────────────────

/// Adapter routing [`RunService`] calls to [`FabricServices::runs`].
pub struct FabricRunServiceAdapter {
    pub fabric: Arc<FabricServices>,
    pub store: Arc<InMemoryStore>,
}

impl FabricRunServiceAdapter {
    pub fn new(fabric: Arc<FabricServices>, store: Arc<InMemoryStore>) -> Self {
        Self { fabric, store }
    }
}

#[async_trait]
impl RunService for FabricRunServiceAdapter {
    async fn start(
        &self,
        _project: &ProjectKey,
        _session_id: &SessionId,
        _run_id: RunId,
        _parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::start")
    }

    async fn get(&self, _run_id: &RunId) -> Result<Option<RunRecord>, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::get")
    }

    async fn list_by_session(
        &self,
        _session_id: &SessionId,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::list_by_session")
    }

    async fn complete(&self, _run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::complete")
    }

    async fn fail(
        &self,
        _run_id: &RunId,
        _failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::fail")
    }

    async fn cancel(&self, _run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::cancel")
    }

    async fn pause(
        &self,
        _run_id: &RunId,
        _reason: PauseReason,
    ) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::pause")
    }

    async fn resume(
        &self,
        _run_id: &RunId,
        _trigger: ResumeTrigger,
        _target: RunResumeTarget,
    ) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::resume")
    }

    async fn enter_waiting_approval(&self, _run_id: &RunId) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::enter_waiting_approval")
    }

    async fn resolve_approval(
        &self,
        _run_id: &RunId,
        _decision: ApprovalDecision,
    ) -> Result<RunRecord, RuntimeError> {
        unimplemented!("FabricRunServiceAdapter::resolve_approval")
    }
}

// ── TaskService adapter ──────────────────────────────────────────────────────

/// Adapter routing [`TaskService`] calls to [`FabricServices::tasks`].
pub struct FabricTaskServiceAdapter {
    pub fabric: Arc<FabricServices>,
    pub store: Arc<InMemoryStore>,
}

impl FabricTaskServiceAdapter {
    pub fn new(fabric: Arc<FabricServices>, store: Arc<InMemoryStore>) -> Self {
        Self { fabric, store }
    }
}

#[async_trait]
impl TaskService for FabricTaskServiceAdapter {
    async fn submit(
        &self,
        _project: &ProjectKey,
        _task_id: TaskId,
        _parent_run_id: Option<RunId>,
        _parent_task_id: Option<TaskId>,
        _priority: u32,
    ) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::submit")
    }

    async fn declare_dependency(
        &self,
        _dependent_task_id: &TaskId,
        _prerequisite_task_id: &TaskId,
    ) -> Result<TaskDependencyRecord, RuntimeError> {
        // Per manager 2026-04-17: route through FF flow-edge fcalls
        // (ff_stage_dependency_edge + ff_apply_dependency_to_child), NOT the
        // cairn-store event log. Keeps task dependencies aligned with Phase 3
        // Session→Flow DAG.
        unimplemented!("FabricTaskServiceAdapter::declare_dependency (FF flow-edge path)")
    }

    async fn check_dependencies(
        &self,
        _task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::check_dependencies (FF flow-edge path)")
    }

    async fn get(&self, _task_id: &TaskId) -> Result<Option<TaskRecord>, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::get")
    }

    async fn claim(
        &self,
        _task_id: &TaskId,
        _lease_owner: String,
        _lease_duration_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::claim")
    }

    async fn heartbeat(
        &self,
        _task_id: &TaskId,
        _lease_extension_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::heartbeat")
    }

    async fn start(&self, _task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::start")
    }

    async fn complete(&self, _task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::complete")
    }

    async fn fail(
        &self,
        _task_id: &TaskId,
        _failure_class: FailureClass,
    ) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::fail")
    }

    async fn cancel(&self, _task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::cancel")
    }

    async fn dead_letter(&self, _task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::dead_letter")
    }

    async fn list_dead_lettered(
        &self,
        _project: &ProjectKey,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::list_dead_lettered (projection)")
    }

    async fn pause(
        &self,
        _task_id: &TaskId,
        _reason: PauseReason,
    ) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::pause")
    }

    async fn resume(
        &self,
        _task_id: &TaskId,
        _trigger: ResumeTrigger,
        _target: TaskResumeTarget,
    ) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::resume")
    }

    async fn list_by_state(
        &self,
        _project: &ProjectKey,
        _state: TaskState,
        _limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::list_by_state (projection)")
    }

    async fn list_expired_leases(
        &self,
        _now: u64,
        _limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::list_expired_leases (projection)")
    }

    async fn release_lease(&self, _task_id: &TaskId) -> Result<TaskRecord, RuntimeError> {
        unimplemented!("FabricTaskServiceAdapter::release_lease")
    }
}

// ── SessionService adapter ───────────────────────────────────────────────────

/// Adapter routing [`SessionService`] calls to [`FabricServices::sessions`].
pub struct FabricSessionServiceAdapter {
    pub fabric: Arc<FabricServices>,
    pub store: Arc<InMemoryStore>,
}

impl FabricSessionServiceAdapter {
    pub fn new(fabric: Arc<FabricServices>, store: Arc<InMemoryStore>) -> Self {
        Self { fabric, store }
    }
}

#[async_trait]
impl SessionService for FabricSessionServiceAdapter {
    async fn create(
        &self,
        _project: &ProjectKey,
        _session_id: SessionId,
    ) -> Result<SessionRecord, RuntimeError> {
        unimplemented!("FabricSessionServiceAdapter::create")
    }

    async fn get(&self, _session_id: &SessionId) -> Result<Option<SessionRecord>, RuntimeError> {
        unimplemented!("FabricSessionServiceAdapter::get")
    }

    async fn list(
        &self,
        _project: &ProjectKey,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<SessionRecord>, RuntimeError> {
        unimplemented!("FabricSessionServiceAdapter::list (projection)")
    }

    async fn archive(&self, _session_id: &SessionId) -> Result<SessionRecord, RuntimeError> {
        unimplemented!("FabricSessionServiceAdapter::archive")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_domain::{
        EventEnvelope, EventId, EventSource, RunCreated, RuntimeEvent, SessionCreated, TaskCreated,
    };
    use cairn_store::event_log::EventLog;

    /// The adapter types should be `Send + Sync` so they can live inside
    /// `Arc<dyn RunService>` / `Arc<dyn TaskService>` / `Arc<dyn SessionService>`
    /// alongside the existing `*ServiceImpl` variants.
    #[test]
    fn adapters_are_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<FabricRunServiceAdapter>();
        assert_send_sync::<FabricTaskServiceAdapter>();
        assert_send_sync::<FabricSessionServiceAdapter>();
    }

    fn test_project() -> ProjectKey {
        ProjectKey::new("tenant-a", "workspace-a", "project-a")
    }

    fn envelope(event: RuntimeEvent) -> EventEnvelope<RuntimeEvent> {
        EventEnvelope::for_runtime_event(EventId::new("evt_test"), EventSource::Runtime, event)
    }

    async fn seed_session(
        store: &Arc<InMemoryStore>,
        project: &ProjectKey,
        session_id: &SessionId,
    ) {
        store
            .append(&[envelope(RuntimeEvent::SessionCreated(SessionCreated {
                project: project.clone(),
                session_id: session_id.clone(),
            }))])
            .await
            .unwrap();
    }

    async fn seed_run(
        store: &Arc<InMemoryStore>,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) {
        store
            .append(&[envelope(RuntimeEvent::RunCreated(RunCreated {
                project: project.clone(),
                session_id: session_id.clone(),
                run_id: run_id.clone(),
                parent_run_id: None,
                agent_role_id: None,
                prompt_release_id: None,
            }))])
            .await
            .unwrap();
    }

    async fn seed_task(
        store: &Arc<InMemoryStore>,
        project: &ProjectKey,
        task_id: &TaskId,
        parent_run_id: Option<&RunId>,
    ) {
        store
            .append(&[envelope(RuntimeEvent::TaskCreated(TaskCreated {
                project: project.clone(),
                task_id: task_id.clone(),
                parent_run_id: parent_run_id.cloned(),
                parent_task_id: None,
                prompt_release_id: None,
            }))])
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn resolve_run_returns_none_for_unknown_id() {
        let store = Arc::new(InMemoryStore::new());
        let result = resolve_project_from_run_id(&store, &RunId::new("run_missing"))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn resolve_task_returns_none_for_unknown_id() {
        let store = Arc::new(InMemoryStore::new());
        let result = resolve_project_from_task_id(&store, &TaskId::new("task_missing"))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn resolve_session_returns_none_for_unknown_id() {
        let store = Arc::new(InMemoryStore::new());
        let result = resolve_project_from_session_id(&store, &SessionId::new("sess_missing"))
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn resolve_run_returns_project_after_insert() {
        let store = Arc::new(InMemoryStore::new());
        let project = test_project();
        let session_id = SessionId::new("sess_1");
        let run_id = RunId::new("run_1");

        seed_session(&store, &project, &session_id).await;
        seed_run(&store, &project, &session_id, &run_id).await;

        let resolved = resolve_project_from_run_id(&store, &run_id)
            .await
            .unwrap()
            .expect("run is seeded, resolver must return Some");
        assert_eq!(resolved, project);
    }

    #[tokio::test]
    async fn resolve_task_returns_project_after_insert() {
        let store = Arc::new(InMemoryStore::new());
        let project = test_project();
        let task_id = TaskId::new("task_1");

        seed_task(&store, &project, &task_id, None).await;

        let resolved = resolve_project_from_task_id(&store, &task_id)
            .await
            .unwrap()
            .expect("task is seeded, resolver must return Some");
        assert_eq!(resolved, project);
    }

    #[tokio::test]
    async fn resolve_session_returns_project_after_insert() {
        let store = Arc::new(InMemoryStore::new());
        let project = test_project();
        let session_id = SessionId::new("sess_1");

        seed_session(&store, &project, &session_id).await;

        let resolved = resolve_project_from_session_id(&store, &session_id)
            .await
            .unwrap()
            .expect("session is seeded, resolver must return Some");
        assert_eq!(resolved, project);
    }

    /// When a run, task, and session all exist for the same scope, every
    /// resolver must return the identical `ProjectKey`. Guards against
    /// accidentally projecting a stale or mismatched scope field (e.g. if
    /// someone ever added a resolver that picked `session.project` for a task
    /// lookup).
    #[tokio::test]
    async fn resolvers_agree_across_run_task_session_for_same_scope() {
        let store = Arc::new(InMemoryStore::new());
        let project = test_project();
        let session_id = SessionId::new("sess_shared");
        let run_id = RunId::new("run_shared");
        let task_id = TaskId::new("task_shared");

        seed_session(&store, &project, &session_id).await;
        seed_run(&store, &project, &session_id, &run_id).await;
        seed_task(&store, &project, &task_id, Some(&run_id)).await;

        let run_proj = resolve_project_from_run_id(&store, &run_id)
            .await
            .unwrap()
            .unwrap();
        let task_proj = resolve_project_from_task_id(&store, &task_id)
            .await
            .unwrap()
            .unwrap();
        let sess_proj = resolve_project_from_session_id(&store, &session_id)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(run_proj, project);
        assert_eq!(task_proj, project);
        assert_eq!(sess_proj, project);
        assert_eq!(run_proj, task_proj);
        assert_eq!(task_proj, sess_proj);
    }
}
