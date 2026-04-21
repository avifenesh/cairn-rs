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
use crate::helpers::{
    check_fcall_success, is_duplicate_result, parse_fail_outcome, parse_public_state,
    try_parse_project_key, FailOutcome,
};
use crate::id_map;
use crate::state_map;

pub struct FabricRunService {
    runtime: Arc<FabricRuntime>,
    bridge: Arc<EventBridge>,
    #[allow(dead_code)] // wired-in for the B-phase ports
    engine: Arc<dyn crate::engine::Engine>,
}

impl FabricRunService {
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

    /// Mint the session-scoped `ExecutionId` for a cairn run.
    ///
    /// Routes through `id_map::session_run_to_execution_id` so every
    /// run of a session co-locates on the session's FlowId partition.
    /// The caller MUST supply the session binding that was
    /// used at run create time; mismatched sessions mint a different
    /// ExecutionId and the lookup misses FF's state entirely.
    fn execution_id(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> ExecutionId {
        id_map::session_run_to_execution_id(
            project,
            session_id,
            run_id,
            &self.runtime.partition_config,
        )
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

    async fn read_run_record(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                })?;

        let public_state = parse_public_state(&snapshot.public_state);
        let (run_state, failure_class) = state_map::ff_public_state_to_run_state(public_state);
        let run_state = state_map::adjust_run_state_for_blocking_reason(
            run_state,
            snapshot.blocking_reason.as_deref().unwrap_or_default(),
        );

        let session_id_str = snapshot
            .tags
            .get("cairn.session_id")
            .cloned()
            .unwrap_or_default();
        let parent_run_id_str = snapshot.tags.get("cairn.parent_run_id").cloned();
        let tag_project = match snapshot
            .tags
            .get("cairn.project")
            .and_then(|s| try_parse_project_key(s))
        {
            Some(tp) => {
                if tp != *project {
                    tracing::warn!(
                        run_id = %run_id,
                        caller = %format!("{}/{}/{}", project.tenant_id, project.workspace_id, project.project_id),
                        tag = %format!("{}/{}/{}", tp.tenant_id, tp.workspace_id, tp.project_id),
                        "run tag project does not match caller project"
                    );
                }
                tp
            }
            None => project.clone(),
        };

        Ok(RunRecord {
            run_id: run_id.clone(),
            session_id: SessionId::new(session_id_str),
            parent_run_id: parent_run_id_str.filter(|s| !s.is_empty()).map(RunId::new),
            project: tag_project,
            state: run_state,
            prompt_release_id: None,
            agent_role_id: None,
            failure_class,
            pause_reason: None,
            resume_trigger: None,
            version: snapshot.current_lease_epoch.map(|e| e.0).unwrap_or(1),
            created_at: snapshot.created_at.0 as u64,
            updated_at: snapshot.last_mutation_at.0 as u64,
        })
    }

    async fn create_execution(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        parent_run_id: Option<&RunId>,
        correlation_id: Option<&str>,
    ) -> Result<bool, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
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
        if let Some(corr) = correlation_id.filter(|s| !s.is_empty()) {
            tags.insert("cairn.correlation_id".to_owned(), corr.to_owned());
        }

        let tags_json = serde_json::to_string(&tags).unwrap_or_else(|_| "{}".to_owned());

        let policy_json = serde_json::json!({
            "max_retries": 0
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
            crate::constants::EXECUTION_KIND_RUN.to_owned(),
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

        let raw: ferriskey::Value = self
            .runtime
            .fcall(
                crate::fcall::names::FF_CREATE_EXECUTION,
                &key_refs,
                &arg_refs,
            )
            .await
            .map_err(|e| FabricError::Internal(format!("ff_create_execution: {e}")))?;

        Ok(!is_duplicate_result(&raw))
    }

    async fn terminal_execution(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        function: &str,
    ) -> Result<RunState, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

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

        let prev_public =
            parse_public_state(&fields.get("public_state").cloned().unwrap_or_default());
        let (prev_run_state, _) = state_map::ff_public_state_to_run_state(prev_public);

        let lane_id = LaneId::new(fields.get("lane_id").map(|s| s.as_str()).unwrap_or("cairn"));
        let att_idx = ff_core::types::AttemptIndex::new(
            fields
                .get("current_attempt_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        let lease_id_str = fields.get("current_lease_id").cloned();
        let lease_epoch_str = fields.get("current_lease_epoch").cloned();
        let attempt_id_str = fields.get("current_attempt_id").cloned();
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            fields
                .get("current_worker_instance_id")
                .map(|s| s.as_str())
                .unwrap_or("cairn"),
        );

        match function {
            crate::fcall::names::FF_COMPLETE_EXECUTION => {
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
            }
            crate::fcall::names::FF_CANCEL_EXECUTION => {
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
                    &worker_instance_id,
                    &lane_id,
                    &wp_id,
                    &eid,
                    crate::constants::CANCEL_SOURCE_OVERRIDE,
                    crate::constants::CANCEL_SOURCE_OVERRIDE,
                    &lease_id_str.unwrap_or_default(),
                    &lease_epoch_str.unwrap_or_else(|| "1".to_owned()),
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
            }
            _ => {
                return Err(FabricError::Internal(format!(
                    "unknown terminal function: {function}"
                )));
            }
        }

        Ok(prev_run_state)
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
        self.start_with_correlation(project, session_id, run_id, parent_run_id, None)
            .await
    }

    /// `start` but threading an external correlation id onto both the FF
    /// exec_core tag (`cairn.correlation_id`) and the emitted
    /// `BridgeEvent::ExecutionCreated` (which propagates to the
    /// `EventEnvelope.correlation_id` field in cairn-store). Used by sqeq
    /// ingress and any other handler path that must preserve a
    /// request-scoped correlation through the run's projection / SSE
    /// audit trail.
    pub async fn start_with_correlation(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: RunId,
        parent_run_id: Option<RunId>,
        correlation_id: Option<&str>,
    ) -> Result<RunRecord, FabricError> {
        let created = self
            .create_execution(
                project,
                session_id,
                &run_id,
                parent_run_id.as_ref(),
                correlation_id,
            )
            .await?;

        if created {
            self.bridge
                .emit(BridgeEvent::ExecutionCreated {
                    run_id: run_id.clone(),
                    session_id: session_id.clone(),
                    project: project.clone(),
                    parent_run_id: parent_run_id.clone(),
                    correlation_id: correlation_id.map(str::to_owned),
                })
                .await;
        }

        self.read_run_record(project, session_id, &run_id).await
    }

    pub async fn get(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<Option<RunRecord>, FabricError> {
        match self.read_run_record(project, session_id, run_id).await {
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

    /// Claim a run's FF execution so it becomes `lifecycle_phase=active`.
    ///
    /// Runs are not FF-scheduled in the worker sense (no cairn worker pulls
    /// them off an eligible zset), but FF's suspension / signal FCALLs
    /// reject non-active executions. Approval gates (`enter_waiting_approval`
    /// → `resolve_approval`) therefore need an explicit claim first.
    ///
    /// This is the run-side mirror of [`FabricTaskService::claim`]. Both
    /// share the `ff_issue_claim_grant` + `ff_claim_execution` sequence via
    /// [`super::claim_common::issue_grant_and_claim`]; neither caches lease
    /// state — downstream FCALLs re-read `current_lease_id` / `_epoch` /
    /// `_attempt_id` from `exec_core`.
    pub async fn claim(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let lane_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "lane_id")
            .await
            .map_err(|e| FabricError::Valkey(format!("HGET lane_id: {e}")))?;
        let lane_id = match lane_str.filter(|s| !s.is_empty()) {
            Some(s) => LaneId::new(s),
            None => self.lane_id(project),
        };

        let _outcome = super::claim_common::issue_grant_and_claim(
            &self.runtime,
            &ctx,
            &idx,
            &eid,
            &lane_id,
            self.runtime.config.lease_ttl_ms,
        )
        .await?;

        // No cairn-side cache. `enter_waiting_approval`, `fail`, `cancel`,
        // etc. all read the lease triple back from exec_core directly.
        self.read_run_record(project, session_id, run_id).await
    }

    pub async fn complete(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, FabricError> {
        let prev = self
            .terminal_execution(
                project,
                session_id,
                run_id,
                crate::fcall::names::FF_COMPLETE_EXECUTION,
            )
            .await?;

        let record = self.read_run_record(project, session_id, run_id).await?;
        self.bridge
            .emit(BridgeEvent::ExecutionCompleted {
                run_id: run_id.clone(),
                project: record.project.clone(),
                prev_state: Some(prev),
            })
            .await;
        Ok(record)
    }

    pub async fn fail(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        failure_class: FailureClass,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

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

        let prev_public =
            parse_public_state(&fields.get("public_state").cloned().unwrap_or_default());
        let (prev_run_state, _) = state_map::ff_public_state_to_run_state(prev_public);

        let lane_id = LaneId::new(fields.get("lane_id").map(|s| s.as_str()).unwrap_or("cairn"));
        let att_idx = ff_core::types::AttemptIndex::new(
            fields
                .get("current_attempt_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        let lease_id_str = fields.get("current_lease_id").cloned();
        let lease_epoch_str = fields.get("current_lease_epoch").cloned();
        let attempt_id_str = fields.get("current_attempt_id").cloned();
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            fields
                .get("current_worker_instance_id")
                .map(|s| s.as_str())
                .unwrap_or("cairn"),
        );

        let category = crate::state_map::failure_class_category(failure_class);
        let reason = crate::state_map::failure_class_reason(failure_class);

        // Fail loud on Valkey errors: a silent `unwrap_or_default()` here
        // turns a transient blip into "FF falls back to its own retry
        // default", which may disable retries entirely for this run.
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

        let record = self.read_run_record(project, session_id, run_id).await?;
        if parse_fail_outcome(&raw) == FailOutcome::TerminalFailed {
            self.bridge
                .emit(BridgeEvent::ExecutionFailed {
                    run_id: run_id.clone(),
                    project: record.project.clone(),
                    failure_class,
                    prev_state: Some(prev_run_state),
                })
                .await;
        }
        Ok(record)
    }

    pub async fn cancel(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, FabricError> {
        let prev = self
            .terminal_execution(
                project,
                session_id,
                run_id,
                crate::fcall::names::FF_CANCEL_EXECUTION,
            )
            .await?;

        let record = self.read_run_record(project, session_id, run_id).await?;
        self.bridge
            .emit(BridgeEvent::ExecutionCancelled {
                run_id: run_id.clone(),
                project: record.project.clone(),
                prev_state: Some(prev),
            })
            .await;
        Ok(record)
    }

    pub async fn pause(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        reason: PauseReason,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let fields: HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL: {e}")))?;

        let prev_public =
            parse_public_state(&fields.get("public_state").cloned().unwrap_or_default());
        let (prev_run_state, _) = state_map::ff_public_state_to_run_state(prev_public);

        let lane_id = LaneId::new(fields.get("lane_id").map(|s| s.as_str()).unwrap_or("cairn"));
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
            let now = TimestampMs::now().0;
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

        // Emit unconditionally — the prior `!is_already_satisfied(&raw)` guard
        // was retry-unsafe. Projection is idempotent on EventId, so a duplicate
        // emit on replay is a harmless re-write; guarding would silently lose
        // the event if the process crashed between the FCALL and the emit.
        //
        // If `read_run_record` fails after the FCALL committed, still emit
        // with `RunState::Paused` as a safe fallback — better a slightly
        // wrong-but-valid state in the projection than a permanent gap.
        // The operator's next read will correct via the FF-backed
        // adjust_run_state_for_blocking_reason path.
        let record_result = self.read_run_record(project, session_id, run_id).await;
        let (to_state, project_for_emit) = match &record_result {
            Ok(r) => (r.state, r.project.clone()),
            Err(e) => {
                tracing::error!(
                    run_id = %run_id,
                    error = %e,
                    "suspend: read_run_record failed after ff_suspend_execution committed — \
                     emitting with fallback state to avoid projection gap"
                );
                (RunState::Paused, project.clone())
            }
        };
        self.bridge
            .emit(BridgeEvent::ExecutionSuspended {
                run_id: run_id.clone(),
                project: project_for_emit,
                prev_state: Some(prev_run_state),
                to: to_state,
            })
            .await;
        record_result
    }

    pub async fn resume(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        _trigger: ResumeTrigger,
        _target: RunResumeTarget,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
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

        // FF ff_resume_execution KEYS(8): exec_core, suspension_current,
        // waitpoint_hash, waitpoint_signals, suspension_timeout_zset,
        // eligible_zset, delayed_zset, suspended_zset
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

        let prev_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "public_state")
            .await
            .unwrap_or(None);
        let prev_public = parse_public_state(&prev_str.unwrap_or_default());
        let (prev_run_state, _) = state_map::ff_public_state_to_run_state(prev_public);

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

        let record = self.read_run_record(project, session_id, run_id).await?;
        self.bridge
            .emit(BridgeEvent::ExecutionResumed {
                run_id: run_id.clone(),
                project: record.project.clone(),
                prev_state: Some(prev_run_state),
            })
            .await;
        Ok(record)
    }

    pub async fn enter_waiting_approval(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
    ) -> Result<RunRecord, FabricError> {
        let params = crate::suspension::for_approval(run_id.as_str(), None);

        let eid = self.execution_id(project, session_id, run_id);
        let partition = self.partition(&eid);
        let ctx = ExecKeyContext::new(&partition, &eid);
        let idx = IndexKeys::new(&partition);

        let fields: std::collections::HashMap<String, String> = self
            .runtime
            .client
            .hgetall(&ctx.core())
            .await
            .map_err(|e| FabricError::Internal(format!("valkey HGETALL: {e}")))?;

        let prev_public =
            parse_public_state(&fields.get("public_state").cloned().unwrap_or_default());
        let (prev_run_state, _) = state_map::ff_public_state_to_run_state(prev_public);

        let lane_id = LaneId::new(fields.get("lane_id").map(|s| s.as_str()).unwrap_or("cairn"));
        let att_idx = ff_core::types::AttemptIndex::new(
            fields
                .get("current_attempt_index")
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        );
        let lease_id_str = fields.get("current_lease_id").cloned().unwrap_or_default();
        let lease_epoch_str = fields
            .get("current_lease_epoch")
            .cloned()
            .unwrap_or_else(|| "1".to_owned());
        let attempt_id_str = fields
            .get("current_attempt_id")
            .cloned()
            .unwrap_or_default();
        let worker_instance_id = ff_core::types::WorkerInstanceId::new(
            fields
                .get("current_worker_instance_id")
                .map(|s| s.as_str())
                .unwrap_or("cairn"),
        );

        let suspension_id = ff_core::types::SuspensionId::new();
        let waitpoint_id = ff_core::types::WaitpointId::new();
        let waitpoint_key = format!("wpk:{waitpoint_id}");

        let signal_names: Vec<&str> = params
            .condition_matchers
            .iter()
            .map(|m| m.signal_name.as_str())
            .collect();
        let timeout_behavior_str = match params.timeout_behavior {
            ff_sdk::task::TimeoutBehavior::Fail => "fail",
            ff_sdk::task::TimeoutBehavior::Escalate => "escalate",
            ff_sdk::task::TimeoutBehavior::Expire => "expire",
            ff_sdk::task::TimeoutBehavior::Cancel => "cancel",
            ff_sdk::task::TimeoutBehavior::AutoResume => "auto_resume_with_timeout_signal",
        };

        let resume_condition_json = serde_json::json!({
            "condition_type": "signal_set",
            "required_signal_names": signal_names,
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

        let timeout_at = String::new();
        let (keys, args) = crate::fcall::suspension::build_suspend_execution(
            &ctx,
            &idx,
            att_idx,
            &worker_instance_id,
            &lane_id,
            &waitpoint_id,
            &eid,
            &attempt_id_str,
            &lease_id_str,
            &lease_epoch_str,
            &suspension_id,
            &waitpoint_key,
            &params.reason_code,
            &timeout_at,
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

        // Emit unconditionally. See `pause` for retry-safety rationale.
        // If `read_run_record` fails after the FCALL committed, emit with
        // `WaitingApproval` (the blocking_reason we just set) as the fallback
        // so the projection still gets the state change.
        let record_result = self.read_run_record(project, session_id, run_id).await;
        let (to_state, project_for_emit) = match &record_result {
            Ok(r) => (r.state, r.project.clone()),
            Err(e) => {
                tracing::error!(
                    run_id = %run_id,
                    error = %e,
                    "enter_waiting_approval: read_run_record failed after ff_suspend_execution \
                     committed — emitting with fallback state to avoid projection gap"
                );
                (RunState::WaitingApproval, project.clone())
            }
        };
        self.bridge
            .emit(BridgeEvent::ExecutionSuspended {
                run_id: run_id.clone(),
                project: project_for_emit,
                prev_state: Some(prev_run_state),
                to: to_state,
            })
            .await;
        record_result
    }

    pub async fn resolve_approval(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        decision: ApprovalDecision,
    ) -> Result<RunRecord, FabricError> {
        let eid = self.execution_id(project, session_id, run_id);
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

        // Validation boundary: if the run isn't awaiting approval, surface
        // that as a state error at the state layer. Otherwise callers hitting
        // double-approve or approve-on-unsuspended-run would get a misleading
        // HMAC-flavored `invalid_token` from FF (which is the oracle-proof
        // default for "no waitpoint" — correct for untrusted callers, wrong
        // for a trusted in-process caller that can know the distinction).
        let current_wp_str: Option<String> = self
            .runtime
            .client
            .hget(&ctx.core(), "current_waitpoint_id")
            .await
            .unwrap_or(None);
        let waitpoint_id = match current_wp_str
            .as_deref()
            .filter(|s| !s.is_empty())
            .and_then(|s| ff_core::types::WaitpointId::parse(s).ok())
        {
            Some(wp) => wp,
            None => {
                return Err(FabricError::Validation {
                    reason: format!(
                        "run {} is not awaiting approval (no current waitpoint)",
                        run_id.as_str()
                    ),
                });
            }
        };

        let signal_name = match decision {
            ApprovalDecision::Approved => format!("approval_granted:{}", run_id.as_str()),
            ApprovalDecision::Rejected => format!("approval_rejected:{}", run_id.as_str()),
        };

        let signal_id = ff_core::types::SignalId::new();
        let now = TimestampMs::now();

        let idem_str = format!("approval:{}", run_id.as_str());
        let idem_key = ctx.signal_dedup(&waitpoint_id, &idem_str);

        // HMAC token: FF owns it, we read from the waitpoint hash right
        // before delivery. Missing token ⇒ waitpoint was never activated
        // (validation-layer failure, not auth-layer).
        let waitpoint_token =
            crate::signal_bridge::read_waitpoint_token(&self.runtime.client, &ctx, &waitpoint_id)
                .await?;

        let (keys, args) = crate::fcall::suspension::build_deliver_signal(
            &ctx,
            &idx,
            &lane_id,
            &signal_id,
            &waitpoint_id,
            idem_key,
            &eid,
            signal_name,
            "approval".to_owned(),
            crate::constants::SOURCE_TYPE_APPROVAL_OPERATOR.to_owned(),
            crate::constants::SOURCE_IDENTITY.to_owned(),
            String::new(),
            idem_str,
            now,
            self.runtime.config.signal_dedup_ttl_ms,
            crate::constants::DEFAULT_SIGNAL_MAXLEN,
            crate::constants::DEFAULT_MAX_SIGNALS_PER_EXECUTION,
            waitpoint_token.as_str(),
        );

        let key_refs: Vec<&str> = keys.iter().map(|s| s.as_str()).collect();
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let raw: ferriskey::Value = self
            .runtime
            .fcall(crate::fcall::names::FF_DELIVER_SIGNAL, &key_refs, &arg_refs)
            .await
            .map_err(|e| FabricError::Internal(format!("ff_deliver_signal: {e}")))?;

        check_fcall_success(&raw, crate::fcall::names::FF_DELIVER_SIGNAL)?;

        match decision {
            ApprovalDecision::Approved => {
                let record = self.read_run_record(project, session_id, run_id).await?;
                self.bridge
                    .emit(BridgeEvent::ExecutionResumed {
                        run_id: run_id.clone(),
                        project: record.project.clone(),
                        prev_state: Some(RunState::WaitingApproval),
                    })
                    .await;
                Ok(record)
            }
            ApprovalDecision::Rejected => {
                self.fail(project, session_id, run_id, FailureClass::ApprovalRejected)
                    .await
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_duplicate_delegates_to_helpers() {
        let dup = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("DUPLICATE".to_owned())),
        ]);
        assert!(is_duplicate_result(&dup));

        let ok = ferriskey::Value::Array(vec![
            Ok(ferriskey::Value::Int(1)),
            Ok(ferriskey::Value::SimpleString("OK".to_owned())),
        ]);
        assert!(!is_duplicate_result(&ok));
    }
}
