use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::RunRecord;

use crate::error::FabricError;
use flowfabric::core::types::{ExecutionId, LaneId, Namespace, TimestampMs};

use crate::boot::FabricRuntime;
use crate::engine::{
    CancelRunInput, CompleteRunInput, ControlPlaneBackend, CreateRunExecutionInput,
    DeliverApprovalSignalInput, Engine, ExecutionLeaseContext, ExecutionSnapshot,
    FailExecutionOutcome, FailRunInput, ResumeRunInput, SuspendRunInput,
};
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::try_parse_project_key;
use crate::id_map;
use crate::state_map;

pub struct FabricRunService {
    runtime: Arc<FabricRuntime>,
    bridge: Arc<EventBridge>,
    engine: Arc<dyn Engine>,
    control_plane: Arc<dyn ControlPlaneBackend>,
}

impl FabricRunService {
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

    /// Mint the session-scoped `ExecutionId` for a cairn run.
    ///
    /// Routes through `id_map::session_run_to_execution_id` so every
    /// run of a session co-locates on the session's FlowId partition.
    /// The caller MUST supply the session binding that was used at run
    /// create time; mismatched sessions mint a different ExecutionId
    /// and the lookup misses FF's state entirely.
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

        build_run_record(&snapshot, project, run_id)
    }

    /// Resolve the lease / attempt context needed by lifecycle FCALLs.
    ///
    /// Fills in "cairn" defaults for lane_id / worker_instance_id when
    /// FF hasn't stamped them yet (early-lifecycle runs that haven't
    /// been claimed). Matches the pre-migration behaviour of the
    /// direct `HGETALL exec_core` reads.
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
        let attempt_id = snapshot
            .current_attempt
            .as_ref()
            .map(|a| a.id.to_string())
            .unwrap_or_default();
        let (lease_id, lease_epoch, worker_instance_id) = match &snapshot.current_lease {
            Some(l) => (
                l.lease_id.to_string(),
                l.epoch.0.to_string(),
                flowfabric::core::types::WorkerInstanceId::new(l.owner.as_str()),
            ),
            None => (
                String::new(),
                snapshot
                    .current_lease_epoch
                    .map(|e| e.0.to_string())
                    .unwrap_or_else(|| "1".to_owned()),
                flowfabric::core::types::WorkerInstanceId::new("cairn"),
            ),
        };
        ExecutionLeaseContext {
            lane_id,
            attempt_index,
            lease_id,
            lease_epoch,
            attempt_id,
            worker_instance_id,
        }
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
        let eid = self.execution_id(project, session_id, &run_id);
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
        // Cross-instance isolation — see task_service::submit for why
        // this tag is mandatory on every execution cairn creates.
        tags.insert(
            "cairn.instance_id".to_owned(),
            self.runtime.config.worker_instance_id.to_string(),
        );
        if let Some(parent) = parent_run_id.as_ref() {
            tags.insert("cairn.parent_run_id".to_owned(), parent.as_str().to_owned());
        }
        if let Some(corr) = correlation_id.filter(|s| !s.is_empty()) {
            tags.insert("cairn.correlation_id".to_owned(), corr.to_owned());
        }

        let policy_json = serde_json::json!({
            "max_retries": 0
        })
        .to_string();

        let outcome = self
            .control_plane
            .create_run_execution(CreateRunExecutionInput {
                execution_id: eid,
                namespace,
                lane_id,
                tags,
                policy_json,
            })
            .await?;

        if outcome.newly_created {
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
    /// Runs are not FF-scheduled in the worker sense, but FF's
    /// suspension / signal FCALLs reject non-active executions.
    /// Approval gates (`enter_waiting_approval` → `resolve_approval`)
    /// therefore need an explicit claim first.
    ///
    /// This is the run-side mirror of [`FabricTaskService::claim`]. Both
    /// delegate to [`ControlPlaneBackend::issue_grant_and_claim`]; neither
    /// caches lease state — downstream FCALLs re-read the lease triple
    /// from the execution snapshot on demand.
    pub async fn claim(
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
        let lane_id = if snapshot.lane_id.as_str().is_empty() {
            self.lane_id(project)
        } else {
            snapshot.lane_id.clone()
        };

        let _outcome = self
            .control_plane
            .issue_grant_and_claim(crate::engine::IssueGrantAndClaimInput {
                execution_id: eid,
                lane_id,
                lease_duration_ms: self.runtime.config.lease_ttl_ms,
            })
            .await?;

        // No cairn-side cache. `enter_waiting_approval`, `fail`, `cancel`,
        // etc. all read the lease triple back from the snapshot directly.
        self.read_run_record(project, session_id, run_id).await
    }

    pub async fn complete(
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
        let prev_state = ff_public_state_to_run_state(&snapshot);
        let lease = self.resolve_lease_context(&snapshot);

        self.control_plane
            .complete_run_execution(CompleteRunInput {
                execution_id: eid,
                lease,
            })
            .await?;

        let record = self.read_run_record(project, session_id, run_id).await?;
        self.bridge
            .emit(BridgeEvent::ExecutionCompleted {
                run_id: run_id.clone(),
                project: record.project.clone(),
                prev_state: Some(prev_state),
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
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                })?;
        let prev_state = ff_public_state_to_run_state(&snapshot);
        let lease = self.resolve_lease_context(&snapshot);

        let category = crate::state_map::failure_class_category(failure_class);
        let reason = crate::state_map::failure_class_reason(failure_class);

        // Empty `retry_policy_json` signals "backend reads FF's
        // `exec_policy` GET key itself". Cairn never caches retry
        // policy on the service side; pre-migration code did a direct
        // `GET ctx.policy()` which is now encapsulated in the backend.
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

        let record = self.read_run_record(project, session_id, run_id).await?;
        if outcome == FailExecutionOutcome::TerminalFailed {
            self.bridge
                .emit(BridgeEvent::ExecutionFailed {
                    run_id: run_id.clone(),
                    project: record.project.clone(),
                    failure_class,
                    prev_state: Some(prev_state),
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
        let eid = self.execution_id(project, session_id, run_id);
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                })?;
        let prev_state = ff_public_state_to_run_state(&snapshot);
        let lease = self.resolve_lease_context(&snapshot);
        let current_waitpoint = snapshot.current_waitpoint.clone();

        self.control_plane
            .cancel_run_execution(CancelRunInput {
                execution_id: eid,
                lease,
                current_waitpoint,
            })
            .await?;

        let record = self.read_run_record(project, session_id, run_id).await?;
        self.bridge
            .emit(BridgeEvent::ExecutionCancelled {
                run_id: run_id.clone(),
                project: record.project.clone(),
                prev_state: Some(prev_state),
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
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                })?;
        let prev_state = ff_public_state_to_run_state(&snapshot);
        let lease = self.resolve_lease_context(&snapshot);

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

        // Pause: use the pre-migration rule — single matcher resumes
        // on ANY signal, multi-matcher requires ALL to arrive.
        let match_mode = if params.condition_matchers.len() <= 1 {
            "any"
        } else {
            "all"
        };
        let suspend_input = build_suspend_input(eid, lease, &params, match_mode);

        self.control_plane
            .suspend_run_execution(suspend_input)
            .await?;

        // Emit unconditionally — see the detailed retry-safety rationale
        // preserved in `enter_waiting_approval`. Projection is idempotent
        // on EventId so a duplicate emit on replay is a harmless
        // re-write, and the fallback state avoids a permanent projection
        // gap when `read_run_record` fails after the FCALL committed.
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
                prev_state: Some(prev_state),
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
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                })?;
        let prev_state = ff_public_state_to_run_state(&snapshot);
        let lane_id = if snapshot.lane_id.as_str().is_empty() {
            LaneId::new("cairn")
        } else {
            snapshot.lane_id.clone()
        };

        self.control_plane
            .resume_run_execution(ResumeRunInput {
                execution_id: eid,
                lane_id,
                waitpoint_id: snapshot.current_waitpoint.clone(),
                resume_source: "operator".to_owned(),
            })
            .await?;

        let record = self.read_run_record(project, session_id, run_id).await?;
        self.bridge
            .emit(BridgeEvent::ExecutionResumed {
                run_id: run_id.clone(),
                project: record.project.clone(),
                prev_state: Some(prev_state),
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
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                })?;
        let prev_state = ff_public_state_to_run_state(&snapshot);
        let lease = self.resolve_lease_context(&snapshot);

        // Approval waitpoints listen for EITHER approval_granted or
        // approval_rejected — `match_mode=any`. (Pre-migration this
        // was hardcoded inside `enter_waiting_approval`.)
        let suspend_input = build_suspend_input(eid, lease, &params, "any");
        self.control_plane
            .suspend_run_execution(suspend_input)
            .await?;

        // Emit unconditionally. See `pause` for retry-safety rationale.
        // If `read_run_record` fails after the FCALL committed, emit
        // with `WaitingApproval` (the blocking_reason we just set) as
        // the fallback so the projection still gets the state change.
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
                prev_state: Some(prev_state),
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
        let snapshot =
            self.engine
                .describe_execution(&eid)
                .await?
                .ok_or_else(|| FabricError::NotFound {
                    entity: "run",
                    id: run_id.to_string(),
                })?;

        let lane_id = if snapshot.lane_id.as_str().is_empty() {
            LaneId::new("cairn")
        } else {
            snapshot.lane_id.clone()
        };

        // Validation boundary: if the run isn't awaiting approval, surface
        // that as a state error at the state layer. Otherwise callers
        // hitting double-approve or approve-on-unsuspended-run would get
        // a misleading HMAC-flavored `invalid_token` from FF.
        let waitpoint_id = match snapshot.current_waitpoint.clone() {
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
        let idem_suffix = format!("approval:{}", run_id.as_str());

        self.control_plane
            .deliver_approval_signal(DeliverApprovalSignalInput {
                execution_id: eid,
                lane_id,
                waitpoint_id,
                signal_name,
                idempotency_suffix: idem_suffix,
                signal_dedup_ttl_ms: self.runtime.config.signal_dedup_ttl_ms,
                maxlen: crate::constants::DEFAULT_SIGNAL_MAXLEN_U64,
                max_signals_per_execution: crate::constants::DEFAULT_MAX_SIGNALS_PER_EXECUTION_U64,
            })
            .await?;

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

/// Derive a cairn `RunState` from a snapshot's FF public state, applying
/// the blocking-reason adjustment so states like "waiting for approval"
/// surface correctly.
fn ff_public_state_to_run_state(snapshot: &ExecutionSnapshot) -> RunState {
    let public_state = crate::helpers::parse_public_state(&snapshot.public_state);
    let (run_state, _) = state_map::ff_public_state_to_run_state(public_state);
    state_map::adjust_run_state_for_blocking_reason(
        run_state,
        snapshot.blocking_reason.as_deref().unwrap_or_default(),
    )
}

/// Build the typed `SuspendRunInput` from a suspension params bundle.
///
/// `match_mode` is caller-selected — approval waitpoints (two matchers:
/// `approval_granted:<id>` / `approval_rejected:<id>`) resume on
/// EITHER, pause waitpoints resume on ALL of their required signals.
/// Pre-migration, `enter_waiting_approval` hardcoded `"any"` while
/// `pause` used `len <= 1 ? "any" : "all"`; merging both through this
/// helper without threading `match_mode` would silently convert the
/// approval case to AND and leak it as the suspended-after-resume
/// regression caught by `test_signal_delivery_resumes_waiter`.
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
            let now = TimestampMs::now().0;
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

fn build_run_record(
    snapshot: &ExecutionSnapshot,
    project: &ProjectKey,
    run_id: &RunId,
) -> Result<RunRecord, FabricError> {
    let public_state = crate::helpers::parse_public_state(&snapshot.public_state);
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

#[cfg(test)]
mod tests {
    use crate::helpers::is_duplicate_result;

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
