use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::{TaskDependencyRecord, TaskRecord};

use crate::error::FabricError;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::{
    check_fcall_success, is_duplicate_result, parse_fail_outcome, parse_project_key,
    parse_public_state, FailOutcome,
};
use ff_core::keys::{ExecKeyContext, IndexKeys};
use ff_core::partition::execution_partition;
use ff_core::types::{ExecutionId, LaneId};

use crate::active_tasks::{ActiveTaskHandle, ActiveTaskRegistry};
use crate::boot::FabricRuntime;
use crate::id_map;
use crate::state_map;

pub struct FabricTaskService {
    runtime: Arc<FabricRuntime>,
    registry: Arc<ActiveTaskRegistry>,
    bridge: Arc<EventBridge>,
}

impl FabricTaskService {
    pub fn new(
        runtime: Arc<FabricRuntime>,
        registry: Arc<ActiveTaskRegistry>,
        bridge: Arc<EventBridge>,
    ) -> Self {
        Self {
            runtime,
            registry,
            bridge,
        }
    }

    fn task_to_execution_id(&self, project: &ProjectKey, task_id: &TaskId) -> ExecutionId {
        id_map::task_to_execution_id(project, task_id)
    }

    async fn resolve_lease_context(
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
        if let Some(ctx) = self.registry.get_lease_context(task_id) {
            return Ok(ctx);
        }
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

        let lease_id = match fields.get("current_lease_id").filter(|s| !s.is_empty()) {
            Some(s) => ff_core::types::LeaseId::parse(s)
                .map_err(|e| FabricError::Internal(format!("bad lease_id: {e}")))?,
            None => ff_core::types::LeaseId::from_uuid(uuid::Uuid::nil()),
        };
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
        let project_str = tags.get("cairn.project").cloned().unwrap_or_default();
        let project = parse_project_key(&project_str);

        let lease_owner = fields.get("current_worker_instance_id").cloned();
        let lease_expires_at = fields.get("lease_expires_at").and_then(|v| v.parse().ok());

        let retry_count = fields
            .get("total_attempt_count")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);

        Ok(TaskRecord {
            task_id: task_id.clone(),
            project,
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
            "cairn_task".to_owned(),
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
            .client
            .fcall("ff_create_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_execution: {e}")))?;

        if !is_duplicate_result(&raw) {
            self.bridge.emit(BridgeEvent::TaskCreated {
                task_id: task_id.clone(),
                project: project.clone(),
                parent_run_id: parent_run_id.clone(),
                parent_task_id: parent_task_id.clone(),
            });
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

        // Step 1: Issue claim grant
        let grant_keys: Vec<String> =
            vec![ctx.core(), ctx.claim_grant(), idx.lane_eligible(&lane_id)];
        let grant_args: Vec<String> = vec![
            eid.to_string(),
            self.runtime.config.worker_id.to_string(),
            self.runtime.config.worker_instance_id.to_string(),
            lane_id.to_string(),
            String::new(),
            "5000".to_owned(),
            String::new(),
            String::new(),
        ];

        let key_refs: Vec<&str> = grant_keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = grant_args.iter().map(|s| s.as_str()).collect();

        let raw_grant: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_issue_claim_grant", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_issue_claim_grant: {e}")))?;

        check_fcall_success(&raw_grant, "ff_issue_claim_grant")?;

        // Step 2: Claim execution
        let total_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "total_attempt_count")
            .await
            .unwrap_or(None);
        let next_idx = total_str
            .as_deref()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        let att_idx = ff_core::types::AttemptIndex::new(next_idx);

        let lease_id = ff_core::types::LeaseId::new();
        let attempt_id = ff_core::types::AttemptId::new();
        let renew_before_ms = lease_duration_ms / 3;

        let claim_keys: Vec<String> = vec![
            ctx.core(),
            ctx.claim_grant(),
            idx.lane_eligible(&lane_id),
            idx.lease_expiry(),
            idx.worker_leases(&self.runtime.config.worker_instance_id),
            ctx.attempt_hash(att_idx),
            ctx.attempt_usage(att_idx),
            ctx.attempt_policy(att_idx),
            ctx.attempts(),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lane_active(&lane_id),
            idx.attempt_timeout(),
            idx.execution_deadline(),
        ];
        let claim_args: Vec<String> = vec![
            eid.to_string(),
            self.runtime.config.worker_id.to_string(),
            self.runtime.config.worker_instance_id.to_string(),
            lane_id.to_string(),
            String::new(),
            lease_id.to_string(),
            lease_duration_ms.to_string(),
            renew_before_ms.to_string(),
            attempt_id.to_string(),
            "{}".to_owned(),
            String::new(),
            String::new(),
        ];

        let key_refs2: Vec<&str> = claim_keys.iter().map(|s| s.as_str()).collect();
        let arg_refs2: Vec<&str> = claim_args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_claim_execution", &key_refs2, &arg_refs2)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_claim_execution: {e}")))?;

        check_fcall_success(&raw, "ff_claim_execution")?;

        // Parse claim result for lease_epoch
        let lease_epoch = parse_claim_lease_epoch(&raw);

        // We don't get a ClaimedTask from the SDK here (we're calling FCALL
        // directly), so we store a lightweight handle for terminal ops.
        let handle =
            ActiveTaskHandle::new_without_claimed_task(eid.clone(), lease_id, lease_epoch, att_idx);
        self.registry.register(task_id, handle);

        let record = self.read_task_record(project, task_id).await?;
        self.bridge.emit(BridgeEvent::TaskLeaseClaimed {
            task_id: task_id.clone(),
            project: record.project.clone(),
            lease_owner: self.runtime.config.worker_instance_id.to_string(),
            lease_epoch: record.version,
            lease_expires_at_ms: record.lease_expires_at.unwrap_or(0),
        });
        Ok(record)
    }

    pub async fn heartbeat(
        &self,
        project: &ProjectKey,
        task_id: &TaskId,
        lease_extension_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        let (lid, epoch, att_idx) = self.resolve_lease_context(task_id, project).await?;

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
            .client
            .fcall("ff_renew_lease", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_renew_lease: {e}")))?;

        check_fcall_success(&raw, "ff_renew_lease")?;

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

        let (lid, epoch, att_idx) = self.resolve_lease_context(task_id, project).await?;

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
            .client
            .fcall("ff_complete_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_complete_execution: {e}")))?;

        check_fcall_success(&raw, "ff_complete_execution")?;

        // Remove from active registry
        let _ = self.registry.take(task_id);

        let record = self.read_task_record(project, task_id).await?;
        self.bridge.emit(BridgeEvent::TaskStateChanged {
            task_id: task_id.clone(),
            project: record.project.clone(),
            to: TaskState::Completed,
            failure_class: None,
        });
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

        let (lid, epoch, att_idx) = self.resolve_lease_context(task_id, project).await?;

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
            .client
            .fcall("ff_fail_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_fail_execution: {e}")))?;

        check_fcall_success(&raw, "ff_fail_execution")?;

        let terminal = parse_fail_outcome(&raw) == FailOutcome::TerminalFailed;

        if terminal {
            let _ = self.registry.take(task_id);
        }

        let record = self.read_task_record(project, task_id).await?;
        if terminal {
            self.bridge.emit(BridgeEvent::TaskStateChanged {
                task_id: task_id.clone(),
                project: record.project.clone(),
                to: TaskState::Failed,
                failure_class: Some(failure_class),
            });
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

        let (lid, epoch, att_idx) = self.resolve_lease_context(task_id, project).await?;

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

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.attempt_hash(att_idx),
            ctx.stream_meta(att_idx),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lease_expiry(),
            idx.worker_leases(worker_instance_id),
            ctx.suspension_current(),
            ctx.waitpoint(&wp_id),
            ctx.waitpoint_condition(&wp_id),
            idx.suspension_timeout(),
            idx.lane_terminal(&lane_id),
            idx.attempt_timeout(),
            idx.execution_deadline(),
            idx.lane_eligible(&lane_id),
            idx.lane_delayed(&lane_id),
            idx.lane_blocked_dependencies(&lane_id),
            idx.lane_blocked_budget(&lane_id),
            idx.lane_blocked_quota(&lane_id),
            idx.lane_blocked_route(&lane_id),
            idx.lane_blocked_operator(&lane_id),
        ];
        let args: Vec<String> = vec![
            eid.to_string(),
            "operator_cancel".to_owned(),
            "operator_override".to_owned(),
            lid.to_string(),
            epoch.to_string(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_cancel_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_cancel_execution: {e}")))?;

        check_fcall_success(&raw, "ff_cancel_execution")?;

        let _ = self.registry.take(task_id);

        let record = self.read_task_record(project, task_id).await?;
        self.bridge.emit(BridgeEvent::TaskStateChanged {
            task_id: task_id.clone(),
            project: record.project.clone(),
            to: TaskState::Canceled,
            failure_class: None,
        });
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

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.attempt_hash(att_idx),
            ctx.lease_current(),
            ctx.lease_history(),
            idx.lease_expiry(),
            idx.worker_leases(&worker_instance_id),
            ctx.suspension_current(),
            ctx.waitpoint(&waitpoint_id),
            ctx.waitpoint_signals(&waitpoint_id),
            idx.suspension_timeout(),
            idx.pending_waitpoint_expiry(),
            idx.lane_active(&lane_id),
            idx.lane_suspended(&lane_id),
            ctx.waitpoints(),
            ctx.waitpoint_condition(&waitpoint_id),
            idx.attempt_timeout(),
        ];
        let args: Vec<String> = vec![
            eid.to_string(),
            att_idx.to_string(),
            fields
                .get("current_attempt_id")
                .cloned()
                .unwrap_or_default(),
            fields.get("current_lease_id").cloned().unwrap_or_default(),
            fields
                .get("current_lease_epoch")
                .cloned()
                .unwrap_or_else(|| "1".to_owned()),
            suspension_id.to_string(),
            waitpoint_id.to_string(),
            waitpoint_key,
            params.reason_code.clone(),
            "cairn".to_owned(),
            timeout_at.unwrap_or_default(),
            resume_condition_json,
            resume_policy_json,
            String::new(),
            String::new(),
            timeout_behavior_str.to_owned(),
            "1000".to_owned(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_suspend_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_suspend_execution: {e}")))?;

        check_fcall_success(&raw, "ff_suspend_execution")?;

        self.read_task_record(project, task_id).await
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
            .client
            .fcall("ff_resume_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_resume_execution: {e}")))?;

        check_fcall_success(&raw, "ff_resume_execution")?;

        self.read_task_record(project, task_id).await
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
        let _ = self.registry.take(task_id);
        self.read_task_record(project, task_id).await
    }
}

fn parse_claim_lease_epoch(raw: &ferriskey::Value) -> ff_core::types::LeaseEpoch {
    if let ferriskey::Value::Array(arr) = raw {
        if let Some(Ok(ferriskey::Value::BulkString(b))) = arr.get(3) {
            if let Ok(n) = String::from_utf8_lossy(b).parse::<u64>() {
                return ff_core::types::LeaseEpoch::new(n);
            }
        }
        if let Some(Ok(ferriskey::Value::Int(n))) = arr.get(3) {
            return ff_core::types::LeaseEpoch::new(*n as u64);
        }
    }
    ff_core::types::LeaseEpoch::new(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_claim_epoch_from_int() {
        let raw = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
            Ok(ferriskey::Value::BulkString(b"lease_abc".to_vec().into())),
            Ok(ferriskey::Value::Int(5)),
        ]);
        let epoch = parse_claim_lease_epoch(&raw);
        assert_eq!(epoch.0, 5);
    }
}
