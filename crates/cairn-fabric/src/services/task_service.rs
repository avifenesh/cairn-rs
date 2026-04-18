use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::{TaskDependencyRecord, TaskRecord};

use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::{
    check_fcall_success, is_already_satisfied, is_duplicate_result, parse_fail_outcome,
    parse_public_state, try_parse_project_key, FailOutcome,
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
}

impl FabricTaskService {
    pub fn new(runtime: Arc<FabricRuntime>, bridge: Arc<EventBridge>) -> Self {
        Self { runtime, bridge }
    }

    fn task_to_execution_id(&self, project: &ProjectKey, task_id: &TaskId) -> ExecutionId {
        id_map::task_to_execution_id(project, task_id)
    }

    async fn read_valkey_lease_fields(
        &self,
        task_id: &TaskId,
        project: &ProjectKey,
    ) -> Result<HashMap<String, String>, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let exec_ctx = ExecKeyContext::new(&partition, &eid);
        let fields: HashMap<String, String> =
            self.runtime
                .client
                .hgetall(&exec_ctx.core())
                .await
                .map_err(|e| FabricError::Internal(format!("valkey HGETALL: {e}")))?;
        if fields.is_empty() {
            return Err(FabricError::NotFound {
                entity: "task",
                id: task_id.to_string(),
            });
        }
        Ok(fields)
    }

    // Always read the lease triple from FF's exec_core. The registry used to
    // cache (lease_id, lease_epoch, attempt_index) to save one HGETALL per
    // terminal op, but that cache is a lean-bridge violation — FF owns every
    // field authoritatively. A stale cache also silently skipped projection
    // emission for tasks claimed outside `FabricTaskService::claim`
    // (insecure-direct-claim, external API callers), which was the HIGH bug
    // fixed in the companion commit.
    async fn resolve_active_lease(
        &self,
        task_id: &TaskId,
        project: &ProjectKey,
    ) -> Result<
        (
            ff_core::types::LeaseId,
            ff_core::types::LeaseEpoch,
            ff_core::types::AttemptIndex,
        ),
        FabricError,
    > {
        let fields = self.read_valkey_lease_fields(task_id, project).await?;
        let lease_id = ff_core::types::LeaseId::parse(
            fields
                .get("current_lease_id")
                .filter(|s| !s.is_empty())
                .ok_or_else(|| FabricError::NotFound {
                    entity: "task_lease",
                    id: task_id.to_string(),
                })?,
        )
        .map_err(|e| FabricError::Internal(format!("bad lease_id: {e}")))?;
        let epoch = ff_core::types::LeaseEpoch::new(
            fields
                .get("current_lease_epoch")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
        );
        let att_idx = ff_core::types::AttemptIndex::new(
            fields
                .get("current_attempt_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        Ok((lease_id, epoch, att_idx))
    }

    // Same HGETALL-only pattern as resolve_active_lease, but tolerates an
    // empty current_lease_id (returns a nil LeaseId placeholder) for the
    // cancel-while-unclaimed path.
    async fn resolve_lease_or_placeholder(
        &self,
        task_id: &TaskId,
        project: &ProjectKey,
    ) -> Result<
        (
            ff_core::types::LeaseId,
            ff_core::types::LeaseEpoch,
            ff_core::types::AttemptIndex,
        ),
        FabricError,
    > {
        let fields = self.read_valkey_lease_fields(task_id, project).await?;
        let lease_id = match fields.get("current_lease_id").filter(|s| !s.is_empty()) {
            Some(s) => ff_core::types::LeaseId::parse(s)
                .map_err(|e| FabricError::Internal(format!("bad lease_id: {e}")))?,
            None => ff_core::types::LeaseId::from_uuid(uuid::Uuid::nil()),
        };
        let epoch = ff_core::types::LeaseEpoch::new(
            fields
                .get("current_lease_epoch")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        let att_idx = ff_core::types::AttemptIndex::new(
            fields
                .get("current_attempt_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        Ok((lease_id, epoch, att_idx))
    }

    async fn read_task_record(
        &self,
        project: &ProjectKey,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);

        let fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL: {e}")))?;

        if fields.is_empty() {
            return Err(FabricError::NotFound {
                entity: "task",
                id: task_id.to_string(),
            });
        }

        let tags: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.tags())
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(task_id = %task_id, error = %e, "failed to read execution tags");
                HashMap::new()
            });

        let public_state_str = fields.get("public_state").cloned().unwrap_or_default();
        let public_state = parse_public_state(&public_state_str);
        let (task_state, failure_class) = state_map::ff_public_state_to_task_state(public_state);
        let blocking_reason = fields.get("blocking_reason").cloned().unwrap_or_default();
        let task_state =
            state_map::adjust_task_state_for_blocking_reason(task_state, &blocking_reason);

        let created_at = fields
            .get("created_at")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let updated_at = fields
            .get("last_mutation_at")
            .and_then(|v| v.parse().ok())
            .unwrap_or(created_at);
        let version = fields
            .get("current_lease_epoch")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        let parent_run_id_str = tags.get("cairn.parent_run_id").cloned();
        let parent_task_id_str = tags.get("cairn.parent_task_id").cloned();
        let tag_project = match tags
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

        let lease_owner = fields.get("current_worker_instance_id").cloned();
        let lease_expires_at = fields.get("lease_expires_at").and_then(|v| v.parse().ok());

        let retry_count = fields
            .get("total_attempt_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        Ok(TaskRecord {
            task_id: task_id.clone(),
            project: tag_project,
            parent_run_id: parent_run_id_str.filter(|s| !s.is_empty()).map(RunId::new),
            parent_task_id: parent_task_id_str
                .filter(|s| !s.is_empty())
                .map(TaskId::new),
            state: task_state,
            prompt_release_id: None,
            failure_class,
            pause_reason: None,
            resume_trigger: None,
            retry_count,
            lease_owner,
            lease_expires_at,
            title: None,
            description: None,
            version,
            created_at,
            updated_at,
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
        let eid = self.task_to_execution_id(project, &task_id);
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
                    parent_run_id: parent_run_id.clone(),
                    parent_task_id: parent_task_id.clone(),
                })
                .await;
        }

        self.read_task_record(project, &task_id).await
    }

    pub async fn declare_dependency(
        &self,
        _dependent_task_id: &TaskId,
        _prerequisite_task_id: &TaskId,
    ) -> Result<TaskDependencyRecord, FabricError> {
        // FF handles dependencies via flow edges (ff_stage_dependency_edge +
        // ff_apply_dependency_to_child). For v1 cairn-fabric bridge, task
        // dependencies use FF's native flow coordination.
        Err(FabricError::Internal(
            "task dependencies use FF flow edges — use flow coordination API".to_owned(),
        ))
    }

    pub async fn check_dependencies(
        &self,
        _task_id: &TaskId,
    ) -> Result<Vec<TaskDependencyRecord>, FabricError> {
        Ok(Vec::new())
    }

    pub async fn get(
        &self,
        project: &ProjectKey,
        task_id: &TaskId,
    ) -> Result<Option<TaskRecord>, FabricError> {
        match self.read_task_record(project, task_id).await {
            Ok(record) => Ok(Some(record)),
            Err(FabricError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn claim(
        &self,
        project: &ProjectKey,
        task_id: &TaskId,
        _lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
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

        let record = self.read_task_record(project, task_id).await?;
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
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        let (lid, epoch, att_idx) = self.resolve_active_lease(task_id, project).await?;

        let eid = self.task_to_execution_id(project, task_id);
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
            "5000".to_owned(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_RENEW_LEASE, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_renew_lease: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_RENEW_LEASE)?;

        self.read_task_record(project, task_id).await
    }

    pub async fn start(
        &self,
        project: &ProjectKey,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        // FF transitions to active on claim — no separate start step needed.
        self.read_task_record(project, task_id).await
    }

    pub async fn complete(
        &self,
        project: &ProjectKey,
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
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

        let (lid, epoch, att_idx) = self.resolve_active_lease(task_id, project).await?;

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
        let record = self.read_task_record(project, task_id).await?;
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
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
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

        let (lid, epoch, att_idx) = self.resolve_active_lease(task_id, project).await?;

        let worker_instance_id = &self.runtime.config.worker_instance_id;

        let reason = format!("{failure_class:?}");
        let category = match failure_class {
            FailureClass::TimedOut => "timeout",
            FailureClass::DependencyFailed => "dependency",
            FailureClass::ApprovalRejected | FailureClass::PolicyDenied => "policy",
            FailureClass::ExecutionError => "execution",
            FailureClass::LeaseExpired => "lease",
            FailureClass::CanceledByOperator => "operator",
        };

        let attempt_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_id")
            .await
            .unwrap_or(None);

        let retry_policy_json: String = self
            .runtime
            .client
            .get(&ctx.policy())
            .await
            .unwrap_or(None)
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
            reason,
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
        let record = self.read_task_record(project, task_id).await?;
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
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
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

        let (lid, epoch, att_idx) = self.resolve_lease_or_placeholder(task_id, project).await?;

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
        let record = self.read_task_record(project, task_id).await?;
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
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        self.read_task_record(project, task_id).await
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
        task_id: &TaskId,
        reason: PauseReason,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
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

        // Emit TaskStateChanged so the cairn-store projection + SSE subscribers
        // observe the suspension. Symmetric with FabricRunService::pause; the
        // audit at docs/design/bridge-event-audit.md §3.1 filed this as G1.
        //
        // `record.state` carries FF's post-commit truth — FF's
        // map_reason_to_blocking can route OperatorPause to either
        // `operator_hold` (→ TaskState::Paused) or `waiting_for_approval`
        // (→ TaskState::WaitingApproval) per reason_code. Reading from the
        // fresh HGETALL in read_task_record avoids hard-coding a single state.
        //
        // `is_already_satisfied` guards the duplicate case: FF returns
        // ok_already_satisfied when the execution is already suspended
        // (idempotent replay). We skip the emit on that path so we don't
        // double-project; the first emit already populated the record.
        let record = self.read_task_record(project, task_id).await?;
        if !is_already_satisfied(&raw) {
            self.bridge
                .emit(BridgeEvent::TaskStateChanged {
                    task_id: task_id.clone(),
                    project: record.project.clone(),
                    to: record.state,
                    failure_class: None,
                })
                .await;
        }
        Ok(record)
    }

    pub async fn resume(
        &self,
        project: &ProjectKey,
        task_id: &TaskId,
        _trigger: ResumeTrigger,
        _target: TaskResumeTarget,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(project, task_id);
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
        let record = self.read_task_record(project, task_id).await?;
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
        task_id: &TaskId,
    ) -> Result<TaskRecord, FabricError> {
        self.read_task_record(project, task_id).await
    }
}
