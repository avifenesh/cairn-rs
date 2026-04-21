use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::TaskRecord;

use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::{
    check_fcall_success, is_duplicate_result, parse_eligibility_result, parse_fail_outcome,
    parse_public_state, parse_stage_result_revision, try_parse_project_key, FailOutcome,
};
use ff_core::keys::{ExecKeyContext, IndexKeys};
use ff_core::partition::execution_partition;
use ff_core::types::{ExecutionId, LaneId};

use crate::boot::FabricRuntime;
use crate::id_map;
use crate::state_map;

pub struct FabricTaskService {
    runtime: Arc<FabricRuntime>,
    bridge: Arc<EventBridge>,
    #[allow(dead_code)] // wired-in for the B-phase ports
    engine: Arc<dyn crate::engine::Engine>,
}

impl FabricTaskService {
    pub fn new(
        runtime: Arc<FabricRuntime>,
        bridge: Arc<EventBridge>,
        engine: Arc<dyn crate::engine::Engine>,
    ) -> Self {
        Self {
            runtime,
            bridge,
            engine,
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

    /// Resolve the active-lease triple (lease_id, epoch, attempt_index)
    /// by reading the execution snapshot. FF owns these authoritatively;
    /// cairn never caches them. Returns `NotFound` on the `"task_lease"`
    /// entity when the execution exists but has no active lease — the
    /// caller path needs the lease_id and cannot proceed without it.
    async fn resolve_active_lease(
        &self,
        task_id: &TaskId,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
    ) -> Result<
        (
            ff_core::types::LeaseId,
            ff_core::types::LeaseEpoch,
            ff_core::types::AttemptIndex,
        ),
        FabricError,
    > {
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "task",
                    id: task_id.to_string(),
                })?;
        let lease = snapshot
            .current_lease
            .ok_or_else(|| FabricError::NotFound {
                entity: "task_lease",
                id: task_id.to_string(),
            })?;
        Ok((lease.lease_id, lease.epoch, lease.attempt_index))
    }

    /// Same as [`Self::resolve_active_lease`] but tolerates an absent
    /// active lease (returns a nil `LeaseId` placeholder). Used by
    /// the cancel-while-unclaimed path which must proceed even
    /// without an active lease to cancel.
    async fn resolve_lease_or_placeholder(
        &self,
        task_id: &TaskId,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
    ) -> Result<
        (
            ff_core::types::LeaseId,
            ff_core::types::LeaseEpoch,
            ff_core::types::AttemptIndex,
        ),
        FabricError,
    > {
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "task",
                    id: task_id.to_string(),
                })?;
        match snapshot.current_lease {
            Some(lease) => Ok((lease.lease_id, lease.epoch, lease.attempt_index)),
            None => Ok((
                ff_core::types::LeaseId::from_uuid(uuid::Uuid::nil()),
                snapshot
                    .current_lease_epoch
                    .unwrap_or_else(|| ff_core::types::LeaseEpoch::new(0)),
                snapshot
                    .current_attempt
                    .map(|a| a.index)
                    .unwrap_or_else(|| ff_core::types::AttemptIndex::new(0)),
            )),
        }
    }

    async fn read_task_record(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "task",
                    id: task_id.to_string(),
                })?;

        let public_state = parse_public_state(&snapshot.public_state);
        let (task_state, failure_class) = state_map::ff_public_state_to_task_state(public_state);
        let task_state = state_map::adjust_task_state_for_blocking_reason(
            task_state,
            snapshot.blocking_reason.as_deref().unwrap_or_default(),
        );

        let parent_run_id_str = snapshot.tags.get("cairn.parent_run_id").cloned();
        let parent_task_id_str = snapshot.tags.get("cairn.parent_task_id").cloned();
        // Session binding written at submit time via the cairn.session_id
        // tag so callers can read the TaskRecord without walking parent_run_id.
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
                Some(l.owner.clone()).filter(|s| !s.is_empty()),
                Some(l.expires_at.0 as u64).filter(|&v| v > 0),
            ),
            None => (None, None),
        };

        Ok(TaskRecord {
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
        })
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
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);
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
        if let Some(sid) = session_id {
            tags.insert("cairn.session_id".to_owned(), sid.as_str().to_owned());
        }
        if let Some(ref run_id) = parent_run_id {
            tags.insert("cairn.parent_run_id".to_owned(), run_id.as_str().to_owned());
        }
        if let Some(ref ptid) = parent_task_id {
            tags.insert("cairn.parent_task_id".to_owned(), ptid.as_str().to_owned());
        }

        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "{}".to_owned());

        let policy_json = serde_json::json!({
            "max_retries": 2,
            "backoff": {
                "type": "exponential",
                "initial_delay_ms": 1000,
                "max_delay_ms": 30000,
                "multiplier": 2
            }
        })
        .to_string();

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.payload(),
            ctx.policy(),
            ctx.tags(),
            idx.lane_eligible(&lane_id),
            ctx.noop(),
            idx.execution_deadline(),
            idx.all_executions(),
        ];
        let args: Vec<String> = vec![
            eid.to_string(),
            namespace.to_string(),
            lane_id.to_string(),
            crate::constants::EXECUTION_KIND_TASK.to_owned(),
            priority.to_string(),
            "cairn".to_owned(),
            policy_json,
            String::new(),
            String::new(),
            "0".to_owned(),
            tags_json,
            String::new(),
            partition.index.to_string(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_CREATE_EXECUTION,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_execution: {e}")))?;

        if !is_duplicate_result(&raw) {
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
        // endpoints. `ff_add_execution_to_flow` is idempotent
        // (returns `ok_already_satisfied` on a duplicate add) so
        // retry-safe. Session-less (bare) tasks never get a flow;
        // declare_dependency rejects them with Validation.
        if let Some(sid) = session_id {
            self.add_execution_to_session_flow(project, sid, &eid)
                .await?;
        }

        self.read_task_record(project, session_id, &task_id).await
    }

    /// Add the execution to its session's flow, creating the flow
    /// first if it doesn't exist. `ff_create_flow` is idempotent
    /// (`ok_already_satisfied` on duplicate), so calling it
    /// defensively costs one extra round-trip on the first task of a
    /// session and zero on subsequent ones (the idempotency guard
    /// short-circuits). The ergonomic benefit: callers don't have to
    /// remember to call `sessions.create` before submitting tasks.
    async fn add_execution_to_session_flow(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        eid: &ExecutionId,
    ) -> Result<(), FabricError> {
        let fid = id_map::session_to_flow_id(project, session_id);
        let flow_partition =
            ff_core::partition::flow_partition(&fid, &self.runtime.partition_config);
        let fctx = ff_core::keys::FlowKeyContext::new(&flow_partition, &fid);
        let flow_idx = ff_core::keys::FlowIndexKeys::new(&flow_partition);
        let exec_partition =
            ff_core::partition::execution_partition(eid, &self.runtime.partition_config);
        let exec_ctx = ff_core::keys::ExecKeyContext::new(&exec_partition, eid);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Ensure the flow exists. Idempotent on the FF side.
        let namespace = id_map::tenant_to_namespace(&project.tenant_id);
        let (create_keys, create_args) = crate::fcall::session::build_create_flow(
            &fctx,
            &flow_partition,
            &fid,
            "cairn_session",
            &namespace,
            ff_core::types::TimestampMs::from_millis(now_ms as i64),
        );
        let create_key_refs: Vec<&str> = create_keys.iter().map(|s| s.as_str()).collect();
        let create_arg_refs: Vec<&str> = create_args.iter().map(|s| s.as_str()).collect();
        let create_raw = self
            .runtime
            .fcall(
                crate::fcall::names::FF_CREATE_FLOW,
                &create_key_refs,
                &create_arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_flow: {e}")))?;
        crate::helpers::check_fcall_success(&create_raw, crate::fcall::names::FF_CREATE_FLOW)?;

        // Add the execution to the flow's members set.
        let (keys, args) = crate::fcall::flow_edges::build_add_execution_to_flow(
            &fctx, &flow_idx, &exec_ctx, &fid, eid, now_ms,
        );
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw = self
            .runtime
            .fcall(
                crate::fcall::names::FF_ADD_EXECUTION_TO_FLOW,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_add_execution_to_flow: {e}")))?;
        crate::helpers::check_fcall_success(&raw, crate::fcall::names::FF_ADD_EXECUTION_TO_FLOW)?;
        Ok(())
    }

    /// Declare that `dependent_task_id` must not start until
    /// `prerequisite_task_id` completes. Both tasks must live in the
    /// same session — FF flow edges cannot cross flows. Caller
    /// supplies the shared `(project, session_id)`; the adapter layer
    /// validates that the two tasks agree before this method is
    /// invoked, so reaching here with mismatched sessions is a bug.
    ///
    /// `dependency_kind` selects the edge kind (today only
    /// `DependencyKind::SuccessOnly`). `data_passing_ref` is an opaque
    /// caller-supplied string stored on the FF edge and surfaced to
    /// the downstream task after upstream resolution; cairn never
    /// dereferences it.
    ///
    /// Idempotency: replaying the same `(project, session, dependent,
    /// prerequisite)` with identical `dependency_kind` and
    /// `data_passing_ref` returns the existing record. Replaying with
    /// a different kind or ref returns `FabricError::DependencyConflict`
    /// (the HTTP layer maps to 409), surfacing both existing and
    /// requested values so operators can see the divergence.
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
        // saves a round-trip and matches the contract cairn exposes.
        if dependent_task_id == prerequisite_task_id {
            return Err(FabricError::Validation {
                reason: "task cannot depend on itself".to_owned(),
            });
        }

        let fid = id_map::session_to_flow_id(project, session_id);
        let flow_partition =
            ff_core::partition::flow_partition(&fid, &self.runtime.partition_config);
        let fctx = ff_core::keys::FlowKeyContext::new(&flow_partition, &fid);

        // Mint both execution ids — deterministic_for_flow guarantees
        // they co-locate with the session's flow partition.
        let dep_eid = self.task_to_execution_id(project, Some(session_id), dependent_task_id);
        let pre_eid = self.task_to_execution_id(project, Some(session_id), prerequisite_task_id);

        // Child's execution partition — aliased to the same {fp:N}
        // tag as the flow, so apply_dependency_to_child is CROSSSLOT-
        // safe.
        let child_exec_partition =
            ff_core::partition::execution_partition(&dep_eid, &self.runtime.partition_config);
        let child_exec_ctx = ff_core::keys::ExecKeyContext::new(&child_exec_partition, &dep_eid);
        let child_idx = ff_core::keys::IndexKeys::new(&child_exec_partition);
        let lane_id = id_map::project_to_lane(project);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Deterministic edge ID: replay of the same (flow, upstream,
        // downstream) triple mints the same ID, so the
        // `dependency_already_exists` branch below can read the
        // staged edge's `(dependency_kind, data_passing_ref)` via
        // HGETALL and compare without scanning.
        let edge_id = id_map::dependency_edge_id(&fid, &pre_eid, &dep_eid);
        let kind_ff = dependency_kind.as_ff_str();
        let data_ref_ff = data_passing_ref.unwrap_or("");

        // Safe-to-log observability (SEC-007): data_passing_ref can be
        // caller-supplied opaque content (artifact IDs, signed URLs,
        // etc.). Live traces surface length + first 16 chars only; the
        // persisted BridgeEvent carries the full value for operator
        // event streams.
        let data_ref_len = data_ref_ff.len();
        let data_ref_prefix: String = data_ref_ff.chars().take(16).collect();
        tracing::debug!(
            flow_id = %fid,
            edge_id = %edge_id,
            dependent_task_id = %dependent_task_id,
            prerequisite_task_id = %prerequisite_task_id,
            dependency_kind = kind_ff,
            data_passing_ref.len = data_ref_len,
            data_passing_ref.prefix = %data_ref_prefix,
            "declare_dependency staging edge",
        );

        // Retry budget for stale_graph_revision. Concurrent declarers
        // on the same session's flow all compete for the HINCRBY;
        // exponential backoff converges quickly. Five attempts at
        // 10/20/40/80/160ms is plenty for typical concurrency;
        // beyond that something is pathologically wrong and we
        // surface as Validation so the caller can retry externally.
        const RETRY_DELAYS_MS: &[u64] = &[10, 20, 40, 80, 160];
        let new_graph_revision: u64;
        let mut attempts = 0usize;
        loop {
            attempts += 1;
            // Read the current graph_revision. New flows have 0.
            let current_rev_str: Option<String> = self
                .runtime
                .client
                .hget(&fctx.core(), "graph_revision")
                .await
                .map_err(|e| FabricError::Internal(format!("hget graph_revision: {e}")))?;
            let current_rev: u64 = current_rev_str.and_then(|s| s.parse().ok()).unwrap_or(0);

            let (keys, args) = crate::fcall::flow_edges::build_stage_dependency_edge(
                &fctx,
                &fid,
                &edge_id,
                &pre_eid,
                &dep_eid,
                kind_ff,
                data_ref_ff,
                current_rev,
                now_ms,
            );
            let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
            let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

            let raw = self
                .runtime
                .fcall(
                    crate::fcall::names::FF_STAGE_DEPENDENCY_EDGE,
                    &key_refs,
                    &arg_refs,
                )
                .await
                .map_err(|e| FabricError::Internal(format!("ff_stage_dependency_edge: {e}")))?;

            if let Some(code) = crate::helpers::fcall_error_code(&raw) {
                if code == "stale_graph_revision" {
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
                match code.as_str() {
                    "cycle_detected" => {
                        return Err(FabricError::Validation {
                            reason: "dependency introduces cycle".to_owned(),
                        });
                    }
                    "self_referencing_edge" => {
                        // Already guarded client-side; this would be
                        // a logic bug.
                        return Err(FabricError::Validation {
                            reason: "task cannot depend on itself".to_owned(),
                        });
                    }
                    "dependency_already_exists" => {
                        // Edge already staged. Deterministic edge_id
                        // means we can read the staged edge hash and
                        // compare `(dependency_kind, data_passing_ref)`
                        // against the replayed caller's values:
                        //   - match → idempotent success, return the
                        //     existing record with the stored values.
                        //   - mismatch → FabricError::DependencyConflict,
                        //     surfaced to the HTTP layer as 409 so
                        //     callers see the divergence instead of
                        //     silently losing their new ref.
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
                    "flow_not_found" => {
                        return Err(FabricError::NotFound {
                            entity: "flow",
                            id: fid.to_string(),
                        });
                    }
                    "flow_already_terminal" => {
                        return Err(FabricError::Validation {
                            reason: format!(
                                "session's flow {fid} is already terminal; \
                                 cannot add new dependency edges"
                            ),
                        });
                    }
                    "execution_not_in_flow" => {
                        return Err(FabricError::NotFound {
                            entity: "task",
                            id: "(one of the endpoints is not a flow member)".to_owned(),
                        });
                    }
                    _ => {
                        return Err(FabricError::Internal(format!(
                            "ff_stage_dependency_edge rejected: {code}"
                        )));
                    }
                }
            }

            // Success — parse `new_graph_revision` out of the OK envelope.
            new_graph_revision = parse_stage_result_revision(&raw).ok_or_else(|| {
                FabricError::Internal("ff_stage_dependency_edge: malformed OK envelope".into())
            })?;
            break;
        }

        // 2. Apply the edge on the child's execution partition.
        let (apply_keys, apply_args) = crate::fcall::flow_edges::build_apply_dependency_to_child(
            &child_exec_ctx,
            &child_idx.lane_eligible(&lane_id),
            &child_idx.lane_blocked_dependencies(&lane_id),
            &edge_id,
            &fid,
            &pre_eid,
            new_graph_revision,
            kind_ff,
            data_ref_ff,
            now_ms,
        );
        let apply_key_refs: Vec<&str> = apply_keys.iter().map(|s| s.as_str()).collect();
        let apply_arg_refs: Vec<&str> = apply_args.iter().map(|s| s.as_str()).collect();
        let apply_raw = self
            .runtime
            .fcall(
                crate::fcall::names::FF_APPLY_DEPENDENCY_TO_CHILD,
                &apply_key_refs,
                &apply_arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_apply_dependency_to_child: {e}")))?;
        crate::helpers::check_fcall_success(
            &apply_raw,
            crate::fcall::names::FF_APPLY_DEPENDENCY_TO_CHILD,
        )?;

        // 3. Append audit record. No projection reads it; callers
        // reconstruct "which deps were resolved when" by joining this
        // against the prerequisite's TaskStateChanged(Completed).
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

    /// Reconcile a replay hit on `dependency_already_exists` via the
    /// engine's [`describe_edge`](crate::engine::Engine::describe_edge)
    /// primitive. Re-declare with identical values is idempotent
    /// success; re-declare with a different `dependency_kind` or
    /// `data_passing_ref` surfaces as a `DependencyConflict` so the
    /// HTTP caller sees 409 instead of silently losing their update.
    #[allow(clippy::too_many_arguments)]
    async fn reconcile_existing_dependency_edge(
        &self,
        flow_id: &ff_core::types::FlowId,
        edge_id: &ff_core::types::EdgeId,
        project: &ProjectKey,
        dependent_task_id: &TaskId,
        prerequisite_task_id: &TaskId,
        requested_kind: DependencyKind,
        requested_data_ref: Option<&str>,
    ) -> Result<TaskDependencyRecord, FabricError> {
        let existing = match self.engine.describe_edge(flow_id, edge_id).await? {
            Some(edge) => edge,
            None => {
                // The FCALL returned `dependency_already_exists` but
                // the edge hash is now empty — a TOCTOU race between
                // the FCALL and the describe_edge read. Extremely
                // unlikely in practice (edges are not concurrently
                // deleted), but surface it cleanly rather than
                // silently treating as success.
                return Err(FabricError::Internal(
                    "dependency_already_exists but edge hash missing on describe_edge".into(),
                ));
            }
        };

        // Normalise the caller's ref (empty string → None) so it
        // compares equal to how FF stores absent refs.
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

        // Match — return the stored record. `created_at_ms` comes
        // from the edge hash (via the snapshot) so the caller sees
        // the original declaration timestamp, not the replay time.
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
    /// running / terminal, or (b) its prereqs are all satisfied. If
    /// FF reports `impossible`, an empty list is also returned —
    /// the push listener has either already skipped the child or
    /// will soon; cairn does not need to take action.
    pub async fn check_dependencies(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, FabricError> {
        let eid = self.task_to_execution_id(project, Some(session_id), task_id);
        let exec_partition =
            ff_core::partition::execution_partition(&eid, &self.runtime.partition_config);
        let exec_ctx = ff_core::keys::ExecKeyContext::new(&exec_partition, &eid);

        let (keys, args) = crate::fcall::flow_edges::build_evaluate_flow_eligibility(&exec_ctx);
        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let raw = self
            .runtime
            .fcall(
                crate::fcall::names::FF_EVALUATE_FLOW_ELIGIBILITY,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_evaluate_flow_eligibility: {e}")))?;

        // Check for an FCALL error envelope before parsing. Without
        // this, an error like `execution_not_found` returns `[0,
        // "execution_not_found", ...]`; `parse_eligibility_result`
        // would extract the error code at index 2 as if it were the
        // eligibility state, which doesn't equal
        // "blocked_by_dependencies", and the function would silently
        // return "no blockers" — hiding the real failure.
        check_fcall_success(&raw, crate::fcall::names::FF_EVALUATE_FLOW_ELIGIBILITY)?;

        let state = parse_eligibility_result(&raw).ok_or_else(|| {
            FabricError::Internal("ff_evaluate_flow_eligibility: malformed OK envelope".into())
        })?;

        if state != "blocked_by_dependencies" {
            return Ok(Vec::new());
        }

        // Enumerate incoming edges via the engine primitive — one
        // typed call replaces the prior `SMEMBERS deps_all_edges` +
        // per-edge `HGETALL dep_edge` loop. Each returned
        // [`EdgeSnapshot`](crate::engine::EdgeSnapshot) carries
        // state + kind + data_passing_ref already parsed.
        let incoming = self.engine.list_incoming_edges(&eid).await?;

        let mut blockers = Vec::with_capacity(incoming.len());
        for edge in incoming {
            // Only `unsatisfied` edges count as current blockers.
            // `satisfied` means the upstream completed in a
            // satisfying way; `impossible` means the push listener
            // has already scheduled a skip for this child.
            if edge.state != crate::engine::EdgeState::Unsatisfied {
                continue;
            }
            // Map the FF kind string back to the typed enum. Unknown
            // values surface as Internal so an FF-side kind addition
            // forces a visible cairn update rather than silent
            // mis-categorisation.
            let dependency_kind = match edge.kind.as_str() {
                "success_only" | "" => DependencyKind::SuccessOnly,
                other => {
                    return Err(FabricError::Internal(format!(
                        "unknown dependency_kind on stored edge: {other}"
                    )));
                }
            };
            // Resolve upstream execution_id → cairn task_id via the
            // `cairn.task_id` tag on the upstream execution. This is
            // the only way today: FF doesn't index executions by
            // cairn TaskId. When FF 0.3 ships execution tags as a
            // first-class read (FlowFabric#58), this stays a single
            // describe_execution call; until then it's cairn's cheapest
            // path because the snapshot also carries cairn.project,
            // cairn.parent_run_id, etc. that we might want later.
            let upstream_snap = match self
                .engine
                .describe_execution(&edge.upstream_execution_id)
                .await?
            {
                Some(s) => s,
                None => continue,
            };
            let upstream_task_id = match upstream_snap.tags.get("cairn.task_id") {
                Some(s) if !s.is_empty() => TaskId::new(s.as_str()),
                _ => continue,
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
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .map_err(|e| FabricError::Valkey(format!("HGET lane_id: {e}")))?;
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        // Shared grant+claim FCALL sequence (see services/claim_common.rs).
        // Cancel-safety: a drop between the two calls leaves only a grant,
        // which FF expires via its grant TTL.
        //
        // The returned ClaimOutcome is intentionally dropped — cairn does
        // NOT cache the lease triple. Every downstream terminal op re-reads
        // current_lease_id / _epoch / _attempt_index from FF's exec_core
        // via `resolve_active_lease`. Caching in cairn was a lean-bridge
        // violation and the root cause of the silent-emission bug fixed in
        // the parent commit.
        crate::services::claim_common::issue_grant_and_claim(
            &self.runtime,
            &ctx,
            &idx,
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
    /// Projection readers that want freshness read `lease_expires_at` via
    /// `FabricTaskService::get` which HGETALLs FF directly.
    ///
    /// If a future surface needs a `TaskLeaseHeartbeated` audit trail,
    /// introduce a dedicated BridgeEvent variant and revisit. Until then
    /// additions here must not emit.
    ///
    /// See `docs/design/bridge-event-audit.md` §2.2.
    pub async fn heartbeat(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        let (lid, epoch, att_idx) = self
            .resolve_active_lease(task_id, project, session_id)
            .await?;

        let eid = self.task_to_execution_id(project, session_id, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let attempt_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_id")
            .await
            .unwrap_or(None);

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lease_expiry(),
        ];
        let args: Vec<String> = vec![
            eid.to_string(),
            att_idx.to_string(),
            attempt_id_str.unwrap_or_default(),
            lid.to_string(),
            epoch.to_string(),
            lease_extension_ms.to_string(),
            crate::constants::DEFAULT_LEASE_HISTORY_GRACE_MS.to_owned(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_RENEW_LEASE, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_renew_lease: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_RENEW_LEASE)?;

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
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .map_err(|e| FabricError::Valkey(format!("HGET lane_id: {e}")))?;
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let (lid, epoch, att_idx) = self
            .resolve_active_lease(task_id, project, session_id)
            .await?;

        let worker_instance_id = &self.runtime.config.worker_instance_id;

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.attempt_hash(att_idx),
            idx.lease_expiry(),
            idx.worker_leases(worker_instance_id),
            idx.lane_terminal(&lane_id),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lane_active(&lane_id),
            ctx.stream_meta(att_idx),
            ctx.result(),
            idx.attempt_timeout(),
            idx.execution_deadline(),
        ];

        let attempt_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_id")
            .await
            .unwrap_or(None);

        let args: Vec<String> = vec![
            eid.to_string(),
            lid.to_string(),
            epoch.to_string(),
            attempt_id_str.unwrap_or_default(),
            String::new(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_COMPLETE_EXECUTION,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_complete_execution: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_COMPLETE_EXECUTION)?;

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
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .map_err(|e| FabricError::Valkey(format!("HGET lane_id: {e}")))?;
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let (lid, epoch, att_idx) = self
            .resolve_active_lease(task_id, project, session_id)
            .await?;

        let worker_instance_id = &self.runtime.config.worker_instance_id;

        let category = crate::state_map::failure_class_category(failure_class);
        let reason = crate::state_map::failure_class_reason(failure_class);

        let attempt_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_id")
            .await
            .unwrap_or(None);

        // Fail loud on Valkey errors — see T4-M10 rationale in run_service::fail.
        let retry_policy_json: String = self
            .runtime
            .client
            .get(&ctx.policy())
            .await
            .map_err(|e| FabricError::Valkey(format!("GET retry_policy: {e}")))?
            .unwrap_or_default();

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.attempt_hash(att_idx),
            idx.lease_expiry(),
            idx.worker_leases(worker_instance_id),
            idx.lane_terminal(&lane_id),
            idx.lane_delayed(&lane_id),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lane_active(&lane_id),
            ctx.stream_meta(att_idx),
            idx.attempt_timeout(),
            idx.execution_deadline(),
        ];
        let args: Vec<String> = vec![
            eid.to_string(),
            lid.to_string(),
            epoch.to_string(),
            attempt_id_str.unwrap_or_default(),
            reason.to_owned(),
            category.to_owned(),
            retry_policy_json,
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_FAIL_EXECUTION, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_fail_execution: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_FAIL_EXECUTION)?;

        let terminal = parse_fail_outcome(&raw) == FailOutcome::TerminalFailed;

        // Emit only on terminal fail; a retry-scheduled fail leaves the task
        // in a non-terminal state (FF will promote it back to eligible later)
        // so the projection should not see a `Failed` transition yet.
        let record = self.read_task_record(project, session_id, task_id).await?;
        if terminal {
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
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .map_err(|e| FabricError::Valkey(format!("HGET lane_id: {e}")))?;
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let (lid, epoch, att_idx) = self
            .resolve_lease_or_placeholder(task_id, project, session_id)
            .await?;

        let worker_instance_id = &self.runtime.config.worker_instance_id;

        let wp_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_waitpoint_id")
            .await
            .ok()
            .flatten();
        let wp_id = wp_id_str
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| ff_core::types::WaitpointId::parse(s).ok())
            .unwrap_or_default();

        let (keys, args) = crate::fcall::execution::build_cancel_execution(
            &ctx,
            &idx,
            att_idx,
            worker_instance_id,
            &lane_id,
            &wp_id,
            &eid,
            crate::constants::CANCEL_REASON_OPERATOR,
            crate::constants::CANCEL_SOURCE_OVERRIDE,
            &lid.to_string(),
            &epoch.to_string(),
        );

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_CANCEL_EXECUTION,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_cancel_execution: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_CANCEL_EXECUTION)?;

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
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL: {e}")))?;

        let lane_id = ff_core::types::LaneId::new(
            fields.get("lane_id").map(|s| s.as_str()).unwrap_or("cairn"),
        );
        let att_idx = ff_core::types::AttemptIndex::new(
            fields
                .get("current_attempt_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            fields
                .get("current_worker_instance_id")
                .map(|s| s.as_str())
                .unwrap_or("cairn"),
        );

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
                    condition_matchers: vec![ff_sdk::task::ConditionMatcher {
                        signal_name: signal_name.to_owned(),
                    }],
                    timeout_ms: reason.resume_after_ms,
                    timeout_behavior: ff_sdk::task::TimeoutBehavior::Fail,
                }
            }
            PauseReasonKind::PolicyHold => {
                let detail = reason.detail.as_deref().unwrap_or("policy");
                crate::suspension::SuspensionParams {
                    reason_code: "paused_by_policy".into(),
                    condition_matchers: vec![ff_sdk::task::ConditionMatcher {
                        signal_name: format!("policy_resolved:{detail}"),
                    }],
                    timeout_ms: reason.resume_after_ms,
                    timeout_behavior: ff_sdk::task::TimeoutBehavior::Fail,
                }
            }
        };

        let timeout_behavior_str = match params.timeout_behavior {
            ff_sdk::task::TimeoutBehavior::Fail => "fail",
            ff_sdk::task::TimeoutBehavior::Cancel => "cancel",
            ff_sdk::task::TimeoutBehavior::Expire => "expire",
            ff_sdk::task::TimeoutBehavior::AutoResume => "auto_resume_with_timeout_signal",
            ff_sdk::task::TimeoutBehavior::Escalate => "escalate",
        };

        let suspension_id = ff_core::types::SuspensionId::new();
        let waitpoint_id = ff_core::types::WaitpointId::new();
        let waitpoint_key = format!("wpk:{waitpoint_id}");

        let required_names: Vec<&str> = params
            .condition_matchers
            .iter()
            .map(|m| m.signal_name.as_str())
            .collect();
        let match_mode = if required_names.len() <= 1 {
            "any"
        } else {
            "all"
        };

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

        let timeout_at = params.timeout_ms.map(|ms| {
            let now = ff_core::types::TimestampMs::now().0;
            now.saturating_add(ms as i64).to_string()
        });

        let attempt_id = fields
            .get("current_attempt_id")
            .cloned()
            .unwrap_or_default();
        let lease_id = fields.get("current_lease_id").cloned().unwrap_or_default();
        let lease_epoch = fields
            .get("current_lease_epoch")
            .cloned()
            .unwrap_or_else(|| "1".to_owned());

        let (keys, args) = crate::fcall::suspension::build_suspend_execution(
            &ctx,
            &idx,
            att_idx,
            &worker_instance_id,
            &lane_id,
            &waitpoint_id,
            &eid,
            &attempt_id,
            &lease_id,
            &lease_epoch,
            &suspension_id,
            &waitpoint_key,
            &params.reason_code,
            &timeout_at.unwrap_or_default(),
            &resume_condition_json,
            &resume_policy_json,
            timeout_behavior_str,
        );

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_SUSPEND_EXECUTION,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_suspend_execution: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_SUSPEND_EXECUTION)?;

        // Emit TaskStateChanged so the cairn-store projection + SSE
        // subscribers observe the suspension. `record.state` carries FF's
        // post-commit truth — FF's `map_reason_to_blocking` can route
        // OperatorPause to either `operator_hold` (→ Paused) or
        // `waiting_for_approval` (→ WaitingApproval). Emission is
        // unconditional: a `!is_already_satisfied(&raw)` guard would be
        // retry-unsafe (silent permanent drift if the process crashes
        // between the FCALL and the emit), while projection idempotency
        // on EventId makes the double-emit replay case harmless.
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
        let eid = self.task_to_execution_id(project, session_id, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .map_err(|e| FabricError::Valkey(format!("HGET lane_id: {e}")))?;
        let lane_id = ff_core::types::LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let wp_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_waitpoint_id")
            .await
            .unwrap_or(None);
        let wp_id = wp_id_str
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| ff_core::types::WaitpointId::parse(s).ok())
            .unwrap_or_default();

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.suspension_current(),
            ctx.waitpoint(&wp_id),
            ctx.waitpoint_signals(&wp_id),
            idx.suspension_timeout(),
            idx.lane_eligible(&lane_id),
            idx.lane_delayed(&lane_id),
            idx.lane_suspended(&lane_id),
        ];
        let args: Vec<String> = vec![eid.to_string(), "operator".to_owned(), "0".to_owned()];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_RESUME_EXECUTION,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_resume_execution: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_RESUME_EXECUTION)?;

        // Emit TaskStateChanged so the cairn-store projection + SSE subscribers
        // observe the resume. Symmetric with FabricRunService::resume; the audit
        // at docs/design/bridge-event-audit.md §3.1 filed this as G2.
        //
        // Unconditional (unlike pause) — FF's ff_resume_execution rejects an
        // already-running execution with `execution_not_suspended` via
        // check_fcall_success above, so reaching this point means a real
        // suspended→runnable transition just committed. No already_satisfied
        // branch exists on the resume FCALL.
        //
        // `record.state` can be Queued / Leased / Running depending on how
        // fast the delayed_promoter / scheduler runs between FF_RESUME_EXECUTION
        // and read_task_record (existing test_suspension.rs integration test
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
        _now: u64,
        _limit: usize,
    ) -> Result<Vec<TaskRecord>, FabricError> {
        // FF's lease_expiry scanner handles this at 1.5s intervals.
        Ok(Vec::new())
    }

    pub async fn release_lease(
        &self,
        project: &ProjectKey,
        session_id: Option<&SessionId>,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        self.read_task_record(project, session_id, task_id).await
    }
}
