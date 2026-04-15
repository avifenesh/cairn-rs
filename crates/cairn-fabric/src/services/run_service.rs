use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::RunRecord;

use crate::error::FabricError;
use ff_core::keys::{ExecKeyContext, IndexKeys};
use ff_core::partition::{execution_partition, Partition};
use ff_core::types::{ExecutionId, LaneId, Namespace, TimestampMs};

use crate::boot::FabricRuntime;
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::{parse_project_key, parse_public_state};
use crate::id_map;
use crate::state_map;

pub struct FabricRunService {
    runtime: Arc<FabricRuntime>,
    bridge: Arc<EventBridge>,
}

impl FabricRunService {
    pub fn new(runtime: Arc<FabricRuntime>, bridge: Arc<EventBridge>) -> Self {
        Self { runtime, bridge }
    }

    fn execution_id(&self, run_id: &RunId) -> ExecutionId {
        id_map::run_to_execution_id(run_id)
    }

    fn partition(&self, eid: &ExecutionId) -> Partition {
        execution_partition(eid, &self.runtime.partition_config)
    }

    fn namespace(&self, project: &ProjectKey) -> Namespace {
        id_map::tenant_to_namespace(&project.tenant_id)
    }

    fn lane_id(&self, project: &ProjectKey) -> LaneId {
        id_map::project_to_lane(project)
    }

    async fn read_run_record(&self, run_id: &RunId) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);

        let fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL: {e}")))?;

        if fields.is_empty() {
            return Err(FabricError::NotFound {
                entity: "run",
                id: run_id.to_string(),
            });
        }

        let public_state_str = fields.get("public_state").cloned().unwrap_or_default();
        let public_state = parse_public_state(&public_state_str);
        let (run_state, failure_class) = state_map::ff_public_state_to_run_state(public_state);

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

        let session_id_str = fields.get("cairn.session_id").cloned().unwrap_or_default();
        let parent_run_id_str = fields.get("cairn.parent_run_id").cloned();
        let project_str = fields.get("cairn.project").cloned().unwrap_or_default();
        let project = parse_project_key(&project_str);

        Ok(RunRecord {
            run_id: run_id.clone(),
            session_id: SessionId::new(session_id_str),
            parent_run_id: parent_run_id_str.filter(|s| !s.is_empty()).map(RunId::new),
            project,
            state: run_state,
            prompt_release_id: None,
            agent_role_id: None,
            failure_class,
            pause_reason: None,
            resume_trigger: None,
            version,
            created_at,
            updated_at,
        })
    }

    async fn create_execution(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        parent_run_id: Option<&RunId>,
    ) -> Result<(), FabricError> {
        let eid = self.execution_id(run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);
        let lane_id = self.lane_id(project);
        let namespace = self.namespace(project);

        let mut tags = HashMap::new();
        tags.insert("cairn.run_id".to_owned(), run_id.as_str().to_owned());
        tags.insert(
            "cairn.session_id".to_owned(),
            session_id.as_str().to_owned(),
        );
        tags.insert(
            "cairn.project".to_owned(),
            format!(
                "{}/{}/{}",
                project.tenant_id, project.workspace_id, project.project_id
            ),
        );
        if let Some(parent) = parent_run_id {
            tags.insert("cairn.parent_run_id".to_owned(), parent.as_str().to_owned());
        }

        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "{}".to_owned());

        let policy_json = serde_json::json!({
            "retry_policy": {
                "max_attempts": 1,
                "backoff_type": "none"
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
            "cairn_run".to_owned(),
            "0".to_owned(),
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

        let _raw: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_create_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_execution: {e}")))?;

        Ok(())
    }

    async fn terminal_execution(&self, run_id: &RunId, function: &str) -> Result<(), FabricError> {
        let eid = self.execution_id(run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL: {e}")))?;

        let lane_id = LaneId::new(fields.get("lane_id").map(|s| s.as_str()).unwrap_or("cairn"));
        let att_idx = ff_core::types::AttemptIndex::new(
            fields
                .get("current_attempt_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        let lease_id_str = fields.get("current_lease_id").cloned();
        let lease_epoch_str = fields.get("lease_epoch").cloned();
        let attempt_id_str = fields.get("current_attempt_id").cloned();
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            fields
                .get("worker_instance_id")
                .map(|s| s.as_str())
                .unwrap_or("cairn"),
        );

        match function {
            "ff_complete_execution" => {
                let keys: Vec<String> = vec![
                    ctx.core(),
                    ctx.attempt_hash(att_idx),
                    idx.lease_expiry(),
                    idx.worker_leases(&worker_instance_id),
                    idx.lane_terminal(&lane_id),
                    ctx.lease_current(),
                    ctx.lease_history(),
                    idx.lane_active(&lane_id),
                    ctx.stream_meta(att_idx),
                    ctx.result(),
                    idx.attempt_timeout(),
                    idx.execution_deadline(),
                ];
                let args: Vec<String> = vec![
                    eid.to_string(),
                    lease_id_str.unwrap_or_default(),
                    lease_epoch_str.unwrap_or_else(|| "1".to_owned()),
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
            }
            "ff_cancel_execution" => {
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
                    "operator_override".to_owned(),
                    "operator_override".to_owned(),
                    lease_id_str.unwrap_or_default(),
                    lease_epoch_str.unwrap_or_else(|| "1".to_owned()),
                ];

                let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
                let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

                let _: ferriskey::Value = self
                    .runtime
                    .client
                    .fcall("ff_cancel_execution", &key_refs, &arg_refs)
                    .await
                    .map_err(|e| FabricError::Internal(format!("ff_cancel_execution: {e}")))?;
            }
            _ => {
                return Err(FabricError::Internal(format!(
                    "unknown terminal function: {function}"
                )));
            }
        }

        Ok(())
    }
}

impl FabricRunService {
    pub async fn start(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
    ) -> Result<RunRecord, FabricError> {
        self.create_execution(project, session_id, &run_id, parent_run_id.as_ref())
            .await?;

        self.bridge.emit(BridgeEvent::ExecutionCreated {
            run_id: run_id.clone(),
            session_id: session_id.clone(),
            project: project.clone(),
        });

        self.read_run_record(&run_id).await
    }

    pub async fn get(&self, run_id: &RunId) -> Result<Option<RunRecord>, FabricError> {
        match self.read_run_record(run_id).await {
            Ok(record) => Ok(Some(record)),
            Err(FabricError::NotFound { .. }) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub async fn list_by_session(
        &self,
        _session_id: &SessionId,
        _limit: usize,
        _offset: usize,
    ) -> Result<Vec<RunRecord>, FabricError> {
        // FF doesn't index by session natively — the event bridge keeps
        // cairn's read model in sync for list queries. Return empty here;
        // the cairn-store projection serves list_by_session from the event log.
        Ok(Vec::new())
    }

    pub async fn complete(&self, run_id: &RunId) -> Result<RunRecord, FabricError> {
        self.terminal_execution(run_id, "ff_complete_execution")
            .await?;

        let record = self.read_run_record(run_id).await?;
        self.bridge.emit(BridgeEvent::ExecutionCompleted {
            run_id: run_id.clone(),
            project: record.project.clone(),
            prev_state: None,
        });
        Ok(record)
    }

    pub async fn fail(
        &self,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

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

        let lease_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_lease_id")
            .await
            .unwrap_or(None);
        let lease_epoch_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lease_epoch")
            .await
            .unwrap_or(None);
        let attempt_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_id")
            .await
            .unwrap_or(None);
        let worker_instance_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "worker_instance_id")
            .await
            .unwrap_or(None);
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            worker_instance_str.as_deref().unwrap_or("cairn"),
        );

        let reason = format!("{failure_class:?}");
        let category = match failure_class {
            FailureClass::TimedOut => "timeout",
            FailureClass::DependencyFailed => "dependency",
            FailureClass::ApprovalRejected => "policy",
            FailureClass::PolicyDenied => "policy",
            FailureClass::ExecutionError => "execution",
            FailureClass::LeaseExpired => "lease",
            FailureClass::CanceledByOperator => "operator",
        };

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.attempt_hash(att_idx),
            idx.lease_expiry(),
            idx.worker_leases(&worker_instance_id),
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
            lease_id_str.unwrap_or_default(),
            lease_epoch_str.unwrap_or_else(|| "1".to_owned()),
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

        let record = self.read_run_record(run_id).await?;
        self.bridge.emit(BridgeEvent::ExecutionFailed {
            run_id: run_id.clone(),
            project: record.project.clone(),
            failure_class,
            prev_state: None,
        });
        Ok(record)
    }

    pub async fn cancel(&self, run_id: &RunId) -> Result<RunRecord, FabricError> {
        self.terminal_execution(run_id, "ff_cancel_execution")
            .await?;

        let record = self.read_run_record(run_id).await?;
        self.bridge.emit(BridgeEvent::ExecutionCancelled {
            run_id: run_id.clone(),
            project: record.project.clone(),
            prev_state: None,
        });
        Ok(record)
    }

    pub async fn pause(
        &self,
        run_id: &RunId,
        reason: PauseReason,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

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

        let lease_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_lease_id")
            .await
            .unwrap_or(None);
        let lease_epoch_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lease_epoch")
            .await
            .unwrap_or(None);
        let attempt_id_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_attempt_id")
            .await
            .unwrap_or(None);
        let worker_instance_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "worker_instance_id")
            .await
            .unwrap_or(None);
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            worker_instance_str.as_deref().unwrap_or("cairn"),
        );

        let (reason_code, timeout_behavior_str) = match reason.kind {
            PauseReasonKind::OperatorPause => ("operator_hold", "escalate"),
            PauseReasonKind::RuntimeSuspension => ("waiting_for_signal", "fail"),
            PauseReasonKind::ToolRequestedSuspension => ("waiting_for_tool_result", "fail"),
            PauseReasonKind::PolicyHold => ("paused_by_policy", "fail"),
        };

        let suspension_id = ff_core::types::SuspensionId::new();
        let waitpoint_id = ff_core::types::WaitpointId::new();
        let waitpoint_key = format!("wpk:{waitpoint_id}");

        let resume_condition_json = serde_json::json!({
            "condition_type": "signal_set",
            "required_signal_names": [reason_code],
            "signal_match_mode": "any",
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

        let timeout_at = reason.resume_after_ms.map(|ms| {
            let now = TimestampMs::now().0;
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
            attempt_id_str.unwrap_or_default(),
            lease_id_str.unwrap_or_default(),
            lease_epoch_str.unwrap_or_else(|| "1".to_owned()),
            suspension_id.to_string(),
            waitpoint_id.to_string(),
            waitpoint_key,
            reason_code.to_owned(),
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

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_suspend_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_suspend_execution: {e}")))?;

        let record = self.read_run_record(run_id).await?;
        self.bridge.emit(BridgeEvent::ExecutionSuspended {
            run_id: run_id.clone(),
            project: record.project.clone(),
            prev_state: None,
        });
        Ok(record)
    }

    pub async fn resume(
        &self,
        run_id: &RunId,
        _trigger: ResumeTrigger,
        _target: RunResumeTarget,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.suspension_current(),
            idx.lane_suspended(&lane_id),
            idx.lane_eligible(&lane_id),
            idx.lane_delayed(&lane_id),
        ];
        let args: Vec<String> = vec![eid.to_string(), "operator".to_owned(), "0".to_owned()];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_resume_execution", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_resume_execution: {e}")))?;

        let record = self.read_run_record(run_id).await?;
        self.bridge.emit(BridgeEvent::ExecutionResumed {
            run_id: run_id.clone(),
            project: record.project.clone(),
            prev_state: None,
        });
        Ok(record)
    }

    pub async fn enter_waiting_approval(&self, run_id: &RunId) -> Result<RunRecord, FabricError> {
        self.pause(
            run_id,
            PauseReason {
                kind: PauseReasonKind::PolicyHold,
                detail: Some("waiting_for_approval".to_owned()),
                resume_after_ms: None,
                actor: None,
            },
        )
        .await
    }

    pub async fn resolve_approval(
        &self,
        run_id: &RunId,
        decision: ApprovalDecision,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .unwrap_or(None);
        let lane_id = LaneId::new(lane_str.as_deref().unwrap_or("cairn"));

        let current_wp_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_waitpoint_id")
            .await
            .unwrap_or(None);

        let waitpoint_id = current_wp_str
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| ff_core::types::WaitpointId::parse(s).ok())
            .unwrap_or_default();

        let signal_name = match decision {
            ApprovalDecision::Approved => format!("approval_granted:{}", run_id.as_str()),
            ApprovalDecision::Rejected => format!("approval_rejected:{}", run_id.as_str()),
        };

        let signal_id = ff_core::types::SignalId::new();
        let now = TimestampMs::now();

        let idem_str = format!("approval:{}", run_id.as_str());
        let idem_key = ctx.signal_dedup(&waitpoint_id, &idem_str);

        let keys: Vec<String> = vec![
            ctx.core(),
            ctx.waitpoint_condition(&waitpoint_id),
            ctx.waitpoint_signals(&waitpoint_id),
            ctx.exec_signals(),
            ctx.signal(&signal_id),
            ctx.signal_payload(&signal_id),
            idem_key,
            ctx.waitpoint(&waitpoint_id),
            ctx.suspension_current(),
            idx.lane_eligible(&lane_id),
            idx.lane_suspended(&lane_id),
            idx.lane_delayed(&lane_id),
            idx.suspension_timeout(),
        ];
        let args: Vec<String> = vec![
            signal_id.to_string(),
            eid.to_string(),
            waitpoint_id.to_string(),
            signal_name.to_owned(),
            "approval".to_owned(),
            "operator".to_owned(),
            "cairn".to_owned(),
            String::new(),
            "json".to_owned(),
            idem_str,
            String::new(),
            "waitpoint".to_owned(),
            now.to_string(),
            "86400000".to_owned(),
            "0".to_owned(),
            "1000".to_owned(),
            "10000".to_owned(),
        ];

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let _: ferriskey::Value = self
            .runtime
            .client
            .fcall("ff_deliver_signal", &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_deliver_signal: {e}")))?;

        match decision {
            ApprovalDecision::Approved => {
                let record = self.read_run_record(run_id).await?;
                self.bridge.emit(BridgeEvent::ExecutionResumed {
                    run_id: run_id.clone(),
                    project: record.project.clone(),
                    prev_state: None,
                });
                Ok(record)
            }
            ApprovalDecision::Rejected => self.fail(run_id, FailureClass::ApprovalRejected).await,
        }
    }
}
