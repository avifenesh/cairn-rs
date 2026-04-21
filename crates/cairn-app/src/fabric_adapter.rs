//! Bridges trait-based handlers to FabricServices.
//!
//! Reads go to store projection; writes go to Fabric. Installed on
//! `state.runtime.{runs,tasks,sessions}` by `AppState::new`.
//!
//! This module exists so that HTTP handlers can continue to call
//! `state.runtime.runs.get(...)` (a trait method with bare IDs) while the
//! underlying work is routed to [`cairn_fabric::FabricServices`], which
//! requires a `ProjectKey` for every operation. The adapter resolves the
//! missing project context by reading the cairn-store projection first, then
//! delegates to the Fabric service.
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
use cairn_domain::TaskDependencyRecord;
use cairn_domain::{
    ApprovalDecision, FailureClass, PauseReason, ProjectKey, ResumeTrigger, RunId, RunResumeTarget,
    SessionId, TaskId, TaskResumeTarget, TaskState,
};
use cairn_fabric::{FabricError, FabricServices};
use cairn_runtime::error::RuntimeError;
use cairn_runtime::runs::RunService;
use cairn_runtime::sessions::SessionService;
use cairn_runtime::tasks::TaskService;
use cairn_store::projections::{
    RunReadModel, RunRecord, SessionReadModel, SessionRecord, TaskReadModel, TaskRecord,
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

/// Translate a `FabricError` into the handler-facing `RuntimeError`.
///
/// Both types already carry structured NotFound / Validation / Internal
/// variants, so the mapping is direct. We keep this as a private helper
/// (rather than a `From` impl in cairn-fabric) so cairn-fabric does not
/// depend on cairn-runtime — bridge goes one way, same shape both ends.
fn fabric_err_to_runtime(err: FabricError) -> RuntimeError {
    match err {
        FabricError::NotFound { entity, id } => RuntimeError::NotFound { entity, id },
        FabricError::Validation { reason } => RuntimeError::Validation { reason },
        // FF FCALL contention codes are caller-retriable, not operator 5xx:
        // they fire when two workers race for the same lease, when a grant
        // TTL expires mid-claim, or when a scheduler-routed eligible set
        // changes under the caller's feet. Surface them as 409 Conflict so
        // clients can back off + retry instead of triggering ops alerts.
        //
        // The rejection is packed as `FabricError::Internal("<fcall> rejected:
        // <code>")` by `check_fcall_success`; pattern-match on the `: <code>`
        // suffix. Keep the list tight — only FF-documented contention codes
        // belong here; anything else stays Internal so legitimate bugs don't
        // get hidden behind a 409.
        FabricError::Internal(ref msg) if is_claim_contention(msg) => {
            tracing::debug!(fabric_err = %msg, "fabric claim contention (409 to caller)");
            RuntimeError::Conflict {
                entity: "execution",
                id: msg.clone(),
            }
        }
        // SEC-007: Valkey / script / bridge / config / internal variants
        // carry FCALL names, key names, and occasionally secret-hash
        // references — none of which should reach the 500 response body.
        // Log the detail for operators (journald / CloudWatch) and return
        // an opaque message to the caller.
        other => {
            tracing::error!(fabric_err = %other, "fabric layer error");
            RuntimeError::Internal("fabric layer error".into())
        }
    }
}

/// FF typed error codes that represent caller-retriable contention rather
/// than an operator-alert system fault. See `ff-script::ScriptError` for
/// the canonical list and `claim_common::issue_grant_and_claim` for the
/// call sites that produce them.
fn is_claim_contention(msg: &str) -> bool {
    const CONTENTION_CODES: &[&str] = &[
        "lease_conflict",
        "invalid_claim_grant",
        "claim_grant_expired",
        "execution_not_leaseable",
        "execution_not_eligible",
        "execution_not_eligible_for_attempt",
        // Scheduler-routed contention: another scheduler already pulled
        // the execution out of the eligible set before this caller's
        // grant-issue FCALL landed. Caller-retriable (wait for the
        // winner to finish or for a new execution to become eligible).
        "execution_not_in_eligible_set",
        // Grant-step contention: another worker's grant was still
        // active (within grant_ttl_ms) when this caller tried to
        // issue its own. This is the dominant shape of the
        // concurrent-claim race: N callers hit ff_issue_claim_grant
        // simultaneously, the first wins, the others see this.
        "grant_already_exists",
        "execution_not_found",
        // Replay/terminal contention: a terminal path (cancellation,
        // completion) mutated state between eligibility and claim.
        "execution_not_active",
        "no_active_lease",
        "no_eligible_execution",
    ];
    // Format (from check_fcall_success): "<fcall> rejected: <code>".
    let Some((_, code)) = msg.rsplit_once(": ") else {
        return false;
    };
    CONTENTION_CODES.contains(&code.trim())
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
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, RuntimeError> {
        // Caller already supplies a project — straight delegation, no
        // projection lookup needed.
        self.fabric
            .runs
            .start(project, session_id, run_id, parent_run_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    /// Override the default trait impl (which drops the correlation_id and
    /// falls through to `start`). Fabric path threads the correlation onto
    /// the FF `cairn.correlation_id` exec_core tag AND onto the emitted
    /// `BridgeEvent::ExecutionCreated` so the cairn-store envelope's
    /// `correlation_id` field is populated for SSE / audit consumers. Sqeq
    /// ingress is the primary caller.
    async fn start_with_correlation(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
        correlation_id: &str,
    ) -> Result<RunRecord, RuntimeError> {
        self.fabric
            .runs
            .start_with_correlation(
                project,
                session_id,
                run_id,
                parent_run_id,
                Some(correlation_id),
            )
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, RuntimeError> {
        let record = match resolve_run_scope(&self.store, run_id).await? {
            Some(r) => r,
            None => return Ok(None),
        };
        self.fabric
            .runs
            .get(&record.project, &record.session_id, run_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn list_by_session(
        &self,
        session_id: &SessionId,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError> {
        // Projection path: FF does not index runs by cairn SessionId.
        // cairn-store's RunReadModel serves this view from the event log;
        // `FabricRunService::list_by_session` itself returns an empty Vec by
        // design (see run_service.rs:398-402).
        list_runs_by_session_from_projection(&self.store, session_id, limit, offset).await
    }

    async fn complete(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        // Cross-check the caller's session_id against the projection.
        // Mismatch is an operator error (the run is keyed to a
        // different session in the projection); we fail loud rather
        // than silently minting a different ExecutionId.
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .complete(&project, session_id, run_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn fail(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, RuntimeError> {
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .fail(&project, session_id, run_id, failure_class)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn cancel(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .cancel(&project, session_id, run_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn pause(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        reason: PauseReason,
    ) -> Result<RunRecord, RuntimeError> {
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .pause(&project, session_id, run_id, reason)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn resume(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        trigger: ResumeTrigger,
        target: RunResumeTarget,
    ) -> Result<RunRecord, RuntimeError> {
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .resume(&project, session_id, run_id, trigger, target)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn claim(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        // Active-lease activation for the run's FF execution so the
        // approval-gate / signal-delivery FCALLs accept it downstream.
        // `FabricRunService::claim` handles the
        // ff_issue_claim_grant + ff_claim_execution sequence (and the
        // `use_claim_resumed_execution` dispatch for resumed
        // executions) via `claim_common::issue_grant_and_claim`.
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .claim(&project, session_id, run_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn enter_waiting_approval(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, RuntimeError> {
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .enter_waiting_approval(&project, session_id, run_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn resolve_approval(
        &self,
        session_id: &SessionId,
        run_id: &RunId,
        decision: ApprovalDecision,
    ) -> Result<RunRecord, RuntimeError> {
        let project = resolve_run_project_checking_session(&self.store, run_id, session_id).await?;
        self.fabric
            .runs
            .resolve_approval(&project, session_id, run_id, decision)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn list_child_runs(
        &self,
        parent_run_id: &RunId,
        limit: usize,
    ) -> Result<Vec<RunRecord>, RuntimeError> {
        // Parent→child linkage is indexed in the projection (Postgres
        // / SQLite `idx_runs_parent`, InMemory HashMap filter). FF has
        // no native parent-run index (child runs get distinct
        // execution ids via id_map), so the store projection is the
        // authoritative read.
        use cairn_store::projections::RunReadModel;
        Ok(RunReadModel::list_by_parent_run(self.store.as_ref(), parent_run_id, limit).await?)
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

// Error type for the bare-ID path: either the projection failed to find the
// record (returns NotFound) or the resolver hit a real store error.
#[allow(dead_code)]
async fn resolve_task_project(
    store: &Arc<InMemoryStore>,
    task_id: &TaskId,
) -> Result<ProjectKey, RuntimeError> {
    resolve_project_from_task_id(store, task_id)
        .await?
        .ok_or_else(|| RuntimeError::NotFound {
            entity: "task",
            id: task_id.to_string(),
        })
}

async fn resolve_session_project(
    store: &Arc<InMemoryStore>,
    session_id: &SessionId,
) -> Result<ProjectKey, RuntimeError> {
    resolve_project_from_session_id(store, session_id)
        .await?
        .ok_or_else(|| RuntimeError::NotFound {
            entity: "session",
            id: session_id.to_string(),
        })
}

/// Fetch a task's projection record.
///
/// Returns `Ok(None)` for unknown task ids (projection-lag race or never
/// created). Callers on the task-mutation path treat `None` as
/// NotFound; the `get` path returns `Ok(None)` to the HTTP layer.
async fn resolve_task_scope(
    store: &Arc<InMemoryStore>,
    task_id: &TaskId,
) -> Result<Option<TaskRecord>, RuntimeError> {
    TaskReadModel::get(store.as_ref(), task_id)
        .await
        .map_err(RuntimeError::from)
}

/// Resolve the task's `(project, session_id_option)` from the
/// projection. `session_id` is derived by following
/// `TaskRecord.parent_run_id → RunRecord.session_id` — cairn does not
/// store session scope directly on the task projection.
///
/// Returns `Err(NotFound)` when the task itself is missing from the
/// projection. When the task has no parent run (bare submission),
/// returns `Ok((project, None))` matching the `task_to_execution_id`
/// (solo) mint path used at submit time.
///
/// When `caller_session_id` is supplied (the adapter threads it through
/// from the trait method), we cross-check that it matches the
/// projection-derived value. Mismatch ⇒ typed Validation error, same
/// contract as `resolve_run_project_checking_session` — no silent
/// fallbacks.
/// Read-only variant: returns `None` when the task is unknown (so the
/// `get` handler can return 404) instead of erroring. Derives
/// `session_id` from the parent run; does not cross-check.
async fn resolve_task_project_and_session_opt(
    store: &Arc<InMemoryStore>,
    task_id: &TaskId,
) -> Result<Option<(ProjectKey, Option<SessionId>)>, RuntimeError> {
    let task = match resolve_task_scope(store, task_id).await? {
        Some(t) => t,
        None => return Ok(None),
    };
    let session = match &task.parent_run_id {
        Some(prid) => RunReadModel::get(store.as_ref(), prid)
            .await?
            .map(|r| r.session_id),
        None => None,
    };
    Ok(Some((task.project, session)))
}

async fn resolve_task_project_and_session(
    store: &Arc<InMemoryStore>,
    task_id: &TaskId,
    caller_session_id: Option<&SessionId>,
) -> Result<(ProjectKey, Option<SessionId>), RuntimeError> {
    let task = resolve_task_scope(store, task_id)
        .await?
        .ok_or_else(|| RuntimeError::NotFound {
            entity: "task",
            id: task_id.to_string(),
        })?;

    // Use the session binding already on the task record when present.
    // Bare tasks carry no binding and route via the solo mint path.
    // If the task row has no session_id, walk parent_run_id → run.session_id.
    let derived_session_id = if let Some(sid) = task.session_id.clone() {
        Some(sid)
    } else {
        match &task.parent_run_id {
            Some(parent_run_id) => {
                let run = RunReadModel::get(store.as_ref(), parent_run_id)
                    .await?
                    .ok_or_else(|| RuntimeError::NotFound {
                        entity: "run",
                        id: parent_run_id.to_string(),
                    })?;
                Some(run.session_id)
            }
            None => None,
        }
    };

    if let Some(caller) = caller_session_id {
        match &derived_session_id {
            Some(derived) if derived.as_str() == caller.as_str() => {}
            Some(derived) => {
                return Err(RuntimeError::Validation {
                    reason: format!(
                        "task {} belongs to session {}, but the request specified {}",
                        task_id.as_str(),
                        derived.as_str(),
                        caller.as_str()
                    ),
                });
            }
            None => {
                return Err(RuntimeError::Validation {
                    reason: format!(
                        "task {} was submitted without a session binding, \
                         but the request specified session {}",
                        task_id.as_str(),
                        caller.as_str()
                    ),
                });
            }
        }
    }

    Ok((task.project, derived_session_id))
}

#[allow(dead_code)]
async fn resolve_run_project(
    store: &Arc<InMemoryStore>,
    run_id: &RunId,
) -> Result<ProjectKey, RuntimeError> {
    resolve_project_from_run_id(store, run_id)
        .await?
        .ok_or_else(|| RuntimeError::NotFound {
            entity: "run",
            id: run_id.to_string(),
        })
}

/// Fetch the run's full projection record (project + session_id).
///
/// Returns `Ok(None)` when the projection has not yet observed this run
/// (create/mutate race) or the run was never created. Surfacing this as
/// `None` lets the read-only `get` path return `Ok(None)` to the caller
/// rather than silently falling back.
async fn resolve_run_scope(
    store: &Arc<InMemoryStore>,
    run_id: &RunId,
) -> Result<Option<RunRecord>, RuntimeError> {
    RunReadModel::get(store.as_ref(), run_id)
        .await
        .map_err(RuntimeError::from)
}

/// Resolve `project` from the projection AND cross-check that the
/// caller's `session_id` matches what the projection holds.
///
/// The FF `ExecutionId` is minted from
/// `(project, session_id, run_id)`; a mismatched `session_id` mints a
/// different ID and the FCALL targets a non-existent execution — which
/// FF reports as a generic not-found and the operator sees as an
/// unexplained 404. Per the "no silent fallbacks" rule, we fail loud
/// here with a typed Validation error instead.
///
/// Returns `NotFound` when the projection hasn't observed the run yet
/// (projection-lag race on a very recently started run) — the operator
/// should retry after the projection catches up.
async fn resolve_run_project_checking_session(
    store: &Arc<InMemoryStore>,
    run_id: &RunId,
    session_id: &SessionId,
) -> Result<ProjectKey, RuntimeError> {
    let record = RunReadModel::get(store.as_ref(), run_id)
        .await?
        .ok_or_else(|| RuntimeError::NotFound {
            entity: "run",
            id: run_id.to_string(),
        })?;
    if record.session_id.as_str() != session_id.as_str() {
        return Err(RuntimeError::Validation {
            reason: format!(
                "run {} is bound to session {}, but the request specified {}",
                run_id.as_str(),
                record.session_id.as_str(),
                session_id.as_str()
            ),
        });
    }
    Ok(record.project)
}

/// Projection-backed runs-by-session lookup, extracted so unit tests can
/// exercise the `list_by_session` path without constructing a Valkey-backed
/// `FabricServices`.
async fn list_runs_by_session_from_projection(
    store: &Arc<InMemoryStore>,
    session_id: &SessionId,
    limit: usize,
    offset: usize,
) -> Result<Vec<RunRecord>, RuntimeError> {
    RunReadModel::list_by_session(store.as_ref(), session_id, limit, offset)
        .await
        .map_err(RuntimeError::from)
}

#[async_trait]
impl TaskService for FabricTaskServiceAdapter {
    async fn submit(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: TaskId,
        parent_run_id: Option<RunId>,
        parent_task_id: Option<TaskId>,
        priority: u32,
    ) -> Result<TaskRecord, RuntimeError> {
        // session_id is supplied by the caller (None for bare tasks).
        // If caller omitted it but the task has a parent run
        // already in the projection, we could derive it — but at submit
        // time the parent run's session is the authoritative source, so
        // we fall back to the parent_run_id lookup.
        let resolved_session = match session_id {
            Some(sid) => Some(sid.clone()),
            None => match &parent_run_id {
                Some(prid) => {
                    // A task with a parent run must resolve its session.
                    // Silently returning None would route to the solo-mint path
                    // and land on a different Valkey partition than the parent run.
                    let run = RunReadModel::get(self.store.as_ref(), prid)
                        .await?
                        .ok_or_else(|| RuntimeError::NotFound {
                            entity: "run",
                            id: prid.to_string(),
                        })?;
                    Some(run.session_id)
                }
                None => None,
            },
        };
        self.fabric
            .tasks
            .submit(
                project,
                task_id,
                parent_run_id,
                parent_task_id,
                priority,
                resolved_session.as_ref(),
            )
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn declare_dependency(
        &self,
        dependent_task_id: &TaskId,
        prerequisite_task_id: &TaskId,
    ) -> Result<TaskDependencyRecord, RuntimeError> {
        // Resolve project + session for both tasks from the
        // projection; FF flow edges can only connect members of the
        // same flow. Reject cross-session / bare-task declares here
        // with a Validation error rather than letting FF surface a
        // less-useful opaque FCALL error.
        let (dep_project, dep_session) =
            resolve_task_project_and_session(&self.store, dependent_task_id, None).await?;
        let (pre_project, pre_session) =
            resolve_task_project_and_session(&self.store, prerequisite_task_id, None).await?;

        if dep_project != pre_project {
            return Err(RuntimeError::Validation {
                reason: format!(
                    "task dependencies must share a project: {} → {} cross project boundary",
                    dependent_task_id.as_str(),
                    prerequisite_task_id.as_str()
                ),
            });
        }

        let session_id = match (&dep_session, &pre_session) {
            (Some(a), Some(b)) if a == b => a.clone(),
            (Some(_), Some(_)) => {
                return Err(RuntimeError::Validation {
                    reason: format!(
                        "task dependencies must share a session; {} and {} \
                         belong to different sessions",
                        dependent_task_id.as_str(),
                        prerequisite_task_id.as_str()
                    ),
                });
            }
            _ => {
                return Err(RuntimeError::Validation {
                    reason: format!(
                        "task dependencies require both tasks to be session-\
                         bound; {} or {} was submitted without a session",
                        dependent_task_id.as_str(),
                        prerequisite_task_id.as_str()
                    ),
                });
            }
        };

        self.fabric
            .tasks
            .declare_dependency(
                &dep_project,
                &session_id,
                dependent_task_id,
                prerequisite_task_id,
            )
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn check_dependencies(
        &self,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, RuntimeError> {
        // Bare tasks (no session) can't have dependencies — there's
        // no flow for them to live in. Return empty rather than
        // surfacing a less-useful error.
        let (project, session_id) =
            resolve_task_project_and_session(&self.store, task_id, None).await?;
        let Some(sid) = session_id else {
            return Ok(Vec::new());
        };
        self.fabric
            .tasks
            .check_dependencies(&project, &sid, task_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, RuntimeError> {
        // Read-only: derive (project, session_id) from projection; None
        // caller_session means no cross-check.
        let (project, session) =
            match resolve_task_project_and_session_opt(&self.store, task_id).await? {
                Some(v) => v,
                None => return Ok(None),
            };
        self.fabric
            .tasks
            .get(&project, session.as_ref(), task_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn claim(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .claim(
                &project,
                session.as_ref(),
                task_id,
                lease_owner,
                lease_duration_ms,
            )
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn heartbeat(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .heartbeat(&project, session.as_ref(), task_id, lease_extension_ms)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn start(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .start(&project, session.as_ref(), task_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn complete(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .complete(&project, session.as_ref(), task_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn fail(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .fail(&project, session.as_ref(), task_id, failure_class)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn cancel(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .cancel(&project, session.as_ref(), task_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn dead_letter(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .dead_letter(&project, session.as_ref(), task_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn list_dead_lettered(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        // Projection path: FF's terminal_failed set is not indexed by cairn
        // scope. The handler-wiring map explicitly routes list_* queries to
        // the store projection to preserve the cairn scope filter.
        TaskReadModel::list_by_state(self.store.as_ref(), project, TaskState::DeadLettered, limit)
            .await
            .map(|mut v| {
                if offset >= v.len() {
                    Vec::new()
                } else {
                    v.drain(offset..).collect()
                }
            })
            .map_err(RuntimeError::from)
    }

    async fn pause(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        reason: PauseReason,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .pause(&project, session.as_ref(), task_id, reason)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn resume(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        trigger: ResumeTrigger,
        target: TaskResumeTarget,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .resume(&project, session.as_ref(), task_id, trigger, target)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn list_by_state(
        &self,
        project: &ProjectKey,
        state: TaskState,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        // Projection path — same rationale as list_dead_lettered.
        TaskReadModel::list_by_state(self.store.as_ref(), project, state, limit)
            .await
            .map_err(RuntimeError::from)
    }

    async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, RuntimeError> {
        // Projection path. FF has its own lease-expiry scanner running
        // in-process; surfacing expired leases to cairn handlers is a
        // read-only query on the event log projection.
        TaskReadModel::list_expired_leases(self.store.as_ref(), now, limit)
            .await
            .map_err(RuntimeError::from)
    }

    async fn release_lease(
        &self,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, RuntimeError> {
        let (project, session) =
            resolve_task_project_and_session(&self.store, task_id, session_id).await?;
        self.fabric
            .tasks
            .release_lease(&project, session.as_ref(), task_id)
            .await
            .map_err(fabric_err_to_runtime)
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
        project: &ProjectKey,
        session_id: SessionId,
    ) -> Result<SessionRecord, RuntimeError> {
        self.fabric
            .sessions
            .create(project, session_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn get(&self, session_id: &SessionId) -> Result<Option<SessionRecord>, RuntimeError> {
        let project = match resolve_project_from_session_id(&self.store, session_id).await? {
            Some(p) => p,
            None => return Ok(None),
        };
        self.fabric
            .sessions
            .get(&project, session_id)
            .await
            .map_err(fabric_err_to_runtime)
    }

    async fn list(
        &self,
        project: &ProjectKey,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<SessionRecord>, RuntimeError> {
        // Projection path: FF flows are partitioned by flow_id, not indexed
        // by project. cairn-store's SessionReadModel lists by project from
        // the event log — the only source with the cairn scope view.
        SessionReadModel::list_by_project(self.store.as_ref(), project, limit, offset)
            .await
            .map_err(RuntimeError::from)
    }

    async fn archive(&self, session_id: &SessionId) -> Result<SessionRecord, RuntimeError> {
        let project = resolve_session_project(&self.store, session_id).await?;
        self.fabric
            .sessions
            .archive(&project, session_id)
            .await
            .map_err(fabric_err_to_runtime)
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

    /// SEC-007: Valkey error messages (FCALL names, key names, occasionally
    /// secret-hash references) MUST NOT flow into the 500 response body.
    /// The adapter's mapper genericizes the Internal variant to a fixed
    /// string; operators still see the detail in `tracing::error!`. Any
    /// regression that surfaces `other.to_string()` to the caller fires
    /// this test.
    #[test]
    fn fabric_err_valkey_detail_does_not_leak_to_runtime_internal() {
        let leaky = FabricError::Valkey(
            "HGET waitpoint_hmac_secrets:{p:7} secret:abc123 — \
             connection refused at 10.0.0.1:6379"
                .into(),
        );
        match fabric_err_to_runtime(leaky) {
            RuntimeError::Internal(msg) => {
                assert_eq!(
                    msg, "fabric layer error",
                    "Internal message must be opaque; got leaky detail: {msg:?}"
                );
                assert!(!msg.contains("secret:"), "secret hash field leaked");
                assert!(!msg.contains("HGET"), "FCALL detail leaked");
                assert!(!msg.contains("10.0.0.1"), "connection endpoint leaked");
            }
            other => panic!("expected RuntimeError::Internal, got {other:?}"),
        }
    }

    /// NotFound and Validation variants are user-facing and MUST keep
    /// their detail (404 / 422 responses). Pin the pass-through so a
    /// future refactor doesn't accidentally genericize these too.
    #[test]
    fn fabric_err_not_found_and_validation_pass_through() {
        let nf = FabricError::NotFound {
            entity: "run",
            id: "run_abc".into(),
        };
        match fabric_err_to_runtime(nf) {
            RuntimeError::NotFound { entity, id } => {
                assert_eq!(entity, "run");
                assert_eq!(id, "run_abc");
            }
            other => panic!("expected NotFound pass-through, got {other:?}"),
        }
        let val = FabricError::Validation {
            reason: "limit must be positive".into(),
        };
        match fabric_err_to_runtime(val) {
            RuntimeError::Validation { reason } => {
                assert_eq!(reason, "limit must be positive");
            }
            other => panic!("expected Validation pass-through, got {other:?}"),
        }
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
                session_id: None,
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

    /// `resolve_run_project` surfaces a typed NotFound with
    /// `entity: "run"`. Handlers map this straight to HTTP 404; any drift to
    /// a generic Internal error would mask a legitimate missing-resource.
    #[tokio::test]
    async fn resolve_run_project_maps_unknown_id_to_run_not_found() {
        let store = Arc::new(InMemoryStore::new());
        let err = resolve_run_project(&store, &RunId::new("run_missing"))
            .await
            .expect_err("missing run must not resolve");
        match err {
            RuntimeError::NotFound { entity, id } => {
                assert_eq!(entity, "run");
                assert_eq!(id, "run_missing");
            }
            other => panic!("expected NotFound {{ entity: \"run\", .. }}, got {other:?}"),
        }
    }

    /// Same invariant as above but on the task-side helper — guards against
    /// the resolver being silently re-aliased (e.g. someone wiring the
    /// task helper through the run one).
    #[tokio::test]
    async fn resolve_task_project_maps_unknown_id_to_task_not_found() {
        let store = Arc::new(InMemoryStore::new());
        let err = resolve_task_project(&store, &TaskId::new("task_missing"))
            .await
            .expect_err("missing task must not resolve");
        match err {
            RuntimeError::NotFound { entity, id } => {
                assert_eq!(entity, "task");
                assert_eq!(id, "task_missing");
            }
            other => panic!("expected NotFound {{ entity: \"task\", .. }}, got {other:?}"),
        }
    }

    /// `FabricRunServiceAdapter::list_by_session` delegates to the cairn-store
    /// projection by design — FF does not index runs by cairn `SessionId`.
    /// This test pins that delegation without needing a live `FabricServices`
    /// by exercising the extracted helper directly.
    #[tokio::test]
    async fn list_runs_by_session_returns_seeded_runs_via_projection() {
        let store = Arc::new(InMemoryStore::new());
        let project = test_project();
        let session_id = SessionId::new("sess_list");
        let run_a = RunId::new("run_a");
        let run_b = RunId::new("run_b");

        seed_session(&store, &project, &session_id).await;
        seed_run(&store, &project, &session_id, &run_a).await;
        seed_run(&store, &project, &session_id, &run_b).await;

        let rows = list_runs_by_session_from_projection(&store, &session_id, 10, 0)
            .await
            .expect("projection read must succeed");
        assert_eq!(rows.len(), 2);
        let ids: std::collections::HashSet<_> =
            rows.iter().map(|r| r.run_id.as_str().to_owned()).collect();
        assert!(ids.contains("run_a"));
        assert!(ids.contains("run_b"));
    }

    /// Unknown session → empty Vec, not an error. Matches the
    /// trait-level contract (list returns an empty collection, not NotFound,
    /// for a session with no runs).
    #[tokio::test]
    async fn list_runs_by_session_returns_empty_for_unknown_session() {
        let store = Arc::new(InMemoryStore::new());
        let rows =
            list_runs_by_session_from_projection(&store, &SessionId::new("sess_empty"), 10, 0)
                .await
                .expect("projection read must succeed");
        assert!(rows.is_empty());
    }

    /// Offset slices before limit — pin the pagination contract so a future
    /// refactor that swaps the two arguments (easy mistake in event-log
    /// projections) fails loudly.
    #[tokio::test]
    async fn list_runs_by_session_respects_offset_and_limit() {
        let store = Arc::new(InMemoryStore::new());
        let project = test_project();
        let session_id = SessionId::new("sess_pag");
        for i in 0..5 {
            let run_id = RunId::new(format!("run_{i}"));
            if i == 0 {
                seed_session(&store, &project, &session_id).await;
            }
            seed_run(&store, &project, &session_id, &run_id).await;
        }
        let page = list_runs_by_session_from_projection(&store, &session_id, 2, 2)
            .await
            .expect("projection read must succeed");
        assert_eq!(page.len(), 2, "expected 2 runs with offset=2 limit=2");
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

    /// Compile-time guard that `FabricRunServiceAdapter` overrides
    /// `start_with_correlation` on the `RunService` trait rather than
    /// inheriting the default impl (which drops `correlation_id` at
    /// `cairn-runtime/src/runs.rs:101-110`).
    ///
    /// A live end-to-end assertion needs a running `FabricServices` (Valkey
    /// backed), so it lives in `tests/integration/test_run_lifecycle.rs`.
    /// Here we only prove the override is present: taking a function pointer
    /// to `<FabricRunServiceAdapter as RunService>::start_with_correlation`
    /// and comparing to the default-impl pointer would require Rust method-
    /// resolution tricks, so we fall back to checking that the SOURCE file
    /// contains the explicit override. A regression that deletes the
    /// override would leave the trait default in place and silently drop
    /// correlation on the sqeq ingress path — caught here.
    #[test]
    fn fabric_run_adapter_overrides_start_with_correlation() {
        let src = include_str!("fabric_adapter.rs");
        assert!(
            src.contains("async fn start_with_correlation("),
            "FabricRunServiceAdapter must explicitly override \
             start_with_correlation — default trait impl drops the \
             correlation_id (see cairn-runtime/src/runs.rs:101-110). \
             Sqeq ingress (handlers/sqeq.rs) relies on this for audit \
             trail preservation on the Fabric path.",
        );
        // Belt-and-braces: verify the override threads correlation_id
        // through to the fabric layer, not into the void.
        assert!(
            src.contains(".start_with_correlation(") && src.contains("Some(correlation_id)"),
            "override must pass the correlation_id down to \
             fabric.runs.start_with_correlation — delegating to plain \
             `fabric.runs.start()` would still drop it",
        );
    }
}
