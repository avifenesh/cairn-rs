//! RFC 020 Track 1 — run-level recovery on cairn-app startup.
//!
//! Owned responsibility: run-level state recovery. FF's 14 background
//! scanners own operational recovery (lease expiry, attempt timeouts,
//! dependency reconciliation, …) and run continuously, so this service
//! does **not** touch any of that.
//!
//! On every cairn-app boot, [`RecoveryServiceImpl::recover_all`] runs once
//! between `SandboxService::recover_all` and the readiness-gate flip. It
//! enumerates non-terminal runs, applies the RFC 020 "Run recovery matrix",
//! and emits:
//!
//! * `RecoveryAttempted { boot_id, run_id, reason }` — one per scanned run,
//! * `RecoveryCompleted { boot_id, run_id, recovered }` — outcome marker,
//! * `RunStateChanged`  — only when recovery legitimately advances state
//!   (e.g. an approval resolved during the crash window, or a wedged run
//!   needs to fail out).
//!
//! All state changes happen via appended events; the orchestrator re-reads
//! the latest projection on next tick. The service is stateless — it holds
//! nothing but an `Arc<S>` to the store.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use cairn_domain::{
    ApprovalDecision, ApprovalId, ApprovalRequested, ApprovalRequirement, BootId, FailureClass,
    ProjectKey, ResumeTrigger, RunRecoveryOutcome, RunRecoverySummary, RunState, RunStateChanged,
    RuntimeEvent, StateTransition,
};
use cairn_domain::{RecoveryAttempted, RecoveryCompleted, RecoverySummaryEmitted};
use cairn_store::projections::{
    ApprovalReadModel, CheckpointReadModel, CheckpointRecord, RunReadModel, RunRecord,
};
use cairn_store::EventLog;

use super::event_helpers::make_envelope;
use crate::error::RuntimeError;

/// Runs in the `Running` state with no recent progress for more than
/// `RUN_WEDGE_THRESHOLD_MS` are treated as wedged (crashed before any
/// checkpoint or user message was appended). Tuneable; RFC 020 Gap 10 picks
/// 5 min as short enough to ignore legitimate startup latency, long enough
/// to avoid flagging just-started runs.
const RUN_WEDGE_THRESHOLD_MS: u64 = 300_000;

/// Upper bound on non-terminal runs fetched per projection lookup. Recovery
/// is best-effort bounded: a pathological backlog won't stall startup here,
/// the next boot will pick up any stragglers.
const RECOVERY_SCAN_LIMIT: usize = 10_000;

/// Page size used when paginating approvals for a project. Large enough to
/// keep the total round-trip count small on realistic workloads (dozens of
/// approvals per project are typical), small enough that a pathological
/// project doesn't balloon memory in a single `list_all` call.
const APPROVAL_PAGE_SIZE: usize = 500;

/// Hard cap on the total number of approvals scanned per project during
/// recovery. If a project truly has more resolved approvals than this in
/// its history, running recovery is the wrong tool to reconcile it anyway
/// — the operator needs a backfill. Tuneable; 50_000 is ~100 pages of
/// round-trips which still completes in seconds against a local SQLite or
/// a well-provisioned Postgres.
const APPROVAL_SCAN_LIMIT: usize = 50_000;

/// RFC 020: run-level recovery on cairn-app startup.
///
/// Enumerates non-terminal runs from the run projection, applies the RFC 020
/// "Run recovery matrix", and emits `RecoveryAttempted`/`RecoveryCompleted`
/// (plus any legitimate `RunStateChanged`) so the orchestrator can pick the
/// run up cleanly on its next tick.
///
/// Implementations must be idempotent: calling `recover_all` twice for the
/// same run across two boots must not duplicate state transitions beyond
/// what the projection already reflects.
/// Runs whose bound sandbox has been detected as missing at boot.
///
/// Produced by `SandboxService::recover_all` (the workspace layer knows
/// about filesystems), consumed by [`RecoveryService::recover_all`] (the
/// runtime layer knows about run lifecycles). Each entry MUST cause the
/// run-recovery sweep to transition the bound run to `Failed` with
/// `reason: sandbox_lost` per RFC 020 §"Run recovery matrix" row
/// "Running / Sandbox missing".
pub type SandboxLostRun = (cairn_domain::RunId, cairn_domain::ProjectKey);

/// Runs whose bound sandbox is preserved because the project's repo
/// allowlist no longer authorises the sandbox's repo.
///
/// Produced by `SandboxService::recover_all` (the workspace layer knows
/// about allowlist state), consumed by [`RecoveryService::recover_all`]
/// (the runtime layer synthesises the operator approval). Each entry MUST
/// cause the run-recovery sweep to transition the bound run to
/// `WaitingApproval` with a fresh `ApprovalRequested{requirement:Required}`
/// asking the operator to re-grant or cancel, per RFC 020 §"Run recovery
/// matrix" row "Running / Sandbox preserved: AllowlistRevoked".
///
/// The third element carries the canonical `owner/repo` string rather
/// than a workspace-crate `RepoId` so this type stays a pure
/// `cairn-runtime` concept; `cairn-app` passes it through without
/// needing to import the workspace-scoped newtype here.
pub type AllowlistRevokedRun = (
    cairn_domain::RunId,
    cairn_domain::ProjectKey,
    /* repo_id = */ String,
);

/// Runs whose bound sandbox survived the crash cleanly — registry
/// sidecar + on-disk root are both intact and the repo binding (if
/// any) is still allowlisted.
///
/// Produced by `SandboxService::recover_all`, consumed by
/// [`RecoveryService::recover_all`]. Each entry MUST cause the
/// run-recovery sweep to emit
/// `RecoveryAttempted { reason: "sandbox_reattached" }` +
/// `RecoveryCompleted { recovered: true }` for audit-trail symmetry
/// with the sandbox-lost and allowlist-revoked rows. No state
/// transition: the run stays in its existing non-terminal state and
/// the orchestrator resumes it on its next tick via
/// `provision_or_reconnect`.
pub type SandboxReattachedRun = (cairn_domain::RunId, cairn_domain::ProjectKey);

#[async_trait]
pub trait RecoveryService: Send + Sync {
    /// Sweep every non-terminal run and emit the appropriate recovery events.
    ///
    /// Called once per cairn-app boot, *after* `SandboxService::recover_all`
    /// and *before* the readiness gate flips to `200`. `boot_id` is threaded
    /// into every emitted event for audit-trail correlation. On error,
    /// startup MUST halt — cairn-app is not a durable system if it serves
    /// traffic with unknown run state.
    ///
    /// `sandbox_lost_runs` carries the `(run_id, project)` pairs whose
    /// sandboxes were detected as missing during the preceding sandbox
    /// recovery sweep. Default implementations MUST transition each such
    /// run to `Failed` with `reason: sandbox_lost` before returning.
    ///
    /// `allowlist_revoked_runs` carries `(run_id, project, repo_id)`
    /// triples whose bound sandboxes are preserved because the project
    /// allowlist no longer authorises the bound repo. Default
    /// implementations MUST transition each such run to `WaitingApproval`
    /// with a synthesized `ApprovalRequested{requirement:Required}`
    /// asking the operator to re-grant or cancel.
    async fn recover_all(
        &self,
        boot_id: &BootId,
        sandbox_lost_runs: &[SandboxLostRun],
        allowlist_revoked_runs: &[AllowlistRevokedRun],
        sandbox_reattached_runs: &[SandboxReattachedRun],
    ) -> Result<RunRecoverySummary, RuntimeError>;
}

/// Stateless RFC 020 Track 1 recovery service.
///
/// All state read from projections; no cache, no locks, no background
/// threads. Multi-instance correctness is out of scope for v1 (RFC 020 delta
/// Gap 2 — deferred to a future multi-node RFC).
pub struct RecoveryServiceImpl<S> {
    store: Arc<S>,
}

impl<S> RecoveryServiceImpl<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl<S> RecoveryService for RecoveryServiceImpl<S>
where
    S: EventLog + RunReadModel + CheckpointReadModel + ApprovalReadModel + 'static,
{
    async fn recover_all(
        &self,
        boot_id: &BootId,
        sandbox_lost_runs: &[SandboxLostRun],
        allowlist_revoked_runs: &[AllowlistRevokedRun],
        sandbox_reattached_runs: &[SandboxReattachedRun],
    ) -> Result<RunRecoverySummary, RuntimeError> {
        // RFC 020 Track 4 — timer for the per-boot RecoverySummary audit event.
        let sweep_started_ms = current_unix_ms();
        let mut summary = RunRecoverySummary {
            boot_id: Some(boot_id.as_str().to_owned()),
            ..Default::default()
        };

        // RFC 020 §"Run recovery matrix" — sandbox_lost rows are handled
        // up front so the later Running-state sweep below does not observe
        // the same runs in a state we're about to mutate. Each row becomes
        // a `RecoveryAttempted{reason:"sandbox_lost"}` + `RunStateChanged
        // → Failed` + `RecoveryCompleted{recovered:false}` triple so the
        // audit trail is symmetric with the normal Running-run paths.
        let mut sandbox_lost_events: Vec<cairn_domain::EventEnvelope<RuntimeEvent>> = Vec::new();
        let mut sandbox_lost_handled: std::collections::HashSet<cairn_domain::RunId> =
            std::collections::HashSet::new();
        for (run_id, project) in sandbox_lost_runs {
            // De-duplicate: a registry entry that was already processed in
            // an earlier recovery sweep (e.g. on a previous boot that
            // crashed before persisting) might appear twice. The sandbox
            // service clears the registry entry after emitting
            // `SandboxLost`, but belt-and-braces the pair here too.
            if !sandbox_lost_handled.insert(run_id.clone()) {
                continue;
            }
            let current = RunReadModel::get(self.store.as_ref(), run_id).await?;
            let from_state = current.as_ref().map(|r| r.state);
            sandbox_lost_events.push(make_envelope(RuntimeEvent::RecoveryAttempted(
                RecoveryAttempted {
                    project: project.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    reason: "sandbox_lost".to_owned(),
                    boot_id: Some(boot_id.as_str().to_owned()),
                },
            )));
            // Only emit the transition if the run is not already terminal.
            // A sandbox can go missing after the run is already complete
            // (operator tidy-up on an archived project) — in that case the
            // advisory `RecoveryAttempted` above is the full audit trail
            // and we must not violate the run state machine.
            if let Some(from) = from_state {
                if !from.is_terminal()
                    && cairn_domain::can_transition_run_state(from, RunState::Failed)
                {
                    sandbox_lost_events.push(make_envelope(RuntimeEvent::RunStateChanged(
                        RunStateChanged {
                            project: project.clone(),
                            run_id: run_id.clone(),
                            transition: StateTransition {
                                from: Some(from),
                                to: RunState::Failed,
                            },
                            failure_class: Some(FailureClass::ExecutionError),
                            pause_reason: None,
                            resume_trigger: None,
                        },
                    )));
                    summary.failed_runs += 1;
                    summary.outcomes.push(RunRecoveryOutcome::Failed {
                        run_id: run_id.clone(),
                        reason: "sandbox_lost".to_owned(),
                    });
                }
            }
            sandbox_lost_events.push(make_envelope(RuntimeEvent::RecoveryCompleted(
                RecoveryCompleted {
                    project: project.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    recovered: false,
                    boot_id: Some(boot_id.as_str().to_owned()),
                },
            )));
        }

        // RFC 020 §"Run recovery matrix" — `AllowlistRevoked` row. For
        // each `(run, project, repo)` triple, emit:
        //   1. `RecoveryAttempted{reason:"sandbox_allowlist_revoked"}`
        //   2. `ApprovalRequested{requirement:Required}` synthesizing an
        //      approval for the operator — the sandbox is preserved on
        //      disk but the repo binding needs re-grant before the run
        //      can resume.
        //   3. `RunStateChanged(Running → WaitingApproval)` — only when
        //      the run is currently Running (the state machine permits
        //      this transition; see `can_transition_run_state`). A run
        //      that is already Failed / terminal gets the audit trail
        //      but not the transition.
        //   4. `RecoveryCompleted{recovered:true}` — the sandbox is
        //      preserved and the run is gated on operator action, which
        //      is a well-defined state per the matrix.
        //
        // The approval_id is derived deterministically from `run_id` so
        // that re-running recovery (e.g. a boot that crashed before its
        // event append committed) does not synthesise a duplicate
        // approval — the store's approval projection upserts on
        // `approval_id`.
        let mut allowlist_revoked_handled: std::collections::HashSet<cairn_domain::RunId> =
            std::collections::HashSet::new();
        for (run_id, project, repo_id) in allowlist_revoked_runs {
            if !allowlist_revoked_handled.insert(run_id.clone()) {
                continue;
            }
            // Guard against the same run appearing in both sandbox_lost
            // and allowlist_revoked lists — only possible on a badly
            // seeded test, but defend against it anyway. A run that is
            // already being failed out for sandbox_lost must not also
            // receive an approval for a repo it will never touch.
            if sandbox_lost_handled.contains(run_id) {
                continue;
            }
            let current = RunReadModel::get(self.store.as_ref(), run_id).await?;
            let from_state = current.as_ref().map(|r| r.state);
            sandbox_lost_events.push(make_envelope(RuntimeEvent::RecoveryAttempted(
                RecoveryAttempted {
                    project: project.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    reason: format!(
                        "sandbox_allowlist_revoked: repo {repo_id} no longer allowlisted; \
                         synthesizing approval for operator re-grant",
                    ),
                    boot_id: Some(boot_id.as_str().to_owned()),
                },
            )));
            // Synthesise the approval. The title/description explain the
            // situation so the operator UI renders something actionable.
            let approval_id =
                ApprovalId::new(format!("approval-allowlist-revoked-{}", run_id.as_str()));
            sandbox_lost_events.push(make_envelope(RuntimeEvent::ApprovalRequested(
                ApprovalRequested {
                    project: project.clone(),
                    approval_id,
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    requirement: ApprovalRequirement::Required,
                    title: Some(format!("Re-grant repo access: {repo_id}",)),
                    description: Some(format!(
                        "Run `{}` was paused at boot because its sandbox is bound to \
                         repo `{}`, which is no longer on this project's allowlist. \
                         Approve to re-grant and resume, or reject to cancel the run.",
                        run_id.as_str(),
                        repo_id,
                    )),
                },
            )));
            if let Some(from) = from_state {
                if !from.is_terminal()
                    && cairn_domain::can_transition_run_state(from, RunState::WaitingApproval)
                {
                    sandbox_lost_events.push(make_envelope(RuntimeEvent::RunStateChanged(
                        RunStateChanged {
                            project: project.clone(),
                            run_id: run_id.clone(),
                            transition: StateTransition {
                                from: Some(from),
                                to: RunState::WaitingApproval,
                            },
                            failure_class: None,
                            pause_reason: None,
                            resume_trigger: None,
                        },
                    )));
                    summary.advanced_runs += 1;
                    summary.outcomes.push(RunRecoveryOutcome::Advanced {
                        run_id: run_id.clone(),
                    });
                }
            }
            sandbox_lost_events.push(make_envelope(RuntimeEvent::RecoveryCompleted(
                RecoveryCompleted {
                    project: project.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    recovered: true,
                    boot_id: Some(boot_id.as_str().to_owned()),
                },
            )));
        }

        // RFC 020 §"Run recovery matrix" — sandbox_reattached row. For
        // every `(run, project)` pair whose bound sandbox survived the
        // crash cleanly, emit an advisory `RecoveryAttempted{reason:
        // "sandbox_reattached"}` so the audit trail records the fact
        // that the reattach sweep picked this run up. We deliberately
        // do NOT emit a matching `RecoveryCompleted` or push a
        // `RunRecoveryOutcome` entry here: the run is still
        // non-terminal in the projection, and the normal Running /
        // WaitingApproval / Paused / WaitingDependency sweep below
        // must still see it so wedge detection (Cursor Bugbot
        // medium-1) and approval-crash-window advancement can run.
        // The `RecoveryCompleted` event is emitted by that sweep's
        // `plan_for_run` path, which is also where the outcome
        // accounting happens. The advisory `RecoveryAttempted`
        // emitted here simply adds a reason-keyed breadcrumb on top
        // of whatever reason string `plan_for_run` generates.
        //
        // Guard against a badly-seeded workspace sweep that lists the
        // same run under both `sandbox_lost_runs` / `allowlist_
        // revoked_runs` and `sandbox_reattached_runs` — those rows
        // mutate state, and emitting a reattach breadcrumb after the
        // Running → Failed / Running → WaitingApproval transition
        // would falsely imply the run is still healthy.
        let mut sandbox_reattached_handled: std::collections::HashSet<cairn_domain::RunId> =
            std::collections::HashSet::new();
        for (run_id, project) in sandbox_reattached_runs {
            if !sandbox_reattached_handled.insert(run_id.clone()) {
                continue;
            }
            if sandbox_lost_handled.contains(run_id) || allowlist_revoked_handled.contains(run_id) {
                continue;
            }
            sandbox_lost_events.push(make_envelope(RuntimeEvent::RecoveryAttempted(
                RecoveryAttempted {
                    project: project.clone(),
                    run_id: Some(run_id.clone()),
                    task_id: None,
                    reason: "sandbox_reattached".to_owned(),
                    boot_id: Some(boot_id.as_str().to_owned()),
                },
            )));
        }

        // Enumerate every non-terminal run state listed in RFC 020's matrix.
        // `Pending` is intentionally excluded — a run that never transitioned
        // out of Pending has no side-effects to reassert.
        let mut runs: Vec<RunRecord> = Vec::new();
        for state in [
            RunState::Running,
            RunState::WaitingApproval,
            RunState::Paused,
            RunState::WaitingDependency,
        ] {
            let batch =
                RunReadModel::list_by_state(self.store.as_ref(), state, RECOVERY_SCAN_LIMIT)
                    .await?;
            runs.extend(batch);
        }

        let now_ms = current_unix_ms();

        // Per-project cache of resolved approvals, lazily populated when the
        // first WaitingApproval run in a given project needs it. Avoids the
        // obvious N+1 of calling `ApprovalReadModel::list_all(project, …)`
        // once per run. `ApprovalReadModel` does not expose a by-run list
        // method and adding one here would pull an otherwise-unrelated
        // projection change into this PR; the per-project cache has
        // equivalent behaviour without touching the store trait.
        let mut resolved_approvals_by_project: std::collections::HashMap<
            cairn_domain::ProjectKey,
            Vec<cairn_store::projections::ApprovalRecord>,
        > = std::collections::HashMap::new();

        // Accumulate every planned event into one buffer and append the
        // lot in a single `EventLog::append`. On a boot with thousands of
        // non-terminal runs this turns what would be N round-trips to
        // Postgres/SQLite into one atomic append, bounding startup latency.
        let mut all_events = sandbox_lost_events;
        // Skip runs that we already transitioned via the sandbox_lost
        // handler above — their state has been staged in `all_events` but
        // the projection `runs` vector still reflects pre-transition state.
        // NOTE: reattached runs are NOT filtered out here (Cursor
        // Bugbot medium-1). Unlike the sandbox-lost and
        // allowlist-revoked handlers which mutate run state and
        // require the scan to skip them (to avoid observing stale
        // projection state), the reattach handler is advisory only —
        // it emits a reason-keyed breadcrumb but does not touch the
        // run. Filtering reattached runs from the scan would suppress
        // wedge detection (Running >5min with no checkpoint → Failed)
        // and approval-crash-window advancement (WaitingApproval with
        // an approval resolved during the crash window), both of
        // which are run-state mutations this sweep owns.
        let runs: Vec<RunRecord> = runs
            .into_iter()
            .filter(|r| {
                !sandbox_lost_handled.contains(&r.run_id)
                    && !allowlist_revoked_handled.contains(&r.run_id)
            })
            .collect();
        // `scanned_runs` must reflect everything recovery touched this
        // boot — the non-terminal runs about to be planned, plus the
        // sandbox-lost runs we already transitioned to Failed, plus the
        // allowlist-revoked runs we already transitioned to
        // WaitingApproval. Undercounting these in the `scanned=` log
        // line would understate recovery work for operators.
        // (Cursor Bugbot low-1.)
        summary.scanned_runs = (runs.len() as u32)
            .saturating_add(summary.failed_runs)
            .saturating_add(summary.advanced_runs);
        for run in runs {
            let plan = self
                .plan_for_run(&run, boot_id, now_ms, &mut resolved_approvals_by_project)
                .await?;
            all_events.extend(plan.events);
            match &plan.outcome {
                RunRecoveryOutcome::Recovered { .. } => summary.recovered_runs += 1,
                RunRecoveryOutcome::Advanced { .. } => summary.advanced_runs += 1,
                RunRecoveryOutcome::Failed { .. } => summary.failed_runs += 1,
            }
            summary.outcomes.push(plan.outcome);
        }

        if !all_events.is_empty() {
            self.store.append(&all_events).await?;
        }

        // RFC 020 Track 4 — emit the once-per-boot `RecoverySummary` audit
        // event. Branch counts Track 4 cannot populate directly (sandboxes,
        // graph, memory, trigger, webhook dedup) default to 0; sibling
        // recovery services (task #163 sandbox_lost, task #166 decision
        // cache) populate their own counters when they land. This emission
        // provides the audit-trail anchor operators correlate with
        // `RecoveryAttempted`/`RecoveryCompleted` pairs for this `boot_id`.
        let now_ms = current_unix_ms();
        let summary_event = RuntimeEvent::RecoverySummaryEmitted(RecoverySummaryEmitted {
            sentinel_project: ProjectKey::new("_system", "_recovery", "_boot"),
            boot_id: boot_id.as_str().to_owned(),
            recovered_runs: summary.recovered_runs,
            // Track 4 only observes runs directly. Tasks, sandboxes, etc.
            // are owned by sibling services; fill their counts as they land.
            recovered_tasks: 0,
            recovered_sandboxes: 0,
            preserved_sandboxes: 0,
            orphaned_sandboxes_cleaned: 0,
            decision_cache_entries: 0,
            stale_pending_cleared: 0,
            tool_result_cache_entries: 0,
            memory_projection_entries: 0,
            graph_nodes_recovered: 0,
            graph_edges_recovered: 0,
            webhook_dedup_entries: 0,
            trigger_projections: 0,
            startup_ms: now_ms.saturating_sub(sweep_started_ms),
            summary_at_ms: now_ms,
        });
        self.store.append(&[make_envelope(summary_event)]).await?;

        Ok(summary)
    }
}

/// Events + outcome produced for one run by the matrix.
struct RunRecoveryPlan {
    events: Vec<cairn_domain::EventEnvelope<RuntimeEvent>>,
    outcome: RunRecoveryOutcome,
}

impl<S> RecoveryServiceImpl<S>
where
    S: EventLog + RunReadModel + CheckpointReadModel + ApprovalReadModel + 'static,
{
    /// Apply the RFC 020 §"Run recovery matrix" to a single run and return
    /// the events to append + the outcome to record on the summary.
    async fn plan_for_run(
        &self,
        run: &RunRecord,
        boot_id: &BootId,
        now_ms: u64,
        resolved_approvals_by_project: &mut std::collections::HashMap<
            cairn_domain::ProjectKey,
            Vec<cairn_store::projections::ApprovalRecord>,
        >,
    ) -> Result<RunRecoveryPlan, RuntimeError> {
        let latest_checkpoint =
            CheckpointReadModel::latest_for_run(self.store.as_ref(), &run.run_id).await?;
        let boot_id_str = boot_id.as_str().to_owned();

        match run.state {
            RunState::Running => self
                .plan_running(run, boot_id_str, latest_checkpoint, now_ms)
                .await
                .map(Ok)?,
            RunState::WaitingApproval => self
                .plan_waiting_approval(
                    run,
                    boot_id_str,
                    latest_checkpoint,
                    resolved_approvals_by_project,
                )
                .await
                .map(Ok)?,
            RunState::Paused | RunState::WaitingDependency => Ok(plan_unchanged(
                run,
                boot_id_str,
                latest_checkpoint,
                "state unchanged; recovery advisory only",
            )),
            // Enumerate-by-state only queries non-terminal states above, so
            // terminal runs shouldn't reach here. Guard defensively.
            other => Ok(RunRecoveryPlan {
                events: Vec::new(),
                outcome: RunRecoveryOutcome::Failed {
                    run_id: run.run_id.clone(),
                    reason: format!("unexpected terminal-looking state in sweep: {other:?}"),
                },
            }),
        }
    }

    async fn plan_running(
        &self,
        run: &RunRecord,
        boot_id_str: String,
        latest_checkpoint: Option<CheckpointRecord>,
        now_ms: u64,
    ) -> Result<RunRecoveryPlan, RuntimeError> {
        // Wedge detection (RFC 020 Gap 10): Running with no checkpoint AND
        // no recent progress for >5 min → fail out with ExecutionError so
        // the run surfaces to the operator instead of hanging forever.
        let wedged = latest_checkpoint.is_none()
            && now_ms.saturating_sub(run.updated_at) > RUN_WEDGE_THRESHOLD_MS;

        if wedged {
            let reason = "crashed_before_first_progress".to_owned();
            let mut events = Vec::with_capacity(3);
            events.push(make_envelope(RuntimeEvent::RecoveryAttempted(
                RecoveryAttempted {
                    project: run.project.clone(),
                    run_id: Some(run.run_id.clone()),
                    task_id: None,
                    reason: format!("wedged running run: {reason}"),
                    boot_id: Some(boot_id_str.clone()),
                },
            )));
            if cairn_domain::can_transition_run_state(run.state, RunState::Failed) {
                events.push(make_envelope(RuntimeEvent::RunStateChanged(
                    RunStateChanged {
                        project: run.project.clone(),
                        run_id: run.run_id.clone(),
                        transition: StateTransition {
                            from: Some(run.state),
                            to: RunState::Failed,
                        },
                        failure_class: Some(FailureClass::ExecutionError),
                        pause_reason: None,
                        resume_trigger: None,
                    },
                )));
            }
            events.push(make_envelope(RuntimeEvent::RecoveryCompleted(
                RecoveryCompleted {
                    project: run.project.clone(),
                    run_id: Some(run.run_id.clone()),
                    task_id: None,
                    recovered: false,
                    boot_id: Some(boot_id_str),
                },
            )));
            return Ok(RunRecoveryPlan {
                events,
                outcome: RunRecoveryOutcome::Failed {
                    run_id: run.run_id.clone(),
                    reason,
                },
            });
        }

        // Default Running path: advisory recovery — emit marker events so
        // the audit trail records the boot. No state change; orchestrator
        // re-reads checkpoint on next tick and decides resume semantics
        // per RFC 020 §"Checkpoint recovery rules" (Intent vs Result).
        Ok(plan_unchanged(
            run,
            boot_id_str,
            latest_checkpoint,
            "running run re-asserted; orchestrator to resume from latest checkpoint",
        ))
    }

    async fn plan_waiting_approval(
        &self,
        run: &RunRecord,
        boot_id_str: String,
        latest_checkpoint: Option<CheckpointRecord>,
        resolved_approvals_by_project: &mut std::collections::HashMap<
            cairn_domain::ProjectKey,
            Vec<cairn_store::projections::ApprovalRecord>,
        >,
    ) -> Result<RunRecoveryPlan, RuntimeError> {
        // RFC 020 Gap 11: an approval may have resolved in the event log
        // *after* the run transitioned to `WaitingApproval` but *before*
        // cairn-app picked up the resolution (operator clicked approve
        // during the crash window). Resolve the next-state transition here
        // so the orchestrator can act on it next tick. If no approvals or
        // all still pending, emit advisory events only.
        let approval_is_pending =
            ApprovalReadModel::has_pending_for_run(self.store.as_ref(), &run.run_id).await?;

        if approval_is_pending {
            return Ok(plan_unchanged(
                run,
                boot_id_str,
                latest_checkpoint,
                "approval still pending; waiting for operator",
            ));
        }

        // No pending approval but the run is still `WaitingApproval` — look
        // at the most recent resolved approval for this run and derive the
        // follow-up transition. We look up approvals project-wide and cache
        // the full, paginated result so a boot with many WaitingApproval
        // runs in the same project hits the store once, not once per run.
        //
        // Pagination: `ApprovalReadModel::list_all` is page-based and does
        // not expose a by-run filter. A fixed 500-row cap would silently
        // drop recovery on any project with more than 500 approvals in its
        // history — the resolved approval we need might be on page 2+ and
        // the run would be stranded in WaitingApproval. Instead, scan
        // forward in `APPROVAL_PAGE_SIZE`-row pages up to a hard cap that
        // is high enough to cover realistic recovery windows without
        // risking unbounded memory. Boots with more than this many
        // resolved approvals total are a real operational problem and
        // surface via `RecoveryAttempted.reason` rather than silently
        // skipping the advance.
        if !resolved_approvals_by_project.contains_key(&run.project) {
            let fetched = fetch_all_approvals(self.store.as_ref(), &run.project).await?;
            resolved_approvals_by_project.insert(run.project.clone(), fetched);
        }
        let all_approvals = resolved_approvals_by_project
            .get(&run.project)
            .expect("just inserted");
        let latest_resolved = all_approvals
            .iter()
            .filter(|a| a.run_id.as_ref() == Some(&run.run_id) && a.decision.is_some())
            .max_by_key(|a| a.updated_at)
            .cloned();

        // Audit-trail correctness (Cursor low-1): emit the
        // RecoveryAttempted reason based on what we actually found, not on
        // what we expected to find. If the "no pending approval" branch
        // was a stale projection and no resolved record exists, the reason
        // says so explicitly instead of claiming an advance that didn't
        // happen.
        let attempted_reason = match &latest_resolved {
            Some(approval) => format!(
                "approval {} resolved during crash window; advancing run state",
                approval.approval_id,
            ),
            None => "no pending approvals remain but no resolved approval found; \
                     leaving run for operator"
                .to_owned(),
        };

        let mut events = Vec::with_capacity(3);
        events.push(make_envelope(RuntimeEvent::RecoveryAttempted(
            RecoveryAttempted {
                project: run.project.clone(),
                run_id: Some(run.run_id.clone()),
                task_id: None,
                reason: attempted_reason,
                boot_id: Some(boot_id_str.clone()),
            },
        )));

        let mut advanced = false;
        if let Some(approval) = latest_resolved {
            // Derive the target state exactly as `ApprovalServiceImpl::resolve`
            // does (keeping the recovery path byte-compatible with the normal
            // resolution path).
            let (to_state, failure_class, resume_trigger) =
                match approval.decision.expect("filtered for Some(decision)") {
                    ApprovalDecision::Approved => {
                        (RunState::Running, None, Some(ResumeTrigger::OperatorResume))
                    }
                    ApprovalDecision::Rejected => {
                        (RunState::Failed, Some(FailureClass::ApprovalRejected), None)
                    }
                };
            if cairn_domain::can_transition_run_state(run.state, to_state) {
                events.push(make_envelope(RuntimeEvent::RunStateChanged(
                    RunStateChanged {
                        project: run.project.clone(),
                        run_id: run.run_id.clone(),
                        transition: StateTransition {
                            from: Some(run.state),
                            to: to_state,
                        },
                        failure_class,
                        pause_reason: None,
                        resume_trigger,
                    },
                )));
                advanced = true;
            }
        }

        events.push(make_envelope(RuntimeEvent::RecoveryCompleted(
            RecoveryCompleted {
                project: run.project.clone(),
                run_id: Some(run.run_id.clone()),
                task_id: None,
                recovered: true,
                boot_id: Some(boot_id_str),
            },
        )));

        Ok(RunRecoveryPlan {
            events,
            outcome: if advanced {
                RunRecoveryOutcome::Advanced {
                    run_id: run.run_id.clone(),
                }
            } else {
                RunRecoveryOutcome::Recovered {
                    run_id: run.run_id.clone(),
                }
            },
        })
    }
}

/// Build the "no state change, just emit advisory markers" plan. The
/// orchestrator is the source of truth for what to do on resume — recovery
/// here only promises that the event log records which cairn-app boot saw
/// the run.
fn plan_unchanged(
    run: &RunRecord,
    boot_id_str: String,
    _latest_checkpoint: Option<CheckpointRecord>,
    reason: &str,
) -> RunRecoveryPlan {
    let events = vec![
        make_envelope(RuntimeEvent::RecoveryAttempted(RecoveryAttempted {
            project: run.project.clone(),
            run_id: Some(run.run_id.clone()),
            task_id: None,
            reason: reason.to_owned(),
            boot_id: Some(boot_id_str.clone()),
        })),
        make_envelope(RuntimeEvent::RecoveryCompleted(RecoveryCompleted {
            project: run.project.clone(),
            run_id: Some(run.run_id.clone()),
            task_id: None,
            recovered: true,
            boot_id: Some(boot_id_str),
        })),
    ];
    RunRecoveryPlan {
        events,
        outcome: RunRecoveryOutcome::Recovered {
            run_id: run.run_id.clone(),
        },
    }
}

fn current_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

/// Paginate through every approval for a project up to
/// [`APPROVAL_SCAN_LIMIT`]. Returns accumulated records in the order the
/// projection yields them. Used by `plan_waiting_approval` to find a run's
/// resolved approval without issuing one store call per run (the obvious
/// N+1) and without silently dropping approvals past page 1 (the failure
/// mode of a fixed 500-row cap).
async fn fetch_all_approvals<S>(
    store: &S,
    project: &cairn_domain::ProjectKey,
) -> Result<Vec<cairn_store::projections::ApprovalRecord>, RuntimeError>
where
    S: ApprovalReadModel + ?Sized,
{
    let mut out = Vec::new();
    let mut offset = 0usize;
    loop {
        if out.len() >= APPROVAL_SCAN_LIMIT {
            break;
        }
        let remaining = APPROVAL_SCAN_LIMIT - out.len();
        let page_limit = APPROVAL_PAGE_SIZE.min(remaining);
        let page = ApprovalReadModel::list_all(store, project, page_limit, offset).await?;
        let page_len = page.len();
        out.extend(page);
        if page_len < page_limit {
            // Short page → end of history; no need to issue a final empty
            // round-trip just to confirm.
            break;
        }
        offset += page_len;
    }
    Ok(out)
}
