//! RFC 020 Track 1 — run-level recovery on cairn-app startup.
//!
//! Owned responsibility (post commit `5fefc76` ownership split): run-level
//! state recovery. FF's 14 background scanners own operational recovery
//! (lease expiry, attempt timeouts, dependency reconciliation, …) and run
//! continuously, so this service does **not** touch any of that.
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
    ApprovalDecision, BootId, FailureClass, ResumeTrigger, RunRecoveryOutcome, RunRecoverySummary,
    RunState, RunStateChanged, RuntimeEvent, StateTransition,
};
use cairn_domain::{RecoveryAttempted, RecoveryCompleted};
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
#[async_trait]
pub trait RecoveryService: Send + Sync {
    /// Sweep every non-terminal run and emit the appropriate recovery events.
    ///
    /// Called once per cairn-app boot, *after* `SandboxService::recover_all`
    /// and *before* the readiness gate flips to `200`. `boot_id` is threaded
    /// into every emitted event for audit-trail correlation. On error,
    /// startup MUST halt — cairn-app is not a durable system if it serves
    /// traffic with unknown run state.
    async fn recover_all(&self, boot_id: &BootId) -> Result<RunRecoverySummary, RuntimeError>;
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
    async fn recover_all(&self, boot_id: &BootId) -> Result<RunRecoverySummary, RuntimeError> {
        let mut summary = RunRecoverySummary {
            boot_id: Some(boot_id.as_str().to_owned()),
            ..Default::default()
        };

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

        summary.scanned_runs = runs.len() as u32;
        let now_ms = current_unix_ms();

        for run in runs {
            let events = self.plan_for_run(&run, boot_id, now_ms).await?;
            let outcome = events.outcome;
            let to_append = events.events;
            if !to_append.is_empty() {
                self.store.append(&to_append).await?;
            }
            match &outcome {
                RunRecoveryOutcome::Recovered { .. } => summary.recovered_runs += 1,
                RunRecoveryOutcome::Advanced { .. } => summary.advanced_runs += 1,
                RunRecoveryOutcome::Failed { .. } => summary.failed_runs += 1,
            }
            summary.outcomes.push(outcome);
        }

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
                .plan_waiting_approval(run, boot_id_str, latest_checkpoint)
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
        // follow-up transition. Bounded scan to avoid loading the whole
        // approval history; recent approvals land last in the `list_all`
        // result, but the projection isn't guaranteed to order by decision
        // time so we sort.
        let project = run.project.clone();
        let all_approvals =
            ApprovalReadModel::list_all(self.store.as_ref(), &project, 500, 0).await?;
        let latest_resolved = all_approvals
            .into_iter()
            .filter(|a| a.run_id.as_ref() == Some(&run.run_id) && a.decision.is_some())
            .max_by_key(|a| a.updated_at);

        let mut events = Vec::with_capacity(3);
        events.push(make_envelope(RuntimeEvent::RecoveryAttempted(
            RecoveryAttempted {
                project: run.project.clone(),
                run_id: Some(run.run_id.clone()),
                task_id: None,
                reason: "approval resolved during crash window; advancing run state".to_owned(),
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
