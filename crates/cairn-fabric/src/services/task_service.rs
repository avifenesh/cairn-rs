use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::TaskRecord;

use crate::engine::{
    AddExecutionToFlowInput, ApplyDependencyToChildInput, CancelRunInput, CompleteRunInput,
    ControlPlaneBackend, EligibilityResult, Engine, ExecutionLeaseContext, ExecutionSnapshot,
    FailExecutionOutcome, FailRunInput, RenewLeaseInput, ResumeRunInput, StageDependencyEdgeInput,
    StageDependencyOutcome, SubmitTaskInput, SuspendRunInput,
};
use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::{parse_public_state, try_parse_project_key};
use flowfabric::core::types::{ExecutionId, LaneId};

use crate::boot::FabricRuntime;
use crate::id_map;
use crate::state_map;

pub struct FabricTaskService {
    runtime: Arc<FabricRuntime>,
    bridge: Arc<EventBridge>,
    engine: Arc<dyn Engine>,
    control_plane: Arc<dyn ControlPlaneBackend>,
}

impl FabricTaskService {
    pub fn new(
        runtime: Arc<FabricRuntime>,
        bridge: Arc<EventBridge>,
        engine: Arc<dyn Engine>,
        control_plane: Arc<dyn ControlPlaneBackend>,
    ) -> Self {
        Self {
            runtime,
            bridge,
            engine,
            control_plane,
        }
    }

    /// Mint the `ExecutionId` for a task.
    ///
    /// When `session_id` is `Some`, routes through
    /// `id_map::session_task_to_execution_id` so the task co-locates on
    /// the session's FlowId partition with every other run/task in the
    /// same session. When `None` (bare task submission, no parent
    /// session), falls back to `id_map::task_to_execution_id` (solo
    /// routing via the project's LaneId). The two paths mint
    /// byte-distinct IDs even for the same `(project, task_id)` — once
    /// chosen at submit time, every downstream operation on that task
    /// MUST pass the same `session_id` or the lookup misses FF's state.
    fn task_to_execution_id(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> ExecutionId {
        match session_id {
            Some(sid) => id_map::session_task_to_execution_id(
                project,
                sid,
                task_id,
                &self.runtime.partition_config,
            ),
            None => id_map::task_to_execution_id(project, task_id, &self.runtime.partition_config),
        }
    }

    /// Load the execution snapshot for a task.
    async fn load_snapshot(
        &self,
        task_id: &TaskId,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
    ) -> Result<ExecutionSnapshot, FabricError> {
        let eid = self.task_to_execution_id(project, session_id, task_id);
        self.engine
            .describe_execution(&eid)
            .await?
            .ok_or_else(|| FabricError::NotFound {
                entity: "task",
                id: task_id.to_string(),
            })
    }

    /// Build the lease context required by lifecycle FCALLs from a
    /// pre-read snapshot.
    ///
    /// Enforces the same fence-triple invariant as
    /// `FabricRunService::resolve_lease_context` (RFC #58.5): either all
    /// three fence tokens are populated from a live lease + current
    /// attempt, or all three are cleared and `source` is set to
    /// `"operator_override"` (unfenced authoritative-writer mode, used
    /// by the cancel-while-unclaimed path). A partial triple would
    /// surface as FF's opaque `partial_fence_triple` rejection (F37).
    fn resolve_lease_context(&self, snapshot: &ExecutionSnapshot) -> ExecutionLeaseContext {
        let lane_id = if snapshot.lane_id.as_str().is_empty() {
            LaneId::new("cairn")
        } else {
            snapshot.lane_id.clone()
        };
        let attempt_index = snapshot
            .current_attempt
            .as_ref()
            .map(|a| a.index)
            .unwrap_or_else(|| flowfabric::core::types::AttemptIndex::new(0));

        match (&snapshot.current_lease, snapshot.current_attempt.as_ref()) {
            (Some(l), Some(att)) => ExecutionLeaseContext {
                lane_id,
                attempt_index,
                lease_id: l.lease_id.to_string(),
                lease_epoch: l.lease_epoch.0.to_string(),
                attempt_id: att.id.to_string(),
                worker_instance_id: l.worker_instance_id.clone(),
                source: String::new(),
            },
            // Any other shape (no lease, or lease without attempt) → use
            // the unfenced path. FF still validates lifecycle phase via
            // `validate_lease_and_mark_expired`.
            _ => ExecutionLeaseContext::unfenced(lane_id, attempt_index),
        }
    }

    /// Require an active lease. Heartbeat + complete + fail require a
    /// live lease — an absent one surfaces as
    /// `FabricError::NotFound { entity: "task_lease", ... }`.
    fn require_active_lease(
        &self,
        snapshot: &ExecutionSnapshot,
        task_id: &TaskId,
    ) -> Result<ExecutionLeaseContext, FabricError> {
        if snapshot.current_lease.is_none() {
            return Err(FabricError::NotFound {
                entity: "task_lease",
                id: task_id.to_string(),
            });
        }
        Ok(self.resolve_lease_context(snapshot))
    }

    fn build_task_record(
        &self,
        snapshot: &ExecutionSnapshot,
        task_id: &TaskId,
        project: &ProjectKey,
    ) -> TaskRecord {
        let public_state = parse_public_state(&snapshot.public_state);
        let (task_state, failure_class) = state_map::ff_public_state_to_task_state(public_state);
        let task_state = state_map::adjust_task_state_for_blocking_reason(
            task_state,
            snapshot.blocking_reason.as_deref().unwrap_or_default(),
        );

        let parent_run_id_str = snapshot.tags.get("cairn.parent_run_id").cloned();
        let parent_task_id_str = snapshot.tags.get("cairn.parent_task_id").cloned();
        let session_id_str = snapshot.tags.get("cairn.session_id").cloned();
        let tag_project = match snapshot
            .tags
            .get("cairn.project")
            .and_then(|s| try_parse_project_key(s))
        {
            Some(tp) => {
                if tp != *project {
                    tracing::warn!(
                        task_id = %task_id,
                        caller = %format!("{}/{}/{}", project.tenant_id, project.workspace_id, project.project_id),
                        tag = %format!("{}/{}/{}", tp.tenant_id, tp.workspace_id, tp.project_id),
                        "task tag project does not match caller project"
                    );
                }
                tp
            }
            None => project.clone(),
        };

        let (lease_owner, lease_expires_at) = match snapshot.current_lease.as_ref() {
            Some(l) => (
                Some(l.worker_instance_id.as_str().to_owned()).filter(|s| !s.is_empty()),
                Some(l.expires_at.0 as u64).filter(|&v| v > 0),
            ),
            None => (None, None),
        };

        TaskRecord {
            task_id: task_id.clone(),
            project: tag_project,
            parent_run_id: parent_run_id_str.filter(|s| !s.is_empty()).map(RunId::new),
            parent_task_id: parent_task_id_str
                .filter(|s| !s.is_empty())
                .map(TaskId::new),
            session_id: session_id_str.filter(|s| !s.is_empty()).map(SessionId::new),
            state: task_state,
            prompt_release_id: None,
            failure_class,
            pause_reason: None,
            resume_trigger: None,
            retry_count: snapshot.total_attempt_count,
            lease_owner,
            lease_expires_at,
            title: None,
            description: None,
            version: snapshot.current_lease_epoch.map(|e| e.0).unwrap_or(1),
            created_at: snapshot.created_at.0 as u64,
            updated_at: snapshot.last_mutation_at.0 as u64,
        }
    }

    async fn read_task_record(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        Ok(self.build_task_record(&snapshot, task_id, project))
    }
}

impl FabricTaskService {
    pub async fn submit(
        &self,
        project: &ProjectKey,
        task_id: TaskId,
        parent_run_id: Option<RunId>,
        parent_task_id: Option<TaskId>,
        priority: u32,
        session_id: Option<&SessionId>,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, session_id, &task_id);
        let lane_id = id_map::project_to_lane(project);
        let namespace = id_map::tenant_to_namespace(&project.tenant_id);

        let mut tags = HashMap::new();
        tags.insert("cairn.task_id".to_owned(), task_id.as_str().to_owned());
        tags.insert(
            "cairn.project".to_owned(),
            format!(
                "{}/{}/{}",
                project.tenant_id, project.workspace_id, project.project_id
            ),
        );
        // Cross-instance isolation: LeaseHistorySubscriber filters
        // every frame by `cairn.instance_id` so a cairn-app sharing a
        // Valkey with another cairn-app (or another tenant's FF
        // consumer) only consumes its own lease-expiry / reclaim
        // frames. Missing this tag on create would make the execution
        // invisible to its owner's subscriber after a lease expiry.
        tags.insert(
            "cairn.instance_id".to_owned(),
            self.runtime.config.worker_instance_id.to_string(),
        );
        if let Some(sid) = session_id {
            tags.insert("cairn.session_id".to_owned(), sid.as_str().to_owned());
        }
        if let Some(ref run_id) = parent_run_id {
            tags.insert("cairn.parent_run_id".to_owned(), run_id.as_str().to_owned());
        }
        if let Some(ref ptid) = parent_task_id {
            tags.insert("cairn.parent_task_id".to_owned(), ptid.as_str().to_owned());
        }

        let outcome = self
            .control_plane
            .submit_task_execution(SubmitTaskInput {
                execution_id: eid.clone(),
                namespace: namespace.clone(),
                lane_id,
                priority,
                tags,
                // Empty means the impl applies the historical default
                // policy (max_retries=2, exponential backoff).
                policy_json: String::new(),
            })
            .await?;

        if outcome.newly_created {
            self.bridge
                .emit(BridgeEvent::TaskCreated {
                    task_id: task_id.clone(),
                    project: project.clone(),
                    session_id: session_id.cloned(),
                    parent_run_id: parent_run_id.clone(),
                    parent_task_id: parent_task_id.clone(),
                })
                .await;
        }

        // Session-bound tasks join their session's flow so future
        // `declare_dependency` calls can reference them as edge
        // endpoints. Both FCALLs (`ff_create_flow` + `ff_add_execution_to_flow`)
        // are idempotent, so retry-safe.
        if let Some(sid) = session_id {
            let fid = id_map::session_to_flow_id(project, sid);
            self.control_plane
                .add_execution_to_flow(AddExecutionToFlowInput {
                    flow_id: fid,
                    execution_id: eid,
                    namespace,
                    flow_kind: "cairn_session".to_owned(),
                })
                .await?;
        }

        self.read_task_record(project, session_id, &task_id).await
    }

    /// Declare that `dependent_task_id` must not start until
    /// `prerequisite_task_id` completes. Both tasks must live in the
    /// same session — FF flow edges cannot cross flows.
    ///
    /// Idempotency: replaying the same `(project, session, dependent,
    /// prerequisite)` with identical `dependency_kind` and
    /// `data_passing_ref` returns the existing record. Replaying with
    /// a different kind or ref returns
    /// `FabricError::DependencyConflict` (HTTP 409).
    pub async fn declare_dependency(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        dependent_task_id: &TaskId,
        prerequisite_task_id: &TaskId,
        dependency_kind: DependencyKind,
        data_passing_ref: Option<&str>,
    ) -> Result<TaskDependencyRecord, FabricError> {
        // Self-dependency: reject client-side. FF's Lua also rejects
        // with `self_referencing_edge`, but a clearer message here
        // saves a round-trip.
        if dependent_task_id == prerequisite_task_id {
            return Err(FabricError::Validation {
                reason: "task cannot depend on itself".to_owned(),
            });
        }

        let fid = id_map::session_to_flow_id(project, session_id);
        let dep_eid = self.task_to_execution_id(project, Some(session_id), dependent_task_id);
        let pre_eid = self.task_to_execution_id(project, Some(session_id), prerequisite_task_id);
        let lane_id = id_map::project_to_lane(project);

        // Deterministic edge ID: replay of the same (flow, upstream,
        // downstream) triple mints the same ID, so the
        // `AlreadyExists` branch below can describe_edge + compare.
        let edge_id = id_map::dependency_edge_id(&fid, &pre_eid, &dep_eid);
        let kind_ff = dependency_kind.as_ff_str().to_owned();
        let data_ref_ff = data_passing_ref.unwrap_or("").to_owned();

        // Safe-to-log observability (SEC-007).
        let data_ref_len = data_ref_ff.len();
        let data_ref_prefix: String = data_ref_ff.chars().take(16).collect();
        tracing::debug!(
            flow_id = %fid,
            edge_id = %edge_id,
            dependent_task_id = %dependent_task_id,
            prerequisite_task_id = %prerequisite_task_id,
            dependency_kind = %kind_ff,
            data_passing_ref.len = data_ref_len,
            data_passing_ref.prefix = %data_ref_prefix,
            "declare_dependency staging edge",
        );

        // Retry budget for stale_graph_revision. Concurrent declarers
        // on the same session's flow all compete for the HINCRBY;
        // exponential backoff converges quickly. Five attempts at
        // 10/20/40/80/160ms preserves the pre-migration observable
        // behaviour — every number identical to the previous loop.
        const RETRY_DELAYS_MS: &[u64] = &[10, 20, 40, 80, 160];
        let new_graph_revision: u64;
        let mut attempts = 0usize;
        loop {
            attempts += 1;
            // Read the current graph_revision via the engine's flow
            // snapshot. New flows have 0.
            let current_rev = match self.engine.describe_flow(&fid).await? {
                Some(snap) => snap.graph_revision,
                None => 0,
            };

            let outcome = self
                .control_plane
                .stage_dependency_edge(StageDependencyEdgeInput {
                    flow_id: fid.clone(),
                    edge_id: edge_id.clone(),
                    upstream_execution_id: pre_eid.clone(),
                    downstream_execution_id: dep_eid.clone(),
                    dependency_kind: kind_ff.clone(),
                    data_passing_ref: data_ref_ff.clone(),
                    expected_graph_revision: current_rev,
                })
                .await?;

            match outcome {
                StageDependencyOutcome::Staged {
                    new_graph_revision: rev,
                } => {
                    new_graph_revision = rev;
                    break;
                }
                StageDependencyOutcome::StaleGraphRevision => {
                    if attempts <= RETRY_DELAYS_MS.len() {
                        tokio::time::sleep(std::time::Duration::from_millis(
                            RETRY_DELAYS_MS[attempts - 1],
                        ))
                        .await;
                        continue;
                    }
                    return Err(FabricError::Validation {
                        reason: format!(
                            "stage_dependency_edge: graph_revision kept racing \
                             after {attempts} attempts — concurrent declarers \
                             overwhelming the flow"
                        ),
                    });
                }
                StageDependencyOutcome::Cycle => {
                    return Err(FabricError::Validation {
                        reason: "dependency introduces cycle".to_owned(),
                    });
                }
                StageDependencyOutcome::SelfReferencing => {
                    // Already guarded client-side; this would be a logic bug.
                    return Err(FabricError::Validation {
                        reason: "task cannot depend on itself".to_owned(),
                    });
                }
                StageDependencyOutcome::AlreadyExists => {
                    return self
                        .reconcile_existing_dependency_edge(
                            &fid,
                            &edge_id,
                            project,
                            dependent_task_id,
                            prerequisite_task_id,
                            dependency_kind,
                            data_passing_ref,
                        )
                        .await;
                }
                StageDependencyOutcome::FlowNotFound => {
                    return Err(FabricError::NotFound {
                        entity: "flow",
                        id: fid.to_string(),
                    });
                }
                StageDependencyOutcome::FlowAlreadyTerminal => {
                    return Err(FabricError::Validation {
                        reason: format!(
                            "session's flow {fid} is already terminal; \
                             cannot add new dependency edges"
                        ),
                    });
                }
                StageDependencyOutcome::ExecutionNotInFlow => {
                    return Err(FabricError::NotFound {
                        entity: "task",
                        id: "(one of the endpoints is not a flow member)".to_owned(),
                    });
                }
            }
        }

        // 2. Apply the edge on the child's execution partition.
        self.control_plane
            .apply_dependency_to_child(ApplyDependencyToChildInput {
                downstream_execution_id: dep_eid,
                flow_id: fid.clone(),
                upstream_execution_id: pre_eid,
                edge_id: edge_id.clone(),
                lane_id,
                graph_revision: new_graph_revision,
                dependency_kind: kind_ff,
                data_passing_ref: data_ref_ff,
            })
            .await?;

        let now_ms = crate::helpers::now_ms();

        // 3. Append audit record.
        self.bridge
            .emit(BridgeEvent::TaskDependencyAdded {
                dependent_task_id: dependent_task_id.clone(),
                prerequisite_task_id: prerequisite_task_id.clone(),
                project: project.clone(),
                edge_id: edge_id.to_string(),
                flow_id: fid.to_string(),
                created_at_ms: now_ms,
                dependency_kind,
                data_passing_ref: data_passing_ref.map(str::to_owned),
            })
            .await;

        Ok(TaskDependencyRecord {
            dependency: TaskDependency {
                dependent_task_id: dependent_task_id.clone(),
                depends_on_task_id: prerequisite_task_id.clone(),
                project: project.clone(),
                created_at_ms: now_ms,
                dependency_kind,
                data_passing_ref: data_passing_ref.map(str::to_owned),
            },
            resolved_at_ms: None,
        })
    }

    /// Reconcile a replay hit on `AlreadyExists` via the engine's
    /// [`describe_edge`](crate::engine::Engine::describe_edge)
    /// primitive. Re-declare with identical values is idempotent
    /// success; re-declare with a different `dependency_kind` or
    /// `data_passing_ref` surfaces as a `DependencyConflict` so the
    /// HTTP caller sees 409 instead of silently losing their update.
    #[allow(clippy::too_many_arguments)]
    async fn reconcile_existing_dependency_edge(
        &self,
        flow_id: &flowfabric::core::types::FlowId,
        edge_id: &flowfabric::core::types::EdgeId,
        project: &ProjectKey,
        dependent_task_id: &TaskId,
        prerequisite_task_id: &TaskId,
        requested_kind: DependencyKind,
        requested_data_ref: Option<&str>,
    ) -> Result<TaskDependencyRecord, FabricError> {
        let existing = match self.engine.describe_edge(flow_id, edge_id).await? {
            Some(edge) => edge,
            None => {
                // FCALL returned AlreadyExists but edge hash now
                // empty — a TOCTOU race. Extremely unlikely (edges
                // aren't concurrently deleted); surface cleanly.
                return Err(FabricError::Internal(
                    "dependency_already_exists but edge hash missing on describe_edge".into(),
                ));
            }
        };

        let requested_data_ref_opt: Option<String> = requested_data_ref
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let requested_kind_str = requested_kind.as_ff_str();

        if existing.kind.as_str() != requested_kind_str
            || existing.data_passing_ref != requested_data_ref_opt
        {
            return Err(FabricError::DependencyConflict(Box::new(
                crate::error::DependencyConflictDetail {
                    dependent_task_id: dependent_task_id.to_string(),
                    prerequisite_task_id: prerequisite_task_id.to_string(),
                    existing_kind: existing.kind,
                    existing_data_passing_ref: existing.data_passing_ref,
                    requested_kind: requested_kind_str.to_owned(),
                    requested_data_passing_ref: requested_data_ref_opt,
                },
            )));
        }

        Ok(TaskDependencyRecord {
            dependency: TaskDependency {
                dependent_task_id: dependent_task_id.clone(),
                depends_on_task_id: prerequisite_task_id.clone(),
                project: project.clone(),
                created_at_ms: existing.created_at.0 as u64,
                dependency_kind: requested_kind,
                data_passing_ref: requested_data_ref_opt,
            },
            resolved_at_ms: None,
        })
    }

    /// Return the list of currently-blocking upstream task_ids for
    /// `task_id`. Empty means either (a) the task is eligible /
    /// running / terminal, or (b) its prereqs are all satisfied.
    pub async fn check_dependencies(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, FabricError> {
        let eid = self.task_to_execution_id(project, Some(session_id), task_id);

        let eligibility = self.control_plane.evaluate_flow_eligibility(&eid).await?;

        if eligibility != EligibilityResult::BlockedByDependencies {
            return Ok(Vec::new());
        }

        // Enumerate incoming edges via the engine primitive.
        let incoming = self.engine.list_incoming_edges(&eid).await?;

        let mut blockers = Vec::with_capacity(incoming.len());
        for edge in incoming {
            if edge.state != crate::engine::EdgeState::Unsatisfied {
                continue;
            }
            let dependency_kind = match edge.kind.as_str() {
                "success_only" | "" => DependencyKind::SuccessOnly,
                other => {
                    return Err(FabricError::Internal(format!(
                        "unknown dependency_kind on stored edge: {other}"
                    )));
                }
            };
            // One HGET per upstream — avoids the 2N HGETALL
            // amplification that a full `describe_execution` would
            // cause in this per-blocker loop.
            let upstream_task_id = match self
                .engine
                .get_execution_tag(&edge.upstream_execution_id, "cairn.task_id")
                .await?
            {
                Some(s) => TaskId::new(s),
                None => continue,
            };
            blockers.push(TaskDependencyRecord {
                dependency: TaskDependency {
                    dependent_task_id: task_id.clone(),
                    depends_on_task_id: upstream_task_id,
                    project: project.clone(),
                    created_at_ms: edge.created_at.0 as u64,
                    dependency_kind,
                    data_passing_ref: edge.data_passing_ref,
                },
                resolved_at_ms: None,
            });
        }

        Ok(blockers)
    }

    pub async fn get(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<Option<TaskRecord>, FabricError> {
        match self.read_task_record(project, session_id, task_id).await {
            Ok(record) => Ok(Some(record)),
            Err(FabricError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn claim(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        _lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        let lane_id = if snapshot.lane_id.as_str().is_empty() {
            LaneId::new("cairn")
        } else {
            snapshot.lane_id.clone()
        };
        let eid = snapshot.execution_id.clone();

        // Shared grant+claim FCALL sequence (see services/claim_common.rs).
        // Cancel-safety: a drop between the two calls leaves only a grant,
        // which FF expires via its grant TTL.
        //
        // The returned ClaimOutcome is intentionally dropped — cairn does
        // NOT cache the lease triple. Every downstream terminal op re-reads
        // current_lease_id / _epoch / _attempt_index from FF's exec_core.
        crate::services::claim_common::issue_grant_and_claim(
            &self.control_plane,
            &eid,
            &lane_id,
            lease_duration_ms,
        )
        .await?;

        let record = self.read_task_record(project, session_id, task_id).await?;
        self.bridge
            .emit(BridgeEvent::TaskLeaseClaimed {
                task_id: task_id.clone(),
                project: record.project.clone(),
                lease_owner: self.runtime.config.worker_instance_id.to_string(),
                lease_epoch: record.version,
                lease_expires_at_ms: record.lease_expires_at.unwrap_or(0),
            })
            .await;
        Ok(record)
    }

    /// Renew an active lease on a task by `lease_extension_ms`.
    ///
    /// **Lean-bridge silence (intentional).** Does not emit a `BridgeEvent`.
    /// Lease renewal extends `lease_expires_at_ms` on FF's exec_core; the
    /// `TaskLeaseClaimed` variant in `BridgeEvent` represents a fresh claim,
    /// not a renewal. Emitting on every heartbeat would also saturate the
    /// bridge (heartbeat cadence is sub-lease-TTL, typically 5-10s).
    ///
    /// See `docs/design/bridge-event-audit.md` §2.2.
    pub async fn heartbeat(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        let lease = self.require_active_lease(&snapshot, task_id)?;
        let eid = snapshot.execution_id.clone();

        self.control_plane
            .renew_task_lease(RenewLeaseInput {
                execution_id: eid,
                lease,
                lease_extension_ms,
            })
            .await?;

        self.read_task_record(project, session_id, task_id).await
    }

    pub async fn start(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        // FF transitions to active on claim — no separate start step needed.
        self.read_task_record(project, session_id, task_id).await
    }

    pub async fn complete(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        let lease = self.require_active_lease(&snapshot, task_id)?;
        let eid = snapshot.execution_id.clone();

        self.control_plane
            .complete_run_execution(CompleteRunInput {
                execution_id: eid,
                lease,
            })
            .await?;

        // FF confirmed the terminal transition; emit unconditionally.
        // Projections are idempotent on (task_id, event_id), so a redundant
        // emit from a parallel path is a no-op re-write.
        let record = self.read_task_record(project, session_id, task_id).await?;
        self.bridge
            .emit(BridgeEvent::TaskStateChanged {
                task_id: task_id.clone(),
                project: record.project.clone(),
                to: TaskState::Completed,
                failure_class: None,
            })
            .await;
        Ok(record)
    }

    pub async fn fail(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        let lease = self.require_active_lease(&snapshot, task_id)?;
        let eid = snapshot.execution_id.clone();

        let category = state_map::failure_class_category(failure_class);
        let reason = state_map::failure_class_reason(failure_class);

        // Empty `retry_policy_json` signals the backend reads FF's
        // `exec_policy` GET key itself. Matches FabricRunService::fail.
        let outcome = self
            .control_plane
            .fail_run_execution(FailRunInput {
                execution_id: eid,
                lease,
                reason: reason.to_owned(),
                category: category.to_owned(),
                retry_policy_json: String::new(),
            })
            .await?;

        // Emit only on terminal fail; a retry-scheduled fail leaves the task
        // in a non-terminal state (FF will promote it back to eligible later)
        // so the projection should not see a `Failed` transition yet.
        let record = self.read_task_record(project, session_id, task_id).await?;
        if outcome == FailExecutionOutcome::TerminalFailed {
            self.bridge
                .emit(BridgeEvent::TaskStateChanged {
                    task_id: task_id.clone(),
                    project: record.project.clone(),
                    to: TaskState::Failed,
                    failure_class: Some(failure_class),
                })
                .await;
        }
        Ok(record)
    }

    pub async fn cancel(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        // Cancel tolerates a missing active lease — fall back to the
        // resolve_lease_context defaults (nil lease_id, epoch from
        // current_lease_epoch or 1). Services cancel operator-held
        // tasks that may have never been claimed.
        let lease = self.resolve_lease_context(&snapshot);
        let eid = snapshot.execution_id.clone();
        let current_waitpoint = snapshot.current_waitpoint.clone();

        self.control_plane
            .cancel_run_execution(CancelRunInput {
                execution_id: eid,
                lease,
                current_waitpoint,
            })
            .await?;

        // Emit unconditionally; see complete() for rationale.
        let record = self.read_task_record(project, session_id, task_id).await?;
        self.bridge
            .emit(BridgeEvent::TaskStateChanged {
                task_id: task_id.clone(),
                project: record.project.clone(),
                to: TaskState::Canceled,
                failure_class: None,
            })
            .await;
        Ok(record)
    }

    pub async fn dead_letter(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        self.read_task_record(project, session_id, task_id).await
    }

    pub async fn list_dead_lettered(
        &self,
        _project: &ProjectKey,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<TaskRecord>, FabricError> {
        // FF terminal_failed IS dead letter — list via cairn event log projection.
        Ok(Vec::new())
    }

    pub async fn pause(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        reason: PauseReason,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        let lease = self.resolve_lease_context(&snapshot);
        let eid = snapshot.execution_id.clone();

        let params = match reason.kind {
            PauseReasonKind::OperatorPause => crate::suspension::for_operator_hold(),
            PauseReasonKind::ToolRequestedSuspension => {
                let invocation_id = reason
                    .detail
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| FabricError::Validation {
                        reason: "ToolRequestedSuspension requires invocation_id in reason.detail"
                            .to_owned(),
                    })?;
                crate::suspension::for_tool_result(invocation_id, reason.resume_after_ms)
            }
            PauseReasonKind::RuntimeSuspension => {
                let signal_name = reason
                    .detail
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| FabricError::Validation {
                        reason: "RuntimeSuspension requires signal_name in reason.detail"
                            .to_owned(),
                    })?;
                crate::suspension::SuspensionParams {
                    reason_code: "waiting_for_signal".into(),
                    condition_matchers: vec![crate::suspension::ConditionMatcher {
                        signal_name: signal_name.to_owned(),
                    }],
                    timeout_ms: reason.resume_after_ms,
                    timeout_behavior: flowfabric::sdk::task::TimeoutBehavior::Fail,
                }
            }
            PauseReasonKind::PolicyHold => {
                let detail = reason.detail.as_deref().unwrap_or("policy");
                crate::suspension::SuspensionParams {
                    reason_code: "paused_by_policy".into(),
                    condition_matchers: vec![crate::suspension::ConditionMatcher {
                        signal_name: format!("policy_resolved:{detail}"),
                    }],
                    timeout_ms: reason.resume_after_ms,
                    timeout_behavior: flowfabric::sdk::task::TimeoutBehavior::Fail,
                }
            }
        };

        // Task pause: match-mode matches the pre-migration rule —
        // single matcher resumes on ANY signal, multi-matcher
        // requires ALL. Identical to FabricRunService::pause.
        let match_mode = if params.condition_matchers.len() <= 1 {
            "any"
        } else {
            "all"
        };
        // TODO(ff-upstream: https://github.com/avifenesh/FlowFabric/issues/322)
        // Service-layer suspend via Lua-glue; migrates to typed
        // `suspend_by_triple` in CG-c once FF#322 lands.
        let suspend_input = build_suspend_input(eid, lease, &params, match_mode);

        self.control_plane
            .suspend_run_execution(suspend_input)
            .await?;

        // Emit TaskStateChanged so the cairn-store projection + SSE
        // subscribers observe the suspension. `record.state` carries FF's
        // post-commit truth. Emission is unconditional: a
        // `!is_already_satisfied(&raw)` guard would be retry-unsafe
        // (silent permanent drift on crash between FCALL and emit),
        // while projection idempotency on EventId makes the double-emit
        // replay case harmless.
        let record = self.read_task_record(project, session_id, task_id).await?;
        self.bridge
            .emit(BridgeEvent::TaskStateChanged {
                task_id: task_id.clone(),
                project: record.project.clone(),
                to: record.state,
                failure_class: None,
            })
            .await;
        Ok(record)
    }

    pub async fn resume(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        _trigger: ResumeTrigger,
        _target: TaskResumeTarget,
    ) -> Result<TaskRecord, FabricError> {
        let snapshot = self.load_snapshot(task_id, project, session_id).await?;
        let lane_id = if snapshot.lane_id.as_str().is_empty() {
            LaneId::new("cairn")
        } else {
            snapshot.lane_id.clone()
        };
        let waitpoint_id = snapshot.current_waitpoint.clone();
        let eid = snapshot.execution_id.clone();

        self.control_plane
            .resume_run_execution(ResumeRunInput {
                execution_id: eid,
                lane_id,
                waitpoint_id,
                resume_source: "operator".to_owned(),
            })
            .await?;

        // Emit TaskStateChanged so the cairn-store projection + SSE subscribers
        // observe the resume. Symmetric with FabricRunService::resume; the audit
        // at docs/design/bridge-event-audit.md §3.1 filed this as G2.
        //
        // `record.state` can be Queued / Leased / Running depending on how
        // fast the delayed_promoter / scheduler runs between the FCALL and
        // read_task_record (existing test_suspension.rs integration test
        // accepts all three). FF's post-FCALL truth, not a hard-coded value.
        let record = self.read_task_record(project, session_id, task_id).await?;
        self.bridge
            .emit(BridgeEvent::TaskStateChanged {
                task_id: task_id.clone(),
                project: record.project.clone(),
                to: record.state,
                failure_class: None,
            })
            .await;
        Ok(record)
    }

    pub async fn list_by_state(
        &self,
        _project: &ProjectKey,
        _state: TaskState,
        _limit: usize,
    ) -> Result<Vec<TaskRecord>, FabricError> {
        // FF doesn't expose list-by-cairn-state natively. The cairn event log
        // projection serves these queries from the bridge events.
        Ok(Vec::new())
    }

    pub async fn list_expired_leases(
        &self,
        now: u64,
        limit: usize,
    ) -> Result<Vec<TaskRecord>, FabricError> {
        // FF's lease_expiry scanner handles reclaim server-side at
        // ~1.5s intervals; this surface exists so operator UIs can
        // render "which tasks timed out right now?" without waiting
        // for the projection to catch up.
        //
        // Delegates to `Engine::list_expired_leases` (ZRANGEBYSCORE
        // across every execution partition). FF's lease_expiry index
        // is cross-kind (it carries runs + tasks); kind filtering is
        // done by checking the presence of the `cairn.task_id` tag,
        // which is only stamped by task submission. Run executions
        // carry `cairn.run_id` instead and are skipped here.
        let expired = self.engine.list_expired_leases(now, limit).await?;

        let mut out = Vec::with_capacity(expired.len());
        for lease in expired {
            let snapshot = match self.engine.describe_execution(&lease.execution_id).await? {
                Some(s) => s,
                None => continue,
            };
            // Tag filter: only surface tasks (runs don't stamp
            // `cairn.task_id`).
            let Some(task_id_str) = snapshot.tags.get("cairn.task_id").cloned() else {
                continue;
            };
            let Some(project_str) = snapshot.tags.get("cairn.project").cloned() else {
                continue;
            };
            let Some(project) = try_parse_project_key(&project_str) else {
                continue;
            };
            let task_id = TaskId::new(task_id_str);
            out.push(self.build_task_record(&snapshot, &task_id, &project));
        }

        Ok(out)
    }

    pub async fn release_lease(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        // FF does not expose a "release lease without completing"
        // primitive — the lease is dropped implicitly on the next
        // terminal FCALL. Services that call `release_lease` today
        // are observing a stub; re-reading the task record gives
        // operators the current lease state so the surface stays
        // truthful.
        self.read_task_record(project, session_id, task_id).await
    }
}

/// Build the typed `SuspendRunInput` from a suspension params bundle.
///
/// Mirror of `FabricRunService::build_suspend_input` — kept
/// task-service-local (rather than hoisted to a shared module) so the
/// match-mode policy (any vs all) can diverge later if tasks grow a
/// different resume-condition model than runs. Today both services
/// use identical logic; deduplication would hide the policy choice
/// behind a function boundary that callers don't read.
///
/// TODO(ff-upstream: <https://github.com/avifenesh/FlowFabric/issues/322>):
/// This helper exists because `EngineBackend::suspend_by_triple` has
/// not yet landed in FF. All service-layer suspend callers hold a
/// `LeaseFencingTriple` (not a `Handle`), so we cannot use the typed
/// `task.suspend(&Handle, SuspendArgs)` path that worker_sdk uses. CG-c
/// migrates this and all call sites once FF#322 publishes.
fn build_suspend_input(
    eid: ExecutionId,
    lease: ExecutionLeaseContext,
    params: &crate::suspension::SuspensionParams,
    match_mode: &'static str,
) -> SuspendRunInput {
    // FF 0.9 (RFC-013) added `TimeoutBehavior::as_wire_str()`; cairn's
    // Lua FCALL args consume this wire form directly. This also
    // handles the `#[non_exhaustive]` enum contract — a new variant
    // added upstream routes through `as_wire_str` without a cairn
    // code change.
    let timeout_behavior_str = params.timeout_behavior.as_wire_str();

    let required_names: Vec<&str> = params
        .condition_matchers
        .iter()
        .map(|m| m.signal_name.as_str())
        .collect();

    let resume_condition_json = serde_json::json!({
        "condition_type": "signal_set",
        "required_signal_names": required_names,
        "signal_match_mode": match_mode,
        "minimum_signal_count": 1,
        "timeout_behavior": timeout_behavior_str,
        "allow_operator_override": true,
    })
    .to_string();

    let resume_policy_json = serde_json::json!({
        "resume_target": "runnable",
        "close_waitpoint_on_resume": true,
        "consume_matched_signals": true,
        "retain_signal_buffer_until_closed": true,
    })
    .to_string();

    let timeout_at = params
        .timeout_ms
        .map(|ms| {
            let now = flowfabric::core::types::TimestampMs::now().0;
            now.saturating_add(ms as i64).to_string()
        })
        .unwrap_or_default();

    SuspendRunInput {
        execution_id: eid,
        lease,
        reason_code: params.reason_code.clone(),
        timeout_at,
        resume_condition_json,
        resume_policy_json,
        timeout_behavior: timeout_behavior_str.to_owned(),
    }
}
