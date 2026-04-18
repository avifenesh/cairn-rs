//! SQLite-backed runtime integration test.
//! Proves the runtime service seams hold against a real database backend.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::tool_invocation::ToolInvocationTarget;
use cairn_domain::workers::{ExternalWorkerOutcome, ExternalWorkerReport};
use cairn_domain::TenantId;
use cairn_domain::*;
use cairn_runtime::{
    ExternalWorkerService, ExternalWorkerServiceImpl, RunService, RunServiceImpl, SessionService,
    SessionServiceImpl, TaskService, TaskServiceImpl, ToolInvocationService,
    ToolInvocationServiceImpl,
};
use cairn_store::event_log::{EntityRef, EventLog, EventPosition, StoredEvent};
use cairn_store::projections::*;
use cairn_store::sqlite::{SqliteAdapter, SqliteEventLog, SqliteSyncProjection};
use cairn_store::StoreError;

/// Combined SQLite store implementing EventLog + all ReadModel traits.
struct SqliteStore {
    event_log: SqliteEventLog,
    adapter: SqliteAdapter,
}

impl SqliteStore {
    async fn in_memory() -> Self {
        let adapter = SqliteAdapter::in_memory().await.unwrap();
        let event_log = SqliteEventLog::new(adapter.pool().clone());
        Self { event_log, adapter }
    }
}

#[async_trait]
impl EventLog for SqliteStore {
    async fn append(
        &self,
        events: &[EventEnvelope<RuntimeEvent>],
    ) -> Result<Vec<EventPosition>, StoreError> {
        // Append events to the log, then apply sync projections within a transaction
        let positions = self.event_log.append(events).await?;

        // Apply projections for each event
        let mut tx = self
            .adapter
            .pool()
            .begin()
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;
        for (envelope, pos) in events.iter().zip(positions.iter()) {
            let stored = StoredEvent {
                position: *pos,
                envelope: envelope.clone(),
                stored_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_millis() as u64,
            };
            SqliteSyncProjection::apply_async(&mut tx, &stored).await?;
        }
        tx.commit()
            .await
            .map_err(|e| StoreError::Connection(e.to_string()))?;

        Ok(positions)
    }
    async fn read_by_entity(
        &self,
        entity: &EntityRef,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        self.event_log.read_by_entity(entity, after, limit).await
    }
    async fn read_stream(
        &self,
        after: Option<EventPosition>,
        limit: usize,
    ) -> Result<Vec<StoredEvent>, StoreError> {
        self.event_log.read_stream(after, limit).await
    }
    async fn head_position(&self) -> Result<Option<EventPosition>, StoreError> {
        self.event_log.head_position().await
    }

    async fn find_by_causation_id(
        &self,
        causation_id: &str,
    ) -> Result<Option<EventPosition>, StoreError> {
        self.event_log.find_by_causation_id(causation_id).await
    }
}

#[async_trait]
impl SessionReadModel for SqliteStore {
    async fn get(&self, id: &SessionId) -> Result<Option<SessionRecord>, StoreError> {
        SessionReadModel::get(&self.adapter, id).await
    }
    async fn list_by_project(
        &self,
        p: &ProjectKey,
        l: usize,
        o: usize,
    ) -> Result<Vec<SessionRecord>, StoreError> {
        self.adapter.list_by_project(p, l, o).await
    }
    async fn list_active(&self, limit: usize) -> Result<Vec<SessionRecord>, StoreError> {
        self.adapter.list_active(limit).await
    }
}

#[async_trait]
impl RunReadModel for SqliteStore {
    async fn get(&self, id: &RunId) -> Result<Option<RunRecord>, StoreError> {
        RunReadModel::get(&self.adapter, id).await
    }
    async fn list_by_session(
        &self,
        s: &SessionId,
        l: usize,
        o: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        RunReadModel::list_by_session(&self.adapter, s, l, o).await
    }
    async fn any_non_terminal(&self, s: &SessionId) -> Result<bool, StoreError> {
        self.adapter.any_non_terminal(s).await
    }
    async fn latest_root_run(&self, s: &SessionId) -> Result<Option<RunRecord>, StoreError> {
        self.adapter.latest_root_run(s).await
    }
    async fn list_by_state(
        &self,
        state: RunState,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        RunReadModel::list_by_state(&self.adapter, state, limit).await
    }
    async fn list_active_by_project(
        &self,
        project: &ProjectKey,
        limit: usize,
    ) -> Result<Vec<RunRecord>, StoreError> {
        RunReadModel::list_active_by_project(&self.adapter, project, limit).await
    }
}

#[async_trait]
impl TaskReadModel for SqliteStore {
    async fn get(&self, id: &TaskId) -> Result<Option<TaskRecord>, StoreError> {
        TaskReadModel::get(&self.adapter, id).await
    }
    async fn list_by_state(
        &self,
        p: &ProjectKey,
        s: TaskState,
        l: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        TaskReadModel::list_by_state(&self.adapter, p, s, l).await
    }
    async fn list_by_parent_run(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, StoreError> {
        TaskReadModel::list_by_parent_run(&self.adapter, parent_run_id, limit).await
    }
    async fn any_non_terminal_children(&self, parent_run_id: &RunId) -> Result<bool, StoreError> {
        TaskReadModel::any_non_terminal_children(&self.adapter, parent_run_id).await
    }
    async fn list_expired_leases(&self, now: u64, l: usize) -> Result<Vec<TaskRecord>, StoreError> {
        self.adapter.list_expired_leases(now, l).await
    }
}

#[async_trait]
impl ApprovalReadModel for SqliteStore {
    async fn get(&self, id: &ApprovalId) -> Result<Option<ApprovalRecord>, StoreError> {
        ApprovalReadModel::get(&self.adapter, id).await
    }
    async fn list_pending(
        &self,
        p: &ProjectKey,
        l: usize,
        o: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError> {
        ApprovalReadModel::list_pending(&self.adapter, p, l, o).await
    }
    async fn list_all(
        &self,
        p: &ProjectKey,
        l: usize,
        o: usize,
    ) -> Result<Vec<ApprovalRecord>, StoreError> {
        ApprovalReadModel::list_all(&self.adapter, p, l, o).await
    }
    async fn has_pending_for_run(&self, run_id: &RunId) -> Result<bool, StoreError> {
        ApprovalReadModel::has_pending_for_run(&self.adapter, run_id).await
    }
}

#[async_trait]
impl CheckpointReadModel for SqliteStore {
    async fn get(&self, id: &CheckpointId) -> Result<Option<CheckpointRecord>, StoreError> {
        CheckpointReadModel::get(&self.adapter, id).await
    }
    async fn latest_for_run(&self, r: &RunId) -> Result<Option<CheckpointRecord>, StoreError> {
        CheckpointReadModel::latest_for_run(&self.adapter, r).await
    }
    async fn list_by_run(&self, r: &RunId, l: usize) -> Result<Vec<CheckpointRecord>, StoreError> {
        CheckpointReadModel::list_by_run(&self.adapter, r, l).await
    }
}

#[async_trait]
impl MailboxReadModel for SqliteStore {
    async fn get(&self, id: &MailboxMessageId) -> Result<Option<MailboxRecord>, StoreError> {
        MailboxReadModel::get(&self.adapter, id).await
    }
    async fn list_by_run(
        &self,
        r: &RunId,
        l: usize,
        o: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        MailboxReadModel::list_by_run(&self.adapter, r, l, o).await
    }
    async fn list_by_task(
        &self,
        t: &TaskId,
        l: usize,
        o: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        MailboxReadModel::list_by_task(&self.adapter, t, l, o).await
    }
    async fn list_pending(
        &self,
        now_ms: u64,
        limit: usize,
    ) -> Result<Vec<MailboxRecord>, StoreError> {
        MailboxReadModel::list_pending(&self.adapter, now_ms, limit).await
    }
}

#[async_trait]
impl TaskDependencyReadModel for SqliteStore {
    async fn list_blocking(
        &self,
        _task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, StoreError> {
        Ok(vec![])
    }
    async fn list_unresolved(
        &self,
        _project: &ProjectKey,
    ) -> Result<Vec<TaskDependencyRecord>, StoreError> {
        Ok(vec![])
    }
    async fn insert_dependency(&self, _record: TaskDependencyRecord) -> Result<(), StoreError> {
        Ok(())
    }
    async fn resolve_dependency(
        &self,
        _prerequisite_task_id: &TaskId,
        _resolved_at_ms: u64,
    ) -> Result<(), StoreError> {
        Ok(())
    }
}

#[async_trait]
impl CheckpointStrategyReadModel for SqliteStore {
    async fn get_by_run(
        &self,
        run_id: &RunId,
    ) -> Result<Option<cairn_domain::CheckpointStrategy>, StoreError> {
        CheckpointStrategyReadModel::get_by_run(&self.adapter, run_id).await
    }
}

#[async_trait]
impl QuotaReadModel for SqliteStore {
    async fn get_quota(
        &self,
        _tenant_id: &TenantId,
    ) -> Result<Option<cairn_domain::TenantQuota>, StoreError> {
        Ok(None)
    }
}

fn project() -> ProjectKey {
    ProjectKey::new("t_sq", "w_sq", "p_sq")
}

#[tokio::test]
async fn sqlite_session_run_task_lifecycle() {
    let store = Arc::new(SqliteStore::in_memory().await);
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let task_svc = TaskServiceImpl::new(store.clone());
    let p = project();

    let session = session_svc.create(&p, SessionId::new("s1")).await.unwrap();
    assert_eq!(session.state, SessionState::Open);

    run_svc
        .start(&p, &SessionId::new("s1"), RunId::new("r1"), None)
        .await
        .unwrap();
    run_svc
        .resume(
            &RunId::new("r1"),
            ResumeTrigger::RuntimeSignal,
            RunResumeTarget::Running,
        )
        .await
        .unwrap();

    task_svc
        .submit(&p, TaskId::new("t1"), Some(RunId::new("r1")), None, 0)
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("t1"), "w".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("t1")).await.unwrap();
    task_svc.complete(&TaskId::new("t1")).await.unwrap();
    run_svc.complete(&RunId::new("r1")).await.unwrap();

    let run = run_svc.get(&RunId::new("r1")).await.unwrap().unwrap();
    assert_eq!(run.state, RunState::Completed);
    let task = task_svc.get(&TaskId::new("t1")).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Completed);
}

#[tokio::test]
async fn sqlite_tool_invocation_seam() {
    let store = Arc::new(SqliteStore::in_memory().await);
    let session_svc = SessionServiceImpl::new(store.clone());
    let run_svc = RunServiceImpl::new(store.clone());
    let svc = ToolInvocationServiceImpl::new(store.clone());
    let p = project();

    // Create prerequisite entities for FK constraints
    session_svc.create(&p, SessionId::new("s1")).await.unwrap();
    run_svc
        .start(&p, &SessionId::new("s1"), RunId::new("r1"), None)
        .await
        .unwrap();

    svc.record_start(
        &p,
        ToolInvocationId::new("inv1"),
        Some(SessionId::new("s1")),
        Some(RunId::new("r1")),
        None,
        ToolInvocationTarget::Builtin {
            tool_name: "fs.read".to_owned(),
        },
        ExecutionClass::SupervisedProcess,
    )
    .await
    .unwrap();
    svc.record_completed(
        &p,
        ToolInvocationId::new("inv1"),
        Some(TaskId::new("t1")),
        "fs.read".to_owned(),
    )
    .await
    .unwrap();

    let events = store.read_stream(None, 100).await.unwrap();
    assert!(events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::ToolInvocationStarted(_))));
    assert!(events
        .iter()
        .any(|e| matches!(e.envelope.payload, RuntimeEvent::ToolInvocationCompleted(_))));
}

#[tokio::test]
async fn sqlite_external_worker_seam() {
    let store = Arc::new(SqliteStore::in_memory().await);
    let task_svc = TaskServiceImpl::new(store.clone());
    let worker_svc = ExternalWorkerServiceImpl::new(store.clone());

    task_svc
        .submit(&project(), TaskId::new("t1"), None, None, 0)
        .await
        .unwrap();
    task_svc
        .claim(&TaskId::new("t1"), "ext".to_owned(), 60_000)
        .await
        .unwrap();
    task_svc.start(&TaskId::new("t1")).await.unwrap();

    worker_svc
        .report(ExternalWorkerReport {
            project: project(),
            worker_id: "ext".into(),
            run_id: None,
            task_id: TaskId::new("t1"),
            lease_token: 1,
            reported_at_ms: 99999,
            progress: None,
            outcome: Some(ExternalWorkerOutcome::Completed),
        })
        .await
        .unwrap();

    let task = task_svc.get(&TaskId::new("t1")).await.unwrap().unwrap();
    assert_eq!(task.state, TaskState::Completed);
}

// `sqlite_resolve_stale_dependencies_e2e` deleted in the Fabric
// finalization round. FF's DependencyReconciler scanner owns
// resolve_stale_dependencies unconditionally
// (ff-engine/src/scanner/dependency_reconciler.rs); a cairn-side sweep
// over a SQLite projection was only meaningful when cairn-runtime did
// recovery itself.
