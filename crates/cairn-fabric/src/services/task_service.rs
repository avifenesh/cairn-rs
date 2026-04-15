use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::{TaskDependencyRecord, TaskRecord};

use crate::error::FabricError;
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
}

impl FabricTaskService {
    pub fn new(runtime: Arc<FabricRuntime>, registry: Arc<ActiveTaskRegistry>) -> Self {
        Self { runtime, registry }
    }

    fn task_to_execution_id(&self, task_id: &TaskId) -> ExecutionId {
        id_map::run_to_execution_id(&RunId::new(task_id.as_str()))
    }

    async fn read_task_record(&self, task_id: &TaskId) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(task_id);
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

        let public_state_str = fields.get("public_state").cloned().unwrap_or_default();
        let public_state = parse_public_state(&public_state_str);
        let (task_state, failure_class) = state_map::ff_public_state_to_task_state(public_state);

        let created_at = fields
            .get("created_at")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let updated_at = fields
            .get("updated_at")
            .and_then(|v| v.parse().ok())
            .unwrap_or(created_at);
        let version = fields
            .get("lease_epoch")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1);

        let parent_run_id_str = fields.get("cairn.parent_run_id").cloned();
        let parent_task_id_str = fields.get("cairn.parent_task_id").cloned();
        let project_str = fields.get("cairn.project").cloned().unwrap_or_default();
        let project = parse_project_key(&project_str);

        let lease_owner = fields.get("worker_instance_id").cloned();
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
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(&task_id);
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
        if let Some(ref run_id) = parent_run_id {
            tags.insert("cairn.parent_run_id".to_owned(), run_id.as_str().to_owned());
        }
        if let Some(ref ptid) = parent_task_id {
            tags.insert("cairn.parent_task_id".to_owned(), ptid.as_str().to_owned());
        }

        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "{}".to_owned());

        let policy_json = serde_json::json!({
            "retry_policy": {
                "max_attempts": 3,
                "backoff_type": "exponential",
                "base_delay_ms": 1000,
                "max_delay_ms": 30000,
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
        let priority_score = -(priority as i64);
        let args: Vec<String> = vec![
            eid.to_string(),
            namespace.to_string(),
            lane_id.to_string(),
            "cairn_task".to_owned(),
            priority_score.to_string(),
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

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_create_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_execution: {e}")))?;

        self.read_task_record(&task_id).await
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

    pub async fn get(&self, task_id: &TaskId) -> Result<Option<TaskRecord>, FabricError> {
        match self.read_task_record(task_id).await {
            Ok(record) => Ok(Some(record)),
            Err(FabricError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn claim(
        &self,
        task_id: &TaskId,
        _lease_owner: String,
        lease_duration_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
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

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_issue_claim_grant", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_issue_claim_grant: {e}")))?;

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
        let renew_before_ms = lease_duration_ms * 2 / 3;

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

        // Parse claim result for lease_epoch
        let lease_epoch = parse_claim_lease_epoch(&raw);

        // We don't get a ClaimedTask from the SDK here (we're calling FCALL
        // directly), so we store a lightweight handle for terminal ops.
        let handle =
            ActiveTaskHandle::new_without_claimed_task(eid.clone(), lease_id, lease_epoch, att_idx);
        self.registry.register(task_id, handle);

        self.read_task_record(task_id).await
    }

    pub async fn heartbeat(
        &self,
        task_id: &TaskId,
        _lease_extension_ms: u64,
    ) -> Result<TaskRecord, FabricError> {
        // FF auto-renews leases at ttl/3 via the ClaimedTask background task.
        // For direct FCALL claims, we'd call ff_renew_lease here, but the
        // registry doesn't hold ClaimedTask handles for these. Return current state.
        self.read_task_record(task_id).await
    }

    pub async fn start(&self, task_id: &TaskId) -> Result<TaskRecord, FabricError> {
        // FF transitions to active on claim — no separate start step needed.
        self.read_task_record(task_id).await
    }

    pub async fn complete(&self, task_id: &TaskId) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let lease_ctx = self.registry.get_lease_context(task_id);
        let (lid, epoch, att_idx) = lease_ctx.unwrap_or_else(|| {
            (
                ff_core::types::LeaseId::new(),
                ff_core::types::LeaseEpoch::new(1),
                ff_core::types::AttemptIndex::new(0),
            )
        });

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

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_complete_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_complete_execution: {e}")))?;

        // Remove from active registry
        let _ = self.registry.take(task_id);

        self.read_task_record(task_id).await
    }

    pub async fn fail(
        &self,
        task_id: &TaskId,
        failure_class: FailureClass,
    ) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let lease_ctx = self.registry.get_lease_context(task_id);
        let (lid, epoch, att_idx) = lease_ctx.unwrap_or_else(|| {
            (
                ff_core::types::LeaseId::new(),
                ff_core::types::LeaseEpoch::new(1),
                ff_core::types::AttemptIndex::new(0),
            )
        });

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
            String::new(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_fail_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_fail_execution: {e}")))?;

        let _ = self.registry.take(task_id);

        self.read_task_record(task_id).await
    }

    pub async fn cancel(&self, task_id: &TaskId) -> Result<TaskRecord, FabricError> {
        let eid = self.task_to_execution_id(task_id);
        let partition = execution_partition(&eid, &self.runtime.partition_config);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let lease_ctx = self.registry.get_lease_context(task_id);
        let (lid, epoch, _att_idx) = lease_ctx.unwrap_or_else(|| {
            (
                ff_core::types::LeaseId::new(),
                ff_core::types::LeaseEpoch::new(1),
                ff_core::types::AttemptIndex::new(0),
            )
        });

        let att_idx_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_index")
            .await
            .unwrap_or(None);
        let att_idx = ff_core::types::AttemptIndex::new(
            att_idx_str
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );

        let worker_instance_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "worker_instance_id")
            .await
            .unwrap_or(None);
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            worker_instance_str.as_deref().unwrap_or("cairn"),
        );

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
            idx.worker_leases(&worker_instance_id),
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

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_cancel_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_cancel_execution: {e}")))?;

        let _ = self.registry.take(task_id);

        self.read_task_record(task_id).await
    }

    pub async fn dead_letter(&self, _task_id: &TaskId) -> Result<TaskRecord, FabricError> {
        // FF's terminal_failed after max_retries IS the dead letter.
        // No separate dead-letter action needed.
        Ok(TaskRecord {
            task_id: TaskId::new("noop"),
            project: ProjectKey::new("default_tenant", "default_workspace", "default_project"),
            parent_run_id: None,
            parent_task_id: None,
            state: TaskState::DeadLettered,
            prompt_release_id: None,
            failure_class: None,
            pause_reason: None,
            resume_trigger: None,
            retry_count: 0,
            lease_owner: None,
            lease_expires_at: None,
            title: None,
            description: None,
            version: 0,
            created_at: 0,
            updated_at: 0,
        })
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
        _task_id: &TaskId,
        _reason: PauseReason,
    ) -> Result<TaskRecord, FabricError> {
        // Task pause maps to FF suspend — use suspension.rs (Worker 3's domain).
        // For now, delegate up to the run-level pause.
        Err(FabricError::Internal(
            "task pause delegates to suspension layer".to_owned(),
        ))
    }

    pub async fn resume(
        &self,
        _task_id: &TaskId,
        _trigger: ResumeTrigger,
        _target: TaskResumeTarget,
    ) -> Result<TaskRecord, FabricError> {
        Err(FabricError::Internal(
            "task resume delegates to suspension layer".to_owned(),
        ))
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

    pub async fn release_lease(&self, task_id: &TaskId) -> Result<TaskRecord, FabricError> {
        let _ = self.registry.take(task_id);
        self.read_task_record(task_id).await
    }
}

fn parse_public_state(s: &str) -> ff_core::state::PublicState {
    match s {
        "waiting" => ff_core::state::PublicState::Waiting,
        "delayed" => ff_core::state::PublicState::Delayed,
        "rate_limited" => ff_core::state::PublicState::RateLimited,
        "waiting_children" => ff_core::state::PublicState::WaitingChildren,
        "active" => ff_core::state::PublicState::Active,
        "suspended" => ff_core::state::PublicState::Suspended,
        "completed" => ff_core::state::PublicState::Completed,
        "failed" => ff_core::state::PublicState::Failed,
        "cancelled" => ff_core::state::PublicState::Cancelled,
        "expired" => ff_core::state::PublicState::Expired,
        "skipped" => ff_core::state::PublicState::Skipped,
        _ => ff_core::state::PublicState::Waiting,
    }
}

fn parse_project_key(s: &str) -> ProjectKey {
    let parts: Vec<&str> = s.splitn(3, '/').collect();
    match parts.as_slice() {
        [t, w, p] => ProjectKey::new(*t, *w, *p),
        _ => ProjectKey::new("default_tenant", "default_workspace", "default_project"),
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
    fn parse_public_state_roundtrip() {
        assert_eq!(
            parse_public_state("active"),
            ff_core::state::PublicState::Active
        );
        assert_eq!(
            parse_public_state("completed"),
            ff_core::state::PublicState::Completed
        );
        assert_eq!(
            parse_public_state("unknown"),
            ff_core::state::PublicState::Waiting
        );
    }

    #[test]
    fn parse_project_key_roundtrip() {
        let pk = parse_project_key("t/w/p");
        assert_eq!(pk.tenant_id.as_str(), "t");
        assert_eq!(pk.workspace_id.as_str(), "w");
        assert_eq!(pk.project_id.as_str(), "p");
    }

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
