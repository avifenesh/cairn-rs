use std::collections::HashMap;
use std::sync::Arc;

use cairn_domain::*;
use cairn_store::projections::RunRecord;

use crate::error::FabricError;
use flowfabric::core::types::{ExecutionId, LaneId, Namespace};

use crate::boot::FabricRuntime;
use crate::engine::{
    CancelRunInput, CompleteRunInput, ControlPlaneBackend, CreateRunExecutionInput,
    DeliverApprovalSignalInput, Engine, ExecutionLeaseContext, ExecutionSnapshot,
    FailExecutionOutcome, FailRunInput, ResumeRunInput,
};
use crate::event_bridge::{BridgeEvent, EventBridge};
use crate::helpers::{now_ms, try_parse_project_key};
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
    /// Two coherent shapes per RFC #58.5 (`resolve_lease_fence` in
    /// `flowfabric.lua`):
    ///
    /// * **Active lease** — `snapshot.current_lease = Some(_)`:
    ///   populate all three fence tokens (`lease_id`, `lease_epoch`,
    ///   `attempt_id`) from the lease + current attempt. Leave `source`
    ///   empty so FF validates the caller against the stored lease.
    /// * **No active lease** (claim never happened, lease expired, or
    ///   `current_lease_id` cleared by a prior lease release) — emit
    ///   **all three fence tokens empty** and set
    ///   `source = "operator_override"`. Any *mix* of set/empty triggers
    ///   FF's `partial_fence_triple` rejection, which surfaces to the
    ///   user as an opaque `fabric layer error` (F37).
    ///
    /// Cairn is the sole authoritative writer of run-execution lifecycle
    /// events — the orchestrator owns the run end-to-end and no other
    /// process can legitimately complete/fail a cairn run — so the
    /// unfenced `operator_override` path is safe here. FF still
    /// validates the execution is in an active lifecycle phase via
    /// `validate_lease_and_mark_expired`, which catches the real
    /// conflict cases (terminal, revoked, double-completion).
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
            // No lease, or lease without a current attempt — use the
            // unfenced path. FF still validates lifecycle phase via
            // `validate_lease_and_mark_expired`.
            _ => ExecutionLeaseContext::unfenced(lane_id, attempt_index),
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
        self.claim_with_snapshot(project, session_id, run_id, eid, &snapshot)
            .await
    }

    /// Internal helper for the shared claim path: given an already-read
    /// snapshot + execution id, walk `issue_grant_and_claim` and
    /// read the final `RunRecord`.
    ///
    /// Callers that just read the snapshot for another reason (e.g.
    /// `renew_lease_if_stale`'s staleness check, or any future code
    /// path that inspects lease/attempt state before deciding between
    /// claim and renew) should route through this helper instead of
    /// calling `claim` directly, which would re-issue
    /// `describe_execution` and pay an extra FF round-trip on the
    /// hot path.
    async fn claim_with_snapshot(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        eid: flowfabric::core::types::ExecutionId,
        snapshot: &ExecutionSnapshot,
    ) -> Result<RunRecord, FabricError> {
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

    /// Idempotent variant of [`Self::claim`] that activates the run's FF
    /// execution only if it is not already active.
    ///
    /// F41 (2026-04-24): `POST /v1/runs/:id/orchestrate` kicks off the
    /// GATHER → DECIDE → EXECUTE loop, which ends by calling
    /// [`Self::complete`]. FF's `ff_complete_execution` gates on
    /// `lifecycle_phase == "active"` via `validate_lease_and_mark_expired`;
    /// a freshly-created run is in `lifecycle_phase = "runnable"`
    /// (see `ff_create_execution`, cannot be completed without first
    /// transitioning to active. The transition is owned by
    /// `ff_claim_execution` via `issue_grant_and_claim`, but the
    /// orchestrate handler had no claim step — so every dogfood run
    /// terminated with `execution_not_active -> completed` regardless of
    /// whether the LLM produced a valid answer.
    ///
    /// `ensure_active` plugs that gap. On the happy path (execution not
    /// yet claimed) it walks the same `issue_grant_and_claim` sequence as
    /// `claim`. On the already-claimed path (`current_lease.is_some()`)
    /// it short-circuits and returns the current record, so operator
    /// paths that already invoked `POST /v1/runs/:id/claim` before
    /// orchestrating are not double-punished by FF's `grant_already_exists`
    /// contention rejection.
    ///
    /// Lease-expiry note: if the lease TTL expires mid-loop, FF's
    /// `validate_lease_and_mark_expired` clears `current_lease_id` and
    /// subsequent terminal FCALLs will reject with `lease_expired`. That
    /// is a distinct failure mode from this fix; surfacing it as a 409
    /// with an actionable message is handled in
    /// `cairn_app::fabric_adapter::fabric_err_to_runtime`.
    pub async fn ensure_active(
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

        // Already-active guard. `current_lease.is_some()` is derived
        // from a non-empty `current_lease_id` in `exec_core`
        // (`valkey_impl::build_execution_snapshot`). FF's
        // `validate_lease_and_mark_expired` clears that field on expiry,
        // so a stale `Some` means the lease is still within its TTL and
        // terminal FCALLs will accept the unfenced operator-override
        // path that cairn uses.
        if snapshot.current_lease.is_some() {
            return build_run_record(&snapshot, project, run_id);
        }

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

        self.read_run_record(project, session_id, run_id).await
    }

    /// F51 (2026-04-26): extend the lease on a run if it is nearing
    /// expiry, or re-claim if it has no active lease at all.
    ///
    /// # Background
    ///
    /// `POST /v1/runs/:id/orchestrate` is a **pull-model driver**: each
    /// HTTP call runs one GATHER → DECIDE → EXECUTE iteration loop and
    /// returns. Between calls (e.g. while an operator approves a
    /// tool-call, or while a human-paced agent clicks through 5–10
    /// tool-calls in a dashboard), no one is renewing the FF lease.
    ///
    /// The default `CAIRN_FABRIC_LEASE_TTL_MS = 30_000` (30 s) is easily
    /// exceeded by any operator-paced workflow. When the next
    /// `/orchestrate` call arrives the lease is already expired, and the
    /// run's terminal FCALL (`ff_complete_execution`) rejects with
    /// `lease_expired`. The operator-visible symptom was
    /// `{"termination":"failed","reason":"invalid run transition to
    /// completed: ... lease_expired ..."}`.
    ///
    /// # Contract
    ///
    /// * If the run is not tracked in FF (not found) → propagates
    ///   [`FabricError::NotFound`] — same shape as [`Self::claim`] and
    ///   [`Self::ensure_active`].
    /// * If the run has no `current_lease` → walk the same
    ///   `issue_grant_and_claim` path as [`Self::claim`] to re-acquire.
    /// * If the run has a healthy lease (more than `min_remaining_ms`
    ///   left) → no-op, no side effects, no FF call.
    /// * If the run has a stale lease (≤ `min_remaining_ms` left) → call
    ///   `renew_task_lease` (the FF fcall is engine-agnostic and works
    ///   for run executions too) with the full `lease_ttl_ms` from
    ///   [`crate::FabricConfig`]. This extends
    ///   `exec_core.lease_expires_at` without rotating the lease epoch
    ///   or attempt; the lease_history cron remains undisturbed.
    ///
    /// The method is idempotent: back-to-back calls in <1s against a
    /// fresh lease produce zero FF mutations (the snapshot read is the
    /// only cost).
    ///
    /// # Why renew instead of always re-claim?
    ///
    /// Full re-claim rotates `lease_epoch` and mints a new
    /// `attempt_id`, which writes a lease-history row and a
    /// `LeaseClaimedEvent` on every orchestrate call. That's bridge
    /// traffic + audit-log noise proportional to the number of
    /// orchestrate invocations on long-running runs. Renewal is a
    /// single hash-set on `exec_core` with no lease-history write and
    /// preserves the existing fence triple so any terminal FCALL
    /// already in flight is not invalidated.
    pub async fn renew_lease_if_stale(
        &self,
        project: &ProjectKey,
        session_id: &SessionId,
        run_id: &RunId,
        min_remaining_ms: u64,
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

        // No active lease: fall back to a full re-claim so the caller
        // can proceed with terminal FCALLs. This is the "inter-call
        // expiry" case that motivated F51 — FF's expiry scanner has
        // already cleared `current_lease_id`. Reuse the snapshot we
        // just read rather than bouncing through `self.claim`, which
        // would `describe_execution` again.
        let Some(lease) = snapshot.current_lease.as_ref() else {
            return self
                .claim_with_snapshot(project, session_id, run_id, eid.clone(), &snapshot)
                .await;
        };

        // Use the crate-local `now_ms` helper so clock-skew logging is
        // centralized (it warns-and-returns-0 on pre-epoch clocks
        // instead of silently collapsing to Duration::default()).
        let now_ms_i = now_ms() as i64;
        let expires_at_ms = lease.expires_at.0;
        let remaining_ms = expires_at_ms.saturating_sub(now_ms_i);

        // Already-expired-but-not-cleared window: the snapshot still
        // carries `current_lease_id` because FF's expiry scanner runs
        // on its own cadence (see `lease_expiry` sorted-set sweeper)
        // and hasn't rolled this execution forward yet. `ff_renew_lease`
        // would reject with `lease_expired` (same reason a terminal
        // FCALL would), so we can't renew — but a fresh
        // `issue_grant_and_claim` is still legal and mints a new
        // lease + epoch. Reclaim from the snapshot we already have.
        if remaining_ms <= 0 {
            return self
                .claim_with_snapshot(project, session_id, run_id, eid.clone(), &snapshot)
                .await;
        }

        // Fresh enough: no-op. Back-to-back orchestrate calls (<1s)
        // against a 30s lease TTL hit this path and do not mutate FF.
        if remaining_ms > min_remaining_ms as i64 {
            return build_run_record(&snapshot, project, run_id);
        }

        // Stale: renew in place. The fence triple carried forward via
        // `resolve_lease_context` is the SAME one that was minted at
        // claim time (renewal does not rotate it), so any in-flight
        // terminal FCALL that read the triple before this renewal
        // remains valid.
        let lease_ctx = self.resolve_lease_context(&snapshot);
        let renew = self
            .control_plane
            .renew_task_lease(crate::engine::RenewLeaseInput {
                execution_id: eid.clone(),
                lease: lease_ctx,
                lease_extension_ms: self.runtime.config.lease_ttl_ms,
            })
            .await;

        // Race window: the snapshot read and the `ff_renew_lease` FCALL
        // are not atomic. FF's expiry scanner can roll the execution
        // forward between them, or a sibling process could rotate the
        // lease (stale fence). Either case surfaces as a typed Lua
        // rejection (`lease_expired`, `stale_lease`). Without a
        // fallback the response bubbles up as a 500 at the handler
        // — `is_terminal_state_conflict` only classifies terminal
        // FCALLs (`ff_complete`/`ff_fail`/`ff_cancel`), not
        // `ff_renew_lease`. Recover by falling back to a full reclaim,
        // which mints a new lease + epoch and produces the same
        // end-state the caller would have gotten via a slightly-later
        // renew attempt.
        if let Err(err) = renew {
            if is_renew_race_error(&err) {
                tracing::warn!(
                    run_id = %run_id,
                    error = %err,
                    "F51: ff_renew_lease lost the scanner/rotation race; \
                     falling back to full reclaim"
                );
                return self.claim(project, session_id, run_id).await;
            }
            return Err(err);
        }

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

        let case = match reason.kind {
            PauseReasonKind::OperatorPause => crate::suspension::SuspendCase::OperatorPause,
            PauseReasonKind::ToolRequestedSuspension => {
                let invocation_id = reason
                    .detail
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| FabricError::Validation {
                        reason: "ToolRequestedSuspension requires invocation_id in reason.detail"
                            .to_owned(),
                    })?;
                crate::suspension::SuspendCase::ToolRequestedSuspension {
                    invocation_id,
                    resume_after_ms: reason.resume_after_ms,
                }
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
                crate::suspension::SuspendCase::RuntimeSuspension {
                    signal_name,
                    resume_after_ms: reason.resume_after_ms,
                }
            }
            PauseReasonKind::PolicyHold => {
                let detail = reason.detail.as_deref().unwrap_or("policy");
                crate::suspension::SuspendCase::PolicyHold {
                    detail,
                    resume_after_ms: reason.resume_after_ms,
                }
            }
        };

        // FF 0.10 typed surface — build LeaseFence from the
        // authoritative lease context and SuspendArgs from the typed
        // SuspendCase enum. `suspend_by_triple` fences against the
        // triple directly; no Lua-ARGV translation happens on cairn's
        // side. Per FF#322, this is the canonical service-layer entry
        // point for callers that hold a fence triple but no worker
        // `Handle`.
        let fence = crate::suspension::build_lease_fence(&lease)?;
        let args = crate::suspension::build_suspend_args(case);
        crate::suspension::suspend_by_triple(self.runtime.backend(), eid, fence, args).await?;

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

        // FF 0.10 typed surface — approval resume condition is a
        // `CompositeBody::Count { n: 1, DistinctWaitpoints }` over
        // `approval_granted:<run_id>` + `approval_rejected:<run_id>`
        // (either fires a resume). Matches the worker-sdk path's
        // `typed_approval` so approval resume semantics are identical
        // regardless of which code path requested the suspension.
        let fence = crate::suspension::build_lease_fence(&lease)?;
        let args = crate::suspension::build_suspend_args(
            crate::suspension::SuspendCase::WaitingForApproval {
                approval_id: run_id.as_str(),
                timeout_ms: None,
            },
        );
        crate::suspension::suspend_by_triple(self.runtime.backend(), eid, fence, args).await?;

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

/// F51: classify `ff_renew_lease` rejections that are safely
/// recoverable via a full reclaim.
///
/// `ff_renew_lease`'s Lua side runs `validate_lease_and_mark_expired`
/// plus fence-triple validation, which can reject with
/// `lease_expired` (FF's scanner rolled the exec forward between
/// snapshot read and FCALL) or `stale_lease` (a sibling rotated the
/// lease). Both represent a race the caller can cleanly resolve by
/// re-claiming — the execution is still valid, we just lost the
/// lease we thought we held.
///
/// Other errors (connection, partition gate, programming bug) bubble
/// up unchanged.
fn is_renew_race_error(err: &FabricError) -> bool {
    let msg = err.to_string();
    msg.contains("lease_expired") || msg.contains("stale_lease")
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
        // F47 PR2: fabric snapshots are FF-derived and never carry the
        // completion annotation — that lives on cairn-store's event-
        // sourced projection. Callers who want the annotation read
        // from the RunReadModel projection; fabric read-path always
        // reports `None` here.
        completion_summary: None,
        completion_verification: None,
        completion_annotated_at_ms: None,
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
