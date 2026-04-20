//! Read-only test fixture standing in for the production Fabric service
//! trio. Built for cairn-app handler tests that only GET (which is most of
//! them — bootstrap_server, metrics_*, entitlement_gates, tenant_overview,
//! sse_streaming_e2e, provider_lifecycle_e2e, and similar handler-shape
//! tests that boot AppState but don't exercise runtime mutation).
//!
//! What it does: forwards every read method
//! (`get`, `list_by_session`, `list_by_state`, `list_expired_leases`,
//! `list_child_runs`, `list_dead_lettered`) directly to the projection
//! store. Tests seed state via direct envelope append on the shared
//! `InMemoryStore`; handlers then read it through these trait objects as
//! if production Fabric had written it.
//!
//! What it does NOT do: any state mutation. `submit`, `start`, `claim`,
//! `complete`, and the rest of the transition methods return
//! `RuntimeError::Internal` with a message routing the test author to
//! `crates/cairn-fabric/tests/integration/` — that is where real
//! task-state coordination (which in production flows through FF FCALLs
//! and bridge events) belongs.
//!
//! `declare_dependency` also fails; `check_dependencies` returns an empty
//! vector (i.e. "no blockers") because a read-only fixture never stages
//! a dependency. A handler that expects a real dependency set in a
//! handler test is in the wrong place and needs to migrate.
//!
//! Lives under `tests/support/` so it never ships in the binary.

use std::sync::Arc;

use async_trait::async_trait;
use cairn_domain::commands::StartRun;
use cairn_domain::{
    ApprovalDecision, FailureClass, PauseReason, ProjectKey, ResumeTrigger, RunId, RunResumeTarget,
    SessionId, TaskId, TaskResumeTarget, TaskState,
};
use cairn_runtime::error::RuntimeError;
use cairn_runtime::runs::RunService;
use cairn_runtime::sessions::SessionService;
use cairn_runtime::tasks::TaskService;
use cairn_store::projections::{
    RunReadModel, RunRecord, SessionReadModel, SessionRecord, TaskReadModel, TaskRecord,
};
use cairn_store::InMemoryStore;

fn readonly(method: &'static str) -> RuntimeError {
    RuntimeError::Internal(format!(
        "FakeFabric is a read-only test fixture: `{method}` is unsupported. \
         Tests that need mutation belong in crates/cairn-fabric/tests/integration/ \
         against live Valkey."
    ))
}

/// Build a trio of read-only trait objects backed by the same in-memory store.
///
/// Wire into `InMemoryServices::with_store_and_core(store, runs, tasks, sessions)`
/// to stand up an AppState without a live Valkey.
pub fn build_fake_fabric(
    store: Arc<InMemoryStore>,
) -> (
    Arc<dyn RunService>,
    Arc<dyn TaskService>,
    Arc<dyn SessionService>,
) {
    (
        Arc::new(FakeFabricRuns {
            store: store.clone(),
        }),
        Arc::new(FakeFabricTasks {
            store: store.clone(),
        }),
        Arc::new(FakeFabricSessions { store }),
    )
}

// ── Sessions ───────────────────────────────────────────────────────────────

pub struct FakeFabricSessions {
    store: Arc<InMemoryStore>,
}

#[async_trait]
impl SessionService for FakeFabricSessions {
    async fn create(
        &self,
        _project: &ProjectKey,
        _session_id: SessionId,
    ) -> Result<SessionRecord, RuntimeError> {
        Err(readonly("sessions.create"))
    }

    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, RuntimeError> {
        Ok(SessionReadModel::get(self.store.as_ref(), session_id).await?)
    }

    async fn list(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, RuntimeError> {
        Ok(SessionReadModel::list_by_project(self.store.as_ref(), project, limit, offset).await?)
    }

    async fn archive(&self, _session_id: &SessionId) -> Result<SessionRecord, RuntimeError> {
        Err(readonly("sessions.archive"))
    }
}

// ── Runs ────────────────────────────────────────────────────────────────────

pub struct FakeFabricRuns {
    store: Arc<InMemoryStore>,
}

#[async_trait]
impl RunService for FakeFabricRuns {
    async fn start(
        &self,
        _project: &ProjectKey,
        _session_id: &SessionId,
        _run_id: RunId,
        _parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.start"))
    }

    async fn start_command(&self, _command: StartRun) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.start_command"))
    }

    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, RuntimeError> {
        Ok(RunReadModel::get(self.store.as_ref(), run_id).await?)
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError> {
        Ok(RunReadModel::list_by_session(self.store.as_ref(), session_id, limit, offset).await?)
    }

    async fn complete(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.complete"))
    }

    async fn fail(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
        _failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.fail"))
    }

    async fn cancel(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.cancel"))
    }

    async fn pause(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
        _reason: PauseReason,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.pause"))
    }

    async fn resume(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
        _trigger: ResumeTrigger,
        _target: RunResumeTarget,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.resume"))
    }

    async fn claim(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.claim"))
    }

    async fn enter_waiting_approval(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.enter_waiting_approval"))
    }

    async fn resolve_approval(
        &self,
        _session_id: &SessionId,
        _run_id: &RunId,
        _decision: ApprovalDecision,
    ) -> Result<RunRecord, RuntimeError> {
        Err(readonly("runs.resolve_approval"))
    }

    async fn list_child_runs(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError> {
        Ok(RunReadModel::list_by_parent_run(self.store.as_ref(), parent_run_id, limit).await?)
    }
}

// ── Tasks ───────────────────────────────────────────────────────────────────

pub struct FakeFabricTasks {
    store: Arc<InMemoryStore>,
}

#[async_trait]
impl TaskService for FakeFabricTasks {
    async fn submit(
        &self,
        _project: &ProjectKey,
        _session_id: Option<&SessionId>,
        _task_id: TaskId,
        _parent_run_id: Option<RunId>,
        _parent_task_id: Option<TaskId>,
        _priority: u32,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.submit"))
    }

    async fn declare_dependency(
        &self,
        _dependent_task_id: &TaskId,
        _prerequisite_task_id: &TaskId,
    ) -> Result<cairn_domain::TaskDependencyRecord, RuntimeError> {
        Err(readonly("tasks.declare_dependency"))
    }

    async fn check_dependencies(
        &self,
        _task_id: &TaskId,
    ) -> Result<Vec<cairn_domain::TaskDependencyRecord>, RuntimeError> {
        // Read-only fixture never stages dependencies; returning the empty
        // set is the correct answer for "this task has no active blockers"
        // and keeps handlers (which just forward this result) working.
        Ok(Vec::new())
    }

    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, RuntimeError> {
        Ok(TaskReadModel::get(self.store.as_ref(), task_id).await?)
    }

    async fn claim(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
        _lease_owner: String,
        _lease_duration_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.claim"))
    }

    async fn heartbeat(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
        _lease_extension_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.heartbeat"))
    }

    async fn start(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.start"))
    }

    async fn complete(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.complete"))
    }

    async fn fail(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
        _failure_class: FailureClass,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.fail"))
    }

    async fn cancel(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.cancel"))
    }

    async fn dead_letter(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.dead_letter"))
    }

    async fn list_dead_lettered(
        &self,
        project: &ProjectKey,
        limit: usize,
        _offset: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(
            TaskReadModel::list_by_state(self.store.as_ref(), project, TaskState::DeadLettered, limit)
                .await?,
        )
    }

    async fn pause(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
        _reason: PauseReason,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.pause"))
    }

    async fn resume(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
        _trigger: ResumeTrigger,
        _target: TaskResumeTarget,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.resume"))
    }

    async fn list_by_state(
        &self,
        project: &ProjectKey,
        state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(TaskReadModel::list_by_state(self.store.as_ref(), project, state, limit).await?)
    }

    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        Ok(TaskReadModel::list_expired_leases(self.store.as_ref(), now, limit).await?)
    }

    async fn release_lease(
        &self,
        _session_id: Option<&SessionId>,
        _task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        Err(readonly("tasks.release_lease"))
    }
}
